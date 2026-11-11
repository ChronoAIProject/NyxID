//! Managed Machine evidence belongs to the existing signed lifecycle. A queued
//! effect cannot prove cleanup. Admin removal retains workspace data and closes
//! execution; the owner policy alone authorizes irreversible deletion.
use super::*;
use crate::models::remote_machine::{AuthorizationState, ManagedMachine, ResourceState};

pub(super) enum Pass {
    Complete,
    Retained,
    Pending,
    Progress(String),
    Action,
}

fn stopped_for_retention(machine: &ManagedMachine) -> bool {
    machine.authorization_state == AuthorizationState::Revoked
        && !machine.writer_admitted
        && machine.resource_state == ResourceState::Paused
        && (super::super::remote_machine_runtime::has_never_submitted_stop(machine)
            || (super::super::remote_machine_native::predecessor_stopped(machine)
                && machine
                    .native_observation
                    .as_ref()
                    .is_some_and(|observation| {
                        machine.active_operation_id.as_deref()
                            == Some(observation.operation_id.as_str())
                    })))
}

// Main #1927 stops a nonterminal delete after ten admitted recovery attempts,
// even when its outbox is not dead. Read its additive durable shape directly so
// this adapter also works before that model addition is integrated.
fn stopped_delete(operation: &Document, at: bson::DateTime) -> AppResult<bool> {
    if operation.get_str("action") != Ok("delete")
        || !matches!(
            operation.get_str("state"),
            Ok("accepted" | "applying" | "reconciling")
        )
        || operation
            .get("terminal_at")
            .is_some_and(|v| v != &bson::Bson::Null)
    {
        return Ok(false);
    }
    let invalid = || AppError::Internal("Invalid Machine delete retry state".into());
    let retry = match operation.get("delete_retry") {
        None | Some(bson::Bson::Null) => None,
        Some(bson::Bson::Document(retry)) => Some(retry),
        _ => return Err(invalid()),
    };
    let attempts = retry
        .and_then(|retry| retry.get("attempts"))
        .or_else(|| operation.get("retry_count"));
    let attempts = match attempts {
        None => 0,
        Some(bson::Bson::Int64(value)) => *value,
        Some(bson::Bson::Int32(value)) => i64::from(*value),
        _ => return Err(invalid()),
    };
    let live = match retry.and_then(|retry| retry.get("claim_until")) {
        None | Some(bson::Bson::Null) => false,
        Some(bson::Bson::DateTime(until)) => *until > at,
        _ => return Err(invalid()),
    };
    Ok(attempts >= 10 && !live)
}

/// Explicit authenticated erasure-domain retry; callers run this with the job
/// transition and chained audit in the same transaction. It never creates a new
/// delete, changes its acceptance basis, or resets its lifetime retry count.
pub(super) async fn retry_in_session(
    db: &Database,
    job: &PersonalErasureJob,
    tx: &mut mongodb::ClientSession,
) -> AppResult<()> {
    use futures::TryStreamExt;
    let after = job
        .domains
        .iter()
        .find(|d| d.domain == "machine")
        .and_then(|d| d.cursor.as_deref())
        .map(serde_json::from_str::<Progress>)
        .transpose()
        .map_err(|_| AppError::Internal("Invalid Machine erasure cursor".into()))?
        .unwrap_or_default()
        .after;
    let mut cursor = db
        .collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
        .find(doc! {"owner_id":&job.user_id,"_id":{"$gt":after}})
        .sort(doc! {"_id":1})
        .limit(32)
        .session(&mut *tx)
        .await?;
    let machines: Vec<Document> = cursor.stream(&mut *tx).try_collect().await?;
    for raw in machines {
        let machine: ManagedMachine = bson::from_document(raw.clone())
            .map_err(|_| AppError::Internal("Invalid Machine lifecycle storage".into()))?;
        let Some(id) = machine.active_operation_id.as_deref() else {
            continue;
        };
        let Some(operation) = db
            .collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
            .find_one(doc! {"_id":id,"machine_id":&machine.node_id,"owner_id":&job.user_id})
            .session(&mut *tx)
            .await?
        else {
            continue;
        };
        if !stopped_delete(&operation, bson::DateTime::now())? {
            continue;
        }
        if job.owner_deletion_policy.as_deref()
            != Some(super::super::admin_user_service::ACCOUNT_DELETION_POLICY_VERSION)
            || machine.resource_state != ResourceState::Deleting
            || machine.writer_admitted
            || machine.authorization_state != AuthorizationState::Revoked
            || !super::super::remote_machine_service::has_owner_deletion_basis(&machine)
        {
            return Err(AppError::Conflict(
                "Machine erasure retry requires the original accepted deletion basis".into(),
            ));
        }
        let changed = db
            .collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
            .update_one(
                super::super::record_storage::exact_document_predicate(&raw),
                vec![doc! {"$set":{
                "revision":{"$add":["$revision",1_i64]},"updated_at":"$$NOW"}}],
            )
            .session(&mut *tx)
            .await?;
        if changed.matched_count != 1 {
            return Err(AppError::Conflict(
                "Machine changed before erasure retry".into(),
            ));
        }
        let changed = db
            .collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
            .update_one(
                super::super::record_storage::exact_document_predicate(&operation),
                vec![doc! {"$set":{
                "delete_retry":{"$mergeObjects":[{"$ifNull":["$delete_retry",{}]},
                    {"attempts":0_i64,"claim_id":bson::Bson::Null,"claim_until":bson::Bson::Null}]},
                "erasure_retry_round":{"$add":[{"$ifNull":["$erasure_retry_round",0_i64]},1_i64]},
                "next_attempt_at":"$$NOW","updated_at":"$$NOW"}}],
            )
            .session(&mut *tx)
            .await?;
        if changed.matched_count != 1 {
            return Err(AppError::Conflict(
                "Machine delete changed before erasure retry".into(),
            ));
        }
    }
    Ok(())
}

async fn current_dead_effect(db: &Database, machine: &ManagedMachine) -> AppResult<bool> {
    let Some(operation_id) = machine.active_operation_id.as_deref() else {
        return Ok(false);
    };
    let Some(operation) = db
        .collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
        .find_one(
            doc! {"_id":operation_id,"machine_id":&machine.node_id,"owner_id":&machine.owner_id,
            "state":{"$ne":"succeeded"}},
        )
        .await?
    else {
        return Ok(false);
    };
    if stopped_delete(&operation, bson::DateTime::now())? {
        return Ok(true);
    }
    let Ok(effect) = operation.get_str("effect_id") else {
        return Ok(false);
    };
    Ok(db.collection::<Document>(crate::models::remote_machine_outbox::COLLECTION_NAME).find_one(
        doc! {"_id":effect,"operation_id":operation_id,"machine_id":&machine.node_id,"owner_id":&machine.owner_id,"state":"dead"}).await?.is_some())
}

pub(super) async fn pass(state: &crate::AppState, job: &PersonalErasureJob) -> AppResult<Pass> {
    let db = super::super::transaction_service::durable_database(&state.db);
    super::super::remote_machine_service::suspend_erasing_owner(
        &db,
        state.billing.clone(),
        &job.user_id,
    )
    .await?;
    let cursor = job
        .domains
        .iter()
        .find(|d| d.domain == "machine")
        .and_then(|d| d.cursor.as_deref());
    inspect_page(
        &db,
        &job.user_id,
        job.owner_deletion_policy.is_some(),
        cursor,
    )
    .await
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Progress {
    after: String,
    retained: bool,
}
async fn inspect_page(
    db: &Database,
    owner: &str,
    destroy: bool,
    cursor: Option<&str>,
) -> AppResult<Pass> {
    use futures::TryStreamExt;
    let mut progress: Progress = cursor
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| AppError::Internal("Invalid Machine erasure cursor".into()))?
        .unwrap_or_default();
    // Historical deleted machines are not bounded by the active-machine quota.
    // Lifecycle owner admission prevents new machines after accepted erasure.
    let machines: Vec<ManagedMachine> = db
        .collection::<ManagedMachine>(crate::models::remote_machine::COLLECTION_NAME)
        .find(doc! {"owner_id":owner,"_id":{"$gt":&progress.after}})
        .sort(doc! {"_id":1})
        .limit(32)
        .await?
        .try_collect()
        .await?;
    let count = machines.len();
    for machine in machines {
        progress.after = machine.node_id.clone();
        if machine.resource_state == ResourceState::Deleted {
            if machine.cleanup_state != "complete"
                || machine
                    .cleanup_evidence_digest
                    .as_deref()
                    .is_none_or(str::is_empty)
                || machine.authorization_state != AuthorizationState::Revoked
                || machine.writer_admitted
            {
                return Ok(Pass::Pending);
            }
            continue;
        }
        if current_dead_effect(db, &machine).await? {
            return Ok(Pass::Action);
        }
        if destroy || !stopped_for_retention(&machine) {
            return Ok(Pass::Pending);
        }
        let Some(operation) = machine.active_operation_id.as_deref() else {
            return Ok(Pass::Pending);
        };
        if db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME).find_one(
            doc! {"_id":operation,"machine_id":&machine.node_id,"owner_id":owner,"state":"succeeded"}).await?.is_none() {return Ok(Pass::Pending);}
        progress.retained = true;
    }
    if count == 32 {
        return Ok(Pass::Progress(serde_json::to_string(&progress).map_err(
            |_| AppError::Internal("Machine erasure cursor encoding failed".into()),
        )?));
    }
    Ok(if progress.retained {
        Pass::Retained
    } else {
        Pass::Complete
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::remote_machine::{NativeRuntimeObservation, RuntimeEvidence};
    fn machine() -> ManagedMachine {
        let mut machine = super::super::super::remote_machine_service::CreateMachineInput::fixed(
            "owner",
            true,
            "sha256:image",
        )
        .preview("machine", chrono::Utc::now());
        machine.resource_state = ResourceState::Paused;
        machine.authorization_state = AuthorizationState::Revoked;
        machine.writer_admitted = false;
        machine.runtime_generation = 3;
        machine.credential_epoch = 5;
        machine.writer_fence = "fence".into();
        machine.active_operation_id = Some("stop-operation".into());
        machine.sandbox_uid = Some("sandbox".into());
        machine.pod_uid = Some("pod".into());
        machine
    }
    #[test]
    fn exhausted_delete_waits_for_its_live_claim_and_honors_round_budget() {
        let at = bson::DateTime::now();
        let mut operation = doc! {"action":"delete","state":"reconciling","retry_count":10_i64};
        assert!(stopped_delete(&operation, at).unwrap());
        operation.insert("delete_retry",doc! {"attempts":10_i64,"claim_until":bson::DateTime::from_millis(at.timestamp_millis()+60_000)});
        assert!(!stopped_delete(&operation, at).unwrap());
        operation.insert(
            "delete_retry",
            doc! {"attempts":0_i64,"claim_until":bson::Bson::Null},
        );
        assert!(!stopped_delete(&operation, at).unwrap());
        operation.insert("delete_retry", doc! {"attempts":10_i64});
        operation.insert("state", "succeeded");
        assert!(!stopped_delete(&operation, at).unwrap());
    }

    #[tokio::test]
    async fn exhausted_delete_without_dead_outbox_is_actionable_and_operator_retry_retains_accepted_basis()
     {
        let (db, owner) = super::super::tests::fixture().await;
        let policy = super::super::super::admin_user_service::ACCOUNT_DELETION_POLICY_VERSION;
        let accepted = super::super::accept(&db, &owner, &owner, Some(policy))
            .await
            .unwrap();
        let mut machine = machine();
        machine.owner_id = owner.clone();
        machine.resource_state = ResourceState::Deleting;
        machine.deletion_basis = Some(crate::models::remote_machine::MachineDeletionBasis {
            owner_id: owner.clone(),
            source: crate::models::remote_machine::MachineDeletionSource::AccountOwner,
            policy_version: policy.to_owned(),
            accepted_at: chrono::Utc::now(),
        });
        let original_basis = bson::to_bson(&machine.deletion_basis).unwrap();
        db.collection::<ManagedMachine>(crate::models::remote_machine::COLLECTION_NAME)
            .insert_one(&machine)
            .await
            .unwrap();
        let original = doc! {"_id":"stop-operation","machine_id":&machine.node_id,"owner_id":&owner,
        "action":"delete","state":"reconciling","retry_count":10_i64,"delete_retry":{"attempts":10_i64},
        "request_digest":"original-accepted-delete","idempotency_key_hash":"original-request","expected_revision":machine.revision};
        db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
            .insert_one(&original)
            .await
            .unwrap();
        assert!(matches!(
            inspect_page(&db, &owner, true, None).await.unwrap(),
            Pass::Action
        ));
        let mut job = super::super::status(&db, &owner).await.unwrap().unwrap();
        job.status = ErasureStatus::OperatorAction;
        job.domains
            .iter_mut()
            .find(|d| d.domain == "machine")
            .unwrap()
            .status = DomainStatus::OperatorAction;
        db.collection::<Document>(crate::models::personal_erasure::COLLECTION_NAME).update_one(doc! {"_id":&accepted.id},
            doc! {"$set":{"status":"operator_action","domains":bson::to_bson(&job.domains).unwrap()}}).await.unwrap();
        let operator = uuid::Uuid::new_v4().to_string();
        let retry_request = super::super::worker::DomainRetryRequest {
            domain: "machine".into(),
            job_id: job.id.clone(),
            expected_retry_generation: 0,
            idempotency_key: uuid::Uuid::new_v4().to_string(),
        };
        super::super::worker::retry_domain(&db, &operator, &owner, &retry_request)
            .await
            .unwrap();
        let resumed = db
            .collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
            .find_one(doc! {"_id":"stop-operation"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resumed.get_i64("retry_count").unwrap(), 10);
        assert_eq!(
            resumed
                .get_document("delete_retry")
                .unwrap()
                .get_i64("attempts")
                .unwrap(),
            0
        );
        for field in [
            "_id",
            "owner_id",
            "machine_id",
            "action",
            "state",
            "request_digest",
            "idempotency_key_hash",
            "expected_revision",
        ] {
            assert_eq!(resumed.get(field), original.get(field));
        }
        let retained = db
            .collection::<ManagedMachine>(crate::models::remote_machine::COLLECTION_NAME)
            .find_one(doc! {"_id":&machine.node_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            bson::to_bson(&retained.deletion_basis).unwrap(),
            original_basis
        );
        assert_eq!(retained.revision, machine.revision + 1);
        assert_eq!(retained.resource_state, ResourceState::Deleting);
        assert!(!retained.writer_admitted);
        assert!(matches!(
            inspect_page(&db, &owner, true, None).await.unwrap(),
            Pass::Pending
        ));
        let another_request = super::super::worker::DomainRetryRequest {
            domain: "machine".into(),
            job_id: job.id.clone(),
            expected_retry_generation: 0,
            idempotency_key: uuid::Uuid::new_v4().to_string(),
        };
        assert!(
            super::super::worker::retry_domain(&db, &operator, &owner, &another_request)
                .await
                .is_err()
        );
        // A delayed copy of the first authenticated request arrives after the
        // renewed budget is exhausted. It must never authorize a third round.
        db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
            .update_one(
                doc! {"_id":"stop-operation"},
                doc! {"$set":{"retry_count":20_i64,"delete_retry.attempts":10_i64}},
            )
            .await
            .unwrap();
        let mut exhausted = super::super::status(&db, &owner).await.unwrap().unwrap();
        exhausted.status = ErasureStatus::OperatorAction;
        exhausted
            .domains
            .iter_mut()
            .find(|d| d.domain == "machine")
            .unwrap()
            .status = DomainStatus::OperatorAction;
        db.collection::<Document>(crate::models::personal_erasure::COLLECTION_NAME).update_one(doc! {"_id":&job.id},doc! {"$set":{"status":"operator_action","domains":bson::to_bson(&exhausted.domains).unwrap()}}).await.unwrap();
        let stopped_source = db
            .collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
            .find_one(doc! {"_id":"stop-operation"})
            .await
            .unwrap()
            .unwrap();
        super::super::worker::retry_domain(&db, &operator, &owner, &retry_request)
            .await
            .unwrap();
        assert_eq!(
            db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
                .find_one(doc! {"_id":"stop-operation"})
                .await
                .unwrap()
                .unwrap(),
            stopped_source
        );
        assert!(
            super::super::worker::retry_domain(&db, &operator, &owner, &another_request)
                .await
                .is_err()
        );
        assert!(matches!(
            inspect_page(&db, &owner, true, None).await.unwrap(),
            Pass::Action
        ));
        let current_request = super::super::worker::DomainRetryRequest {
            domain: "machine".into(),
            job_id: job.id.clone(),
            expected_retry_generation: 1,
            idempotency_key: uuid::Uuid::new_v4().to_string(),
        };
        super::super::worker::retry_domain(&db, &operator, &owner, &current_request)
            .await
            .unwrap();
        assert_eq!(
            super::super::status(&db, &owner)
                .await
                .unwrap()
                .unwrap()
                .domains
                .iter()
                .find(|d| d.domain == "machine")
                .unwrap()
                .retry_generation,
            2
        );
        let user = db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id":&owner})
            .await
            .unwrap()
            .unwrap();
        assert!(
            !user.is_active,
            "Operator recovery does not restore deleted-owner authentication"
        );
        assert_eq!(
            db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }

    #[test]
    fn retention_requires_bound_native_or_never_submitted_evidence() {
        let mut machine = machine();
        machine.native_observation = Some(NativeRuntimeObservation {
            sandbox_uid: "sandbox".into(),
            pod_uid: Some("pod".into()),
            runtime_generation: 3,
            credential_epoch: 5,
            writer_fence: "fence".into(),
            operation_id: "stop-operation".into(),
            sandbox_generation: 2,
            sandbox_resource_version: "rv".into(),
            observed_at: chrono::Utc::now(),
            running: false,
            suspended: true,
            stop_confirmed: true,
            run_not_after: None,
            evidence_ref: "native-proof".into(),
        });
        assert!(stopped_for_retention(&machine));
        machine.credential_epoch += 1;
        assert!(!stopped_for_retention(&machine));
        machine.credential_epoch -= 1;
        machine.active_operation_id = Some("successor-operation".into());
        assert!(!stopped_for_retention(&machine));
        machine.native_observation = None;
        machine.pod_uid = None;
        machine.bootstrap_delivery_id = None;
        machine.runtime_submission_started_at = None;
        machine.last_runtime_evidence = Some(RuntimeEvidence {
            sandbox_uid: "sandbox".into(),
            pod_uid: None,
            runtime_generation: 3,
            credential_epoch: 5,
            writer_fence: "fence".into(),
            ready: false,
            physically_stopped: true,
            observed_at: chrono::Utc::now(),
            source: "lifecycle-never-submitted".into(),
            provider_evidence_ref: None,
        });
        assert!(stopped_for_retention(&machine));
        machine.last_runtime_evidence.as_mut().unwrap().writer_fence = "obsolete".into();
        assert!(!stopped_for_retention(&machine));
        machine.last_runtime_evidence.as_mut().unwrap().writer_fence = "fence".into();
        machine.last_runtime_evidence.as_mut().unwrap().source = "legacy-unverified".into();
        assert!(!stopped_for_retention(&machine));
    }
    #[tokio::test]
    async fn deleted_history_is_walked_in_bounded_resumable_pages() {
        let (db, owner) = super::super::tests::fixture().await;
        let collection =
            db.collection::<ManagedMachine>(crate::models::remote_machine::COLLECTION_NAME);
        for index in 0..70 {
            let mut row = machine();
            row.node_id = format!("machine-{index:04}");
            row.owner_id = owner.clone();
            row.resource_state = ResourceState::Deleted;
            row.cleanup_state = "complete".into();
            row.cleanup_evidence_digest = Some("verified-cleanup".into());
            collection.insert_one(row).await.unwrap();
        }
        let Pass::Progress(first) = inspect_page(&db, &owner, true, None).await.unwrap() else {
            panic!("bounded first page")
        };
        assert_eq!(
            serde_json::from_str::<Progress>(&first).unwrap().after,
            "machine-0031"
        );
        let Pass::Progress(second) = inspect_page(&db, &owner, true, Some(&first)).await.unwrap()
        else {
            panic!("bounded second page")
        };
        assert_eq!(
            serde_json::from_str::<Progress>(&second).unwrap().after,
            "machine-0063"
        );
        assert!(matches!(
            inspect_page(&db, &owner, true, Some(&second))
                .await
                .unwrap(),
            Pass::Complete
        ));
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn reconciled_operation_is_not_blocked_by_historical_dead_effect() {
        let (db, _) = super::super::tests::fixture().await;
        let machine = machine();
        db.collection::<Document>(crate::models::remote_machine_outbox::COLLECTION_NAME).insert_one(doc! {"_id":"old-effect","operation_id":"old-op","machine_id":&machine.node_id,"owner_id":&machine.owner_id,"state":"dead"}).await.unwrap();
        db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME).insert_one(doc! {"_id":"stop-operation","machine_id":&machine.node_id,"owner_id":&machine.owner_id,"effect_id":"current-effect","state":"succeeded"}).await.unwrap();
        assert!(!current_dead_effect(&db, &machine).await.unwrap());
        db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
            .update_one(
                doc! {"_id":"stop-operation"},
                doc! {"$set":{"state":"reconciling"}},
            )
            .await
            .unwrap();
        db.collection::<Document>(crate::models::remote_machine_outbox::COLLECTION_NAME).insert_one(doc! {"_id":"current-effect","operation_id":"stop-operation","machine_id":&machine.node_id,"owner_id":&machine.owner_id,"state":"dead"}).await.unwrap();
        assert!(current_dead_effect(&db, &machine).await.unwrap());
        db.drop().await.unwrap();
    }
}
