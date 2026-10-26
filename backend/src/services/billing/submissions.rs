use crate::errors::{AppError, AppResult};
use crate::models::credit_allocation_request::{COLLECTION_NAME, CreditAllocationRequest};
use mongodb::bson::{self, Document, doc};
use uuid::Uuid;

pub fn identity(actor: &str, request_id: &str) -> AppResult<String> {
    let key = Uuid::parse_str(request_id)
        .map_err(|_| AppError::ValidationError("request_id must be a UUID".into()))?;
    Ok(Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("credit-allocation:{actor}:{key}").as_bytes(),
    )
    .to_string())
}

pub async fn existing(
    db: &mongodb::Database,
    actor: &str,
    key: Option<&str>,
    payload: &str,
) -> AppResult<Option<CreditAllocationRequest>> {
    let Some(key) = key else {
        return Ok(None);
    };
    let id = identity(actor, key)?;
    let row = db
        .collection::<CreditAllocationRequest>(COLLECTION_NAME)
        .find_one(doc! { "_id": id })
        .await?;
    if row.as_ref().is_some_and(|r| r.payload != payload) {
        return Err(AppError::Conflict("This allocation request was already used with different settings; start a new allocation".into()));
    }
    Ok(row)
}

pub async fn freeze(
    db: &mongodb::Database,
    actor: &str,
    key: &str,
    payload: &str,
    snapshot: Document,
) -> AppResult<CreditAllocationRequest> {
    let id = identity(actor, key)?;
    let row = CreditAllocationRequest {
        id: id.clone(),
        actor_id: actor.into(),
        payload: payload.into(),
        snapshot,
        created_at: chrono::Utc::now(),
    };
    let encoded = bson::to_document(&row)
        .map_err(|e| AppError::Internal(format!("Allocation snapshot encoding failed: {e}")))?;
    let result = db
        .collection::<CreditAllocationRequest>(COLLECTION_NAME)
        .update_one(doc! { "_id": &id }, doc! { "$setOnInsert": encoded })
        .upsert(true)
        .await;
    match result {
        Ok(_) => {}
        Err(error) if super::schedules::is_duplicate_key_error(&error) => {}
        Err(error) => return Err(error.into()),
    }
    existing(db, actor, Some(key), payload)
        .await?
        .ok_or_else(|| AppError::Internal("Allocation submission disappeared".into()))
}
