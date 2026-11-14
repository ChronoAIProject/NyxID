//! Operator controlled cleanup for rows left by account deletions that
//! predate durable personal-erasure jobs. The registry supplies the owner
//! predicate; this module never guesses ownership from attribution fields.

use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    services::personal_data_registry::{self, Erasure, OwnerSelector},
};

const PAGE: i64 = 64;
const APPLY_BATCH: usize = 64;

#[derive(Clone, Debug, Serialize)]
pub struct OrphanCandidate {
    pub collection: String,
    pub id: String,
    pub owner_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct OrphanCollectionReport {
    pub collection: String,
    pub candidate_count: usize,
    pub owner_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OrphanSweepReport {
    pub dry_run: bool,
    pub digest: String,
    pub collections: Vec<OrphanCollectionReport>,
    pub candidate_count: usize,
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
            // MongoDB dotted predicates traverse arrays of documents. Keep
            // the same semantics for nested owner selectors in the sweep.
            Bson::Array(values) => {
                for value in values {
                    collect(value, segments, output);
                }
            }
            _ => {}
        }
    }

    let mut values = Vec::new();
    let root = Bson::Document(row.clone());
    collect(&root, &field.split('.').collect::<Vec<_>>(), &mut values);
    values.sort();
    values.dedup();
    values
}

async fn owner_is_live(db: &mongodb::Database, owner: &str) -> AppResult<bool> {
    if db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": owner})
        .projection(doc! {"_id": 1})
        .await?
        .is_some()
    {
        return Ok(true);
    }
    Ok(db
        .collection::<Document>(crate::models::service_account::COLLECTION_NAME)
        .find_one(doc! {"_id": owner})
        .projection(doc! {"_id": 1})
        .await?
        .is_some())
}

async fn collect_candidates(
    db: &mongodb::Database,
) -> AppResult<Vec<(OrphanCandidate, Bson, &'static str, Erasure)>> {
    let mut out = Vec::new();
    for entry in personal_data_registry::entries() {
        if !matches!(entry.erasure, Erasure::LocalInventory | Erasure::Redacted) {
            continue;
        }
        let Some(fields) = owner_fields(entry) else {
            continue;
        };
        for field in fields {
            let mut filter = Document::new();
            filter.insert(*field, doc! {"$exists": true});
            let mut projection = doc! {"_id": 1};
            if *field == "usages.user_id" {
                projection.insert("usages", 1);
            } else {
                projection.insert(*field, 1);
            }
            let mut cursor = db
                .collection::<Document>(entry.collection)
                .find(filter)
                .projection(projection)
                // Cursor batches bound each server response while still walking
                // the complete collection for an exact dry-run digest.
                .batch_size(PAGE as u32)
                .await?;
            while let Some(row) = cursor.try_next().await? {
                let id = row
                    .get("_id")
                    .cloned()
                    .ok_or_else(|| AppError::Internal("Orphan row is missing _id".into()))?;
                for owner in owner_values(&row, field) {
                    if owner.is_empty() || owner_is_live(db, &owner).await? {
                        continue;
                    }
                    out.push((
                        OrphanCandidate {
                            collection: entry.collection.to_owned(),
                            id: id.to_string(),
                            owner_id: owner,
                        },
                        id.clone(),
                        *field,
                        entry.erasure,
                    ));
                }
            }
        }
    }
    out.sort_by(|left, right| {
        (&left.0.collection, &left.0.id, &left.0.owner_id).cmp(&(
            &right.0.collection,
            &right.0.id,
            &right.0.owner_id,
        ))
    });
    Ok(out)
}

fn digest(candidates: &[OrphanCandidate]) -> AppResult<String> {
    let bytes = serde_json::to_vec(candidates)
        .map_err(|_| AppError::Internal("Orphan digest encoding failed".into()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

async fn was_applied(db: &mongodb::Database, digest: &str) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {
            "event_type": "user.account.erasure_orphan_sweep",
            "event_data.digest": digest,
        })
        .projection(doc! {"_id": 1})
        .await?
        .is_some())
}

pub async fn scan(db: &mongodb::Database) -> AppResult<OrphanSweepReport> {
    let candidates = collect_candidates(db).await?;
    let public: Vec<OrphanCandidate> = candidates.iter().map(|item| item.0.clone()).collect();
    let mut grouped = std::collections::BTreeMap::<String, OrphanCollectionReport>::new();
    for candidate in &public {
        let item = grouped
            .entry(candidate.collection.clone())
            .or_insert_with(|| OrphanCollectionReport {
                collection: candidate.collection.clone(),
                candidate_count: 0,
                owner_ids: Vec::new(),
            });
        item.candidate_count += 1;
        if !item.owner_ids.contains(&candidate.owner_id) {
            item.owner_ids.push(candidate.owner_id.clone());
        }
    }
    Ok(OrphanSweepReport {
        dry_run: true,
        digest: digest(&public)?,
        candidate_count: public.len(),
        collections: grouped.into_values().collect(),
    })
}

pub async fn apply(
    db: &mongodb::Database,
    actor: &str,
    expected_digest: &str,
) -> AppResult<OrphanSweepReport> {
    let candidates = collect_candidates(db).await?;
    let public: Vec<OrphanCandidate> = candidates.iter().map(|item| item.0.clone()).collect();
    let actual = digest(&public)?;
    if actual != expected_digest {
        // A successful apply removes the candidate rows, so a second apply
        // with the reviewed digest sees an empty set. The chained event is
        // the durable idempotency marker for that exact reviewed set.
        if public.is_empty() && was_applied(db, expected_digest).await? {
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
    let mut grouped = std::collections::BTreeMap::<String, OrphanCollectionReport>::new();
    for candidate in &public {
        let item = grouped
            .entry(candidate.collection.clone())
            .or_insert_with(|| OrphanCollectionReport {
                collection: candidate.collection.clone(),
                candidate_count: 0,
                owner_ids: Vec::new(),
            });
        item.candidate_count += 1;
        if !item.owner_ids.contains(&candidate.owner_id) {
            item.owner_ids.push(candidate.owner_id.clone());
        }
    }
    for batch in candidates.chunks(APPLY_BATCH) {
        for (candidate, id, field, disposition) in batch {
            // The owner may have been recreated after the reviewed dry run.
            // Recheck immediately before each mutation so a live person,
            // organization user, or service account is always protected.
            if owner_is_live(db, &candidate.owner_id).await? {
                continue;
            }
            let mut filter = doc! {"_id": id.clone()};
            if field.ends_with("_ids") {
                filter.insert(*field, doc! {"$in": [candidate.owner_id.as_str()]});
            } else {
                filter.insert(*field, candidate.owner_id.as_str());
            }
            match disposition {
                Erasure::Redacted => {
                    if candidate.collection == "service_change_events" && *field == "owner_id" {
                        db.collection::<Document>(&candidate.collection)
                            .delete_one(filter)
                            .await?;
                        continue;
                    }
                    if candidate.collection == "invite_codes" && *field == "usages.user_id" {
                        db.collection::<Document>(&candidate.collection)
                            .update_one(
                                filter,
                                doc! {"$pull": {"usages": {"user_id": candidate.owner_id.as_str()}}},
                            )
                            .await?;
                        continue;
                    }
                    let update = if candidate.collection == "service_change_events"
                        && *field == "actor.person_id"
                    {
                        doc! {"$set": {"actor.name": "Deleted account", "actor.person_id": Bson::Null, "actor.api_key_id": Bson::Null, "actor.app_id": Bson::Null}}
                    } else {
                        let mut set = Document::new();
                        set.insert(*field, Bson::Null);
                        doc! {"$set": set}
                    };
                    db.collection::<Document>(&candidate.collection)
                        .update_one(filter, update)
                        .await?;
                }
                Erasure::LocalInventory => {
                    db.collection::<Document>(&candidate.collection)
                        .delete_one(filter)
                        .await?;
                }
                _ => {}
            }
        }
    }
    crate::services::audit_service::log_actor_event(
        db.clone(),
        &crate::services::audit_service::AuditActor { user_id: actor.to_owned(), ip_address: None, user_agent: None, api_key_id: None, api_key_name: None },
        "user.account.erasure_orphan_sweep",
        Some(serde_json::json!({"digest": expected_digest, "candidate_count": public.len(), "collections": grouped.values().map(|item| (&item.collection, item.candidate_count)).collect::<Vec<_>>() })),
    ).await.map_err(|_| AppError::Internal("Orphan sweep audit append failed".into()))?;
    Ok(OrphanSweepReport {
        dry_run: false,
        digest: actual,
        candidate_count: public.len(),
        collections: grouped.into_values().collect(),
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

        let nested_array = doc! {
            "actors": [
                {"person": {"id": "person-3"}},
                {"person": {"id": "person-4"}},
            ],
        };
        assert_eq!(
            owner_values(&nested_array, "actors.person.id"),
            ["person-3", "person-4"]
        );
    }

    #[test]
    fn digest_is_ordered_and_changes_with_the_exact_candidate_set() {
        let first = vec![OrphanCandidate {
            collection: "one".into(),
            id: "a".into(),
            owner_id: "u".into(),
        }];
        let second = vec![OrphanCandidate {
            collection: "two".into(),
            id: "a".into(),
            owner_id: "u".into(),
        }];
        assert_ne!(digest(&first).unwrap(), digest(&second).unwrap());
        assert_eq!(digest(&first).unwrap(), digest(&first).unwrap());
    }
}
