//! Best-effort occupied-slot diagnostics, isolated from reservation writers.
use crate::{errors::AppResult, models::assistant_cma_sandbox::AssistantCmaSandbox};
use mongodb::{
    Database, IndexModel,
    bson::{Document, doc},
    options::IndexOptions,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

pub(crate) const COLLECTION: &str = "assistant_cma_slot_audit_claims";
const WINDOW: Duration = Duration::from_secs(3600);
const MAX_LOCAL_CLAIMS: usize = 65_536;

#[derive(Default)]
pub(super) struct ReplicaCache(Mutex<HashMap<String, Instant>>);
impl ReplicaCache {
    fn claim(&self, key: String) -> bool {
        let now = Instant::now();
        let mut claims = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if claims
            .get(&key)
            .is_some_and(|at| now.duration_since(*at) < WINDOW)
        {
            return false;
        }
        if claims.len() >= MAX_LOCAL_CLAIMS {
            claims.retain(|_, at| now.duration_since(*at) < WINDOW);
            // Diagnostics must not make a retrying client unbounded load.
            if claims.len() >= MAX_LOCAL_CLAIMS {
                return false;
            }
        }
        claims.insert(key, now);
        true
    }
}
static CACHE: LazyLock<ReplicaCache> = LazyLock::new(ReplicaCache::default);

pub(crate) fn indexes() -> Vec<IndexModel> {
    vec![
        IndexModel::builder()
            .keys(doc! {"expires_at":1})
            .options(
                IndexOptions::builder()
                    .name("cma_slot_audit_expiry".to_owned())
                    .expire_after(Duration::ZERO)
                    .build(),
            )
            .build(),
    ]
}
fn identity(owner: &str, reservation: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"nyxid:cma-slot-audit:v1\0");
    hash.update(owner.as_bytes());
    hash.update(b"\0");
    hash.update(reservation.as_bytes());
    hex::encode(hash.finalize())
}
pub(super) async fn record(db: &Database, row: &AssistantCmaSandbox) -> AppResult<()> {
    record_with_cache(db, row, &CACHE).await
}
pub(super) async fn record_with_cache(
    db: &Database,
    row: &AssistantCmaSandbox,
    cache: &ReplicaCache,
) -> AppResult<()> {
    let id = identity(&row.user_id, &row.id);
    if !cache.claim(format!("{}:{id}", db.name())) {
        return Ok(());
    }
    let context = (db.clone(), id, row.clone());
    let result = super::transaction_service::run_db_only(db, &context, |tx, (db, id, row)| {
        Box::pin(async move {
            // Only this coordination record and the audit chain are written.
            // In particular no owner fence, reservation or creation is touched.
            let claimed = db.collection::<Document>(COLLECTION).update_one(doc! {
                "_id":id
            },vec![doc! {"$set":{
                "owner_id":{"$literal":&row.user_id},"reservation_id":{"$literal":&row.id},
                "expires_at":{"$cond":[
                    {"$lte":[{"$ifNull":["$expires_at",mongodb::bson::DateTime::from_millis(0)]},"$$NOW"]},
                    {"$add":["$$NOW",3_600_000_i64]},"$expires_at"
                ]}
            }}]).upsert(true).session(&mut *tx).await?;
            // An active lease is an exact no-op; the server's expiry decision
            // therefore needs no extra read or client-clock comparison.
            if claimed.modified_count == 0 && claimed.upserted_id.is_none() {
                return Ok(());
            }
            super::audit_service::log_actor_event_in_transaction(db,tx,&super::audit_service::AuditActor {
                user_id:row.user_id.clone(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None,
            },"cma_sandbox_slot_occupied",serde_json::json!({
                "owner_id":row.user_id,"reservation_id":row.id,
                "holding":super::assistant_cma_sandbox_service::occupied_holding(row),"cma_service_id":row.cma_service_id,
            })).await
        })
    }).await;
    // Concurrent first inserts may collide with the same _id. The losing
    // transaction aborts without an audit; the winner owns the hourly lease.
    match result {
        Err(crate::errors::AppError::DatabaseError(ref error)) if duplicate(error) => Ok(()),
        other => other,
    }
}
fn duplicate(error: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    matches!(error.kind.as_ref(), ErrorKind::Write(WriteFailure::WriteError(e)) if e.code == 11000)
        || matches!(error.kind.as_ref(), ErrorKind::Command(e) if e.code == 11000)
}

/// Only the exceptional unique-slot insertion race lacks a loaded winner.
/// Bound that diagnostic lookup before reading; steady occupied admission
/// always passes its already-loaded reservation to record().
pub(super) async fn record_race(db: &Database, owner: &str) -> AppResult<()> {
    if !CACHE.claim(format!("{}:race:{owner}", db.name())) {
        return Ok(());
    }
    if let Some(row) = db
        .collection::<AssistantCmaSandbox>(crate::models::assistant_cma_sandbox::COLLECTION_NAME)
        .find_one(doc! {"user_id":owner,"occupies_account_slot":true})
        .await?
    {
        record(db, &row).await?;
    }
    Ok(())
}
