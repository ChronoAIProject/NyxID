use bson::doc;
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{Database, IndexModel};

use crate::{
    errors::{AppError, AppResult},
    models::{
        audit_log::AuditLog,
        service_change_event::{COLLECTION_NAME, ServiceChangeEvent},
    },
    services::{audit_chain_service, audit_service},
};

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<ServiceChangeEvent>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "service_id": 1, "service_sequence": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await?;
    for keys in [
        doc! { "owner_id": 1, "service_id": 1, "committed_at": -1, "_id": -1 },
        doc! { "service_id": 1, "change_group_id": 1, "committed_at": 1 },
        doc! { "audited_at": 1, "committed_at": 1 },
    ] {
        db.collection::<ServiceChangeEvent>(COLLECTION_NAME)
            .create_index(IndexModel::builder().keys(keys).build())
            .await?;
    }
    Ok(())
}

pub fn mirror(event: &ServiceChangeEvent) -> AppResult<AuditLog> {
    // Publication fields are mutable relay progress, never part of the immutable payload.
    let mut payload = serde_json::to_value(event)
        .map_err(|_| AppError::Internal("Could not encode service history".into()))?;
    let fields = payload.as_object_mut().expect("event is an object");
    fields.remove("audited_at");
    fields.remove("audit_log_id");
    fields.remove("audit_attempts");
    fields.remove("next_audit_attempt_at");
    Ok(AuditLog {
        id: event.id.clone(),
        user_id: event.actor.person_id.clone(),
        event_type: "service_change_recorded".into(),
        event_data: Some(payload),
        ip_address: None,
        user_agent: None,
        api_key_id: event.actor.api_key_id.clone(),
        api_key_name: event
            .actor
            .api_key_id
            .as_ref()
            .map(|_| event.actor.name.clone()),
        seq: None,
        prev_hash: None,
        entry_hash: None,
        created_at: event.committed_at,
    })
}

pub async fn publish(db: &Database, event: &ServiceChangeEvent, key: &[u8]) -> AppResult<()> {
    audit_chain_service::append_chained_entry(db, mirror(event)?, key).await?;
    db.collection::<ServiceChangeEvent>(COLLECTION_NAME).update_one(doc! { "_id": &event.id }, doc! { "$set": { "audited_at": bson::DateTime::from_chrono(Utc::now()), "audit_log_id": &event.id } }).await?;
    Ok(())
}

pub async fn sweep(db: &Database) -> AppResult<()> {
    let key = audit_service::audit_chain_hmac_key()
        .ok_or_else(|| AppError::Internal("Audit chain is not initialized".into()))?;
    let collection = db.collection::<ServiceChangeEvent>(COLLECTION_NAME);
    let pending = collection
        .count_documents(doc! { "audited_at": null })
        .await?;
    let events: Vec<_> = collection.find(doc! { "audited_at": null, "$or": [ { "next_audit_attempt_at": null }, { "next_audit_attempt_at": { "$lte": bson::DateTime::from_chrono(Utc::now()) } } ] }).sort(doc! { "committed_at": 1, "_id": 1 }).limit(100).await?.try_collect().await?;
    let oldest = collection
        .find_one(doc! { "audited_at": null })
        .sort(doc! { "committed_at": 1 })
        .await?;
    let oldest_age_seconds = oldest
        .as_ref()
        .map_or(0, |e| (Utc::now() - e.committed_at).num_seconds().max(0));
    let mut failures = 0;
    for event in events {
        if let Err(error) = publish(db, &event, key).await {
            failures += 1;
            let delay = 10_i64 * (1_i64 << event.audit_attempts.min(8));
            collection.update_one(doc! { "_id": &event.id, "audited_at": null }, doc! { "$inc": { "audit_attempts": 1 }, "$set": { "next_audit_attempt_at": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::seconds(delay)) } }).await?;
            tracing::error!(event_id = %event.id, %error, "Service history audit publication failed; will retry");
        }
    }
    tracing::info!(
        pending,
        oldest_age_seconds,
        failures,
        "Service history audit publication backlog"
    );
    Ok(())
}

/// Verify the journal against its immutable mirror in addition to global chain verification.
#[cfg(test)]
pub async fn verify_mirror(db: &Database, event: &ServiceChangeEvent) -> AppResult<bool> {
    let stored = db
        .collection::<AuditLog>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! { "_id": &event.id })
        .await?;
    Ok(stored.is_some_and(|stored| mirror_matches(event, &stored)))
}

pub fn mirror_matches(event: &ServiceChangeEvent, stored: &AuditLog) -> bool {
    let Some(key) = audit_service::audit_chain_hmac_key() else {
        return false;
    };
    stored.seq.is_some()
        && stored.prev_hash.is_some()
        && stored.entry_hash.as_ref().is_some_and(|hash| {
            audit_chain_service::compute_entry_hash(stored, key).is_ok_and(|actual| &actual == hash)
        })
        && mirror(event)
            .is_ok_and(|expected| audit_chain_service::immutable_entry_matches(stored, &expected))
}

pub fn start(db: Database) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            interval.tick().await;
            if let Err(error) = sweep(&db).await {
                tracing::error!(%error, "Service history relay failed; will retry");
            }
        }
    });
}
