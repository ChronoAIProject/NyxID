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
) -> AppResult<bool> {
    let Some(mut row) = db
        .collection::<Document>("invite_codes")
        .find_one(doc! {"_id": id, "usages": {"$elemMatch": {"user_id": owner, "email": {"$exists": true}}}})
        .session(&mut *tx)
        .await?
    else {
        return Ok(false);
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
    Ok(changed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplyOutcome {
    Handled,
    Noop,
    Skipped,
}

#[cfg(test)]
async fn apply_group(
    db: &mongodb::Database,
    group: &OrphanGroup,
    lease: &Document,
) -> AppResult<ApplyOutcome> {
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result = apply_group_in_session(db, group, lease, &mut tx).await;
    super::org_membership_authority::finish(tx, result).await
}

async fn apply_group_in_session(
    db: &mongodb::Database,
    group: &OrphanGroup,
    lease: &Document,
    tx: &mut mongodb::ClientSession,
) -> AppResult<ApplyOutcome> {
    // A page supplies at most APPLY_BATCH ids, and normal apply uses one
    // reviewed owner/row at a time within the page's single transaction.
    if group.row_ids.len() > APPLY_BATCH {
        return Err(AppError::Internal(
            "Orphan mutation page exceeds its bound".into(),
        ));
    }
    let mut filter = doc! {"_id": {"$in": &group.row_ids}};
    // Bind both identity and the reviewed parent/owner relationship.
    if group.collection == "channel_send_claims" && group.field == "_id" {
        filter = doc! {"$and":[filter, {"_id":{"$regex":format!("^{}:", regex::escape(&group.owner_id))}}]};
    } else {
        filter = doc! {"$and":[filter, {group.field: &group.owner_id}]};
    }
    if db
        .collection::<Document>(jobs::JOBS)
        .update_one(lease.clone(), doc! {"$inc": {"mutation_revision": 1_i64}})
        .session(&mut *tx)
        .await?
        .matched_count
        != 1
    {
        return Err(AppError::Conflict("Orphan sweep lease expired".into()));
    }
    if db
        .collection::<Document>(group.collection)
        .find_one(filter.clone())
        .projection(doc! {"_id":1})
        .session(&mut *tx)
        .await?
        .is_none()
    {
        return Ok(ApplyOutcome::Noop);
    }
    if let Some(parent) = group.joined_parent {
        if db
            .collection::<Document>(parent)
            .find_one(doc! {"_id": &group.owner_id})
            .session(&mut *tx)
            .await?
            .is_some()
        {
            return Ok(ApplyOutcome::Skipped);
        }
        db.collection::<Document>(group.collection)
            .delete_many(filter)
            .session(&mut *tx)
            .await?;
        return Ok(ApplyOutcome::Handled);
    }
    for (collection, key) in [
        ("users", "_id"),
        ("service_accounts", "_id"),
        ("personal_erasure_jobs", "user_id"),
    ] {
        if db
            .collection::<Document>(collection)
            .find_one(doc! {key: &group.owner_id})
            .session(&mut *tx)
            .await?
            .is_some()
        {
            return Ok(ApplyOutcome::Skipped);
        }
    }
    super::erasure_admission::close_in_session(db, &group.owner_id, &mut *tx).await?;
    db.collection::<Document>(super::erasure_admission::COLLECTION)
        .update_one(
            doc! {"_id": &group.owner_id, "erasure_accepted": true},
            doc! {"$inc": {"cleanup_revision": 1_i64}},
        )
        .session(&mut *tx)
        .await?;
    let changed = if let Some(spec) =
        personal_data_registry::attribution(group.collection, group.field)
    {
        db.collection::<Document>(group.collection).update_many(
            doc! {"$and": [filter, super::personal_attribution_service::pending_filter(spec, Some(&group.owner_id))]},
            super::personal_attribution_service::update(spec),
        ).session(&mut *tx).await?.modified_count > 0
    } else if group.collection == crate::models::assistant_agent::COLLECTION_NAME
        && matches!(group.field, "org_user_ids" | "org_maintainer_user_ids")
    {
        db.collection::<Document>(group.collection)
            .update_many(
                doc! {"$and": [filter, super::org_agent_service::member_list_filter(&group.owner_id)]},
                super::org_agent_service::prune_member_update(&group.owner_id),
            )
            .session(&mut *tx)
            .await?
            .modified_count > 0
    } else if group.collection == "invite_codes" {
        if group.field == "created_by" {
            db.collection::<Document>("invite_codes")
                .update_many(filter, doc! {"$unset": {"note": ""}})
                .session(&mut *tx)
                .await?
                .modified_count
                > 0
        } else {
            let mut changed = false;
            for id in &group.row_ids {
                changed |= redact_invite_row(db, id, &group.owner_id, &mut *tx).await?;
            }
            changed
        }
    } else if matches!(
        group.disposition,
        Erasure::LocalInventory | Erasure::Redacted
    ) {
        if !super::personal_erasure_service::local::remote_cleanup_clear(
            db,
            &group.owner_id,
            &mut *tx,
        )
        .await?
        {
            return Ok(ApplyOutcome::Skipped);
        }
        let mut predicates: Vec<Document> =
            super::personal_erasure_service::local::inventory(&group.owner_id)
                .into_iter()
                .filter(|(collection, _)| *collection == group.collection)
                .map(|(_, filter)| filter)
                .collect();
        if group.collection == "service_change_events" && group.field == "owner_id" {
            predicates.push(doc! {"owner_id": &group.owner_id});
        }
        if predicates.is_empty() {
            return Ok(ApplyOutcome::Skipped);
        }
        if db
            .collection::<Document>(group.collection)
            .delete_many(doc! {"$and": [filter, {"$or": predicates}]})
            .session(&mut *tx)
            .await?
            .deleted_count
            == 0
        {
            return Ok(ApplyOutcome::Skipped);
        }
        true
    } else {
        return Ok(ApplyOutcome::Skipped);
    };
    Ok(if changed {
        ApplyOutcome::Handled
    } else {
        ApplyOutcome::Noop
    })
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
            report.candidate_count
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
        let specialists =
            db.collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME);
        specialists.insert_one(doc! {"_id":"kept-specialist","kind":"specialist","user_id":&org,
            "org_user_ids":[&orphan,&person],"org_maintainer_user_ids":[&orphan],"org_access_revision":7_i64})
            .await.unwrap();
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
        let specialist = specialists
            .find_one(doc! {"_id":"kept-specialist"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            specialist.get_array("org_user_ids").unwrap(),
            &[Bson::String(person.clone())]
        );
        assert!(
            specialist
                .get_array("org_maintainer_user_ids")
                .unwrap()
                .is_empty()
        );
        assert_eq!(specialist.get_i64("org_access_revision").unwrap(), 8);
        assert_eq!(specialist.get_i64("org_use_revision").unwrap(), 1);
        assert_eq!(specialist.get_i64("org_maintain_revision").unwrap(), 1);
        assert!(
            !scan(&db)
                .await
                .unwrap()
                .collections
                .iter()
                .any(|c| c.collection == "assistant_agents" && c.candidate_count > 0)
        );
        let second = apply(&db, &person, &report.digest).await.unwrap();
        assert_eq!(second.candidate_count, report.candidate_count);
        assert!(db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME).find_one(doc! {"event_type": "user.account.erasure_orphan_sweep", "event_data.digest": &report.digest}).await.unwrap().is_some());
        db.drop().await.unwrap();
    }
}
