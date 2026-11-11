//! A shared, durable attempt budget for the original pending owner delete.
use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{self, Bson, Document, doc},
    options::ReturnDocument,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{api_key_mutation_service, remote_machine_service as lifecycle};
use crate::{
    errors::{AppError, AppResult},
    models::{
        remote_machine::{COLLECTION_NAME as MACHINES, ManagedMachine, ResourceState},
        remote_machine_operation::{
            COLLECTION_NAME as OPERATIONS, DeleteRetryState, LifecycleAction, MachineOperation,
            OperationState,
        },
    },
};

pub(crate) const LIMIT: i64 = 10;
pub(crate) const INTERVAL_SECONDS: i64 = 60;
pub(crate) const ATTEMPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(40);

pub(crate) fn attempts(operation: &MachineOperation) -> i64 {
    operation
        .delete_retry
        .as_ref()
        .map_or(operation.retry_count, |r| r.attempts)
        .max(0)
}

fn claim_is_live(operation: &MachineOperation, at: DateTime<Utc>) -> bool {
    operation
        .delete_retry
        .as_ref()
        .and_then(|r| r.claim_until)
        .is_some_and(|until| until > at)
}

pub(crate) fn stopped(operation: &MachineOperation, at: DateTime<Utc>) -> bool {
    operation.action == LifecycleAction::Delete
        && operation.terminal_at.is_none()
        && matches!(
            operation.state,
            OperationState::Accepted | OperationState::Applying | OperationState::Reconciling
        )
        && attempts(operation) >= LIMIT
        && !claim_is_live(operation, at)
}

fn attempts_expression() -> Document {
    doc! {"$max":[0_i64,{"$ifNull":["$delete_retry.attempts",{"$ifNull":["$retry_count",0_i64]}]}]}
}

/// Used before the scanner's limit as well as by the atomic claim, so stopped
/// operations cannot consume other pending deletes' observation share.
pub(crate) fn eligible_expression(at: DateTime<Utc>) -> Document {
    doc! {"$and":[
        {"$lt":[attempts_expression(),LIMIT]},
        {"$lte":[{"$ifNull":["$delete_retry.claim_until",bson::DateTime::from_millis(0)]},bson::DateTime::from_chrono(at)]},
    ]}
}

/// One increment per admitted recovery invocation, shared by all entry points.
/// The lease exceeds the bounded invocation; process loss consumes the attempt
/// and allows a later invocation only after its reservation has expired.
pub(crate) async fn claim(db: &Database, machine: &ManagedMachine) -> AppResult<Option<String>> {
    let at = Utc::now();
    let until = bson::DateTime::from_chrono(at + chrono::Duration::seconds(INTERVAL_SECONDS));
    let claim_id = Uuid::new_v4().to_string();
    let claimed = db.collection::<Document>(OPERATIONS).find_one_and_update(
        doc! {"_id":&machine.active_operation_id,"owner_id":&machine.owner_id,"machine_id":&machine.node_id,
            "action":"delete","state":{"$in":["accepted","applying","reconciling"]},"terminal_at":Bson::Null,
            "$or":[{"next_attempt_at":Bson::Null},{"next_attempt_at":{"$lte":bson::DateTime::from_chrono(at)}}],
            "$expr":eligible_expression(at)},
        vec![doc! {"$set":{
            "retry_count":{"$add":[{"$ifNull":["$retry_count",0_i64]},1_i64]},
            "delete_retry":{"$mergeObjects":[{"$ifNull":["$delete_retry",{}]},
                {"attempts":{"$add":[attempts_expression(),1_i64]},"claim_id":{"$literal":&claim_id},"claim_until":until}]},
            "next_attempt_at":until,"updated_at":bson::DateTime::from_chrono(at),
        }}],
    ).return_document(ReturnDocument::After).await?;
    Ok(claimed.map(|_| claim_id))
}

pub(crate) async fn finish(db: &Database, operation_id: &str, claim_id: &str) -> AppResult<()> {
    let at = Utc::now();
    db.collection::<Document>(OPERATIONS).update_one(
        doc! {"_id":operation_id,"action":"delete","delete_retry.claim_id":claim_id},
        vec![doc! {"$set":{
            "delete_retry.claim_id":Bson::Null,"delete_retry.claim_until":Bson::Null,
            "next_attempt_at":{"$cond":[{"$and":[{"$lt":[attempts_expression(),LIMIT]},
                {"$in":["$state",["accepted","applying","reconciling"]]},
                {"$eq":[{"$ifNull":["$terminal_at",Bson::Null]},Bson::Null]}]},
                bson::DateTime::from_chrono(at+chrono::Duration::seconds(INTERVAL_SECONDS)),Bson::Null]},
            "updated_at":bson::DateTime::from_chrono(at),
        }}],
    ).await?;
    Ok(())
}

/// Resume the existing delete. The HTTP layer supplies first-party human
/// authority; this transaction binds that owner to the original operation,
/// exhausted round and still-disabled Machine. No new deletion is accepted.
pub async fn retry(
    db: &Database,
    owner_id: &str,
    operation_id: &str,
    expected_revision: i64,
    expected_retry_count: i64,
    idempotency_key: &str,
) -> AppResult<MachineOperation> {
    if owner_id.is_empty()
        || expected_revision < 0
        || expected_retry_count < 0
        || idempotency_key.trim().is_empty()
        || idempotency_key.len() > 256
    {
        return Err(AppError::ValidationError(
            "invalid deletion retry request".into(),
        ));
    }
    let owner_id = owner_id.to_owned();
    let operation_id = operation_id.to_owned();
    let key_hash = hex::encode(Sha256::digest(idempotency_key.as_bytes()));
    let digest = hex::encode(Sha256::digest(
        serde_json::json!({
            "owner_id":owner_id,"operation_id":operation_id,
            "expected_revision":expected_revision,"expected_retry_count":expected_retry_count,
        })
        .to_string()
        .as_bytes(),
    ));
    let db_owned = db.clone();
    db.client().start_session().await?.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<MachineOperation> = async {
                let operation = db_owned.collection::<MachineOperation>(OPERATIONS)
                    .find_one(doc! {"_id":&operation_id,"owner_id":&owner_id})
                    .session(&mut *session).await?
                    .ok_or_else(||AppError::NotFound("machine operation not found".into()))?;
                if let Some(previous) = &operation.delete_retry
                    && previous.last_request_key_hash.as_deref() == Some(&key_hash)
                {
                    if previous.last_request_digest.as_deref() != Some(&digest) {
                        return Err(AppError::Conflict("idempotency_key_reused_with_different_request".into()));
                    }
                    return Ok(operation);
                }
                let at = Utc::now();
                if !stopped(&operation, at) || operation.retry_count != expected_retry_count {
                    return Err(AppError::Conflict("delete_retry_state_conflict".into()));
                }
                let machine = db_owned.collection::<ManagedMachine>(MACHINES)
                    .find_one(doc! {"_id":&operation.machine_id,"owner_id":&owner_id})
                    .session(&mut *session).await?
                    .ok_or_else(||AppError::NotFound("managed machine not found".into()))?;
                if machine.revision != expected_revision || machine.resource_state != ResourceState::Deleting
                    || machine.writer_admitted || machine.active_operation_id.as_deref() != Some(&operation_id)
                    || !lifecycle::has_owner_deletion_basis(&machine)
                {
                    return Err(AppError::Conflict("managed_machine_revision_conflict".into()));
                }
                let changed = db_owned.collection::<Document>(MACHINES).update_one(
                    doc! {"_id":&machine.node_id,"owner_id":&owner_id,"revision":expected_revision,
                        "resource_state":"deleting","writer_admitted":false,"active_operation_id":&operation_id},
                    doc! {"$inc":{"revision":1_i64},"$set":{"updated_at":bson::DateTime::from_chrono(at)}},
                ).session(&mut *session).await?;
                if changed.matched_count != 1 {
                    return Err(AppError::Conflict("managed_machine_revision_conflict".into()));
                }
                let state = DeleteRetryState { attempts:0, claim_id:None, claim_until:None,
                    last_request_key_hash:Some(key_hash.clone()), last_request_digest:Some(digest.clone()) };
                let resumed = db_owned.collection::<MachineOperation>(OPERATIONS).find_one_and_update(
                    doc! {"_id":&operation_id,"owner_id":&owner_id,"retry_count":expected_retry_count,
                        "action":"delete","terminal_at":Bson::Null,"state":{"$in":["accepted","applying","reconciling"]}},
                    doc! {"$set":{"delete_retry":bson::to_bson(&state).map_err(|_|AppError::Internal("deletion retry encoding failed".into()))?,
                        "next_attempt_at":bson::DateTime::from_chrono(at),"updated_at":bson::DateTime::from_chrono(at)}},
                ).return_document(ReturnDocument::After).session(&mut *session).await?
                    .ok_or_else(||AppError::Conflict("delete_retry_state_conflict".into()))?;
                lifecycle::append_owner_audit(&db_owned, session, &owner_id,
                    "remote_machine_delete_retried", &machine.node_id, &operation_id).await?;
                Ok(resumed)
            }.await;
            api_key_mutation_service::transaction_result(result)
        }).await.map_err(api_key_mutation_service::map_transaction_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation(total: i64) -> MachineOperation {
        bson::from_document(doc! {
            "_id":Uuid::new_v4().to_string(),"owner_id":Uuid::new_v4().to_string(),
            "machine_id":Uuid::new_v4().to_string(),"action":"delete",
            "idempotency_key_hash":"original-key-hash","request_digest":"original-body-digest",
            "state":"reconciling","cleanup_state":"pending","retry_count":total,
            "next_attempt_at":bson::DateTime::now(),"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        }).unwrap()
    }

    #[test]
    fn legacy_exhaustion_and_manual_round_preserve_lifetime_count_in_safe_view() {
        for (total, exhausted) in [(0, false), (9, false), (10, true), (15_639, true)] {
            let op = operation(total);
            assert!(op.delete_retry.is_none());
            assert_eq!(stopped(&op, Utc::now()), exhausted);
            let view = lifecycle::safe_operation_view(&op);
            assert_eq!(view.retry_count, total);
            assert_eq!(view.delete_retry_attempts, Some(total.min(LIMIT)));
            assert_eq!(view.delete_retry_limit, Some(LIMIT));
            assert_eq!(view.delete_retry_stopped, exhausted);
            assert_eq!(view.next_attempt_at.is_none(), exhausted);
        }
        let mut renewed = operation(15_639);
        renewed.delete_retry = Some(DeleteRetryState {
            attempts: 0,
            claim_id: None,
            claim_until: None,
            last_request_key_hash: Some("private-retry-key-hash".into()),
            last_request_digest: Some("private-retry-digest".into()),
        });
        assert!(!stopped(&renewed, Utc::now()));
        let view = lifecycle::safe_operation_view(&renewed);
        assert_eq!(view.retry_count, 15_639);
        assert_eq!(view.delete_retry_attempts, Some(0));
        let public = serde_json::to_string(&view).unwrap();
        assert!(!public.contains("private-retry"));
        assert!(!public.contains("delete_retry\""));
    }

    #[test]
    fn last_admitted_attempt_stops_only_after_claim_finishes_or_expires() {
        let at = Utc::now();
        let mut op = operation(10);
        op.delete_retry = Some(DeleteRetryState {
            attempts: 10,
            claim_id: Some(Uuid::new_v4().to_string()),
            claim_until: Some(at + chrono::Duration::seconds(60)),
            last_request_key_hash: None,
            last_request_digest: None,
        });
        assert!(!stopped(&op, at));
        assert!(stopped(&op, at + chrono::Duration::seconds(61)));
        let stored = bson::to_document(op.delete_retry.as_ref().unwrap()).unwrap();
        assert!(stored.get_datetime("claim_until").is_ok());
        assert_eq!(
            bson::from_document::<DeleteRetryState>(stored)
                .unwrap()
                .attempts,
            10
        );
        op.delete_retry.as_mut().unwrap().claim_until = None;
        assert!(stopped(&op, at));
        op.state = OperationState::Succeeded;
        op.terminal_at = Some(at);
        assert!(!stopped(&op, at));
        op.state = OperationState::Reconciling;
        op.terminal_at = None;
        op.action = LifecycleAction::Pause;
        assert!(!stopped(&op, at));
        let view = lifecycle::safe_operation_view(&op);
        assert_eq!(view.delete_retry_limit, None);
        assert_eq!(view.delete_retry_attempts, None);
    }
}
