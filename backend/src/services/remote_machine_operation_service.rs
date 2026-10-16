//! CAS and durable effect processing for managed-machine lifecycle operations.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{Bson, Document, doc},
    options::{IndexOptions, ReturnDocument},
};
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::{
        remote_machine_operation::{
            COLLECTION_NAME as OPERATIONS, MachineOperation, OperationState,
        },
        remote_machine_outbox::{COLLECTION_NAME as OUTBOX, EffectState, MachineEffectOutbox},
    },
};

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<Document>(OPERATIONS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"owner_id": 1, "action": 1, "idempotency_key_hash": 1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .name("machine_operation_create_idempotency".to_string())
                        .partial_filter_expression(doc! {"action": "create"})
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(OPERATIONS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"owner_id": 1, "machine_id": 1, "action": 1, "idempotency_key_hash": 1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .name("machine_operation_intent_idempotency".to_string())
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(OUTBOX)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"operation_id": 1, "kind": 1, "effect_digest":1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .name("machine_effect_intent_v2".to_string())
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(OUTBOX).create_index(IndexModel::builder()
        .keys(doc!{"operation_id":1})
        .options(IndexOptions::builder().unique(true).name("machine_lifecycle_effect_unique".to_string())
            .partial_filter_expression(doc!{"kind":{"$in":["create_sandbox","suspend_sandbox","resume_sandbox","rekey_sandbox","delete_sandbox","reconcile_sandbox"]}}).build()).build()).await?;
    // Install replacement definitions before retiring the old single-effect
    // constraint. It prevented multiple finite deadline deliveries per run.
    if db
        .collection::<Document>(OUTBOX)
        .list_indexes()
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .iter()
        .any(|i| i.options.as_ref().and_then(|o| o.name.as_deref()) == Some("operation_id_1"))
        && let Err(error) = db
            .collection::<Document>(OUTBOX)
            .drop_index("operation_id_1")
            .await
        && !matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if command.code == 27)
    {
        return Err(error);
    }
    db.collection::<Document>(OUTBOX)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"state": 1, "available_at": 1})
                .build(),
        )
        .await?;
    db.collection::<Document>(OPERATIONS).create_index(IndexModel::builder()
        .keys(doc!{"reclamation_finalized_at":1,"reclamation_checked_at":1,"terminal_at":1,"_id":1})
        .options(IndexOptions::builder().name("machine_operation_reclamation_v1".to_string()).build()).build()).await?;
    Ok(())
}

#[cfg(test)]
pub fn is_idempotent_replay(existing_digest: &str, incoming_digest: &str, state: &str) -> bool {
    existing_digest == incoming_digest
        && matches!(state, "accepted" | "applying" | "reconciling" | "succeeded")
}

#[cfg(test)]
pub fn next_resource_state(
    action: crate::models::remote_machine_operation::LifecycleAction,
) -> crate::models::remote_machine::ResourceState {
    super::remote_machine_service::next_resource_state(action)
}

pub async fn get_operation(
    db: &Database,
    owner_id: &str,
    operation_id: &str,
) -> AppResult<Option<MachineOperation>> {
    Ok(db
        .collection::<MachineOperation>(OPERATIONS)
        .find_one(doc! {"_id": operation_id, "owner_id": owner_id})
        .await?)
}

/// Display retention never deletes the idempotency/finality source row.
pub async fn get_display_operation(
    db: &Database,
    owner_id: &str,
    operation_id: &str,
) -> AppResult<Option<MachineOperation>> {
    Ok(db.collection::<MachineOperation>(OPERATIONS).find_one(doc! {"_id":operation_id,"owner_id":owner_id,
        "$or":[{"terminal_at":Bson::Null},{"terminal_at":{"$gt":mongodb::bson::DateTime::from_chrono(Utc::now()-ChronoDuration::days(90))}}]}).await?)
}

pub async fn list_operations(
    db: &Database,
    owner_id: &str,
    machine_id: Option<&str>,
) -> AppResult<Vec<MachineOperation>> {
    let mut filter = doc! {"owner_id": owner_id,"$or":[{"terminal_at":Bson::Null},{"terminal_at":{"$gt":mongodb::bson::DateTime::from_chrono(Utc::now()-ChronoDuration::days(90))}}]};
    if let Some(machine_id) = machine_id {
        filter.insert("machine_id", machine_id);
    }
    Ok(db
        .collection::<MachineOperation>(OPERATIONS)
        .find(filter)
        .sort(doc! {"created_at": -1,"_id":-1})
        .limit(100)
        .await?
        .try_collect()
        .await?)
}

/// Claim one pending effect with a short lease. The CAS filter means a lost
/// worker cannot acknowledge a later worker's effect.
pub async fn claim_effect(
    db: &Database,
    now: DateTime<Utc>,
    lease: std::time::Duration,
) -> AppResult<Option<MachineEffectOutbox>> {
    claim_effect_kind(db, now, lease, None).await
}
async fn claim_effect_kind(
    db: &Database,
    now: DateTime<Utc>,
    lease: std::time::Duration,
    kind: Option<&str>,
) -> AppResult<Option<MachineEffectOutbox>> {
    let mut filter = doc! {"$or": [
        {"state": {"$in": ["pending", "retry"]}, "$or": [{"available_at": null}, {"available_at": {"$lte": mongodb::bson::DateTime::from_chrono(now)}}]},
        {"state": "claimed", "claimed_until": {"$lte": mongodb::bson::DateTime::from_chrono(now)}}
    ]};
    if let Some(kind) = kind {
        filter.insert("kind", kind);
    }
    Ok(db.collection::<MachineEffectOutbox>(OUTBOX).find_one_and_update(filter, doc! {"$set": {"state": "claimed", "claim_id": Uuid::new_v4().to_string(), "claimed_until": mongodb::bson::DateTime::from_chrono(now + lease), "updated_at": mongodb::bson::DateTime::from_chrono(now)}, "$inc": {"attempts": 1}}).sort(doc! {"created_at": 1,"_id":1}).return_document(ReturnDocument::After).await?)
}

pub async fn acknowledge_effect(
    db: &Database,
    effect_id: &str,
    claim_id: &str,
    applied: bool,
    error: Option<&str>,
    now: DateTime<Utc>,
) -> AppResult<bool> {
    let effect = db
        .collection::<MachineEffectOutbox>(OUTBOX)
        .find_one(doc! {"_id":effect_id,"state":"claimed","claim_id":claim_id})
        .await?;
    let Some(effect) = effect else {
        return Ok(false);
    };
    let retry_seconds = retry_backoff_seconds(effect.attempts);
    let (state, available) = if applied {
        (EffectState::Applied.as_str(), None)
    } else {
        (
            EffectState::Retry.as_str(),
            Some(mongodb::bson::DateTime::from_chrono(
                now + ChronoDuration::seconds(retry_seconds),
            )),
        )
    };
    let mut update = doc! {"$set": {"state": state, "last_error": error, "claimed_until": Bson::Null, "claim_id": Bson::Null, "updated_at": mongodb::bson::DateTime::from_chrono(now)}};
    if let Some(available) = available {
        update
            .get_document_mut("$set")
            .expect("set document")
            .insert("available_at", available);
    }
    let result = db
        .collection::<MachineEffectOutbox>(OUTBOX)
        .update_one(
            doc! {"_id": effect_id, "state": "claimed", "claim_id": claim_id},
            update,
        )
        .await?;
    if result.modified_count != 1 {
        return Ok(false);
    }
    if applied {
        let op = db
            .collection::<MachineEffectOutbox>(OUTBOX)
            .find_one(doc! {"_id": effect_id})
            .await?
            .ok_or_else(|| AppError::Internal("acknowledged effect disappeared".into()))?;
        db.collection::<MachineOperation>(OPERATIONS).update_one(doc! {"_id": &op.operation_id, "state": {"$in": ["accepted", "applying", "reconciling"]}}, doc! {"$set": {"state": OperationState::Applying.as_str(), "updated_at": mongodb::bson::DateTime::from_chrono(now)}}).await?;
    }
    Ok(true)
}

/// Capped exponential backoff prevents tight provider retries; stop/cleanup keeps retrying durably.
pub fn retry_backoff_seconds(attempts: i64) -> i64 {
    5_i64
        .saturating_mul(1_i64 << attempts.saturating_sub(1).clamp(0, 6) as u32)
        .min(300)
}
