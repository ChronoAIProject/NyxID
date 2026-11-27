//! Platform approval for the exact catalog destination receiving a caller bearer.
use std::collections::{BTreeMap, BTreeSet};

use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{COLLECTION_NAME, DownstreamService};
use crate::services::audit_service::{self, AuditActor};

pub(crate) const FIELD: &str = "caller_token_approved_destinations";
const EVENT: &str = "catalog_caller_token_approval_changed";
const MIGRATION: &str = "catalog-caller-token-base-approval-v1";

#[derive(Clone, Deserialize, serde::Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRequest {
    pub target_id: Option<String>,
    pub destination_url: String,
    pub approved: bool,
}

fn fingerprint(id: &str, target: Option<&str>, url: &str) -> String {
    let binding = serde_json::to_vec(&(id, target, url)).expect("strings serialize to JSON");
    hex::encode(Sha256::digest(binding))
}

pub(crate) fn approved(row: &DownstreamService, target: Option<&str>, url: &str) -> bool {
    let live = match target {
        None => Some(&row.base_url),
        Some(id) => row.destination_targets.get(id),
    };
    row.forward_access_token
        && live.is_some_and(|live| live == url)
        && row
            .caller_token_approved_destinations
            .contains(&fingerprint(&row.id, target, url))
}

/// The transaction fences the previous destinations and approval set together.
pub(crate) fn prepare_update(
    row: &DownstreamService,
    fields: &mut Document,
    request: Option<&ApprovalRequest>,
) -> AppResult<()> {
    if request.is_none()
        && !fields.contains_key("base_url")
        && !fields.contains_key("destination_targets")
    {
        return Ok(());
    }
    let base = fields.get_str("base_url").unwrap_or(&row.base_url);
    let replacements: Option<BTreeMap<String, String>> = fields
        .get("destination_targets")
        .cloned()
        .map(bson::from_bson)
        .transpose()
        .map_err(|_| AppError::ValidationError("Invalid destination targets".into()))?;
    let targets = replacements.as_ref().unwrap_or(&row.destination_targets);
    let mut next = row.caller_token_approved_destinations.clone();
    if base != row.base_url {
        next.remove(&fingerprint(&row.id, None, &row.base_url));
    }
    for (id, old_url) in &row.destination_targets {
        if targets.get(id) != Some(old_url) {
            next.remove(&fingerprint(&row.id, Some(id), old_url));
        }
    }
    if let Some(request) = request {
        let selector = request.target_id.as_deref();
        let actual = match selector {
            None => Some(base),
            Some(id) => targets.get(id).map(String::as_str),
        };
        if actual != Some(request.destination_url.as_str()) {
            return Err(AppError::Conflict(
                "The selected catalog destination changed; review its current URL".into(),
            ));
        }
        let binding = fingerprint(&row.id, selector, &request.destination_url);
        if request.approved {
            next.insert(binding);
        } else {
            next.remove(&binding);
        }
    }
    if request.is_some() || next != row.caller_token_approved_destinations {
        fields.insert(
            FIELD,
            bson::to_bson(&next).map_err(|e| AppError::Internal(e.to_string()))?,
        );
    }
    Ok(())
}

pub(crate) async fn audit_change(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
    actor: Option<&AuditActor>,
    before: &DownstreamService,
    after: &BTreeSet<String>,
) -> AppResult<()> {
    let previous = &before.caller_token_approved_destinations;
    if previous == after {
        return Ok(());
    }
    let data = serde_json::json!({
        "service_id": before.id,
        "added_destination_digests": after.difference(previous).collect::<Vec<_>>(),
        "removed_destination_digests": previous.difference(after).collect::<Vec<_>>(),
    });
    match actor {
        Some(actor) => {
            audit_service::log_actor_event_in_session(db, session, key, actor, EVENT, data).await
        }
        None => audit_service::log_system_event_in_session(db, session, key, EVENT, data).await,
    }
}

/// Seed reconciliation can also move a previously approved catalog URL.
pub(crate) async fn move_seed_destinations(
    db: &Database,
    filter: Document,
    fields: Document,
) -> AppResult<u64> {
    let rows = db.collection::<DownstreamService>(COLLECTION_NAME);
    if rows.count_documents(filter.clone()).await? == 0 {
        return Ok(0);
    }
    super::transaction_service::run_db_only(
        db,
        &(db, filter, fields),
        |session, (db, filter, fields)| {
            Box::pin(move_seed_destinations_in_session(
                db, session, filter, fields,
            ))
        },
    )
    .await
}

async fn move_seed_destinations_in_session(
    db: &Database,
    session: &mut ClientSession,
    filter: &Document,
    fields: &Document,
) -> AppResult<u64> {
    let rows = db.collection::<DownstreamService>(COLLECTION_NAME);
    let mut cursor = rows.find(filter.clone()).session(&mut *session).await?;
    let mut modified = 0;
    while let Some(before) = cursor.next(&mut *session).await.transpose()? {
        let mut set = fields.clone();
        prepare_update(&before, &mut set, None)?;
        let after = rows
            .find_one_and_update(doc! {"_id": &before.id}, doc! {"$set":set})
            .return_document(mongodb::options::ReturnDocument::After)
            .session(&mut *session)
            .await?
            .ok_or_else(|| {
                AppError::Conflict("Catalog destination changed during seed reconciliation".into())
            })?;
        if before.caller_token_approved_destinations != after.caller_token_approved_destinations {
            let key = audit_service::audit_chain_hmac_key().ok_or_else(|| {
                AppError::Internal("Catalog destination audit key unavailable".into())
            })?;
            audit_change(
                db,
                session,
                key,
                None,
                &before,
                &after.caller_token_approved_destinations,
            )
            .await?;
        }
        modified += 1;
    }
    Ok(modified)
}

/// Capture eligible base destinations and the completion marker in one snapshot.
/// Subsequent startups cannot approve newly enabled rows or restore a revoked approval.
pub(crate) async fn bootstrap(db: &Database, key: &[u8]) -> AppResult<Vec<(String, String)>> {
    let db = super::transaction_service::durable_database(db);
    let markers = db.collection::<Document>("schema_migrations");
    let marker = doc! {"_id":MIGRATION};
    if markers.find_one(marker.clone()).await?.is_some() {
        return Ok(Vec::new());
    }
    let result =
        super::transaction_service::run_db_only(&db, &(&db, key), |session, &(db, key)| {
            Box::pin(approve_existing_rows(db, session, key))
        })
        .await;
    match result {
        Err(_) if markers.find_one(marker).await?.is_some() => Ok(Vec::new()),
        result => result,
    }
}

async fn approve_existing_rows(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
) -> AppResult<Vec<(String, String)>> {
    let claimed = db
        .collection::<Document>("schema_migrations")
        .update_one(
            doc! {"_id": MIGRATION},
            doc! {"$setOnInsert": {"completed_at": bson::DateTime::now()}},
        )
        .upsert(true)
        .session(&mut *session)
        .await?;
    if claimed.upserted_id.is_none() {
        return Ok(Vec::new());
    }

    let rows = db.collection::<DownstreamService>(COLLECTION_NAME);
    let eligible = doc! {
        "is_active": true,
        "forward_access_token": true,
        "catalog_manager_created": {"$ne": true},
        "tools_manager_created": {"$ne": true},
    };
    let mut cursor = rows.find(eligible).session(&mut *session).await?;
    let mut changed = Vec::new();
    while let Some(row) = cursor.next(&mut *session).await.transpose()? {
        let binding = fingerprint(&row.id, None, &row.base_url);
        let mut next = row.caller_token_approved_destinations.clone();
        if next.insert(binding.clone()) {
            rows.update_one(doc! {"_id": &row.id}, doc! {"$addToSet": {FIELD: binding}})
                .session(&mut *session)
                .await?;
            audit_change(db, session, key, None, &row, &next).await?;
            changed.push((row.id, row.slug));
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::test_helpers::dummy_service;
    use crate::models::user::{User, UserType};
    use crate::services::{audit_chain_service, role_service};
    use crate::test_utils::{
        connect_transaction_test_database, test_app_state, test_auth_user, test_user,
    };
    use axum::Json;
    use axum::extract::{Path, State};
    use futures::TryStreamExt;

    #[tokio::test]
    async fn caller_token_approval_bootstrap_is_once_and_excludes_manager_rows() {
        let db = connect_transaction_test_database("bearer_bootstrap").await;
        let services = db.collection::<DownstreamService>(COLLECTION_NAME);
        for (id, enabled, active, catalog_manager, tools_manager) in [
            ("aevatar", true, true, false, false),
            ("disabled", false, true, false, false),
            ("inactive", true, false, false, false),
            ("catalog-manager", true, true, true, false),
            ("tools-manager", true, true, false, true),
        ] {
            let mut row = dummy_service();
            row.id = id.into();
            row.slug = id.into();
            row.created_by = "former-admin".into();
            row.forward_access_token = enabled;
            row.is_active = active;
            row.catalog_manager_created = catalog_manager;
            row.tools_manager_created = tools_manager;
            services.insert_one(row).await.unwrap();
        }
        crate::test_utils::init_test_audit_chain_hmac_key();
        let key = audit_service::audit_chain_hmac_key().unwrap();
        let (a, b) = tokio::join!(bootstrap(&db, key), bootstrap(&db, key));
        assert_eq!(a.unwrap().len() + b.unwrap().len(), 1);
        let logs = db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME);
        assert_eq!(
            logs.count_documents(doc! {"event_type": EVENT})
                .await
                .unwrap(),
            1
        );
        let raw = db.collection::<Document>(COLLECTION_NAME);
        assert_eq!(
            raw.count_documents(doc! {FIELD: {"$exists": true}})
                .await
                .unwrap(),
            1
        );
        for url in ["https://changed.example.test", "http://localhost"] {
            assert_eq!(
                move_seed_destinations(&db, doc! {"_id": "aevatar"}, doc! {"base_url": url})
                    .await
                    .unwrap(),
                1
            );
        }
        assert!(bootstrap(&db, key).await.unwrap().is_empty());
        let restored = services
            .find_one(doc! {"_id":"aevatar"})
            .await
            .unwrap()
            .unwrap();
        assert!(restored.caller_token_approved_destinations.is_empty());
        assert_eq!(
            logs.count_documents(doc! {"event_type": EVENT})
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn caller_token_approval_requires_admin_and_binds_each_destination() {
        let db = connect_transaction_test_database("bearer_approval").await;
        role_service::seed_system_roles(&db).await.unwrap();
        let roles = role_service::get_platform_role_ids(&db).await.unwrap();
        let mut admin = test_user(&uuid::Uuid::new_v4().to_string(), UserType::Person);
        admin.role_ids = vec![roles.admin];
        let mut manager = test_user(&uuid::Uuid::new_v4().to_string(), UserType::Person);
        let catalog_manager = db
            .collection::<Document>("roles")
            .find_one(doc! {"slug":crate::models::role::CATALOG_MANAGER_ROLE_SLUG})
            .await
            .unwrap()
            .unwrap();
        manager.role_ids = vec![
            roles.operator,
            catalog_manager.get_str("_id").unwrap().into(),
        ];
        let mut catalog = dummy_service();
        catalog.created_by = manager.id.clone();
        catalog.forward_access_token = true;
        catalog.auth_method = "bearer".into();
        catalog.auth_key_name = "Authorization".into();
        catalog.requires_user_credential = true;
        catalog.proxy_operation_policy =
            Some(crate::models::downstream_service::ProxyOperationPolicy {
                rules: vec![crate::models::downstream_service::ProxyOperationRule {
                    method: "GET".into(),
                    path_template: "/docs".into(),
                    target_id: Some("docs".into()),
                    ..Default::default()
                }],
            });
        let target = "https://target.example.test";
        catalog
            .destination_targets
            .insert("docs".into(), target.into());
        db.collection::<User>("users")
            .insert_many([&admin, &manager])
            .await
            .unwrap();
        db.collection::<DownstreamService>(COLLECTION_NAME)
            .insert_one(&catalog)
            .await
            .unwrap();
        let state = test_app_state(db.clone());
        let approval = |approved: bool, url: &str| {
            serde_json::from_value(serde_json::json!({
                "approve_caller_token_destination": {
                    "target_id": "docs",
                    "destination_url": url,
                    "approved": approved,
                },
            }))
            .unwrap()
        };
        let update = |actor: &str, body| {
            crate::handlers::services::update_service(
                State(state.clone()),
                test_auth_user(actor),
                Default::default(),
                Path(catalog.id.clone()),
                Json(body),
            )
        };
        // The handler returns before service_updated commits. Finish that write
        // before another approval transaction reads this fixture's unindexed tail.
        let admin_update = async |body| {
            let written =
                audit_service::notify_on_audit_write_for_user("service_updated", &admin.id);
            let _ = update(&admin.id, body).await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(30), written)
                .await
                .expect("timed out waiting for service_updated to commit")
                .expect("service_updated audit watcher closed");
        };
        assert!(matches!(
            update(&manager.id, approval(true, target)).await,
            Err(AppError::Forbidden(_))
        ));
        let stale = update(&admin.id, approval(true, &catalog.base_url))
            .await
            .err()
            .unwrap();
        assert!(matches!(stale, AppError::Conflict(_)), "{stale}");
        admin_update(approval(true, target)).await;
        admin_update(approval(true, target)).await;
        let rows = db.collection::<DownstreamService>(COLLECTION_NAME);
        let policy = rows
            .find_one(doc! {"_id":&catalog.id})
            .await
            .unwrap()
            .unwrap();
        assert!(approved(&policy, Some("docs"), target));
        assert!(!approved(&policy, None, &catalog.base_url));
        assert!(!approved(&policy, Some("docs"), &catalog.base_url));
        admin_update(approval(false, target)).await;
        assert!(!approved(
            &rows
                .find_one(doc! {"_id":&catalog.id})
                .await
                .unwrap()
                .unwrap(),
            Some("docs"),
            target
        ));
        admin_update(approval(true, target)).await;
        for url in ["https://updated.example.test", target] {
            let change =
                serde_json::from_value(serde_json::json!({"destination_targets":{"docs":url}}))
                    .unwrap();
            admin_update(change).await;
            let current = rows
                .find_one(doc! {"_id":&catalog.id})
                .await
                .unwrap()
                .unwrap();
            assert!(!approved(&current, Some("docs"), url));
        }
        let logs = db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME);
        assert_eq!(
            logs.count_documents(doc! {"event_type": EVENT})
                .await
                .unwrap(),
            4
        );
        let audit = logs
            .find_one(doc! {"event_type": EVENT})
            .await
            .unwrap()
            .unwrap();
        assert!(audit.contains_key("entry_hash"));
        assert!(!format!("{:?}", audit.get_document("event_data").unwrap()).contains(target));
        let report = audit_chain_service::verify_chain(
            &db,
            state.audit_chain_hmac_key.as_slice(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let nearby_rows = async {
            let Some(broken) = &report.break_info else {
                return Ok(Vec::new());
            };
            let seq = broken.break_seq;
            logs.find(doc! {"seq": {"$gte": seq.saturating_sub(2), "$lte": seq.saturating_add(2)}})
                .projection(
                    doc! {"_id": 1, "seq": 1, "event_type": 1, "prev_hash": 1, "entry_hash": 1},
                )
                .sort(doc! {"seq": 1, "_id": 1})
                .limit(12)
                .await?
                .try_collect::<Vec<_>>()
                .await
        };
        assert_eq!(
            report.status,
            audit_chain_service::AuditChainStatus::Ok,
            "{report:?}\nNearby audit rows: {:?}",
            nearby_rows.await
        );
    }
}
