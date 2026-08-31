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
};

pub async fn ensure_indexes(db: &mongodb::Database) -> Result<(), mongodb::error::Error> {
    for collection_name in [
        LEASE_COLLECTION_NAME,
        REPLAY_COLLECTION_NAME,
        RATE_WINDOW_COLLECTION_NAME,
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
        let filter = expired_record_filter(name);
        let update = vec![doc! {
            "$set": {
                "holder": holder_bson,
                "lease_id": &lease_id,
                "acquired_at": "$$NOW",
                "updated_at": "$$NOW",
                "expires_at": date_add_now(ttl_ms),
            }
        }];
        let result = db
            .collection::<CoordinationLease>(LEASE_COLLECTION_NAME)
            .find_one_and_update(filter, update)
            .upsert(true)
            .return_document(ReturnDocument::After)
            .await;

        match result {
            Ok(Some(_)) => Ok(Some(LeaseToken {
                name: name.to_string(),
                holder: holder.clone(),
                lease_id,
            })),
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
        let result = db
            .collection::<CoordinationLease>(LEASE_COLLECTION_NAME)
            .delete_one(exact_lease_filter(
                &token.name,
                &token.holder,
                &token.lease_id,
            ))
            .await?;
        Ok(result.deleted_count == 1)
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
        let ttl = chrono_duration(ttl, "replay TTL")?;
        let key_hash = hash_parts(&[key]);
        let now = chrono::Utc::now();
        let record = ReplayRecord {
            id: hash_parts(&[namespace, key]),
            namespace: namespace.to_string(),
            key_hash,
            created_at: now,
            expires_at: now + ttl,
        };
        match db
            .collection::<ReplayRecord>(REPLAY_COLLECTION_NAME)
            .insert_one(record)
            .await
        {
            Ok(_) => Ok(true),
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
            let update = vec![doc! {
                "$set": {
                    "namespace": namespace,
                    "scope_hash": &scope_hash,
                    "slot": i64::from(slot),
                    "holder": holder_bson.clone(),
                    "lease_id": &lease_id,
                    "acquired_at": "$$NOW",
                    "updated_at": "$$NOW",
                    "expires_at": date_add_now(ttl_ms),
                }
            }];
            let result = db
                .collection::<CoordinationSlot>(SLOT_COLLECTION_NAME)
                .find_one_and_update(expired_record_filter(&id), update)
                .upsert(true)
                .return_document(ReturnDocument::After)
                .await;
            match result {
                Ok(Some(_)) => {
                    return Ok(Some(SlotToken {
                        id,
                        namespace: namespace.to_string(),
                        scope_hash,
                        slot,
                        holder: holder.clone(),
                        lease_id,
                    }));
                }
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
        let update = vec![doc! {
            "$set": {
                "namespace": namespace,
                "scope_hash": hash_parts(&[scope]),
                "event_hash": hash_parts(&[event_id]),
                "state": "claimed",
                "claim_id": &claim_id,
                "created_at": "$$NOW",
                "updated_at": "$$NOW",
                "expires_at": date_add_now(ttl_ms),
            }
        }];
        let result = db
            .collection::<EventDedupRecord>(EVENT_DEDUP_COLLECTION_NAME)
            .find_one_and_update(expired_record_filter(&id), update)
            .upsert(true)
            .return_document(ReturnDocument::After)
            .await;
        match result {
            Ok(Some(_)) => Ok(EventDedupClaimResult::Claimed(EventDedupClaim {
                id,
                claim_id,
            })),
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

fn chrono_duration(duration: Duration, name: &str) -> AppResult<chrono::Duration> {
    let millis = duration_millis(duration, name)?;
    Ok(chrono::Duration::milliseconds(millis))
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

fn expired_record_filter(id: &str) -> Document {
    doc! {
        "_id": id,
        "$expr": {
            "$lte": [
                { "$ifNull": ["$expires_at", DateTime::from_millis(0)] },
                "$$NOW",
            ]
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
