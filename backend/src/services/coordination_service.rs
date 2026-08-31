use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;

use mongodb::IndexModel;
use mongodb::bson::{self, Bson, DateTime, Document, doc};
use mongodb::options::{IndexOptions, ReturnDocument};
use sha2::{Digest, Sha256};

use crate::errors::{AppError, AppResult};
use crate::models::coordination::{
    CoordinationHolder, CoordinationLease, CoordinationSlot, EVENT_DEDUP_COLLECTION_NAME,
    EventDedupRecord, EventDedupState, LEASE_COLLECTION_NAME, RATE_WINDOW_COLLECTION_NAME,
    REPLAY_COLLECTION_NAME, RateWindowRecord, ReplayRecord, SLOT_COLLECTION_NAME,
    TOKEN_BUCKET_COLLECTION_NAME, TokenBucketRecord,
};

pub async fn ensure_indexes(db: &mongodb::Database) -> Result<(), mongodb::error::Error> {
    let leases = db.collection::<Document>(LEASE_COLLECTION_NAME);
    for index_name in [
        format!("{LEASE_COLLECTION_NAME}_expiry_ttl"),
        format!("{LEASE_COLLECTION_NAME}_ephemeral_expiry_ttl"),
    ] {
        // Concurrent startup drops are harmless; another replica may have
        // removed a superseded definition first.
        let _ = leases.drop_index(index_name).await;
    }
    leases
        .update_many(
            doc! {
                "record_kind": { "$exists": false },
                "checkpoint": { "$exists": true },
            },
            doc! { "$set": { "record_kind": "checkpoint" } },
        )
        .await?;
    leases
        .update_many(
            doc! { "record_kind": { "$exists": false } },
            doc! { "$set": { "record_kind": "ephemeral" } },
        )
        .await?;
    leases
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(
                    IndexOptions::builder()
                        .name(format!("{LEASE_COLLECTION_NAME}_ephemeral_expiry_ttl"))
                        .expire_after(Duration::from_secs(0))
                        .partial_filter_expression(doc! { "record_kind": "ephemeral" })
                        .build(),
                )
                .build(),
        )
        .await?;

    for collection_name in [
        REPLAY_COLLECTION_NAME,
        RATE_WINDOW_COLLECTION_NAME,
        TOKEN_BUCKET_COLLECTION_NAME,
        SLOT_COLLECTION_NAME,
        EVENT_DEDUP_COLLECTION_NAME,
    ] {
        db.collection::<Document>(collection_name)
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "expires_at": 1 })
                    .options(
                        IndexOptions::builder()
                            .name(format!("{collection_name}_expiry_ttl"))
                            .expire_after(Duration::from_secs(0))
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }

    db.collection::<Document>(REPLAY_COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "namespace": 1, "key_hash": 1 })
                .options(
                    IndexOptions::builder()
                        .name("coordination_replay_namespace_key_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(SLOT_COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "namespace": 1, "scope_hash": 1, "slot": 1 })
                .options(
                    IndexOptions::builder()
                        .name("coordination_slot_identity_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(EVENT_DEDUP_COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "namespace": 1, "scope_hash": 1, "event_hash": 1 })
                .options(
                    IndexOptions::builder()
                        .name("event_dedup_identity_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct LeaseToken {
    pub name: String,
    pub holder: CoordinationHolder,
    pub lease_id: String,
}

#[derive(Debug, Clone)]
pub struct ClusterLeaseRuntime {
    pub holder: CoordinationHolder,
    pub ttl: Duration,
    pub renew_interval: Duration,
}

impl ClusterLeaseRuntime {
    pub fn new(holder: CoordinationHolder, ttl: Duration, renew_interval: Duration) -> Self {
        Self {
            holder,
            ttl,
            renew_interval,
        }
    }

    pub async fn acquire(
        &self,
        db: &mongodb::Database,
        name: &str,
    ) -> AppResult<Option<LeaseToken>> {
        LeaseStore::acquire(db, name, &self.holder, self.ttl).await
    }

    pub fn contender_wait(&self) -> Duration {
        self.renew_interval / 10
    }

    /// Poll `operation` and lease renewal together. Returning `None` means
    /// ownership could no longer be proven and the operation was cancelled.
    pub async fn run_while_renewed<T, F>(
        &self,
        db: &mongodb::Database,
        token: &LeaseToken,
        operation: F,
    ) -> Option<T>
    where
        F: Future<Output = T>,
    {
        tokio::pin!(operation);
        let mut renewal = tokio::time::interval(self.renew_interval);
        renewal.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        renewal.tick().await;

        loop {
            tokio::select! {
                result = &mut operation => return Some(result),
                _ = renewal.tick() => {
                    match LeaseStore::renew(db, token, self.ttl).await {
                        Ok(true) => {}
                        Ok(false) => return None,
                        Err(error) => {
                            tracing::warn!(lease_name = %token.name, error = %error, "Lease renewal failed; cancelling fenced operation");
                            return None;
                        }
                    }
                }
            }
        }
    }
}

static CLUSTER_LEASE_RUNTIME: OnceLock<ClusterLeaseRuntime> = OnceLock::new();

pub fn initialize_cluster_lease_runtime(runtime: ClusterLeaseRuntime) {
    CLUSTER_LEASE_RUNTIME
        .set(runtime)
        .expect("cluster lease runtime initialized more than once");
}

pub fn cluster_lease_runtime() -> &'static ClusterLeaseRuntime {
    #[cfg(not(test))]
    {
        CLUSTER_LEASE_RUNTIME
            .get()
            .expect("cluster lease runtime must be initialized before background work starts")
    }
    #[cfg(test)]
    {
        CLUSTER_LEASE_RUNTIME.get_or_init(|| {
            ClusterLeaseRuntime::new(
                CoordinationHolder {
                    instance_id: "test-instance".to_string(),
                    generation_id: uuid::Uuid::new_v4().to_string(),
                },
                Duration::from_secs(30),
                Duration::from_secs(10),
            )
        })
    }
}

pub struct LeaseStore;

impl LeaseStore {
    pub async fn acquire(
        db: &mongodb::Database,
        name: &str,
        holder: &CoordinationHolder,
        ttl: Duration,
    ) -> AppResult<Option<LeaseToken>> {
        let ttl_ms = duration_millis(ttl, "lease TTL")?;
        let lease_id = uuid::Uuid::new_v4().to_string();
        let holder_bson = bson::to_bson(holder).map_err(|error| {
            AppError::Internal(format!("Failed to encode lease holder: {error}"))
        })?;
        let update = vec![
            doc! {
                "$set": {
                    "__claimable": {
                        "$lte": [
                            { "$ifNull": ["$expires_at", DateTime::from_millis(0)] },
                            "$$NOW",
                        ]
                    },
                }
            },
            doc! {
                "$set": {
                    "holder": { "$cond": ["$__claimable", holder_bson, "$holder"] },
                    "lease_id": { "$cond": ["$__claimable", &lease_id, "$lease_id"] },
                    "record_kind": {
                        "$cond": [
                            "$__claimable",
                            { "$ifNull": ["$record_kind", "ephemeral"] },
                            "$record_kind",
                        ]
                    },
                    "acquired_at": {
                        "$cond": ["$__claimable", "$$NOW", "$acquired_at"]
                    },
                    "updated_at": { "$cond": ["$__claimable", "$$NOW", "$updated_at"] },
                    "expires_at": {
                        "$cond": ["$__claimable", date_add_now(ttl_ms), "$expires_at"]
                    },
                }
            },
            doc! { "$unset": "__claimable" },
        ];
        let result = db
            .collection::<CoordinationLease>(LEASE_COLLECTION_NAME)
            .find_one_and_update(doc! { "_id": name }, update)
            .upsert(true)
            .return_document(ReturnDocument::After)
            .await;

        match result {
            Ok(Some(record)) if record.lease_id == lease_id => Ok(Some(LeaseToken {
                name: name.to_string(),
                holder: holder.clone(),
                lease_id,
            })),
            Ok(Some(_)) => Ok(None),
            Ok(None) => Err(AppError::Internal(
                "Lease acquisition returned no record".to_string(),
            )),
            Err(error) if is_duplicate_key_error(&error) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn renew(
        db: &mongodb::Database,
        token: &LeaseToken,
        ttl: Duration,
    ) -> AppResult<bool> {
        let ttl_ms = duration_millis(ttl, "lease TTL")?;
        let result = db
            .collection::<CoordinationLease>(LEASE_COLLECTION_NAME)
            .update_one(
                active_lease_filter(&token.name, &token.holder, &token.lease_id),
                vec![doc! {
                    "$set": {
                        "updated_at": "$$NOW",
                        "expires_at": date_add_now(ttl_ms),
                    }
                }],
            )
            .await?;
        Ok(result.modified_count == 1)
    }

    pub async fn release(db: &mongodb::Database, token: &LeaseToken) -> AppResult<bool> {
        let mut without_checkpoint =
            exact_lease_filter(&token.name, &token.holder, &token.lease_id);
        without_checkpoint.insert("checkpoint", doc! { "$exists": false });
        let deleted = db
            .collection::<CoordinationLease>(LEASE_COLLECTION_NAME)
            .delete_one(without_checkpoint)
            .await?;
        if deleted.deleted_count == 1 {
            return Ok(true);
        }

        let mut with_checkpoint = exact_lease_filter(&token.name, &token.holder, &token.lease_id);
        with_checkpoint.insert("checkpoint", doc! { "$exists": true });
        let released = db
            .collection::<CoordinationLease>(LEASE_COLLECTION_NAME)
            .update_one(
                with_checkpoint,
                doc! { "$set": {
                    "updated_at": bson::DateTime::now(),
                    "expires_at": bson::DateTime::from_millis(0),
                }},
            )
            .await?;
        Ok(released.modified_count == 1)
    }

    pub async fn load_checkpoint(
        db: &mongodb::Database,
        token: &LeaseToken,
    ) -> AppResult<Option<Bson>> {
        let record = db
            .collection::<CoordinationLease>(LEASE_COLLECTION_NAME)
            .find_one(active_lease_filter(
                &token.name,
                &token.holder,
                &token.lease_id,
            ))
            .await?;
        Ok(record.and_then(|lease| lease.checkpoint))
    }

    pub async fn store_checkpoint(
        db: &mongodb::Database,
        token: &LeaseToken,
        checkpoint: Bson,
    ) -> AppResult<bool> {
        let result = db
            .collection::<CoordinationLease>(LEASE_COLLECTION_NAME)
            .update_one(
                active_lease_filter(&token.name, &token.holder, &token.lease_id),
                doc! { "$set": {
                    "checkpoint": checkpoint,
                    "record_kind": "checkpoint",
                    "updated_at": bson::DateTime::now(),
                }},
            )
            .await?;
        Ok(result.modified_count == 1)
    }
}

pub struct ReplayStore;

impl ReplayStore {
    pub async fn claim(
        db: &mongodb::Database,
        namespace: &str,
        key: &str,
        ttl: Duration,
    ) -> AppResult<bool> {
        let ttl_ms = duration_millis(ttl, "replay TTL")?;
        let claim_id = uuid::Uuid::new_v4().to_string();
        let key_hash = hash_parts(&[key]);
        let id = hash_parts(&[namespace, key]);
        let update = vec![doc! {
            "$set": {
                "namespace": { "$ifNull": ["$namespace", namespace] },
                "key_hash": { "$ifNull": ["$key_hash", &key_hash] },
                "claim_id": { "$ifNull": ["$claim_id", &claim_id] },
                "created_at": { "$ifNull": ["$created_at", "$$NOW"] },
                "expires_at": { "$ifNull": ["$expires_at", date_add_now(ttl_ms)] },
            }
        }];
        let result = db
            .collection::<ReplayRecord>(REPLAY_COLLECTION_NAME)
            .find_one_and_update(doc! { "_id": &id }, update)
            .upsert(true)
            .return_document(ReturnDocument::After)
            .await;
        match result {
            Ok(Some(record)) => Ok(record.claim_id == claim_id),
            Ok(None) => Err(AppError::Internal(
                "Replay claim returned no record".to_string(),
            )),
            Err(error) if is_duplicate_key_error(&error) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RateAdmission {
    pub allowed: bool,
    pub remaining: u64,
    pub reset_at: chrono::DateTime<chrono::Utc>,
}

pub struct RateWindowStore;

impl RateWindowStore {
    pub async fn admit(
        db: &mongodb::Database,
        namespace: &str,
        key: &str,
        limit: u64,
        window: Duration,
    ) -> AppResult<RateAdmission> {
        let window_ms = duration_millis(window, "rate window")?;
        let limit = i64::try_from(limit).map_err(|_| {
            AppError::Internal("Rate limit exceeds MongoDB integer range".to_string())
        })?;
        let admission_id = uuid::Uuid::new_v4().to_string();
        let id = hash_parts(&[namespace, key]);
        let key_hash = hash_parts(&[key]);
        let update = rate_window_pipeline(namespace, &key_hash, &admission_id, limit, window_ms);
        let record = db
            .collection::<RateWindowRecord>(RATE_WINDOW_COLLECTION_NAME)
            .find_one_and_update(doc! { "_id": &id }, update)
            .upsert(true)
            .return_document(ReturnDocument::After)
            .await?
            .ok_or_else(|| {
                AppError::Internal("Rate-window update returned no record".to_string())
            })?;
        let allowed = record.last_admission_id.as_deref() == Some(admission_id.as_str());
        let remaining = if allowed {
            u64::try_from((limit - record.count).max(0)).unwrap_or(0)
        } else {
            0
        };
        let reset_at = record.window_start + chrono::Duration::milliseconds(window_ms);
        Ok(RateAdmission {
            allowed,
            remaining,
            reset_at,
        })
    }
}

const TOKEN_SCALE: i64 = 1_000;

pub struct TokenBucketStore;

impl TokenBucketStore {
    pub async fn admit(
        db: &mongodb::Database,
        namespace: &str,
        key: &str,
        rate_per_second: u32,
        burst: u32,
    ) -> AppResult<RateAdmission> {
        if rate_per_second == 0 || burst == 0 {
            return Ok(RateAdmission {
                allowed: false,
                remaining: 0,
                reset_at: chrono::Utc::now(),
            });
        }
        let rate_per_second = i64::from(rate_per_second);
        let capacity = i64::from(burst)
            .checked_mul(TOKEN_SCALE)
            .ok_or_else(|| AppError::Internal("Token-bucket capacity overflow".to_string()))?;
        let admission_id = uuid::Uuid::new_v4().to_string();
        let id = hash_parts(&[namespace, key]);
        let key_hash = hash_parts(&[key]);
        let refill_ms = capacity
            .saturating_add(rate_per_second - 1)
            .saturating_div(rate_per_second);
        let retention_ms = refill_ms.max(1_000).saturating_mul(2);
        let update = token_bucket_pipeline(
            namespace,
            &key_hash,
            &admission_id,
            rate_per_second,
            capacity,
            retention_ms,
        );
        let record = db
            .collection::<TokenBucketRecord>(TOKEN_BUCKET_COLLECTION_NAME)
            .find_one_and_update(doc! { "_id": &id }, update)
            .upsert(true)
            .return_document(ReturnDocument::After)
            .await?
            .ok_or_else(|| {
                AppError::Internal("Token-bucket update returned no record".to_string())
            })?;
        let allowed = record.last_admission_id.as_deref() == Some(admission_id.as_str());
        let remaining = if allowed {
            u64::try_from(record.tokens_millis.max(0) / TOKEN_SCALE).unwrap_or(0)
        } else {
            0
        };
        let missing = capacity.saturating_sub(record.tokens_millis.max(0));
        let reset_ms = missing
            .saturating_add(rate_per_second - 1)
            .saturating_div(rate_per_second);
        Ok(RateAdmission {
            allowed,
            remaining,
            reset_at: record.last_refill_at + chrono::Duration::milliseconds(reset_ms),
        })
    }
}

#[derive(Debug, Clone)]
pub struct SlotToken {
    pub id: String,
    pub namespace: String,
    pub scope_hash: String,
    pub slot: u32,
    pub holder: CoordinationHolder,
    pub lease_id: String,
}

pub struct SlotStore;

impl SlotStore {
    pub async fn acquire(
        db: &mongodb::Database,
        namespace: &str,
        scope: &str,
        limit: u32,
        holder: &CoordinationHolder,
        ttl: Duration,
    ) -> AppResult<Option<SlotToken>> {
        if limit == 0 {
            return Ok(None);
        }
        let ttl_ms = duration_millis(ttl, "slot TTL")?;
        let lease_id = uuid::Uuid::new_v4().to_string();
        let scope_hash = hash_parts(&[scope]);
        let holder_bson = bson::to_bson(holder).map_err(|error| {
            AppError::Internal(format!("Failed to encode slot holder: {error}"))
        })?;
        let start = slot_start(&lease_id, limit);

        for offset in 0..limit {
            let slot = (start + offset) % limit;
            let id = hash_parts(&[namespace, scope, &slot.to_string()]);
            let update = vec![
                claimable_stage(),
                doc! {
                    "$set": {
                        "namespace": { "$cond": ["$__claimable", namespace, "$namespace"] },
                        "scope_hash": {
                            "$cond": ["$__claimable", &scope_hash, "$scope_hash"]
                        },
                        "slot": { "$cond": ["$__claimable", i64::from(slot), "$slot"] },
                        "holder": {
                            "$cond": ["$__claimable", holder_bson.clone(), "$holder"]
                        },
                        "lease_id": {
                            "$cond": ["$__claimable", &lease_id, "$lease_id"]
                        },
                        "acquired_at": {
                            "$cond": ["$__claimable", "$$NOW", "$acquired_at"]
                        },
                        "updated_at": {
                            "$cond": ["$__claimable", "$$NOW", "$updated_at"]
                        },
                        "expires_at": {
                            "$cond": ["$__claimable", date_add_now(ttl_ms), "$expires_at"]
                        },
                    }
                },
                doc! { "$unset": "__claimable" },
            ];
            let result = db
                .collection::<CoordinationSlot>(SLOT_COLLECTION_NAME)
                .find_one_and_update(doc! { "_id": &id }, update)
                .upsert(true)
                .return_document(ReturnDocument::After)
                .await;
            match result {
                Ok(Some(record)) if record.lease_id == lease_id => {
                    return Ok(Some(SlotToken {
                        id,
                        namespace: namespace.to_string(),
                        scope_hash,
                        slot,
                        holder: holder.clone(),
                        lease_id,
                    }));
                }
                Ok(Some(_)) => continue,
                Ok(None) => {
                    return Err(AppError::Internal(
                        "Slot acquisition returned no record".to_string(),
                    ));
                }
                Err(error) if is_duplicate_key_error(&error) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(None)
    }

    pub async fn renew(
        db: &mongodb::Database,
        token: &SlotToken,
        ttl: Duration,
    ) -> AppResult<bool> {
        let ttl_ms = duration_millis(ttl, "slot TTL")?;
        let result = db
            .collection::<CoordinationSlot>(SLOT_COLLECTION_NAME)
            .update_one(
                active_lease_filter(&token.id, &token.holder, &token.lease_id),
                vec![doc! {
                    "$set": {
                        "updated_at": "$$NOW",
                        "expires_at": date_add_now(ttl_ms),
                    }
                }],
            )
            .await?;
        Ok(result.modified_count == 1)
    }

    pub async fn release(db: &mongodb::Database, token: &SlotToken) -> AppResult<bool> {
        let result = db
            .collection::<CoordinationSlot>(SLOT_COLLECTION_NAME)
            .delete_one(exact_lease_filter(
                &token.id,
                &token.holder,
                &token.lease_id,
            ))
            .await?;
        Ok(result.deleted_count == 1)
    }
}

#[derive(Debug, Clone)]
pub struct EventDedupClaim {
    pub id: String,
    pub claim_id: String,
}

#[derive(Debug, Clone)]
pub enum EventDedupClaimResult {
    Claimed(EventDedupClaim),
    Duplicate,
}

pub struct EventDedupStore;

impl EventDedupStore {
    pub async fn claim(
        db: &mongodb::Database,
        namespace: &str,
        scope: &str,
        event_id: &str,
        ttl: Duration,
    ) -> AppResult<EventDedupClaimResult> {
        let ttl_ms = duration_millis(ttl, "event claim TTL")?;
        let id = hash_parts(&[namespace, scope, event_id]);
        let claim_id = uuid::Uuid::new_v4().to_string();
        let update = vec![
            claimable_stage(),
            doc! {
                "$set": {
                    "namespace": { "$cond": ["$__claimable", namespace, "$namespace"] },
                    "scope_hash": {
                        "$cond": ["$__claimable", hash_parts(&[scope]), "$scope_hash"]
                    },
                    "event_hash": {
                        "$cond": ["$__claimable", hash_parts(&[event_id]), "$event_hash"]
                    },
                    "state": { "$cond": ["$__claimable", "claimed", "$state"] },
                    "claim_id": { "$cond": ["$__claimable", &claim_id, "$claim_id"] },
                    "created_at": {
                        "$cond": ["$__claimable", "$$NOW", "$created_at"]
                    },
                    "updated_at": {
                        "$cond": ["$__claimable", "$$NOW", "$updated_at"]
                    },
                    "expires_at": {
                        "$cond": ["$__claimable", date_add_now(ttl_ms), "$expires_at"]
                    },
                }
            },
            doc! { "$unset": "__claimable" },
        ];
        let result = db
            .collection::<EventDedupRecord>(EVENT_DEDUP_COLLECTION_NAME)
            .find_one_and_update(doc! { "_id": &id }, update)
            .upsert(true)
            .return_document(ReturnDocument::After)
            .await;
        match result {
            Ok(Some(record)) if record.claim_id == claim_id => {
                Ok(EventDedupClaimResult::Claimed(EventDedupClaim {
                    id,
                    claim_id,
                }))
            }
            Ok(Some(_)) => Ok(EventDedupClaimResult::Duplicate),
            Ok(None) => Err(AppError::Internal(
                "Event dedup claim returned no record".to_string(),
            )),
            Err(error) if is_duplicate_key_error(&error) => Ok(EventDedupClaimResult::Duplicate),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn renew(
        db: &mongodb::Database,
        claim: &EventDedupClaim,
        ttl: Duration,
    ) -> AppResult<bool> {
        let ttl_ms = duration_millis(ttl, "event claim TTL")?;
        let result = db
            .collection::<EventDedupRecord>(EVENT_DEDUP_COLLECTION_NAME)
            .update_one(
                doc! {
                    "_id": &claim.id,
                    "claim_id": &claim.claim_id,
                    "state": event_state(EventDedupState::Claimed),
                    "$expr": { "$gt": ["$expires_at", "$$NOW"] },
                },
                vec![doc! {
                    "$set": {
                        "updated_at": "$$NOW",
                        "expires_at": date_add_now(ttl_ms),
                    }
                }],
            )
            .await?;
        Ok(result.modified_count == 1)
    }

    pub async fn commit(
        db: &mongodb::Database,
        claim: &EventDedupClaim,
        ttl: Duration,
    ) -> AppResult<bool> {
        let ttl_ms = duration_millis(ttl, "event dedup TTL")?;
        let result = db
            .collection::<EventDedupRecord>(EVENT_DEDUP_COLLECTION_NAME)
            .update_one(
                doc! {
                    "_id": &claim.id,
                    "claim_id": &claim.claim_id,
                    "state": event_state(EventDedupState::Claimed),
                },
                vec![doc! {
                    "$set": {
                        "state": event_state(EventDedupState::Committed),
                        "updated_at": "$$NOW",
                        "expires_at": date_add_now(ttl_ms),
                    }
                }],
            )
            .await?;
        Ok(result.modified_count == 1)
    }

    pub async fn release(db: &mongodb::Database, claim: &EventDedupClaim) -> AppResult<bool> {
        let result = db
            .collection::<EventDedupRecord>(EVENT_DEDUP_COLLECTION_NAME)
            .delete_one(doc! {
                "_id": &claim.id,
                "claim_id": &claim.claim_id,
                "state": event_state(EventDedupState::Claimed),
            })
            .await?;
        Ok(result.deleted_count == 1)
    }
}

fn duration_millis(duration: Duration, name: &str) -> AppResult<i64> {
    let millis = i64::try_from(duration.as_millis())
        .map_err(|_| AppError::Internal(format!("{name} exceeds MongoDB duration range")))?;
    if millis <= 0 {
        return Err(AppError::Internal(format!("{name} must be positive")));
    }
    Ok(millis)
}

fn hash_parts(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn date_add_now(amount_ms: i64) -> Document {
    doc! {
        "$dateAdd": {
            "startDate": "$$NOW",
            "unit": "millisecond",
            "amount": amount_ms,
        }
    }
}

fn claimable_stage() -> Document {
    doc! {
        "$set": {
            "__claimable": {
                "$lte": [
                    { "$ifNull": ["$expires_at", DateTime::from_millis(0)] },
                    "$$NOW",
                ]
            },
        }
    }
}

fn exact_lease_filter(id: &str, holder: &CoordinationHolder, lease_id: &str) -> Document {
    doc! {
        "_id": id,
        "holder.instance_id": &holder.instance_id,
        "holder.generation_id": &holder.generation_id,
        "lease_id": lease_id,
    }
}

fn active_lease_filter(id: &str, holder: &CoordinationHolder, lease_id: &str) -> Document {
    let mut filter = exact_lease_filter(id, holder, lease_id);
    filter.insert("$expr", doc! { "$gt": ["$expires_at", "$$NOW"] });
    filter
}

fn rate_window_pipeline(
    namespace: &str,
    key_hash: &str,
    admission_id: &str,
    limit: i64,
    window_ms: i64,
) -> Vec<Document> {
    let current_window = doc! {
        "$dateTrunc": {
            "date": "$$NOW",
            "unit": "millisecond",
            "binSize": window_ms,
        }
    };
    vec![
        doc! {
            "$set": {
                "__current_window": current_window.clone(),
                "__new_window": {
                    "$ne": [
                        { "$ifNull": ["$window_start", DateTime::from_millis(0)] },
                        current_window,
                    ]
                },
            }
        },
        doc! {
            "$set": {
                "namespace": namespace,
                "key_hash": key_hash,
                "window_start": "$__current_window",
                "count": {
                    "$cond": [
                        "$__new_window",
                        if limit > 0 { 1_i64 } else { 0_i64 },
                        {
                            "$cond": [
                                { "$lt": [{ "$ifNull": ["$count", 0_i64] }, limit] },
                                { "$add": [{ "$ifNull": ["$count", 0_i64] }, 1_i64] },
                                { "$ifNull": ["$count", 0_i64] },
                            ]
                        },
                    ]
                },
                "last_admission_id": {
                    "$cond": [
                        { "$or": [
                            { "$and": ["$__new_window", limit > 0] },
                            { "$and": [
                                { "$not": ["$__new_window"] },
                                { "$lt": [{ "$ifNull": ["$count", 0_i64] }, limit] },
                            ] },
                        ] },
                        admission_id,
                        { "$ifNull": ["$last_admission_id", Bson::Null] },
                    ]
                },
                "updated_at": "$$NOW",
                "expires_at": {
                    "$dateAdd": {
                        "startDate": "$__current_window",
                        "unit": "millisecond",
                        "amount": window_ms.saturating_mul(2),
                    }
                },
            }
        },
        doc! { "$unset": ["__current_window", "__new_window"] },
    ]
}

fn token_bucket_pipeline(
    namespace: &str,
    key_hash: &str,
    admission_id: &str,
    rate_per_second: i64,
    capacity: i64,
    retention_ms: i64,
) -> Vec<Document> {
    vec![
        doc! {
            "$set": {
                "__elapsed_ms": {
                    "$max": [
                        0_i64,
                        {
                            "$dateDiff": {
                                "startDate": { "$ifNull": ["$last_refill_at", "$$NOW"] },
                                "endDate": "$$NOW",
                                "unit": "millisecond",
                            }
                        },
                    ]
                },
            }
        },
        doc! {
            "$set": {
                "__available": {
                    "$min": [
                        capacity,
                        {
                            "$add": [
                                { "$ifNull": ["$tokens_millis", capacity] },
                                { "$multiply": ["$__elapsed_ms", rate_per_second] },
                            ]
                        },
                    ]
                },
            }
        },
        doc! {
            "$set": {
                "namespace": namespace,
                "key_hash": key_hash,
                "tokens_millis": {
                    "$cond": [
                        { "$gte": ["$__available", TOKEN_SCALE] },
                        { "$subtract": ["$__available", TOKEN_SCALE] },
                        "$__available",
                    ]
                },
                "last_refill_at": "$$NOW",
                "last_admission_id": {
                    "$cond": [
                        { "$gte": ["$__available", TOKEN_SCALE] },
                        admission_id,
                        { "$ifNull": ["$last_admission_id", Bson::Null] },
                    ]
                },
                "updated_at": "$$NOW",
                "expires_at": date_add_now(retention_ms),
            }
        },
        doc! { "$unset": ["__elapsed_ms", "__available"] },
    ]
}

fn slot_start(lease_id: &str, limit: u32) -> u32 {
    let digest = Sha256::digest(lease_id.as_bytes());
    let seed = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    seed % limit
}

fn event_state(state: EventDedupState) -> &'static str {
    match state {
        EventDedupState::Claimed => "claimed",
        EventDedupState::Committed => "committed",
    }
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    match error.kind.as_ref() {
        mongodb::error::ErrorKind::Command(command) => command.code == 11000,
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) => {
            write.code == 11000
        }
        _ => false,
    }
}
