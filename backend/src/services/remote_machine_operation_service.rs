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
        remote_machine::{COLLECTION_NAME as MACHINES, ManagedMachine, ResourceState},
        remote_machine_operation::{
            COLLECTION_NAME as OPERATIONS, MachineOperation, OperationState,
        },
        remote_machine_outbox::{COLLECTION_NAME as OUTBOX, EffectState, MachineEffectOutbox},
    },
};

const RESPONSIBILITY_BATCH: i64 = 64;
const RESPONSIBILITY_QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

fn running_effect_filter() -> Document {
    doc! {"kind":{"$in":["create_sandbox","resume_sandbox","rekey_sandbox","reconcile_sandbox","update_sandbox_deadline"]},
    "$or":[{"desired_mode":"running"},{"kind":"rekey_sandbox"}]}
}

fn unattempted_effect_filter() -> Document {
    let mut filter = running_effect_filter();
    filter.insert("state", "pending");
    filter.insert("submission", Bson::Null);
    filter.insert("claim_id", Bson::Null);
    filter.insert("claimed_until", Bson::Null);
    filter.insert(
        "$expr",
        doc! {"$in":[{"$type":"$attempts"},["int","long","missing"]]},
    );
    filter.insert(
        "$and",
        vec![doc! {"$or":[{"attempts":0_i64},{"attempts":{"$exists":false}}]}],
    );
    filter
}

/// Seal only original work whose durable delivery history proves it has never
/// been claimed or submitted. A delivery acknowledgement alone is not proof.
/// This bounded sweep runs before ordinary claiming; the same CAS is repeated
/// by the original-delete finality transaction for direct observation callers.
pub(crate) async fn cancel_undispatched_delete_effects(db: &Database) -> AppResult<u64> {
    let candidates: Vec<Document> = db
        .collection::<Document>(OUTBOX)
        .aggregate(vec![
            doc! {"$match":unattempted_effect_filter()},
            doc! {"$lookup":{"from":MACHINES,"let":{"machine":"$machine_id","owner":"$owner_id"},
            "pipeline":[{"$match":{"resource_state":"deleting","writer_admitted":false,
                "$expr":{"$and":[{"$eq":["$_id","$$machine"]},{"$eq":["$owner_id","$$owner"]}]}}},
                {"$project":{"_id":1}}],"as":"deleting_machine"}},
            doc! {"$match":{"deleting_machine.0":{"$exists":true}}},
            doc! {"$sort":{"created_at":1,"_id":1}},
            doc! {"$limit":RESPONSIBILITY_BATCH},
            doc! {"$project":{"machine_id":1,"owner_id":1}},
        ])
        .max_time(RESPONSIBILITY_QUERY_TIMEOUT)
        .await?
        .try_collect()
        .await?;
    let mut visited = std::collections::HashSet::new();
    let mut cancelled = 0;
    for candidate in candidates {
        let (Ok(machine_id), Ok(owner_id)) = (
            candidate.get_str("machine_id"),
            candidate.get_str("owner_id"),
        ) else {
            continue;
        };
        if !visited.insert((machine_id.to_owned(), owner_id.to_owned())) {
            continue;
        }
        let Some(machine) = db
            .collection::<ManagedMachine>(MACHINES)
            .find_one(doc! {"_id":machine_id,"owner_id":owner_id,"resource_state":"deleting"})
            .await?
        else {
            continue;
        };
        let db_owned = db.clone();
        cancelled += db
            .client()
            .start_session()
            .await?
            .start_transaction()
            .write_concern(mongodb::options::WriteConcern::majority())
            .and_run2(async move |session| {
                super::api_key_mutation_service::transaction_result(
                    cancel_unattempted_in_session(&db_owned, session, &machine, None).await,
                )
            })
            .await
            .map_err(super::api_key_mutation_service::map_transaction_error)?;
    }
    Ok(cancelled)
}

/// The current worker may seal its attested fresh claim before any external call.
/// A reclaimed legacy claim has no such attestation and remains unknown. The caller
/// invokes this only on an inert superseded/deleting path, before dispatch.
pub(crate) async fn cancel_undispatched_claim(
    db: &Database,
    machine: &ManagedMachine,
    effect: &MachineEffectOutbox,
) -> AppResult<bool> {
    if effect.attempts != 1
        || effect.state != EffectState::Claimed
        || effect.submission.is_some()
        || effect.claim_id.is_none()
        || effect.undispatched_claim_id != effect.claim_id
    {
        return Ok(false);
    }
    let db_owned = db.clone();
    let machine = machine.clone();
    let effect = effect.clone();
    db.client()
        .start_session()
        .await?
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            super::api_key_mutation_service::transaction_result(
                cancel_unattempted_in_session(&db_owned, session, &machine, Some(&effect))
                    .await
                    .map(|count| count == 1),
            )
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

async fn cancel_unattempted_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    machine: &ManagedMachine,
    current_first_claim: Option<&MachineEffectOutbox>,
) -> AppResult<u64> {
    if machine.resource_state != ResourceState::Deleting
        || machine.writer_admitted
        || !super::remote_machine_service::deletion_authorized_in_session(
            db,
            session,
            &machine.node_id,
        )
        .await?
    {
        return Ok(0);
    }
    if db.collection::<Document>(OPERATIONS).count_documents(doc! {
        "_id":&machine.active_operation_id,"machine_id":&machine.node_id,"owner_id":&machine.owner_id,
        "action":"delete","state":{"$in":["accepted","applying","reconciling"]},"terminal_at":Bson::Null,
    }).limit(1).session(&mut *session).await? != 1 { return Ok(0); }
    let mut filter = if let Some(effect) = current_first_claim {
        let mut filter = running_effect_filter();
        filter.extend(doc! {"_id":&effect.id,"state":"claimed","attempts":1_i64,
            "claim_id":&effect.claim_id,"undispatched_claim_id":&effect.claim_id,"submission":Bson::Null});
        filter
    } else {
        unattempted_effect_filter()
    };
    filter.insert("machine_id", &machine.node_id);
    filter.insert("owner_id", &machine.owner_id);
    let effects: Vec<Document> = db
        .collection::<Document>(OUTBOX)
        .find(filter.clone())
        .projection(doc! {"_id":1})
        .sort(doc! {"created_at":1,"_id":1})
        .limit(RESPONSIBILITY_BATCH)
        .max_time(RESPONSIBILITY_QUERY_TIMEOUT)
        .session(&mut *session)
        .await?
        .stream(&mut *session)
        .try_collect()
        .await?;
    if effects.is_empty() {
        return Ok(0);
    }
    let changed = db.collection::<Document>(MACHINES).update_one(doc! {
        "_id":&machine.node_id,"owner_id":&machine.owner_id,"revision":machine.revision,
        "active_operation_id":&machine.active_operation_id,"runtime_generation":machine.runtime_generation,
        "credential_epoch":machine.credential_epoch,"writer_fence":&machine.writer_fence,
        "resource_state":"deleting","writer_admitted":false,"native_uncertainty":&machine.native_uncertainty,
        "deletion_basis":mongodb::bson::to_bson(&machine.deletion_basis)
            .map_err(|_|AppError::Internal("submission cancellation encoding failed".into()))?,
    },doc! {"$inc":{"submission_evidence_fence":1_i64}}).session(&mut *session).await?;
    if changed.matched_count != 1 {
        return Ok(0);
    }
    let ids: Vec<Bson> = effects
        .iter()
        .filter_map(|effect| effect.get("_id").cloned())
        .collect();
    filter.insert("_id", doc! {"$in":ids});
    let now = mongodb::bson::DateTime::now();
    let result = db.collection::<Document>(OUTBOX).update_many(filter, vec![doc! {"$set":{
        "submission":{"request_id":"$_id","runtime_generation":machine.runtime_generation,
            "credential_epoch":machine.credential_epoch,"writer_fence":{"$literal":&machine.writer_fence},
            "started_at":now,"completed_at":now,"cancelled_before_dispatch":true,
            "result_sandbox_uid":Bson::Null,"result_resource_version":Bson::Null,"dispatches":[]},
        "state":"applied","claim_id":Bson::Null,"undispatched_claim_id":Bson::Null,
        "claimed_until":Bson::Null,"available_at":Bson::Null,
        "last_error":Bson::Null,"updated_at":now,
    }}]).session(&mut *session).await?;
    Ok(result.modified_count)
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<Document>(OUTBOX)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"machine_id":1,"owner_id":1,"kind":1,"desired_mode":1})
                .options(
                    IndexOptions::builder()
                        .name("machine_creation_responsibility_v1".to_owned())
                        .build(),
                )
                .build(),
        )
        .await?;
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

/// Persist the original external request before I/O under the same lifecycle
/// write fence as owner delete. A lease timeout never completes this record.
pub(crate) async fn begin_submission(
    db: &Database,
    machine: &crate::models::remote_machine::ManagedMachine,
    effect: &MachineEffectOutbox,
) -> AppResult<String> {
    let db = db.clone();
    let machine = machine.clone();
    let effect = effect.clone();
    let dispatch_id = Uuid::new_v4().to_string();
    db.client().start_session().await?.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move |session| {
            let result:AppResult<String>=async {
                let changed=db.collection::<Document>(crate::models::remote_machine::COLLECTION_NAME).update_one(
                    doc!{"_id":&machine.node_id,"revision":machine.revision,"runtime_generation":machine.runtime_generation,
                        "credential_epoch":machine.credential_epoch,"writer_fence":&machine.writer_fence,
                        "active_operation_id":&effect.operation_id,"resource_state":{"$nin":["deleting","deleted","stopping","paused"]}},
                    doc!{"$inc":{"submission_evidence_fence":1_i64}}).session(&mut *session).await?;
                if changed.matched_count!=1{return Err(AppError::Conflict("runtime_submission_fenced".into()));}
                let existing=db.collection::<Document>(OUTBOX).find_one(doc!{"_id":&effect.id,
                    "operation_id":&effect.operation_id,"machine_id":&machine.node_id}).session(&mut *session)
                    .await?.ok_or_else(||AppError::Conflict("runtime_submission_identity_or_capacity_conflict".into()))?;
                let mut dispatches=Vec::new();
                if let Some(value)=existing.get("submission").filter(|value|!matches!(value,Bson::Null)) {
                    let old:crate::models::remote_machine_outbox::MachineSubmission=mongodb::bson::from_bson(value.clone())
                        .map_err(|_|AppError::Conflict("runtime_submission_identity_or_capacity_conflict".into()))?;
                    if old.dispatches.is_empty() {
                        // Preserve the pre-journal request; a newer successful
                        // retry must never hide its unresolved old response.
                        dispatches.push(crate::models::remote_machine_outbox::MachineSubmissionDispatch {
                            id:format!("{}:legacy",effect.id),started_at:old.started_at,completed_at:old.completed_at,
                            cancelled_before_dispatch:old.cancelled_before_dispatch,
                            result_sandbox_uid:old.result_sandbox_uid,result_resource_version:old.result_resource_version,
                        });
                    }
                }
                let submission=crate::models::remote_machine_outbox::MachineSubmission {
                    request_id:effect.id.clone(),runtime_generation:machine.runtime_generation,credential_epoch:machine.credential_epoch,
                    writer_fence:machine.writer_fence.clone(),started_at:Utc::now(),completed_at:None,cancelled_before_dispatch:false,
                    result_sandbox_uid:None,result_resource_version:None,dispatches:Vec::new(),
                };
                db.collection::<Document>(OUTBOX).update_one(doc!{"_id":&effect.id,"operation_id":&effect.operation_id,"machine_id":&machine.node_id,"submission":Bson::Null},
                    doc!{"$set":{"submission":mongodb::bson::to_bson(&submission).map_err(|_|AppError::Internal("submission encoding failed".into()))?,
                        "undispatched_claim_id":Bson::Null}})
                    .session(&mut *session).await?;
                let dispatch=crate::models::remote_machine_outbox::MachineSubmissionDispatch {
                    id:dispatch_id.clone(),started_at:Utc::now(),completed_at:None,cancelled_before_dispatch:false,
                    result_sandbox_uid:None,result_resource_version:None,
                };
                dispatches.push(dispatch);
                // A bounded journal fails closed rather than dropping a request.
                let changed=db.collection::<Document>(OUTBOX).update_one(doc!{"_id":&effect.id,"operation_id":&effect.operation_id,"machine_id":&machine.node_id,
                    "submission.request_id":&effect.id,"submission.runtime_generation":machine.runtime_generation,
                    "submission.credential_epoch":machine.credential_epoch,"submission.writer_fence":&machine.writer_fence,
                    "submission.dispatches.255":{"$exists":false}},
                    doc!{"$set":{"undispatched_claim_id":Bson::Null},"$push":{"submission.dispatches":{"$each":mongodb::bson::to_bson(&dispatches)
                        .map_err(|_|AppError::Internal("submission encoding failed".into()))?}}})
                    .session(&mut *session).await?;
                if changed.matched_count!=1{return Err(AppError::Conflict("runtime_submission_identity_or_capacity_conflict".into()));}
                Ok(dispatch_id.clone())
            }.await;super::api_key_mutation_service::transaction_result(result)
        }).await.map_err(super::api_key_mutation_service::map_transaction_error)
}

/// Record one original dispatch even when owner delete won during the request.
/// An outcome never completes a different retry or certifies CSI terminality.
pub(crate) async fn finish_submission(
    db: &Database,
    effect_id: &str,
    dispatch_id: &str,
    observation: Option<&nyxid_machine_updater::sandbox::SandboxObservation>,
) -> AppResult<()> {
    let uid = observation.and_then(|o| o.sandbox_uid.as_deref());
    let mut dispatch_match = doc! {"id":dispatch_id,"completed_at":Bson::Null};
    if let Some(uid) = uid {
        dispatch_match.insert(
            "$or",
            vec![
                doc! {"result_sandbox_uid":Bson::Null},
                doc! {"result_sandbox_uid":uid},
            ],
        );
    }
    let changed=db.collection::<Document>(OUTBOX).update_one(doc!{"_id":effect_id,"submission.request_id":effect_id,"submission.dispatches":{"$elemMatch":dispatch_match}},
        doc!{"$set":{"submission.dispatches.$.completed_at":mongodb::bson::DateTime::now(),
        "submission.dispatches.$.cancelled_before_dispatch":observation.is_none(),
        "submission.dispatches.$.result_sandbox_uid":uid,
        "submission.dispatches.$.result_resource_version":observation.and_then(|o|o.resource_version.as_deref())}}).await?;
    if changed.matched_count == 1 {
        return Ok(());
    }
    // A lost acknowledgement may re-record exactly the same result, while
    // preserving the first completed_at upper bound.
    if db.collection::<Document>(OUTBOX).count_documents(doc!{"_id":effect_id,"submission.dispatches":{"$elemMatch":{
        "id":dispatch_id,"completed_at":{"$ne":Bson::Null},"cancelled_before_dispatch":observation.is_none(),
        "result_sandbox_uid":uid,"result_resource_version":observation.and_then(|o|o.resource_version.as_deref())}}}).await?==1 {return Ok(());}
    Err(AppError::Conflict(
        "runtime_submission_result_conflict".into(),
    ))
}

/// Select an unresolved responsibility directly instead of materializing every
/// completed historical renewal. The result count and returned data are bounded;
/// Mongo's maxTimeMS bounds work when a very large history has no unresolved row.
/// BSON types are checked explicitly, including historical dispatch arrays, so
/// malformed rows fail closed without operator type errors.
fn terminal_responsibility_expression(machine: &ManagedMachine) -> Document {
    let result = |completed: &str, cancelled: &str, uid: &str| {
        doc! {"$and":[
            {"$eq":[{"$type":completed},"date"]},
            {"$or":[
                {"$eq":[cancelled,true]},
                {"$and":[
                    {"$eq":[cancelled,false]},
                    {"$eq":[{"$type":uid},"string"]},
                    {"$eq":[uid,{"$literal":&machine.sandbox_uid}]},
                ]},
            ]},
        ]}
    };
    let legacy = result(
        "$submission.completed_at",
        "$submission.cancelled_before_dispatch",
        "$submission.result_sandbox_uid",
    );
    let dispatch_result = result(
        "$$dispatch.completed_at",
        "$$dispatch.cancelled_before_dispatch",
        "$$dispatch.result_sandbox_uid",
    );
    doc! {"$let":{"vars":{"dispatches":{"$cond":[{"$isArray":"$submission.dispatches"},"$submission.dispatches",[]]}},
    "in":{"$and":[
        {"$eq":[{"$type":"$submission"},"object"]},
        {"$eq":[{"$type":"$_id"},"string"]},
        {"$eq":["$submission.request_id","$_id"]},
        {"$in":[{"$type":"$submission.runtime_generation"},["int","long"]]},
        {"$in":[{"$type":"$submission.credential_epoch"},["int","long"]]},
        {"$eq":[{"$type":"$submission.writer_fence"},"string"]},
        {"$ne":["$submission.writer_fence",""]},
        {"$eq":[{"$type":"$submission.started_at"},"date"]},
        {"$or":[
            {"$eq":[{"$type":"$submission.dispatches"},"missing"]},
            {"$isArray":"$submission.dispatches"},
        ]},
        {"$cond":[{"$gt":[{"$size":"$$dispatches"},0]},
            {"$allElementsTrue":[{"$map":{"input":"$$dispatches","as":"dispatch","in":{
                "$and":[{"$eq":[{"$type":"$$dispatch"},"object"]},
                    {"$eq":[{"$type":"$$dispatch.id"},"string"]},
                    {"$eq":[{"$type":"$$dispatch.started_at"},"date"]},
                    {"$ne":["$$dispatch.id",""]},dispatch_result],
            }}}]},legacy]},
    ]}}}
}

pub(crate) async fn creation_responsibilities_terminal_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    machine: &ManagedMachine,
) -> AppResult<bool> {
    cancel_unattempted_in_session(db, session, machine, None).await?;
    let mut filter = running_effect_filter();
    filter.insert("machine_id", &machine.node_id);
    filter.insert("owner_id", &machine.owner_id);
    filter.insert(
        "$expr",
        doc! {"$not":[terminal_responsibility_expression(machine)]},
    );
    Ok(db
        .collection::<Document>(OUTBOX)
        .find_one(filter)
        .projection(doc! {"_id":1})
        .max_time(RESPONSIBILITY_QUERY_TIMEOUT)
        .session(&mut *session)
        .await?
        .is_none())
}

pub(crate) async fn creation_responsibilities_terminal(
    db: &Database,
    machine: &ManagedMachine,
) -> AppResult<bool> {
    let db_owned = db.clone();
    let machine = machine.clone();
    db.client()
        .start_session()
        .await?
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            super::api_key_mutation_service::transaction_result(
                creation_responsibilities_terminal_in_session(&db_owned, session, &machine).await,
            )
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
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
    let claim = Uuid::new_v4().to_string();
    // The pre-update predicate is retained as a claim-specific attestation.
    // Historical Claimed/Retry/Applied rows can never acquire it by replay.
    let unattempted = doc! {"$and":[
        {"$eq":["$state","pending"]},
        {"$in":[{"$type":"$attempts"},["int","long","missing"]]},
        {"$eq":[{"$ifNull":["$attempts",0_i64]},0_i64]},
        {"$eq":[{"$ifNull":["$submission",Bson::Null]},Bson::Null]},
        {"$eq":[{"$ifNull":["$claim_id",Bson::Null]},Bson::Null]},
        {"$eq":[{"$ifNull":["$claimed_until",Bson::Null]},Bson::Null]},
    ]};
    Ok(db
        .collection::<MachineEffectOutbox>(OUTBOX)
        .find_one_and_update(
            filter,
            vec![doc! {"$set":{
                "undispatched_claim_id":{"$cond":[unattempted,{"$literal":&claim},Bson::Null]},
                "state":"claimed","claim_id":{"$literal":&claim},
                "claimed_until":mongodb::bson::DateTime::from_chrono(now+lease),
                "updated_at":mongodb::bson::DateTime::from_chrono(now),
                "attempts":{"$add":[{"$ifNull":["$attempts",0_i64]},1_i64]},
            }}],
        )
        .sort(doc! {"created_at":1,"_id":1})
        .return_document(ReturnDocument::After)
        .await?)
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
    let mut update = doc! {"$set": {"state": state, "last_error": error, "claimed_until": Bson::Null, "claim_id": Bson::Null,
    "undispatched_claim_id":Bson::Null,"updated_at": mongodb::bson::DateTime::from_chrono(now)}};
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

#[cfg(test)]
mod responsibility_tests {
    use super::*;
    use crate::models::remote_machine::{MachineDeletionBasis, MachineDeletionSource};

    fn machine() -> ManagedMachine {
        super::super::remote_machine_service::CreateMachineInput::fixed(
            &Uuid::new_v4().to_string(),
            true,
            &format!("registry.test/node@sha256:{}", "a".repeat(64)),
        )
        .preview(&Uuid::new_v4().to_string(), Utc::now())
    }

    fn complete_effect(machine: &ManagedMachine, id: &str) -> Document {
        doc! {"_id":id,"operation_id":Uuid::new_v4().to_string(),"machine_id":&machine.node_id,
        "owner_id":&machine.owner_id,"kind":"update_sandbox_deadline","desired_mode":"running",
        "state":"applied","attempts":1_i64,"created_at":mongodb::bson::DateTime::now(),
        "submission":{"request_id":id,"runtime_generation":1_i64,"credential_epoch":1_i64,
            "writer_fence":&machine.writer_fence,"started_at":mongodb::bson::DateTime::now(),
            "completed_at":mongodb::bson::DateTime::now(),"cancelled_before_dispatch":false,
            "result_sandbox_uid":&machine.sandbox_uid,"result_resource_version":"17","dispatches":[]}}
    }

    #[tokio::test]
    async fn remote_machine_service_responsibility_history_has_no_lifetime_cap_and_unknown_stays_blocking()
     {
        let db =
            crate::test_utils::connect_transaction_test_database("rm_responsibility_history").await;
        ensure_indexes(&db).await.unwrap();
        let mut machine = machine();
        machine.sandbox_uid = Some(Uuid::new_v4().to_string());
        let effects: Vec<Document> = (0..300)
            .map(|_| complete_effect(&machine, &Uuid::new_v4().to_string()))
            .collect();
        db.collection::<Document>(OUTBOX)
            .insert_many(effects)
            .await
            .unwrap();
        assert!(
            creation_responsibilities_terminal(&db, &machine)
                .await
                .unwrap(),
            "more than 256 completed original effects must not permanently block deletion"
        );
        let unknown_id = Uuid::new_v4().to_string();
        let mut unknown = complete_effect(&machine, &unknown_id);
        unknown.insert("submission", Bson::Null);
        db.collection::<Document>(OUTBOX)
            .insert_one(unknown)
            .await
            .unwrap();
        assert!(
            !creation_responsibilities_terminal(&db, &machine)
                .await
                .unwrap(),
            "Applied plus any bound Sandbox is not external-request terminal evidence"
        );
        for bad_submission in [
            Bson::String("invalid".into()),
            Bson::Document(doc! {"request_id":&unknown_id,"dispatches":"invalid"}),
            Bson::Document(
                doc! {"request_id":&unknown_id,"runtime_generation":1_i64,"credential_epoch":1_i64,
                "writer_fence":&machine.writer_fence,"started_at":mongodb::bson::DateTime::now(),
                "dispatches":[{"id":"dispatch","started_at":mongodb::bson::DateTime::now(),
                    "completed_at":Bson::Null,"cancelled_before_dispatch":false,
                    "result_sandbox_uid":&machine.sandbox_uid}]},
            ),
        ] {
            db.collection::<Document>(OUTBOX)
                .update_one(
                    doc! {"_id":&unknown_id},
                    doc! {"$set":{"submission":bad_submission}},
                )
                .await
                .unwrap();
            assert!(
                !creation_responsibilities_terminal(&db, &machine)
                    .await
                    .unwrap(),
                "malformed or unresolved original requests must remain unknown"
            );
        }
        let mut complete = complete_effect(&machine, &unknown_id);
        let terminal = complete.remove("submission").unwrap();
        db.collection::<Document>(OUTBOX)
            .update_one(
                doc! {"_id":&unknown_id},
                doc! {"$set":{"submission":terminal}},
            )
            .await
            .unwrap();
        assert!(
            creation_responsibilities_terminal(&db, &machine)
                .await
                .unwrap()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn remote_machine_service_cancellation_requires_attested_non_dispatch_not_legacy_delivery()
     {
        let db =
            crate::test_utils::connect_transaction_test_database("rm_responsibility_cancel").await;
        ensure_indexes(&db).await.unwrap();
        let mut machine = machine();
        machine.resource_state = ResourceState::Deleting;
        machine.active_operation_id = Some(Uuid::new_v4().to_string());
        machine.deletion_basis = Some(MachineDeletionBasis {
            owner_id: machine.owner_id.clone(),
            source: MachineDeletionSource::MachineOwner,
            policy_version: machine.data_policy_version.clone(),
            accepted_at: Utc::now(),
        });
        db.collection::<ManagedMachine>(MACHINES)
            .insert_one(&machine)
            .await
            .unwrap();
        db.collection::<Document>(OPERATIONS).insert_one(doc! {"_id":&machine.active_operation_id,
            "machine_id":&machine.node_id,"owner_id":&machine.owner_id,"action":"delete","state":"accepted",
            "terminal_at":Bson::Null}).await.unwrap();
        let pending_id = Uuid::new_v4().to_string();
        let mut pending = complete_effect(&machine, &pending_id);
        pending.insert("state", "pending");
        pending.remove("attempts");
        pending.insert("submission", Bson::Null);
        db.collection::<Document>(OUTBOX)
            .insert_one(pending.clone())
            .await
            .unwrap();
        assert_eq!(cancel_undispatched_delete_effects(&db).await.unwrap(), 1);
        let sealed = db
            .collection::<Document>(OUTBOX)
            .find_one(doc! {"_id":&pending_id})
            .await
            .unwrap()
            .unwrap();
        assert!(
            sealed
                .get_document("submission")
                .unwrap()
                .get_bool("cancelled_before_dispatch")
                .unwrap()
        );
        assert!(
            creation_responsibilities_terminal(&db, &machine)
                .await
                .unwrap()
        );
        // A historical Claimed row with attempts=0 is not a first-claim proof.
        let legacy_id = Uuid::new_v4().to_string();
        pending.insert("_id", &legacy_id);
        pending.insert("operation_id", Uuid::new_v4().to_string());
        pending.insert("state", "claimed");
        pending.insert("attempts", 0_i64);
        pending.insert("claim_id", "legacy-claim");
        pending.insert(
            "claimed_until",
            mongodb::bson::DateTime::from_chrono(Utc::now() - ChronoDuration::hours(1)),
        );
        // The real claim decoder requires the rest of the durable effect shape.
        pending.extend(
            doc! {"effect_digest":Uuid::new_v4().to_string(),"namespace":"nyxid-machine",
            "sandbox_name":&machine.sandbox_name,"updated_at":mongodb::bson::DateTime::now()},
        );
        db.collection::<Document>(OUTBOX)
            .insert_one(pending)
            .await
            .unwrap();
        let claimed = claim_effect(&db, Utc::now(), std::time::Duration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.id, legacy_id);
        assert_eq!(claimed.attempts, 1);
        assert!(claimed.undispatched_claim_id.is_none());
        assert!(
            !cancel_undispatched_claim(&db, &machine, &claimed)
                .await
                .unwrap()
        );
        acknowledge_effect(
            &db,
            &claimed.id,
            claimed.claim_id.as_deref().unwrap(),
            true,
            None,
            Utc::now(),
        )
        .await
        .unwrap();
        assert!(
            !creation_responsibilities_terminal(&db, &machine)
                .await
                .unwrap(),
            "claim/ack delivery changes cannot discharge legacy uncertainty"
        );
        db.drop().await.unwrap();
    }
}
