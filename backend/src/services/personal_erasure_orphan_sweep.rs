//! Operator controlled cleanup for rows left by account deletions that
//! predate durable personal-erasure jobs. The registry supplies the owner
//! predicate; this module never guesses ownership from attribution fields.

use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{AppState, errors::{AppError, AppResult}, services::personal_data_registry::{self, Erasure, OwnerSelector}};

const PAGE: i64 = 64;

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

fn owner_field(entry: &personal_data_registry::Entry) -> Option<&'static str> {
    match entry.owner {
        OwnerSelector::Fields(fields) if fields.len() == 1 => Some(fields[0]),
        _ => None,
    }
}

async fn owner_is_live(db: &mongodb::Database, owner: &str) -> AppResult<bool> {
    if db.collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": owner}).projection(doc! {"_id": 1}).await?.is_some() {
        return Ok(true);
    }
    Ok(db.collection::<Document>(crate::models::service_account::COLLECTION_NAME)
        .find_one(doc! {"_id": owner}).projection(doc! {"_id": 1}).await?.is_some())
}

async fn collect_candidates(db: &mongodb::Database) -> AppResult<Vec<(OrphanCandidate, Bson, &'static str, Erasure)>> {
    let mut out = Vec::new();
    for entry in personal_data_registry::entries() {
        if !matches!(entry.erasure, Erasure::LocalInventory | Erasure::Redacted) {
            continue;
        }
        let Some(field) = owner_field(entry) else { continue };
        let mut filter = Document::new();
        filter.insert(field, doc! {"$exists": true});
        let mut projection = doc! {"_id": 1};
        projection.insert(field, 1);
        let mut cursor = db.collection::<Document>(entry.collection)
            .find(filter)
            .projection(projection)
            .limit(PAGE)
            .await?;
        while let Some(row) = cursor.try_next().await? {
            let Some(owner_value) = row.get(field) else { continue };
            let owners: Vec<&str> = match owner_value {
                Bson::String(value) => vec![value.as_str()],
                Bson::Array(values) => values.iter().filter_map(Bson::as_str).collect(),
                _ => continue,
            };
            let id = row.get("_id").cloned().ok_or_else(|| AppError::Internal("Orphan row is missing _id".into()))?;
            for owner in owners {
                if owner.is_empty() || owner_is_live(db, owner).await? { continue; }
                out.push((OrphanCandidate { collection: entry.collection.to_owned(), id: id.to_string(), owner_id: owner.to_owned() }, id.clone(), field, entry.erasure));
            }
        }
    }
    out.sort_by(|left, right| (&left.0.collection, &left.0.id, &left.0.owner_id).cmp(&(&right.0.collection, &right.0.id, &right.0.owner_id)));
    Ok(out)
}

fn digest(candidates: &[OrphanCandidate]) -> AppResult<String> {
    let bytes = serde_json::to_vec(candidates).map_err(|_| AppError::Internal("Orphan digest encoding failed".into()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub async fn scan(db: &mongodb::Database) -> AppResult<OrphanSweepReport> {
    let candidates = collect_candidates(db).await?;
    let public: Vec<OrphanCandidate> = candidates.iter().map(|item| item.0.clone()).collect();
    let mut grouped = std::collections::BTreeMap::<String, OrphanCollectionReport>::new();
    for candidate in &public {
        let item = grouped.entry(candidate.collection.clone()).or_insert_with(|| OrphanCollectionReport { collection: candidate.collection.clone(), candidate_count: 0, owner_ids: Vec::new() });
        item.candidate_count += 1;
        if !item.owner_ids.contains(&candidate.owner_id) { item.owner_ids.push(candidate.owner_id.clone()); }
    }
    Ok(OrphanSweepReport { dry_run: true, digest: digest(&public)?, candidate_count: public.len(), collections: grouped.into_values().collect() })
}

pub async fn apply(db: &mongodb::Database, actor: &str, expected_digest: &str) -> AppResult<OrphanSweepReport> {
    let candidates = collect_candidates(db).await?;
    let public: Vec<OrphanCandidate> = candidates.iter().map(|item| item.0.clone()).collect();
    let actual = digest(&public)?;
    if actual != expected_digest {
        return Err(AppError::Conflict("Orphan candidate set changed; run a new dry-run".into()));
    }
    let mut grouped = std::collections::BTreeMap::<String, OrphanCollectionReport>::new();
    for candidate in &public {
        let item = grouped.entry(candidate.collection.clone()).or_insert_with(|| OrphanCollectionReport { collection: candidate.collection.clone(), candidate_count: 0, owner_ids: Vec::new() });
        item.candidate_count += 1;
        if !item.owner_ids.contains(&candidate.owner_id) { item.owner_ids.push(candidate.owner_id.clone()); }
    }
    for (candidate, id, field, disposition) in candidates {
        let mut filter = doc! {"_id": id};
        filter.insert(field, candidate.owner_id.as_str());
        match disposition {
            Erasure::Redacted => {
                let update = if candidate.collection == "service_change_events" || candidate.collection == "service_history_heads" {
                    doc! {"$set": {"owner_id": Bson::Null, "actor.name": "Deleted account", "actor.person_id": Bson::Null, "actor.api_key_id": Bson::Null, "actor.app_id": Bson::Null}}
                } else {
                    doc! {"$set": {field: Bson::Null}}
                };
                db.collection::<Document>(&candidate.collection).update_one(filter, update).await?;
            }
            Erasure::LocalInventory => {
                db.collection::<Document>(&candidate.collection).delete_one(filter).await?;
            }
            _ => {}
        }
    }
    crate::services::audit_service::log_actor_event(
        db.clone(),
        &crate::services::audit_service::AuditActor { user_id: actor.to_owned(), ip_address: None, user_agent: None, api_key_id: None, api_key_name: None },
        "user.account.erasure_orphan_sweep",
        Some(serde_json::json!({"digest": expected_digest, "candidate_count": public.len(), "collections": grouped.values().map(|item| (&item.collection, item.candidate_count)).collect::<Vec<_>>() })),
    ).await.map_err(|_| AppError::Internal("Orphan sweep audit append failed".into()))?;
    Ok(OrphanSweepReport { dry_run: false, digest: actual, candidate_count: public.len(), collections: grouped.into_values().collect() })
}

pub async fn run(state: &AppState, actor: &str, apply_requested: bool, digest: Option<&str>) -> AppResult<OrphanSweepReport> {
    if !apply_requested {
        return scan(&state.db).await;
    }
    let digest = digest.ok_or_else(|| AppError::ValidationError("Apply requires the dry-run digest".into()))?;
    apply(&state.db, actor, digest).await
}

