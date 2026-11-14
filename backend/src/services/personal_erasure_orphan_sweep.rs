//! Operator controlled cleanup for rows left by account deletions that
//! predate durable personal-erasure jobs. The registry supplies the owner
//! predicate; this module never guesses ownership from attribution fields.

use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use uuid::Uuid;

use crate::{
    AppState,
    errors::{AppError, AppResult},
    services::personal_data_registry::{self, Erasure, OwnerSelector},
};

const AGGREGATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const APPLY_BATCH: usize = 64;
const OWNER_ID_REPORT_CAP: usize = 128;

#[derive(Clone, Debug, Serialize)]
pub struct OrphanCollectionReport {
    pub collection: String,
    pub candidate_count: usize,
    pub owner_ids: Vec<String>,
    pub owner_ids_truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
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

#[derive(Serialize)]
struct DigestGroup<'a> {
    collection: &'a str,
    field: &'a str,
    owner_id: &'a str,
    row_count: usize,
    row_ids_sha256: String,
}

fn owner_fields(entry: &personal_data_registry::Entry) -> Option<&'static [&'static str]> {
    match entry.owner {
        OwnerSelector::Fields(fields) if !fields.is_empty() => Some(fields),
        OwnerSelector::Fields(_) | OwnerSelector::NotPersonal => None,
    }
}

#[cfg(test)]
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

fn owner_pipeline(field: &str) -> Vec<Document> {
    let mut owner_match = Document::new();
    owner_match.insert(field, doc! {"$exists": true});
    let mut pipeline = vec![doc! {"$match": owner_match}];
    let mut path = String::new();
    let segments: Vec<&str> = field.split('.').collect();
    // Unwind every segment, including the terminal value.  Several redacted
    // records (invite usages, actor attribution) store owner ids in arrays;
    // grouping an array as `_id` would skip those owners entirely.
    for segment in segments {
        if !path.is_empty() {
            path.push('.');
        }
        path.push_str(segment);
        pipeline.push(doc! {"$unwind": format!("${path}")});
    }
    pipeline.push(doc! {
        "$group": {
            "_id": format!("${field}"),
            "row_ids": {"$addToSet": "$_id"},
            "row_count": {"$sum": 1_i32},
        }
    });
    pipeline
}

async fn aggregate_groups(
    db: &mongodb::Database,
    collection: &'static str,
    field: &'static str,
    disposition: Erasure,
) -> AppResult<Vec<OrphanGroup>> {
    let mut cursor = db
        .collection::<Document>(collection)
        .aggregate(owner_pipeline(field))
        .allow_disk_use(true)
        .max_time(AGGREGATE_TIMEOUT)
        .await?;
    let mut groups = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        let Ok(owner_id) = row.get_str("_id") else {
            continue;
        };
        if Uuid::parse_str(owner_id).is_err() {
            continue;
        }
        let row_ids = row.get_array("row_ids").cloned().unwrap_or_default();
        if row_ids.is_empty() {
            continue;
        }
        groups.push(OrphanGroup {
            collection,
            field,
            owner_id: owner_id.to_owned(),
            row_ids,
            disposition,
            joined_parent: None,
        });
    }
    Ok(groups)
}

/// Find joined children whose authoritative parent no longer exists. These
/// rows are safe to sweep even when the parent was an organization resource;
/// the orphan condition is structural, not an owner-liveness guess.
async fn aggregate_joined_children(
    db: &mongodb::Database,
    collection: &'static str,
    parent: &'static str,
    field: &'static str,
) -> AppResult<Vec<OrphanGroup>> {
    let mut cursor = db
        .collection::<Document>(collection)
        .aggregate(owner_pipeline(field))
        .allow_disk_use(true)
        .max_time(AGGREGATE_TIMEOUT)
        .await?;
    let mut groups = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        let Ok(parent_id) = row.get_str("_id") else {
            continue;
        };
        if Uuid::parse_str(parent_id).is_err() {
            continue;
        }
        let parent_exists = db
            .collection::<Document>(parent)
            .find_one(doc! {"_id": parent_id})
            .projection(doc! {"_id": 1})
            .max_time(AGGREGATE_TIMEOUT)
            .await?
            .is_some();
        if parent_exists {
            continue;
        }
        let row_ids = row.get_array("row_ids").cloned().unwrap_or_default();
        if !row_ids.is_empty() {
            groups.push(OrphanGroup {
                collection,
                field,
                owner_id: parent_id.to_owned(),
                row_ids,
                disposition: Erasure::JoinedChild { parent },
                joined_parent: Some(parent),
            });
        }
    }
    Ok(groups)
}

async fn live_owner_ids(
    db: &mongodb::Database,
    groups: &[OrphanGroup],
) -> AppResult<HashSet<String>> {
    let owners: Vec<String> = groups
        .iter()
        .map(|group| group.owner_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if owners.is_empty() {
        return Ok(HashSet::new());
    }
    let mut live = HashSet::new();
    for collection in [
        crate::models::user::COLLECTION_NAME,
        crate::models::service_account::COLLECTION_NAME,
    ] {
        let mut cursor = db
            .collection::<Document>(collection)
            .find(doc! {"_id": {"$in": &owners}})
            .projection(doc! {"_id": 1})
            .batch_size(256)
            .max_time(AGGREGATE_TIMEOUT)
            .await?;
        while let Some(row) = cursor.try_next().await? {
            if let Ok(id) = row.get_str("_id") {
                live.insert(id.to_owned());
            }
        }
    }
    let mut jobs = db
        .collection::<Document>(crate::models::personal_erasure::COLLECTION_NAME)
        .find(doc! {"user_id": {"$in": &owners}})
        .projection(doc! {"user_id": 1})
        .batch_size(256)
        .max_time(AGGREGATE_TIMEOUT)
        .await?;
    while let Some(row) = jobs.try_next().await? {
        if let Ok(id) = row.get_str("user_id") {
            live.insert(id.to_owned());
        }
    }
    Ok(live)
}

async fn collect_groups(db: &mongodb::Database) -> AppResult<Vec<OrphanGroup>> {
    let mut all = Vec::new();
    for entry in personal_data_registry::entries() {
        if let Erasure::JoinedChild { parent } = entry.erasure {
            if let Some((field, parent_collection)) = joined_child_spec(entry.collection, parent) {
                all.extend(
                    aggregate_joined_children(db, entry.collection, parent_collection, field)
                        .await?,
                );
            }
            continue;
        }
        if !matches!(entry.erasure, Erasure::LocalInventory | Erasure::Redacted) {
            continue;
        }
        let Some(fields) = owner_fields(entry) else {
            continue;
        };
        for field in fields {
            all.extend(aggregate_groups(db, entry.collection, field, entry.erasure).await?);
        }
    }
    let live = live_owner_ids(db, &all).await?;
    all.retain(|group| !live.contains(&group.owner_id));
    all.sort_by(|left, right| {
        (&left.collection, &left.field, &left.owner_id).cmp(&(
            &right.collection,
            &right.field,
            &right.owner_id,
        ))
    });
    Ok(all)
}

fn joined_child_spec(collection: &str, parent: &str) -> Option<(&'static str, &'static str)> {
    match (collection, parent) {
        ("assistant_agent_learning_rejections", "assistant_agents") => {
            Some(("agent_id", "assistant_agents"))
        }
        ("channel_event_logs", "channel_conversations") => {
            Some(("conversation_id", "channel_conversations"))
        }
        ("channel_send_claims", "channel_conversations") => {
            // Claims use `<conversation_id>:<claim_id>` ids rather than a
            // dedicated parent field; they are handled by the owning domain.
            None
        }
        ("service_history_heads", "user_services") => Some(("_id", "user_services")),
        ("service_pool_health_observations", "service_pools") => Some(("pool_id", "service_pools")),
        _ => None,
    }
}

fn row_id_hash(row_ids: &[Bson]) -> AppResult<String> {
    // BSON serialization requires a document root. Wrapping each scalar in a
    // fixed key gives every BSON type a canonical, length-delimited encoding.
    let mut values = row_ids
        .iter()
        .map(|value| {
            bson::to_vec(&doc! {"value": value})
                .map_err(|_| AppError::Internal("Orphan row id encoding failed".into()))
        })
        .collect::<AppResult<Vec<_>>>()?;
    values.sort();
    let mut encoded = Vec::new();
    for bytes in values {
        encoded.extend((bytes.len() as u64).to_be_bytes());
        encoded.extend(bytes);
    }
    Ok(hex::encode(Sha256::digest(encoded)))
}

fn digest(groups: &[OrphanGroup]) -> AppResult<String> {
    let entries: Vec<DigestGroup<'_>> = groups
        .iter()
        .map(|group| {
            Ok(DigestGroup {
                collection: group.collection,
                field: group.field,
                owner_id: &group.owner_id,
                row_count: group.row_ids.len(),
                row_ids_sha256: row_id_hash(&group.row_ids)?,
            })
        })
        .collect::<AppResult<_>>()?;
    let bytes = serde_json::to_vec(&entries)
        .map_err(|_| AppError::Internal("Orphan digest encoding failed".into()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn reports(groups: &[OrphanGroup]) -> (Vec<OrphanCollectionReport>, usize) {
    let mut grouped = BTreeMap::<String, OrphanCollectionReport>::new();
    let mut count = 0;
    for group in groups {
        count += group.row_ids.len();
        let report = grouped
            .entry(group.collection.to_owned())
            .or_insert_with(|| OrphanCollectionReport {
                collection: group.collection.to_owned(),
                candidate_count: 0,
                owner_ids: Vec::new(),
                owner_ids_truncated: false,
            });
        report.candidate_count += group.row_ids.len();
        if report.owner_ids.len() < OWNER_ID_REPORT_CAP
            && !report.owner_ids.contains(&group.owner_id)
        {
            report.owner_ids.push(group.owner_id.clone());
        } else if !report.owner_ids.contains(&group.owner_id) {
            report.owner_ids_truncated = true;
        }
    }
    (grouped.into_values().collect(), count)
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

pub async fn scan(db: &mongodb::Database) -> AppResult<OrphanSweepReport> {
    let groups = collect_groups(db).await?;
    let (collections, candidate_count) = reports(&groups);
    Ok(OrphanSweepReport {
        dry_run: true,
        digest: digest(&groups)?,
        collections,
        candidate_count,
    })
}

async fn redact_invite_row(db: &mongodb::Database, id: &Bson, owner: &str) -> AppResult<()> {
    let Some(mut row) = db
        .collection::<Document>("invite_codes")
        .find_one(doc! {"_id": id, "usages.user_id": owner})
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
                doc! {"_id": id, "usages.user_id": owner, "usages.email": {"$exists": true}},
                row,
            )
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
            Erasure::Redacted
                if group.collection == "invite_codes" && group.field == "usages.user_id" =>
            {
                for id in chunk.iter() {
                    redact_invite_row(db, id, &group.owner_id).await?;
                }
            }
            Erasure::Redacted
                if group.collection == "service_change_events"
                    && group.field == "actor.person_id" =>
            {
                db.collection::<Document>(group.collection)
                    .update_many(
                        filter,
                        doc! {
                            "$set": {
                                "actor.name": "Deleted account",
                                "actor.display_name": "Deleted account",
                            },
                            "$unset": {
                                "actor.email": "",
                                "actor.email_address": "",
                            },
                        },
                    )
                    .await?;
            }
            Erasure::Redacted => {
                let mut set = Document::new();
                set.insert(group.field, Bson::Null);
                db.collection::<Document>(group.collection)
                    .update_many(filter, doc! {"$set": set})
                    .await?;
            }
            _ => {}
        }
    }
    Ok(())
}

pub async fn apply(
    db: &mongodb::Database,
    actor: &str,
    expected_digest: &str,
) -> AppResult<OrphanSweepReport> {
    let groups = collect_groups(db).await?;
    let actual = digest(&groups)?;
    if actual != expected_digest {
        if groups.is_empty() && was_applied(db, expected_digest).await? {
            return Ok(OrphanSweepReport {
                dry_run: false,
                digest: expected_digest.to_owned(),
                collections: Vec::new(),
                candidate_count: 0,
            });
        }
        return Err(AppError::Conflict(
            "Orphan candidate set changed; run a new dry-run".into(),
        ));
    }
    for group in &groups {
        // Recheck liveness immediately before each bounded mutation. A newly
        // recreated account or service account must never be swept.
        let live = live_owner_ids(db, std::slice::from_ref(group)).await?;
        if live.contains(&group.owner_id) {
            continue;
        }
        apply_group(db, group).await?;
    }
    let (collections, candidate_count) = reports(&groups);
    crate::services::audit_service::log_actor_event(
        db.clone(),
        &crate::services::audit_service::AuditActor {
            user_id: actor.to_owned(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        "user.account.erasure_orphan_sweep",
        Some(serde_json::json!({
            "digest": expected_digest,
            "candidate_count": candidate_count,
            "collections": collections.iter().map(|item| {
                serde_json::json!({"collection": item.collection, "candidate_count": item.candidate_count})
            }).collect::<Vec<_>>(),
        })),
    )
    .await
    .map_err(|_| AppError::Internal("Orphan sweep audit append failed".into()))?;
    Ok(OrphanSweepReport {
        dry_run: false,
        digest: actual,
        collections,
        candidate_count,
    })
}

pub async fn run(
    state: &AppState,
    actor: &str,
    apply_requested: bool,
    digest: Option<&str>,
) -> AppResult<OrphanSweepReport> {
    if !apply_requested {
        return scan(&state.db).await;
    }
    let digest = digest
        .ok_or_else(|| AppError::ValidationError("Apply requires the dry-run digest".into()))?;
    apply(&state.db, actor, digest).await
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
    fn digest_binds_collection_field_owner_count_and_sorted_row_hash() {
        let one = vec![OrphanGroup {
            collection: "one",
            field: "owner_id",
            owner_id: Uuid::new_v4().to_string(),
            row_ids: vec![Bson::String("b".into()), Bson::String("a".into())],
            disposition: Erasure::LocalInventory,
            joined_parent: None,
        }];
        let mut two = one.clone();
        two[0].field = "other_owner";
        assert_ne!(digest(&one).unwrap(), digest(&two).unwrap());
        let mut reordered = one.clone();
        reordered[0].row_ids.reverse();
        assert_eq!(digest(&one).unwrap(), digest(&reordered).unwrap());
    }

    #[test]
    fn sentinel_and_non_uuid_owners_are_not_candidates() {
        assert!(Uuid::parse_str("system").is_err());
        assert!(Uuid::parse_str("00000000-0000-0000-0000-000000000001").is_ok());
    }

    #[test]
    fn report_caps_owner_ids_without_changing_exact_counts() {
        let owner = Uuid::new_v4().to_string();
        let groups = (0..OWNER_ID_REPORT_CAP + 1)
            .map(|index| OrphanGroup {
                collection: "rows",
                field: "owner_id",
                owner_id: if index == 0 {
                    owner.clone()
                } else {
                    Uuid::new_v4().to_string()
                },
                row_ids: vec![Bson::String(index.to_string())],
                disposition: Erasure::LocalInventory,
                joined_parent: None,
            })
            .collect::<Vec<_>>();
        let (reports, count) = reports(&groups);
        assert_eq!(count, OWNER_ID_REPORT_CAP + 1);
        assert_eq!(reports[0].owner_ids.len(), OWNER_ID_REPORT_CAP);
        assert!(reports[0].owner_ids_truncated);
    }
}
