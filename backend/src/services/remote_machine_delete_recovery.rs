//! Owner-authorized deletion is separate from ordinary runtime authority.
use crate::models::remote_machine::{
    DeleteRecoveryProgress, DeleteStopKind, DeleteStorageDisposition, ManagedMachine, ResourceState,
};

pub(crate) fn basis_digest(machine: &ManagedMachine) -> Option<String> {
    use sha2::{Digest, Sha256};
    let basis = machine.deletion_basis.as_ref()?;
    Some(hex::encode(Sha256::digest(bson::to_vec(basis).ok()?)))
}

pub(crate) const EVIDENCE_VERSION: i64 = 2;
const CONTROL_PLANE_SOURCE: &str = "sandbox-kubernetes-control-plane";

pub(crate) fn binding_matches(machine: &ManagedMachine, progress: &DeleteRecoveryProgress) -> bool {
    progress.evidence_version == EVIDENCE_VERSION && tuple_matches(machine, progress)
}

fn tuple_matches(machine: &ManagedMachine, progress: &DeleteRecoveryProgress) -> bool {
    machine.resource_state == ResourceState::Deleting
        && !machine.writer_admitted
        && lifecycle::has_owner_deletion_basis(machine)
        && machine.owner_id == progress.owner_id
        && machine.active_operation_id.as_deref() == Some(progress.operation_id.as_str())
        && machine.runtime_generation == progress.runtime_generation
        && machine.credential_epoch == progress.credential_epoch
        && machine.writer_fence == progress.writer_fence
        && machine.native_uncertainty == progress.original_uncertainty
        && machine.sandbox_uid == progress.sandbox_uid
        && machine.pod_uid == progress.pod_uid
        && machine.native_node_uid == progress.node_uid
        && machine.pvc_uid == progress.pvc_uid
        && basis_digest(machine).as_deref() == Some(progress.deletion_basis_digest.as_str())
        && progress.evaluated_revision <= machine.revision
        && !progress.evidence_id.is_empty()
}

pub(crate) fn stop_is_valid(machine: &ManagedMachine, progress: &DeleteRecoveryProgress) -> bool {
    if !binding_matches(machine, progress) {
        return false;
    }
    let Some(stop) = &progress.stop else {
        return false;
    };
    if stop.source.is_empty()
        || stop.evidence_ref.is_empty()
        || stop.observed_at > chrono::Utc::now() + chrono::Duration::seconds(1)
    {
        return false;
    }
    if stop.kind != DeleteStopKind::ControlPlaneWorkloadAbsent
        && (stop.sandbox_generation <= 0 || stop.sandbox_resource_version.is_empty())
    {
        return false;
    }
    match stop.kind {
        DeleteStopKind::OrdinaryNative => {
            stop.source == "kubernetes-native"
                && super::remote_machine_native::predecessor_stopped(machine)
                && machine.native_observation.as_ref().is_some_and(|o| {
                    Some(o.operation_id.as_str())
                        == stop
                            .source_operation_id
                            .as_deref()
                            .or(Some(progress.operation_id.as_str()))
                        && o.evidence_ref == stop.evidence_ref
                        && o.observed_at == stop.observed_at
                        && o.sandbox_generation == stop.sandbox_generation
                        && o.sandbox_resource_version == stop.sandbox_resource_version
                })
        }
        DeleteStopKind::ControlPlaneSuspended => {
            matches!(
                machine.native_uncertainty.as_deref(),
                None | Some("predecessor_node_unhealthy_or_stale")
            ) && machine.sandbox_uid.is_some()
                && stop.source == CONTROL_PLANE_SOURCE
                && stop.source_operation_id.as_deref() == Some(progress.operation_id.as_str())
                && uuid::Uuid::parse_str(&stop.evidence_ref).is_ok()
        }
        DeleteStopKind::ControlPlaneWorkloadAbsent => {
            matches!(
                machine.native_uncertainty.as_deref(),
                None | Some("predecessor_node_unhealthy_or_stale")
            ) && machine.sandbox_uid.is_some()
                && stop.source == CONTROL_PLANE_SOURCE
                && stop.source_operation_id.as_deref() == Some(progress.operation_id.as_str())
                && uuid::Uuid::parse_str(&stop.evidence_ref).is_ok()
                && stop.sandbox_generation == 0
                && stop.sandbox_resource_version.is_empty()
                && progress.creation_terminal
        }
        DeleteStopKind::TrustedRuntime => false,
    }
}
pub(crate) fn finality_ready(machine: &ManagedMachine, progress: &DeleteRecoveryProgress) -> bool {
    stop_is_valid(machine, progress)
        && progress.creation_terminal
        && progress.resources_complete
        && storage_is_final(machine, progress)
}
pub(crate) fn storage_is_final(
    machine: &ManagedMachine,
    progress: &DeleteRecoveryProgress,
) -> bool {
    binding_matches(machine, progress)
        && progress.resources_complete
        && progress.storage == DeleteStorageDisposition::SandboxCascadeObserved
}

use super::{
    remote_machine_manager::Manager, remote_machine_operation_service as operations,
    remote_machine_service as lifecycle,
};
use crate::errors::{AppError, AppResult};
use crate::models::remote_machine::{COLLECTION_NAME as MACHINES, DeleteStopEvidence};
use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{self, Document, doc},
    options::ReturnDocument,
};
use nyxid_machine_updater::sandbox::{OwnedResourceObservation, SandboxObservation};

fn delete_projection(machine: &ManagedMachine) -> ManagedMachine {
    let mut projected = machine.clone();
    projected.resource_state = ResourceState::Deleting;
    projected
}

async fn finalized_authorized(manager: &Manager, machine: &ManagedMachine) -> AppResult<()> {
    let progress = machine
        .delete_recovery
        .as_ref()
        .ok_or_else(|| pending("delete_late_cleanup_evidence_missing"))?;
    let projected = delete_projection(machine);
    // A v1 tombstone may be re-observed under v2 without reopening its
    // financial or lifecycle terminal state. Validate only its historical stop
    // here; fresh object inventories must establish the new completion marker.
    let mut historical = progress.clone();
    if historical.evidence_version == 1 {
        historical.evidence_version = EVIDENCE_VERSION;
    }
    if machine.resource_state != ResourceState::Deleted
        || machine.writer_admitted
        || machine.billing_state != crate::models::remote_machine::BillingState::Closed
        || machine.authorization_state != crate::models::remote_machine::AuthorizationState::Revoked
        || !lifecycle::has_owner_deletion_basis(machine)
        || !stop_is_valid(&projected, &historical)
        || machine.cleanup_evidence_digest.as_deref() != Some(progress.evidence_id.as_str())
    {
        return Err(pending("delete_late_cleanup_authority_mismatch"));
    }
    let op = operations::get_operation(&manager.db, &machine.owner_id, &progress.operation_id)
        .await?
        .ok_or_else(|| pending("delete_operation_missing"))?;
    if op.action != crate::models::remote_machine_operation::LifecycleAction::Delete
        || op.machine_id.as_deref() != Some(machine.node_id.as_str())
        || op.state != crate::models::remote_machine_operation::OperationState::Succeeded
        || op.terminal_at.is_none()
    {
        return Err(pending("delete_late_cleanup_operation_mismatch"));
    }
    Ok(())
}

fn pending(code: &str) -> AppError {
    AppError::Conflict(code.into())
}
/// Delete-only projection: upstream text never enters diagnostics or logs.
pub(crate) fn api_pending(machine: &ManagedMachine, error: anyhow::Error) -> AppError {
    use nyxid_machine_updater::sandbox_observation::{
        ObservationErrorKind, SandboxObservationError,
    };
    let (stage, resource_kind, verb, error_kind, http_status, code) = if let Some(error) =
        error.downcast_ref::<SandboxObservationError>()
    {
        let code = match error.error_kind {
            ObservationErrorKind::Authentication => "delete_kubernetes_authentication_failed",
            ObservationErrorKind::Forbidden => "delete_kubernetes_access_denied",
            ObservationErrorKind::Timeout => "delete_kubernetes_observation_timed_out",
            ObservationErrorKind::CredentialUnavailable => {
                "delete_kubernetes_credentials_unavailable"
            }
            ObservationErrorKind::Transport => "delete_kubernetes_transport_failed",
            ObservationErrorKind::HttpStatus => "delete_kubernetes_http_failed",
            ObservationErrorKind::InvalidResponse | ObservationErrorKind::InvalidIdentity => {
                "delete_kubernetes_response_invalid"
            }
            ObservationErrorKind::PaginationIncomplete => "delete_kubernetes_inventory_incomplete",
            ObservationErrorKind::InventoryLimit => "delete_kubernetes_inventory_limit_exceeded",
        };
        (
            error.stage.as_str(),
            error.resource_kind.as_str(),
            error.verb.as_str(),
            error.error_kind.as_str(),
            error.http_status,
            code,
        )
    } else {
        (
            "unclassified",
            "unclassified",
            "unclassified",
            "unclassified",
            None,
            "delete_resource_observation_unknown",
        )
    };
    tracing::warn!(machine_id = %machine.node_id,
        operation_id = machine.active_operation_id.as_deref().unwrap_or("unavailable"),
        stage, resource_kind, verb, error_kind, http_status,
        "Remote Machine Kubernetes observation failed");
    pending(code)
}

pub(crate) fn observation_error_code(error: &AppError) -> Option<&str> {
    let AppError::Conflict(code) = error else {
        return None;
    };
    matches!(
        code.as_str(),
        "delete_kubernetes_authentication_failed"
            | "delete_kubernetes_access_denied"
            | "delete_kubernetes_observation_timed_out"
            | "delete_kubernetes_credentials_unavailable"
            | "delete_kubernetes_transport_failed"
            | "delete_kubernetes_http_failed"
            | "delete_kubernetes_response_invalid"
            | "delete_kubernetes_inventory_incomplete"
            | "delete_kubernetes_inventory_limit_exceeded"
            | "delete_resource_observation_unknown"
    )
    .then_some(code)
}
fn now() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).expect("valid current timestamp")
}
/// Both the outbox and observation scanner enter this same original-delete
/// orchestration. Normal native stop keeps its established control-plane
/// contract; recovery facts never widen ordinary lifecycle authority.
pub(crate) fn advance<'a>(
    manager: &'a Manager,
    machine: &'a ManagedMachine,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(advance_inner(manager, machine))
}

async fn advance_inner(manager: &Manager, snapshot: &ManagedMachine) -> AppResult<()> {
    let mut machine = lifecycle::managed_machine_for_node(&manager.db, &snapshot.node_id)
        .await?
        .ok_or_else(|| pending("delete_machine_missing"))?;
    if machine.resource_state == ResourceState::Deleted {
        // Finalized tombstones retain the original deletion scope. A bounded
        // Manager scan keeps resolving late Kubernetes resources without
        // reopening execution, the operation, accounting or the free slot.
        finalized_authorized(manager, &machine).await?;
        let result = recover_resources(manager, &mut machine, None).await;
        if let Err(error) = &result
            && let Some(code) = observation_error_code(error)
        {
            save_observation_failure(&manager.db, &machine, None, code).await?;
        }
        return result;
    }
    let op_id = machine
        .active_operation_id
        .as_deref()
        .ok_or_else(|| pending("delete_operation_missing"))?;
    let op = operations::get_operation(&manager.db, &machine.owner_id, op_id)
        .await?
        .ok_or_else(|| pending("delete_operation_missing"))?;
    if op.action != crate::models::remote_machine_operation::LifecycleAction::Delete
        || op.machine_id.as_deref() != Some(machine.node_id.as_str())
        || op.terminal_at.is_some()
    {
        return Err(pending("delete_operation_mismatch"));
    }
    if machine.resource_state != ResourceState::Deleting
        || !lifecycle::deletion_authorized(&manager.db, &machine.node_id).await?
    {
        return Err(pending("machine_owner_deletion_basis_required"));
    }
    if op.next_attempt_at.is_some_and(|at| at > Utc::now()) {
        return Ok(());
    }
    let Some(claim_id) = super::remote_machine_delete_retry::claim(&manager.db, &machine).await?
    else {
        return Ok(());
    };
    let op_id = op.id;
    let mut result = tokio::time::timeout(
        super::remote_machine_delete_retry::ATTEMPT_TIMEOUT,
        pending_attempt(manager, &mut machine, &claim_id),
    )
    .await
    .unwrap_or_else(|_| Err(pending("delete_attempt_timed_out")));
    if let Err(error) = &result
        && let Some(code) = observation_error_code(error)
        && let Err(save_error) =
            save_observation_failure(&manager.db, &machine, Some(&claim_id), code).await
    {
        result = Err(save_error);
    }
    super::remote_machine_delete_retry::finish(&manager.db, &op_id, &claim_id).await?;
    result
}

async fn pending_attempt(
    manager: &Manager,
    machine: &mut ManagedMachine,
    claim_id: &str,
) -> AppResult<()> {
    // This path proves local non-submission, never remote absence. Paid zero
    // runtime settlement remains its existing restricted implementation.
    let _ = super::remote_machine_runtime::finalize_never_submitted(
        &manager.db,
        manager.billing.clone(),
        machine,
    )
    .await;
    if lifecycle::finalize_unallocated_delete(&manager.db, &machine.node_id)
        .await?
        .is_some()
    {
        return manager.complete_if_final(machine).await;
    }
    if machine.sandbox_uid.is_none() {
        manager.observe_delete_resource(machine).await?;
    }
    recover_resources(manager, machine, Some(claim_id)).await
}

fn initial_progress(machine: &ManagedMachine) -> AppResult<DeleteRecoveryProgress> {
    if let Some(progress) = &machine.delete_recovery {
        if !tuple_matches(&delete_projection(machine), progress)
            || !matches!(progress.evidence_version, 0 | 1 | EVIDENCE_VERSION)
        {
            return Err(pending("delete_evidence_binding_conflict"));
        }
        let mut progress = progress.clone();
        if progress.evidence_version != EVIDENCE_VERSION {
            // Keep exact known identities/conflicts but require current
            // control-plane observation and inventories for every milestone.
            // v1's correctly attributed stop remains the first accounting
            // boundary. Its storage/completion milestone never proves v2.
            let mut historical = progress.clone();
            historical.evidence_version = EVIDENCE_VERSION;
            if progress.evidence_version != 1
                || !stop_is_valid(&delete_projection(machine), &historical)
            {
                progress.stop = None;
            }
            progress.evidence_version = EVIDENCE_VERSION;
            progress.creation_terminal = false;
            progress.resources_complete = false;
            progress.storage = DeleteStorageDisposition::ProvisioningUnknown;
            progress.blocker = None;
        }
        return Ok(progress);
    }
    Ok(DeleteRecoveryProgress {
        evidence_version: EVIDENCE_VERSION,
        evidence_id: uuid::Uuid::new_v4().to_string(),
        operation_id: machine
            .active_operation_id
            .clone()
            .ok_or_else(|| pending("delete_operation_missing"))?,
        owner_id: machine.owner_id.clone(),
        runtime_generation: machine.runtime_generation,
        credential_epoch: machine.credential_epoch,
        writer_fence: machine.writer_fence.clone(),
        deletion_basis_digest: basis_digest(machine)
            .ok_or_else(|| pending("machine_owner_deletion_basis_required"))?,
        evaluated_revision: machine.revision,
        original_uncertainty: machine.native_uncertainty.clone(),
        sandbox_uid: machine.sandbox_uid.clone(),
        pod_uid: machine.pod_uid.clone(),
        node_uid: machine.native_node_uid.clone(),
        pvc_uid: machine.pvc_uid.clone(),
        provisioning_request_id: None,
        provisioning_terminal: None,
        volume: None,
        stop: None,
        storage: DeleteStorageDisposition::ProvisioningUnknown,
        creation_terminal: false,
        resources_complete: false,
        blocker: None,
        observed_at: now(),
    })
}

async fn save(
    db: &Database,
    machine: &ManagedMachine,
    progress: &mut DeleteRecoveryProgress,
    mut bindings: Document,
) -> AppResult<ManagedMachine> {
    progress.evaluated_revision = machine.revision;
    progress.observed_at = now();
    bindings.insert(
        "delete_recovery",
        bson::to_bson(progress).map_err(|_| pending("delete_evidence_encoding_failed"))?,
    );
    bindings.insert("writer_admitted", false);
    if machine.resource_state == ResourceState::Deleted {
        bindings.insert(
            "cleanup_state",
            if progress.resources_complete {
                "complete"
            } else {
                "pending"
            },
        );
    }
    bindings.insert("updated_at", bson::DateTime::now());
    db.collection::<ManagedMachine>(MACHINES)
        .find_one_and_update(
            save_filter(machine)?,
            doc! {"$set":bindings,"$inc":{"revision":1_i64}},
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| pending("delete_evidence_raced"))
}

fn save_filter(machine: &ManagedMachine) -> AppResult<Document> {
    Ok(doc! {"_id":&machine.node_id,"owner_id":&machine.owner_id,
    "revision":machine.revision,"resource_state":bson::to_bson(&machine.resource_state).map_err(|_|pending("delete_evidence_encoding_failed"))?,
    "active_operation_id":&machine.active_operation_id,"runtime_generation":machine.runtime_generation,
    "credential_epoch":machine.credential_epoch,"writer_fence":&machine.writer_fence,
    "native_uncertainty":&machine.native_uncertainty,
    "deletion_basis":bson::to_bson(&machine.deletion_basis).map_err(|_|pending("delete_evidence_encoding_failed"))?})
}

/// Clear the resolved paired diagnostic at the complete observation milestone.
/// A successor snapshot or lost original retry claim aborts both writes. A
/// different current operation cause remains visible as observation progresses.
async fn save_observation_progress(
    db: &Database,
    machine: &ManagedMachine,
    progress: &mut DeleteRecoveryProgress,
    claim_id: &str,
    resolved_code: &str,
) -> AppResult<ManagedMachine> {
    use crate::models::remote_machine_operation::COLLECTION_NAME as OPERATIONS;
    if !binding_matches(machine, progress) {
        return Err(pending("delete_evidence_raced"));
    }
    progress.evaluated_revision = machine.revision;
    progress.observed_at = now();
    let mut machine_filter = save_filter(machine)?;
    machine_filter.insert("delete_recovery.blocker", resolved_code);
    let operation_filter = doc! {"_id":&progress.operation_id,"owner_id":&machine.owner_id,
    "machine_id":&machine.node_id,"action":"delete",
    "state":{"$in":["accepted","applying","reconciling"]},"terminal_at":bson::Bson::Null,
    "delete_retry.claim_id":claim_id,"$expr":{"$gt":["$delete_retry.claim_until", "$$NOW"]}};
    let resolved = doc! {"$eq":["$error_code", {"$literal":resolved_code}]};
    let bindings = doc! {"delete_recovery":bson::to_bson(progress).map_err(|_|pending("delete_evidence_encoding_failed"))?,
    "writer_admitted":false,"updated_at":bson::DateTime::now()};
    let db = db.clone();
    db.client()
        .start_session()
        .await?
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<ManagedMachine> = async {
                let changed = db
                    .collection::<Document>(OPERATIONS)
                    .update_one(
                        operation_filter.clone(),
                        vec![doc! {"$set":{
                            "error_code":{"$cond":[resolved.clone(),bson::Bson::Null,"$error_code"]},
                            "error_message":{"$cond":[resolved.clone(),bson::Bson::Null,"$error_message"]},
                            "updated_at":bson::DateTime::now(),
                        }}],
                    )
                    .session(&mut *session)
                    .await?;
                if changed.matched_count != 1 {
                    return Err(pending("delete_evidence_raced"));
                }
                db.collection::<ManagedMachine>(MACHINES)
                    .find_one_and_update(
                        machine_filter.clone(),
                        doc! {"$set":bindings.clone(),"$inc":{"revision":1_i64}},
                    )
                    .return_document(ReturnDocument::After)
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| pending("delete_evidence_raced"))
            }
            .await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

/// Save only the snapshot advanced by this authorized invocation. The machine
/// CAS and pending operation's captured retry claim commit together; a late
/// failure cannot adopt a successor revision, operation or retry reservation.
async fn save_observation_failure(
    db: &Database,
    machine: &ManagedMachine,
    claim_id: Option<&str>,
    code: &str,
) -> AppResult<()> {
    use crate::models::remote_machine_operation::COLLECTION_NAME as OPERATIONS;
    let mut progress = initial_progress(machine)?;
    progress.resources_complete = false;
    progress.blocker = Some(code.into());
    progress.evaluated_revision = machine.revision;
    progress.observed_at = now();
    let machine_filter = save_filter(machine)?;
    let finalized = machine.resource_state == ResourceState::Deleted;
    if (!finalized && (machine.resource_state != ResourceState::Deleting || claim_id.is_none()))
        || (finalized && claim_id.is_some())
        || !lifecycle::has_owner_deletion_basis(machine)
        || machine.writer_admitted
    {
        return Err(pending("delete_evidence_raced"));
    }
    let mut operation_filter = doc! {"_id":&progress.operation_id,"owner_id":&machine.owner_id,
    "machine_id":&machine.node_id,"action":"delete"};
    if finalized {
        operation_filter.insert("state", "succeeded");
        operation_filter.insert("terminal_at", doc! {"$ne":bson::Bson::Null});
    } else {
        operation_filter.insert("state", doc! {"$in":["accepted","applying","reconciling"]});
        operation_filter.insert("terminal_at", bson::Bson::Null);
        operation_filter.insert("delete_retry.claim_id", claim_id);
        operation_filter.insert("$expr", doc! {"$gt":["$delete_retry.claim_until", "$$NOW"]});
    }
    let bindings = doc! {"delete_recovery":bson::to_bson(&progress).map_err(|_|pending("delete_evidence_encoding_failed"))?,
    "writer_admitted":false,"cleanup_state":"pending","updated_at":bson::DateTime::now()};
    let code = code.to_owned();
    let db = db.clone();
    db.client().start_session().await?.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                let operations = db.collection::<Document>(OPERATIONS);
                if finalized {
                    if operations.find_one(operation_filter.clone()).session(&mut *session).await?.is_none() {
                        return Err(pending("delete_evidence_raced"));
                    }
                } else {
                    let changed = operations.update_one(operation_filter.clone(), doc! {"$set":{
                        "state":"reconciling","error_code":&code,
                        "error_message":"Awaiting verified lifecycle responsibilities; work data remains retained",
                        "updated_at":bson::DateTime::now(),
                    }}).session(&mut *session).await?;
                    if changed.matched_count != 1 { return Err(pending("delete_evidence_raced")); }
                }
                let changed = db.collection::<Document>(MACHINES).update_one(machine_filter.clone(),
                    doc! {"$set":bindings.clone(),"$inc":{"revision":1_i64}})
                    .session(&mut *session).await?;
                if changed.matched_count != 1 { return Err(pending("delete_evidence_raced")); }
                Ok(())
            }.await;
            super::api_key_mutation_service::transaction_result(result)
        }).await.map_err(super::api_key_mutation_service::map_transaction_error)
}

async fn block(
    manager: &Manager,
    machine: &ManagedMachine,
    progress: &mut DeleteRecoveryProgress,
    code: &str,
) -> AppResult<()> {
    progress.blocker = Some(code.into());
    if machine.resource_state == ResourceState::Deleted {
        progress.resources_complete = false;
    }
    save(&manager.db, machine, progress, Document::new()).await?;
    manager.progress(&progress.operation_id, code).await?;
    Err(pending(code))
}

fn owned(machine: &ManagedMachine, object: &OwnedResourceObservation) -> bool {
    object.namespace.as_deref() == Some(machine.sandbox_namespace.as_str())
        && object.uid.is_some()
        && object.resource_version.is_some()
        && object.ownership_valid
        && object.controller_owner_uid == machine.sandbox_uid
        && object.machine_id.as_deref() == Some(machine.node_id.as_str())
        && object.owner_id.as_deref() == Some(machine.owner_id.as_str())
}
fn sandbox_matches(machine: &ManagedMachine, sandbox: &SandboxObservation) -> bool {
    sandbox.sandbox_uid == machine.sandbox_uid
        && sandbox.machine_id.as_deref() == Some(machine.node_id.as_str())
        && sandbox.owner_id.as_deref() == Some(machine.owner_id.as_str())
        && sandbox.runtime_generation.as_deref()
            == Some(machine.runtime_generation.to_string().as_str())
        && sandbox.credential_epoch.as_deref()
            == Some(machine.credential_epoch.to_string().as_str())
        && sandbox.writer_fence.as_deref() == Some(machine.writer_fence.as_str())
}
pub(crate) async fn live_before_effect(db: &Database, machine: &ManagedMachine) -> AppResult<()> {
    if !matches!(machine.resource_state, ResourceState::Deleting | ResourceState::Deleted)
        || !lifecycle::has_owner_deletion_basis(machine)
        || (machine.resource_state == ResourceState::Deleted
            && (machine.billing_state != crate::models::remote_machine::BillingState::Closed
                || machine.authorization_state != crate::models::remote_machine::AuthorizationState::Revoked))
        || db.collection::<Document>(MACHINES).count_documents(doc!{
            "_id":&machine.node_id,"owner_id":&machine.owner_id,"revision":machine.revision,
            "resource_state":bson::to_bson(&machine.resource_state).map_err(|_|pending("delete_external_effect_fenced"))?,
            "active_operation_id":&machine.active_operation_id,"runtime_generation":machine.runtime_generation,
            "credential_epoch":machine.credential_epoch,"writer_fence":&machine.writer_fence,"writer_admitted":false,
            "native_uncertainty":&machine.native_uncertainty,
            "deletion_basis":bson::to_bson(&machine.deletion_basis).map_err(|_|pending("delete_external_effect_fenced"))?,
        }).await? != 1
    {
        return Err(pending("delete_external_effect_fenced"));
    }
    Ok(())
}

async fn storage_observation_error<T>(
    manager: &Manager,
    machine: &ManagedMachine,
    progress: &mut DeleteRecoveryProgress,
    error: AppError,
) -> AppResult<T> {
    if observation_error_code(&error).is_some() {
        return Err(error);
    }
    let code = match &error {
        AppError::Conflict(code) => code.as_str(),
        _ => "delete_resource_observation_unknown",
    };
    block(manager, machine, progress, code).await?;
    Err(error)
}

/// Both ordinary stop cleanup and recovery retain the same current claim
/// identity before a later observation can fail or any DELETE can be sent.
async fn discover_original_claim(
    manager: &Manager,
    machine: &mut ManagedMachine,
    progress: &mut DeleteRecoveryProgress,
) -> AppResult<Option<OwnedResourceObservation>> {
    match discover_original_claim_inner(manager, machine, progress).await {
        Ok(pvc) => Ok(pvc),
        Err(error) => storage_observation_error(manager, machine, progress, error).await,
    }
}

async fn discover_original_claim_inner(
    manager: &Manager,
    machine: &mut ManagedMachine,
    progress: &mut DeleteRecoveryProgress,
) -> AppResult<Option<OwnedResourceObservation>> {
    let uid = machine
        .sandbox_uid
        .as_deref()
        .ok_or_else(|| pending("creation_result_unknown"))?;
    let pvcs = manager
        .api
        .list_owned_resources(&machine.sandbox_namespace, "persistentvolumeclaims", uid)
        .await
        .map_err(|error| api_pending(machine, error))?;
    if pvcs.len() > 1 || pvcs.iter().any(|p| !owned(machine, p)) {
        return Err(pending("delete_pvc_ownership_conflict"));
    }
    let mut pvc = pvcs.first().cloned();
    if let Some(p) = &pvc {
        // A later exact404 affects only DELETE presence. Keep this original
        // UID and known volume anchor durable for this and later inventories.
        retain_original_claim_identity(manager, machine, progress, p).await?;
    }
    if let Some(name) = &machine.pvc_name {
        let exact = manager
            .api
            .resource(&machine.sandbox_namespace, "persistentvolumeclaims", name)
            .await
            .map_err(|error| api_pending(machine, error))?;
        if exact.as_ref().is_some_and(|p| {
            !owned(machine, p)
                || machine
                    .pvc_uid
                    .as_ref()
                    .is_some_and(|uid| p.uid.as_ref() != Some(uid))
        }) {
            return Err(pending("delete_pvc_identity_conflict"));
        }
        if pvc.as_ref().is_some_and(|p| p.name != *name) {
            return Err(pending("delete_multiple_pvc_identity_conflict"));
        }
        pvc = exact;
    }
    if let Some(p) = &pvc {
        retain_original_claim_identity(manager, machine, progress, p).await?;
    }
    Ok(pvc)
}

async fn retain_original_claim_identity(
    manager: &Manager,
    machine: &mut ManagedMachine,
    progress: &mut DeleteRecoveryProgress,
    p: &OwnedResourceObservation,
) -> AppResult<()> {
    if machine
        .pvc_uid
        .as_ref()
        .is_some_and(|uid| p.uid.as_ref() != Some(uid))
    {
        return Err(pending("delete_pvc_identity_conflict"));
    }
    if p.volume_name.as_ref().is_some_and(|name| {
        machine
            .native_pv_name
            .as_ref()
            .is_some_and(|old| old != name)
            || progress
                .volume
                .as_ref()
                .is_some_and(|old| &old.pv_name != name)
    }) {
        return Err(pending("delete_pv_claim_identity_conflict"));
    }
    if machine
        .pvc_name
        .as_ref()
        .is_some_and(|name| name != &p.name)
    {
        return Err(pending("delete_multiple_pvc_identity_conflict"));
    }
    // Re-observing an already retained identity is read-only. In particular,
    // it must not advance the machine before the paired retry-claim fence.
    if machine.pvc_uid == p.uid
        && machine.pvc_name.as_deref() == Some(p.name.as_str())
        && progress.pvc_uid == p.uid
        && p.volume_name
            .as_ref()
            .is_none_or(|name| machine.native_pv_name.as_ref() == Some(name))
    {
        return Ok(());
    }
    let mut bindings = doc! {"pvc_name": &p.name, "pvc_uid": &p.uid};
    if let Some(name) = &p.volume_name {
        machine.native_pv_name = Some(name.clone());
        bindings.insert("native_pv_name", name);
    }
    machine.pvc_uid = p.uid.clone();
    machine.pvc_name = Some(p.name.clone());
    progress.pvc_uid = p.uid.clone();
    *machine = save(&manager.db, machine, progress, bindings).await?;
    Ok(())
}

/// Re-observe the original Sandbox and namespace children under the delete
/// authority; storage inventories and child deletion are outside this contract.
fn recover_resources<'a>(
    manager: &'a Manager,
    machine: &'a mut ManagedMachine,
    claim_id: Option<&'a str>,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(recover_resources_inner(manager, machine, claim_id))
}

async fn recover_resources_inner(
    manager: &Manager,
    machine: &mut ManagedMachine,
    claim_id: Option<&str>,
) -> AppResult<()> {
    let mut progress = initial_progress(machine)?;
    // A retained completion is historical evidence until this pass has
    // completed its current inventory. Persist pending before any new effect.
    progress.resources_complete = false;
    if machine
        .native_uncertainty
        .as_deref()
        .is_some_and(|r| r != "predecessor_node_unhealthy_or_stale")
    {
        return block(
            manager,
            machine,
            &mut progress,
            "delete_uncertainty_reason_not_allowed",
        )
        .await;
    }
    let sandbox = manager
        .api
        .get(&machine.sandbox_namespace, &machine.sandbox_name)
        .await
        .map_err(|error| api_pending(machine, error))?;
    if let Some(sandbox) = &sandbox {
        if !sandbox_matches(machine, sandbox) {
            return block(
                manager,
                machine,
                &mut progress,
                "delete_sandbox_identity_conflict",
            )
            .await;
        }
        if sandbox.deletion_timestamp.is_none()
            && (!sandbox.desired_suspended
                || sandbox.shutdown_time.is_some()
                || sandbox.operation_id != machine.active_operation_id)
        {
            let intent = super::remote_machine_adapter::intent(
                machine,
                &progress.operation_id,
                manager.storage_class.as_deref(),
                false,
                &manager.bootstrap_base_url,
            );
            *machine = save(&manager.db, machine, &mut progress, Document::new()).await?;
            live_before_effect(&manager.db, machine).await?;
            manager
                .api
                .suspend_if_identity(
                    &intent,
                    machine
                        .sandbox_uid
                        .as_deref()
                        .ok_or_else(|| pending("delete_sandbox_identity_missing"))?,
                    sandbox
                        .resource_version
                        .as_deref()
                        .ok_or_else(|| pending("delete_sandbox_revision_missing"))?,
                )
                .await
                .map_err(|error| api_pending(machine, error))?;
            return Ok(());
        }
    }
    if sandbox.as_ref().is_some_and(|s| {
        !s.desired_suspended
            || !s.suspended
            || s.observed_generation != s.generation
            || s.generation.is_none_or(|generation| generation <= 0)
            || s.resource_version.as_ref().is_none_or(String::is_empty)
    }) {
        return block(
            manager,
            machine,
            &mut progress,
            "delete_current_suspension_unconfirmed",
        )
        .await;
    }
    let uid = machine
        .sandbox_uid
        .clone()
        .ok_or_else(|| pending("creation_result_unknown"))?;
    let pvc = discover_original_claim(manager, machine, &mut progress).await?;
    let pods = manager
        .api
        .recovery_pods(
            &machine.sandbox_namespace,
            machine.pod_uid.as_deref(),
            Some(&uid),
            machine.pvc_name.as_deref(),
        )
        .await
        .map_err(|error| api_pending(machine, error))?;
    if pods.iter().any(|p| {
        !owned(machine, p)
            || machine
                .pod_uid
                .as_ref()
                .is_some_and(|uid| p.uid.as_ref() != Some(uid))
    }) {
        return block(
            manager,
            machine,
            &mut progress,
            "delete_pod_identity_conflict",
        )
        .await;
    }
    if let Some(name) = &machine.native_pod_name
        && let Some(pod) = manager
            .api
            .resource(&machine.sandbox_namespace, "pods", name)
            .await
            .map_err(|error| api_pending(machine, error))?
    {
        if pod.uid != machine.pod_uid || !owned(machine, &pod) {
            return block(
                manager,
                machine,
                &mut progress,
                "delete_pod_identity_conflict",
            )
            .await;
        }
        return block(manager, machine, &mut progress, "delete_pod_still_present").await;
    }
    if !pods.is_empty() {
        return block(manager, machine, &mut progress, "delete_pod_still_present").await;
    }
    let current_creation_terminal =
        operations::creation_responsibilities_terminal(&manager.db, machine).await?;
    // Retain the terminal-work milestone that authorized the first absence
    // stop. Fresh responsibilities still gate completion below, including on
    // tombstone scans; they cannot replace that first accounting boundary.
    progress.creation_terminal |= current_creation_terminal;
    if progress.stop.is_none() {
        if super::remote_machine_native::predecessor_stopped(machine) {
            let o = machine
                .native_observation
                .as_ref()
                .expect("checked observation");
            progress.stop = Some(DeleteStopEvidence {
                kind: DeleteStopKind::OrdinaryNative,
                source_operation_id: Some(o.operation_id.clone()),
                evidence_ref: o.evidence_ref.clone(),
                source: "kubernetes-native".into(),
                observed_at: o.observed_at,
                sandbox_generation: o.sandbox_generation,
                sandbox_resource_version: o.sandbox_resource_version.clone(),
            });
        } else if sandbox.is_none() {
            if !current_creation_terminal {
                return block(manager, machine, &mut progress, "creation_result_unknown").await;
            }
            progress.stop = Some(DeleteStopEvidence {
                kind: DeleteStopKind::ControlPlaneWorkloadAbsent,
                source_operation_id: Some(progress.operation_id.clone()),
                evidence_ref: uuid::Uuid::new_v4().to_string(),
                source: CONTROL_PLANE_SOURCE.into(),
                // Complete original-workload absence supplies an accounting
                // observation upper bound, never an invented controller fact.
                observed_at: now(),
                sandbox_generation: 0,
                sandbox_resource_version: String::new(),
            });
        } else {
            let Some(s) = sandbox.as_ref().filter(|s| {
                s.desired_suspended
                    && s.suspended
                    && s.operation_id == machine.active_operation_id
                    && s.generation.is_some()
                    && s.observed_generation == s.generation
                    && s.deletion_timestamp.is_none()
            }) else {
                return block(
                    manager,
                    machine,
                    &mut progress,
                    "delete_current_suspension_unconfirmed",
                )
                .await;
            };
            progress.stop = Some(DeleteStopEvidence {
                kind: DeleteStopKind::ControlPlaneSuspended,
                source_operation_id: Some(progress.operation_id.clone()),
                evidence_ref: uuid::Uuid::new_v4().to_string(),
                source: CONTROL_PLANE_SOURCE.into(),
                // This is the actual first controller-observation upper bound,
                // retained across retries; it is not a physical stop instant.
                observed_at: now(),
                sandbox_generation: s.generation.unwrap_or(0),
                sandbox_resource_version: s.resource_version.clone().unwrap_or_default(),
            });
        }
    }
    progress.resources_complete = false;
    let resolved_observation = progress
        .blocker
        .as_deref()
        .filter(|code| observation_error_code(&pending(code)).is_some())
        .map(str::to_owned);
    progress.blocker = None;
    *machine = if let (Some(claim_id), Some(code)) = (claim_id, resolved_observation) {
        save_observation_progress(&manager.db, machine, &mut progress, claim_id, &code).await?
    } else {
        save(&manager.db, machine, &mut progress, Document::new()).await?
    };
    if !stop_is_valid(&delete_projection(machine), &progress) {
        return block(
            manager,
            machine,
            &mut progress,
            "delete_stop_evidence_unverified",
        )
        .await;
    }
    // Accounting is retried independently. Its failure never prevents an
    // otherwise safe resource effect; finality still checks the original run.
    if machine.resource_state == ResourceState::Deleting
        && let Some(billing) = &manager.billing
    {
        let _ = super::remote_machine_runtime::settle_persisted_delete(
            &manager.db,
            billing.clone(),
            machine,
        )
        .await;
    }
    if let Some(s) = &sandbox {
        if s.deletion_timestamp.is_some() {
            return block(manager, machine, &mut progress, "delete_sandbox_gc_pending").await;
        }
        live_before_effect(&manager.db, machine).await?;
        manager
            .api
            .delete_if_identity(
                &machine.sandbox_namespace,
                &machine.sandbox_name,
                &uid,
                s.resource_version
                    .as_deref()
                    .ok_or_else(|| pending("delete_sandbox_revision_missing"))?,
            )
            .await
            .map_err(|error| api_pending(machine, error))?;
        return block(manager, machine, &mut progress, "delete_sandbox_gc_pending").await;
    }
    if let Some(pvc) = &pvc {
        let code = if pvc.deletion_timestamp.is_some() {
            "delete_pvc_finalizer_pending"
        } else {
            "delete_pvc_gc_pending"
        };
        return block(manager, machine, &mut progress, code).await;
    }
    // Only namespace objects were observed. No PV/CSI release or erasure
    // assertion is made by the v2 Sandbox lifecycle contract.
    progress.storage = DeleteStorageDisposition::SandboxCascadeObserved;
    progress.resources_complete = true;
    if !current_creation_terminal {
        return block(manager, machine, &mut progress, "creation_result_unknown").await;
    }
    progress.blocker = None;
    let cleanup = doc! {"cleanup_state":"complete","cleanup_evidence_digest":&progress.evidence_id};
    *machine = save(&manager.db, machine, &mut progress, cleanup).await?;
    if machine.resource_state == ResourceState::Deleted {
        // Only the resource milestone changes. Original stop/accounting and
        // successful operation/slot state remain final and idempotent.
        return Ok(());
    }
    if !finality_ready(machine, &progress) {
        return block(
            manager,
            machine,
            &mut progress,
            "delete_remaining_responsibilities_pending",
        )
        .await;
    }
    if super::remote_machine_runtime::require_accounting_final(&manager.db, machine)
        .await
        .is_err()
    {
        return block(manager, machine, &mut progress, "accounting_pending").await;
    }
    super::remote_machine_native::finalize_persisted_delete(&manager.db, machine).await?;
    manager.complete_if_final(machine).await
}

#[cfg(test)]
#[path = "remote_machine_delete_recovery_tests.rs"]
mod tests;
