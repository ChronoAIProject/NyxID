//! Operator controlled cleanup for rows left by account deletions that
//! predate durable personal-erasure jobs. The registry supplies the owner
//! predicate; this module never guesses ownership from attribution fields.

use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    services::personal_data_registry::{self, Erasure, OwnerSelector},
};

// Production collections are scanned through owner indexes.  This bound allows
// a large indexed pass to finish while still preventing an unbounded request.
const AGGREGATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const APPLY_BATCH: usize = 64;
const OWNER_ID_REPORT_CAP: usize = 128;

fn candidate_owner_id(value: &str) -> bool {
    Uuid::parse_str(value).is_ok()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrphanCollectionReport {
    pub collection: String,
    pub candidate_count: usize,
    pub owner_ids: Vec<String>,
    pub owner_ids_truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrphanSweepReport {
    pub dry_run: bool,
    pub digest: String,
    pub collections: Vec<OrphanCollectionReport>,
    pub candidate_count: usize,
}

#[derive(Clone, Debug)]
struct OrphanGroup {
    collection: &'static str,
    field: &'static str,
    owner_id: String,
    row_ids: Vec<Bson>,
    disposition: Erasure,
    /// Joined-child rows have no owner field. `owner_id` is the parent id and
    /// this marker tells apply to remove the row only because its parent is
    /// gone, never because an account id looked dead.
    joined_parent: Option<&'static str>,
}

fn owner_fields(entry: &personal_data_registry::Entry) -> Option<&'static [&'static str]> {
    match entry.owner {
        OwnerSelector::Fields(fields) if !fields.is_empty() => Some(fields),
        OwnerSelector::Fields(_) | OwnerSelector::NotPersonal => None,
    }
}

fn owner_values(row: &Document, field: &str) -> Vec<String> {
    fn collect(value: &Bson, segments: &[&str], output: &mut Vec<String>) {
        if segments.is_empty() {
            match value {
                Bson::String(value) => output.push(value.clone()),
                Bson::Array(values) => {
                    for value in values {
                        collect(value, &[], output);
                    }
                }
                _ => {}
            }
            return;
        }
        match value {
            Bson::Document(document) => {
                if let Some(next) = document.get(segments[0]) {
                    collect(next, &segments[1..], output);
                }
            }
            Bson::Array(values) => {
                for value in values {
                    collect(value, segments, output);
                }
            }
            _ => {}
        }
    }

    let mut values = Vec::new();
    collect(
        &Bson::Document(row.clone()),
        &field.split('.').collect::<Vec<_>>(),
        &mut values,
    );
    values.sort();
    values.dedup();
    values
}

fn joined_child_spec(collection: &str, parent: &str) -> Option<(&'static str, &'static str)> {
    personal_data_registry::JOINED_CHILD_SPECS
        .iter()
        .find(|spec| spec.child == collection && spec.parent == parent)
        .map(|spec| (spec.child_parent_field, spec.parent))
}

async fn was_applied(db: &mongodb::Database, digest: &str) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {
            "event_type": "user.account.erasure_orphan_sweep",
            "event_data.digest": digest,
        })
        .projection(doc! {"_id": 1})
        .max_time(AGGREGATE_TIMEOUT)
        .await?
        .is_some())
}

pub mod jobs;

#[cfg(test)]
async fn scan(db: &mongodb::Database) -> AppResult<OrphanSweepReport> {
    let job = jobs::start(db, "test-operator", false, None).await?;
    jobs::finish(db, &job.job_id).await
}

async fn redact_invite_row(
    db: &mongodb::Database,
    id: &Bson,
    owner: &str,
    tx: &mut mongodb::ClientSession,
) -> AppResult<()> {
    let Some(mut row) = db
        .collection::<Document>("invite_codes")
        .find_one(doc! {"_id": id, "usages": {"$elemMatch": {"user_id": owner, "email": {"$exists": true}}}})
        .session(&mut *tx)
        .await?
    else {
        return Ok(());
    };
    let usages = row.get_array("usages").cloned().unwrap_or_default();
    let mut changed = false;
    let usages = usages
        .into_iter()
        .map(|value| {
            let Some(mut usage) = value.as_document().cloned() else {
                return value;
            };
            if usage.get_str("user_id").ok() == Some(owner) && usage.contains_key("email") {
                usage.remove("email");
                changed = true;
            }
            Bson::Document(usage)
        })
        .collect::<Vec<_>>();
    if changed {
        row.insert("usages", usages);
        db.collection::<Document>("invite_codes")
            // Match an unredacted usage as well as the id.  A second apply
            // must not rewrite an already-redacted shared invite row.
            .replace_one(
                doc! {"_id": id, "usages": {"$elemMatch": {"user_id": owner, "email": {"$exists": true}}}},
                row,
            )
            .session(tx)
            .await?;
    }
    Ok(())
}

async fn apply_group(db: &mongodb::Database, group: &OrphanGroup) -> AppResult<()> {
    let mut ids = group.row_ids.clone();
    for chunk in ids.chunks_mut(APPLY_BATCH) {
        let mut filter = Document::new();
        filter.insert("_id", doc! {"$in": chunk.to_vec()});
        if group.joined_parent.is_none() {
            filter.insert(group.field, group.owner_id.as_str());
        }
        let attribution = personal_data_registry::attribution(group.collection, group.field);
        let invite = group.collection == "invite_codes" && group.field == "usages.user_id";
        if attribution.is_some() || invite {
            let mut tx = db.client().start_session().await?;
            tx.start_transaction()
                .read_concern(mongodb::options::ReadConcern::snapshot())
                .write_concern(mongodb::options::WriteConcern::majority())
                .await?;
            let result: AppResult<()> = async {
                // Pre-job deletions have no job tombstone. Close the same
                // durable admission fence before changing this bounded page.
                for collection in [crate::models::user::COLLECTION_NAME, crate::models::service_account::COLLECTION_NAME] {
                    if db.collection::<Document>(collection).find_one(doc! {"_id": &group.owner_id})
                        .session(&mut tx).await?.is_some() { return Ok(()); }
                }
                super::erasure_admission::close_in_session(db, &group.owner_id, &mut tx).await?;
                if let Some(spec) = attribution {
                    db.collection::<Document>(group.collection).update_many(
                        doc! {"$and": [filter, super::personal_attribution_service::pending_filter(spec, Some(&group.owner_id))]},
                        super::personal_attribution_service::update(spec),
                    ).session(&mut tx).await?;
                } else {
                    for id in chunk.iter() {
                        redact_invite_row(db, id, &group.owner_id, &mut tx).await?;
                    }
                }
                Ok(())
            }.await;
            super::org_membership_authority::finish(tx, result).await?;
            continue;
        }
        match group.disposition {
            Erasure::JoinedChild { .. } => {
                db.collection::<Document>(group.collection)
                    .delete_many(filter)
                    .await?;
            }
            Erasure::LocalInventory => {
                db.collection::<Document>(group.collection)
                    .delete_many(filter)
                    .await?;
            }
            Erasure::Redacted
                if group.collection == "service_change_events" && group.field == "owner_id" =>
            {
                db.collection::<Document>(group.collection)
                    .delete_many(filter)
                    .await?;
            }
            Erasure::Redacted => {
                // Hybrid entries erase only the real resource-owner predicate;
                // attribution selectors above can only redact human labels.
                let owned: Vec<Document> =
                    super::personal_erasure_service::local::inventory(&group.owner_id)
                        .into_iter()
                        .filter(|(collection, _)| *collection == group.collection)
                        .map(|(_, filter)| filter)
                        .collect();
                if owned.is_empty() {
                    return Err(AppError::Internal(
                        "Orphan redaction predicate is not registered".into(),
                    ));
                }
                db.collection::<Document>(group.collection)
                    .delete_many(doc! {"$and": [filter, {"$or": owned}]})
                    .await?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
async fn apply(db: &mongodb::Database, actor: &str, digest: &str) -> AppResult<OrphanSweepReport> {
    let job = jobs::start(db, actor, true, Some(digest)).await?;
    jobs::finish(db, &job.job_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_values_supports_nested_and_array_predicates() {
        let row = doc! {
            "owner_id": "org-1",
            "actor": {"person_id": "person-1"},
            "target_user_ids": ["person-1", "person-2"],
            "usages": [
                {"user_id": "person-1", "email": "removed@example.test"},
                {"user_id": "person-2"},
            ],
        };
        assert_eq!(owner_values(&row, "owner_id"), ["org-1"]);
        assert_eq!(owner_values(&row, "actor.person_id"), ["person-1"]);
        assert_eq!(
            owner_values(&row, "target_user_ids"),
            ["person-1", "person-2"]
        );
        assert_eq!(
            owner_values(&row, "usages.user_id"),
            ["person-1", "person-2"]
        );
    }

    #[test]
    fn every_registry_joined_child_has_an_orphan_sweep_spec() {
        for entry in personal_data_registry::entries() {
            if let Erasure::JoinedChild { parent } = entry.erasure {
                assert!(
                    joined_child_spec(entry.collection, parent).is_some(),
                    "joined child {} -> {} is missing from the orphan sweep",
                    entry.collection,
                    parent
                );
            }
        }
    }

    #[test]
    fn sentinel_and_non_uuid_owners_are_not_candidates() {
        assert!(!candidate_owner_id("system"));
        assert!(!candidate_owner_id("service-account"));
        assert!(candidate_owner_id("00000000-0000-0000-0000-000000000001"));
    }

    #[tokio::test]
    async fn sweep_redacts_all_attribution_shapes_preserves_live_actors_and_global_identity_scan_is_clean()
     {
        use super::super::personal_attribution_service::test_support as attribution;
        let db = crate::test_utils::connect_transaction_test_database("orphan_attribution").await;
        let orphan = Uuid::new_v4().to_string();
        let person = Uuid::new_v4().to_string();
        let org = Uuid::new_v4().to_string();
        let service_account = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_many([
                crate::test_utils::test_user(&person, crate::models::user::UserType::Person),
                crate::test_utils::test_user(&org, crate::models::user::UserType::Org),
            ])
            .await
            .unwrap();
        db.collection::<Document>(crate::models::service_account::COLLECTION_NAME)
            .insert_one(doc! {"_id": &service_account})
            .await
            .unwrap();
        let cases = attribution::seed(&db, &orphan, &org, APPLY_BATCH + 1).await;
        db.collection::<Document>("invite_codes").insert_one(doc! {
            "_id": "orphan-invite", "used_count": 2, "max_uses": 5,
            "usages": [{"user_id": &orphan, "email": attribution::EMAIL}, {"user_id": &person, "email": "live@example.test"}],
        }).await.unwrap();
        db.collection::<Document>("assistant_agent_shares")
            .insert_one(doc! {
                "_id": "orphan-share", "owner_id": &org, "subject_kind": "person",
                "subject_id": &orphan, "subject_email": attribution::EMAIL,
            })
            .await
            .unwrap();
        super::super::audit_service::log_actor_event(
            db.clone(),
            &super::super::audit_service::AuditActor {
                user_id: orphan.clone(),
                ip_address: None,
                user_agent: None,
                api_key_id: None,
                api_key_name: None,
            },
            "attribution.test.immutable",
            Some(serde_json::json!({"name": attribution::NAME, "email": attribution::EMAIL})),
        )
        .await
        .unwrap();
        let audit = db
            .collection::<Document>("audit_log")
            .find_one(doc! {"event_type": "attribution.test.immutable"})
            .await
            .unwrap()
            .unwrap();
        let ledger = doc! {"_id": "immutable-ledger", "owner_id": &orphan, "entry_hash": "immutable-test-hash"};
        db.collection::<Document>("billing_ledger")
            .insert_one(ledger.clone())
            .await
            .unwrap();
        assert!(cases.iter().all(|case| {
            serde_json::to_string(&case.before)
                .unwrap()
                .contains(attribution::NAME)
                || serde_json::to_string(&case.before)
                    .unwrap()
                    .contains(attribution::EMAIL)
        }));
        let mut live_rows = Vec::new();
        for (index, spec) in personal_data_registry::ATTRIBUTIONS.iter().enumerate() {
            for live in [&person, &org, &service_account] {
                let id = format!("live-attribution-{index}-{live}");
                let mut row = doc! {"_id": &id, "user_id": &org, "owner_id": &org};
                attribution::put(&mut row, spec.person_field, live.as_str().into());
                for field in spec.name_fields {
                    attribution::put(&mut row, field, "Live actor".into());
                }
                for field in spec.email_fields {
                    attribution::put(&mut row, field, "live@example.test".into());
                }
                db.collection::<Document>(spec.collection)
                    .insert_one(row.clone())
                    .await
                    .unwrap();
                live_rows.push((spec.collection, id, row));
            }
        }
        let report = scan(&db).await.unwrap();
        assert!(report.candidate_count >= cases.len());
        apply(&db, &person, &report.digest).await.unwrap();
        attribution::assert_cases(&db, &cases, false).await;
        attribution::assert_no_identity_outside_audit(&db).await;
        assert_eq!(
            db.collection::<Document>("audit_log")
                .find_one(doc! {"_id": audit.get("_id").unwrap()})
                .await
                .unwrap(),
            Some(audit)
        );
        assert_eq!(
            db.collection::<Document>("billing_ledger")
                .find_one(doc! {"_id": "immutable-ledger"})
                .await
                .unwrap(),
            Some(ledger)
        );
        let invite = db
            .collection::<Document>("invite_codes")
            .find_one(doc! {"_id": "orphan-invite"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(invite.get_i32("used_count"), Ok(2));
        assert_eq!(invite.get_i32("max_uses"), Ok(5));
        let usages = invite.get_array("usages").unwrap();
        assert_eq!(
            usages[0].as_document().unwrap().get_str("user_id"),
            Ok(orphan.as_str())
        );
        assert!(!usages[0].as_document().unwrap().contains_key("email"));
        assert_eq!(
            usages[1].as_document().unwrap().get_str("email"),
            Ok("live@example.test")
        );
        for (collection, id, expected) in live_rows {
            assert_eq!(
                db.collection::<Document>(collection)
                    .find_one(doc! {"_id": id})
                    .await
                    .unwrap(),
                Some(expected)
            );
        }
        assert!(
            db.collection::<Document>(super::super::erasure_admission::COLLECTION)
                .find_one(doc! {"_id": &orphan, "erasure_accepted": true})
                .await
                .unwrap()
                .is_some()
        );
        for live in [&person, &org, &service_account] {
            assert!(
                db.collection::<Document>(super::super::erasure_admission::COLLECTION)
                    .find_one(doc! {"_id": live, "erasure_accepted": true})
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        assert_eq!(scan(&db).await.unwrap().candidate_count, 0);
        assert_eq!(
            apply(&db, &person, &report.digest)
                .await
                .unwrap()
                .candidate_count,
            0
        );
    }

    #[tokio::test]
    async fn sweep_preserves_live_person_org_and_service_account_and_is_idempotent() {
        let db =
            crate::test_utils::connect_transaction_test_database("orphan_sweep_behavior").await;
        let person = Uuid::new_v4().to_string();
        let org = Uuid::new_v4().to_string();
        let service_account = Uuid::new_v4().to_string();
        let orphan = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_many([
                crate::test_utils::test_user(&person, crate::models::user::UserType::Person),
                crate::test_utils::test_user(&org, crate::models::user::UserType::Org),
            ])
            .await
            .unwrap();
        db.collection::<Document>(crate::models::service_account::COLLECTION_NAME)
            .insert_one(doc! {"_id": &service_account})
            .await
            .unwrap();
        db.collection::<Document>("assistant_wire_logs")
            .insert_many([
                doc! {"_id": "live-person", "user_id": &person},
                doc! {"_id": "live-org", "user_id": &org},
                doc! {"_id": "live-service", "user_id": &service_account},
                doc! {"_id": "orphan", "user_id": &orphan},
            ])
            .await
            .unwrap();
        let orphan_child = Uuid::new_v4().to_string();
        db.collection::<Document>("service_history_heads")
            .insert_one(doc! {"_id": &orphan_child, "fixture_marker": "orphan-child"})
            .await
            .unwrap();
        let report = scan(&db).await.unwrap();
        let wire = report
            .collections
            .iter()
            .find(|item| item.collection == "assistant_wire_logs")
            .expect("wire log candidate report");
        assert_eq!(wire.candidate_count, 1);
        assert_eq!(wire.owner_ids, vec![orphan.clone()]);
        assert!(
            report
                .collections
                .iter()
                .any(|item| item.collection == "service_history_heads")
        );
        assert!(matches!(
            apply(&db, &person, "wrong-digest").await,
            Err(AppError::Conflict(_))
        ));
        let applied = apply(&db, &person, &report.digest).await.unwrap();
        assert_eq!(applied.candidate_count, report.candidate_count);
        for id in ["live-person", "live-org", "live-service"] {
            assert!(
                db.collection::<Document>("assistant_wire_logs")
                    .find_one(doc! {"_id": id})
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        assert!(
            db.collection::<Document>("assistant_wire_logs")
                .find_one(doc! {"_id": "orphan"})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            db.collection::<Document>("service_history_heads")
                .find_one(doc! {"_id": &orphan_child})
                .await
                .unwrap()
                .is_none()
        );
        let second = apply(&db, &person, &report.digest).await.unwrap();
        assert_eq!(second.candidate_count, 0);
        assert!(db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME).find_one(doc! {"event_type": "user.account.erasure_orphan_sweep", "event_data.digest": &report.digest}).await.unwrap().is_some());
        db.drop().await.unwrap();
    }
}
