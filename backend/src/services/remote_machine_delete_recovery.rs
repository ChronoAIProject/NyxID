//! Owner-authorized deletion is separate from ordinary runtime authority.
use crate::models::remote_machine::{
    DeleteRecoveryProgress, DeleteStopKind, DeleteStorageDisposition, ManagedMachine, ResourceState,
};

pub(crate) fn basis_digest(machine: &ManagedMachine) -> Option<String> {
    use sha2::{Digest, Sha256};
    let basis = machine.deletion_basis.as_ref()?;
    Some(hex::encode(Sha256::digest(bson::to_vec(basis).ok()?)))
}

pub(crate) const EVIDENCE_VERSION: i64 = 1;
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
        && progress.storage == DeleteStorageDisposition::ControlPlaneReleased
        && known_storage_obligations_covered(machine, progress)
}

/// Historical fields remain readable as known resource identities, never as
/// authority that provisioning terminated or a cloud disk was erased.
fn known_storage_obligations_covered(
    machine: &ManagedMachine,
    progress: &DeleteRecoveryProgress,
) -> bool {
    // Legacy cloud/provisioner hints are retained for investigation only.
    // Historical external handles cannot create a new external-proof gate.
    match &progress.volume {
        Some(v) => {
            v.reclaim_policy == "Delete"
                && machine.native_pv_name.as_deref() == Some(v.pv_name.as_str())
                && machine.native_pv_uid.as_deref() == Some(v.pv_uid.as_str())
                && machine.native_pv_reclaim_policy.as_deref() == Some("Delete")
        }
        None => {
            machine.native_pv_name.is_none()
                && machine.native_pv_uid.is_none()
                && machine.native_pv_reclaim_policy.is_none()
        }
    }
}

use super::{
    remote_machine_manager::Manager, remote_machine_operation_service as operations,
    remote_machine_service as lifecycle,
};
use crate::errors::{AppError, AppResult};
use crate::models::remote_machine::{
    COLLECTION_NAME as MACHINES, DeleteStopEvidence, DeleteVolumeIdentity,
};
use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{self, Document, doc},
    options::ReturnDocument,
};
use nyxid_machine_updater::sandbox::{
    OwnedResourceObservation, PersistentVolumeObservation, SandboxObservation,
};

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
    if machine.resource_state != ResourceState::Deleted
        || machine.writer_admitted
        || machine.billing_state != crate::models::remote_machine::BillingState::Closed
        || machine.authorization_state != crate::models::remote_machine::AuthorizationState::Revoked
        || !lifecycle::has_owner_deletion_basis(machine)
        || !stop_is_valid(&projected, progress)
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
fn api_pending(_: anyhow::Error) -> AppError {
    pending("delete_resource_observation_unknown")
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
    let machine = lifecycle::managed_machine_for_node(&manager.db, &snapshot.node_id)
        .await?
        .ok_or_else(|| pending("delete_machine_missing"))?;
    if machine.resource_state == ResourceState::Deleted {
        // Finalized tombstones retain the original deletion scope. A bounded
        // Manager scan keeps resolving late Kubernetes resources without
        // reopening execution, the operation, accounting or the free slot.
        finalized_authorized(manager, &machine).await?;
        let result = recover_resources(manager, machine.clone()).await;
        if let Err(error) = &result {
            // Inventory transport/parse failures may occur before a milestone
            // save. Retain an explicit pending diagnostic under the same live
            // tombstone fence, without reopening its completed operation.
            if let Some(latest) =
                lifecycle::managed_machine_for_node(&manager.db, &machine.node_id).await?
                && finalized_authorized(manager, &latest).await.is_ok()
            {
                let mut progress = initial_progress(&latest)?;
                progress.resources_complete = false;
                progress.blocker = Some(match error {
                    AppError::Conflict(code) => code.clone(),
                    _ => "delete_resource_observation_unknown".into(),
                });
                save(&manager.db, &latest, &mut progress, Document::new()).await?;
            }
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
    let result = tokio::time::timeout(
        super::remote_machine_delete_retry::ATTEMPT_TIMEOUT,
        pending_attempt(manager, machine),
    )
    .await
    .unwrap_or_else(|_| Err(pending("delete_attempt_timed_out")));
    super::remote_machine_delete_retry::finish(&manager.db, &op_id, &claim_id).await?;
    result
}

async fn pending_attempt(manager: &Manager, mut machine: ManagedMachine) -> AppResult<()> {
    // This path proves local non-submission, never remote absence. Paid zero
    // runtime settlement remains its existing restricted implementation.
    let _ = super::remote_machine_runtime::finalize_never_submitted(
        &manager.db,
        manager.billing.clone(),
        &machine,
    )
    .await;
    if lifecycle::finalize_unallocated_delete(&manager.db, &machine.node_id)
        .await?
        .is_some()
    {
        return manager.complete_if_final(&machine).await;
    }
    if machine.sandbox_uid.is_none() {
        manager.observe_resource(&machine).await?;
        machine = lifecycle::managed_machine_for_node(&manager.db, &machine.node_id)
            .await?
            .ok_or_else(|| pending("delete_machine_missing"))?;
    }
    // An already valid ordinary stop remains ordinary authority. Retain its
    // real source operation when delete only relabels an already stopped CR.
    if super::remote_machine_native::predecessor_stopped(&machine)
        && machine.pvc_name.is_some()
        && machine.pvc_uid.is_some()
        && machine.native_pv_name.is_some()
        && machine.native_pv_uid.is_some()
        && machine.native_pv_reclaim_policy.is_some()
    {
        if let Some(machine) = prepare_ordinary_stop(manager, &machine).await? {
            return manager.finish_ordinary_native_delete(&machine).await;
        }
        return Ok(());
    }
    recover_resources(manager, machine).await
}

/// Bind a previously established ordinary stop to this delete without
/// relabelling its controller observation or adding host/cloud prerequisites.
async fn prepare_ordinary_stop(
    manager: &Manager,
    machine: &ManagedMachine,
) -> AppResult<Option<ManagedMachine>> {
    let mut progress = initial_progress(machine)?;
    progress.resources_complete = false;
    let native = machine
        .native_observation
        .as_ref()
        .ok_or_else(|| pending("delete_ordinary_stop_missing"))?;
    let sandbox = manager
        .api
        .get(&machine.sandbox_namespace, &machine.sandbox_name)
        .await
        .map_err(api_pending)?;
    if let Some(sandbox) = &sandbox {
        if !sandbox_matches(machine, sandbox) {
            block(
                manager,
                machine,
                &mut progress,
                "delete_sandbox_identity_conflict",
            )
            .await?;
        }
        if !sandbox.desired_suspended || sandbox.shutdown_time.is_some() {
            // A real spec transition receives its own generation anchor and
            // controller observation through the existing ordinary verifier.
            manager.observe_resource(machine).await?;
            return Ok(None);
        }
        if sandbox.operation_id != machine.active_operation_id {
            let intent = super::remote_machine_adapter::intent(
                machine,
                &progress.operation_id,
                manager.storage_class.as_deref(),
                false,
                &manager.bootstrap_base_url,
            );
            let saved = save(&manager.db, machine, &mut progress, Document::new()).await?;
            live_before_effect(&manager.db, &saved).await?;
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
                .map_err(api_pending)?;
            return Ok(None);
        }
        if !sandbox.suspended || sandbox.observed_generation != sandbox.generation {
            block(
                manager,
                machine,
                &mut progress,
                "delete_current_suspension_unconfirmed",
            )
            .await?;
        }
    }
    if progress.stop.is_none() {
        progress.stop = Some(DeleteStopEvidence {
            kind: DeleteStopKind::OrdinaryNative,
            source_operation_id: Some(native.operation_id.clone()),
            evidence_ref: native.evidence_ref.clone(),
            source: "kubernetes-native".into(),
            observed_at: native.observed_at,
            sandbox_generation: native.sandbox_generation,
            sandbox_resource_version: native.sandbox_resource_version.clone(),
        });
    }
    retain_known_volume(machine, &mut progress);
    let mut machine = machine.clone();
    let pvc = discover_original_claim(manager, &mut machine, &mut progress).await?;
    discover_original_volume(manager, &mut machine, &mut progress, pvc.as_ref()).await?;
    Ok(Some(
        save(&manager.db, &machine, &mut progress, Document::new()).await?,
    ))
}

/// Called only after the established ordinary cleanup contract checked exact
/// Sandbox/Pod/PVC/PV absence and Delete reclaim semantics. It shares the same
/// private final transaction while retaining that narrower native contract.
pub(crate) async fn finalize_ordinary_evidence(
    manager: &Manager,
    snapshot: &ManagedMachine,
) -> AppResult<()> {
    let db = &manager.db;
    let mut machine = snapshot.clone();
    let mut progress = initial_progress(&machine)?;
    if !matches!(
        progress.stop.as_ref().map(|s| &s.kind),
        Some(DeleteStopKind::OrdinaryNative)
    ) || !stop_is_valid(&machine, &progress)
    {
        return Err(pending("delete_ordinary_stop_mismatch"));
    }
    progress.creation_terminal =
        operations::creation_responsibilities_terminal(db, &machine).await?;
    if !progress.creation_terminal {
        return block(manager, &machine, &mut progress, "creation_result_unknown").await;
    }
    if !known_storage_obligations_covered(&machine, &progress) {
        return block(
            manager,
            &machine,
            &mut progress,
            "delete_known_volume_identity_unresolved",
        )
        .await;
    }
    progress.resources_complete = true;
    if machine.pvc_name.is_none() || machine.pvc_uid.is_none() || progress.volume.is_none() {
        return block(
            manager,
            &machine,
            &mut progress,
            "delete_provisioning_identity_missing",
        )
        .await;
    }
    progress.storage = DeleteStorageDisposition::ControlPlaneReleased;
    progress.blocker = None;
    // Preserve the established resource-only milestone during accounting
    // outages. Final machine, operation and slot state still wait below.
    let cleanup = doc! {"cleanup_state":"complete","cleanup_evidence_digest":&progress.evidence_id};
    machine = save(db, &machine, &mut progress, cleanup).await?;
    if super::remote_machine_runtime::require_accounting_final(db, &machine)
        .await
        .is_err()
    {
        return block(manager, &machine, &mut progress, "accounting_pending").await;
    }
    super::remote_machine_native::finalize_persisted_delete(db, &machine).await?;
    Ok(())
}

fn initial_progress(machine: &ManagedMachine) -> AppResult<DeleteRecoveryProgress> {
    if let Some(progress) = &machine.delete_recovery {
        if !tuple_matches(&delete_projection(machine), progress)
            || !matches!(progress.evidence_version, 0 | EVIDENCE_VERSION)
            || (machine.resource_state == ResourceState::Deleted
                && progress.evidence_version != EVIDENCE_VERSION)
        {
            return Err(pending("delete_evidence_binding_conflict"));
        }
        let mut progress = progress.clone();
        if progress.evidence_version != EVIDENCE_VERSION {
            // Keep exact known identities/conflicts but require current
            // control-plane observation and inventories for every milestone.
            progress.evidence_version = EVIDENCE_VERSION;
            progress.stop = None;
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
    db.collection::<ManagedMachine>(MACHINES).find_one_and_update(doc!{"_id":&machine.node_id,"owner_id":&machine.owner_id,
        "revision":machine.revision,"resource_state":bson::to_bson(&machine.resource_state).map_err(|_|pending("delete_evidence_encoding_failed"))?,"active_operation_id":&machine.active_operation_id,
        "runtime_generation":machine.runtime_generation,"credential_epoch":machine.credential_epoch,"writer_fence":&machine.writer_fence,
        "native_uncertainty":&machine.native_uncertainty,"deletion_basis":bson::to_bson(&machine.deletion_basis).map_err(|_|pending("delete_evidence_encoding_failed"))?},
        doc!{"$set":bindings,"$inc":{"revision":1_i64}}).return_document(ReturnDocument::After).await?.ok_or_else(||pending("delete_evidence_raced"))
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

fn retain_known_volume(machine: &ManagedMachine, progress: &mut DeleteRecoveryProgress) {
    if progress.volume.is_none()
        && let (Some(name), Some(uid), Some(policy)) = (
            &machine.native_pv_name,
            &machine.native_pv_uid,
            &machine.native_pv_reclaim_policy,
        )
    {
        progress.volume = Some(DeleteVolumeIdentity {
            pv_name: name.clone(),
            pv_uid: uid.clone(),
            reclaim_policy: policy.clone(),
            csi_driver: None,
            csi_handle: None,
            immutable_disk_id: None,
        });
        progress.storage = DeleteStorageDisposition::Provisioned;
    }
}

async fn storage_observation_error<T>(
    manager: &Manager,
    machine: &ManagedMachine,
    progress: &mut DeleteRecoveryProgress,
    error: AppError,
) -> AppResult<T> {
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
        .map_err(api_pending)?;
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
            .map_err(api_pending)?;
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

/// A present exact PV has the same obligations as a list candidate, even
/// when a removed or reassigned claimRef made it escape the claim selector.
fn current_volume_identity(
    machine: &ManagedMachine,
    pvc: Option<&OwnedResourceObservation>,
    volume: &PersistentVolumeObservation,
    previous: Option<&DeleteVolumeIdentity>,
) -> AppResult<DeleteVolumeIdentity> {
    if volume.claim_uid != machine.pvc_uid
        || volume.claim_namespace.as_deref() != Some(machine.sandbox_namespace.as_str())
        || volume.claim_name != machine.pvc_name
        || machine.pvc_uid.is_none()
        || machine.pvc_name.is_none()
        || volume.uid.is_none()
        || machine
            .native_pv_uid
            .as_ref()
            .is_some_and(|uid| volume.uid.as_ref() != Some(uid))
        || machine
            .native_pv_name
            .as_ref()
            .is_some_and(|name| &volume.name != name)
        || pvc
            .and_then(|p| p.volume_name.as_ref())
            .is_some_and(|name| &volume.name != name)
    {
        return Err(pending("delete_pv_claim_identity_conflict"));
    }
    let identity = DeleteVolumeIdentity {
        pv_name: volume.name.clone(),
        pv_uid: volume.uid.clone().unwrap_or_default(),
        reclaim_policy: volume.reclaim_policy.clone().unwrap_or_default(),
        csi_driver: volume.csi_driver.clone(),
        csi_handle: volume.csi_volume_handle.clone(),
        immutable_disk_id: previous.and_then(|old| old.immutable_disk_id.clone()),
    };
    if machine
        .native_pv_reclaim_policy
        .as_ref()
        .is_some_and(|policy| &identity.reclaim_policy != policy)
        || previous.is_some_and(|old| {
            old.pv_name != identity.pv_name
                || old.pv_uid != identity.pv_uid
                || old.reclaim_policy != identity.reclaim_policy
                || old
                    .csi_driver
                    .as_ref()
                    .is_some_and(|driver| identity.csi_driver.as_ref() != Some(driver))
                || old
                    .csi_handle
                    .as_ref()
                    .is_some_and(|handle| identity.csi_handle.as_ref() != Some(handle))
        })
    {
        return Err(pending("delete_volume_identity_conflict"));
    }
    Ok(identity)
}

async fn discover_original_volume(
    manager: &Manager,
    machine: &mut ManagedMachine,
    progress: &mut DeleteRecoveryProgress,
    pvc: Option<&OwnedResourceObservation>,
) -> AppResult<Option<PersistentVolumeObservation>> {
    match discover_original_volume_inner(manager, machine, progress, pvc).await {
        Ok(pv) => Ok(pv),
        Err(error) => storage_observation_error(manager, machine, progress, error).await,
    }
}

async fn discover_original_volume_inner(
    manager: &Manager,
    machine: &mut ManagedMachine,
    progress: &mut DeleteRecoveryProgress,
    pvc: Option<&OwnedResourceObservation>,
) -> AppResult<Option<PersistentVolumeObservation>> {
    // A missing claim UID prevents adopting a PV, not observing same-name
    // conflicts. The identity verifier below still requires the original UID.
    let candidates = if let Some(name) = &machine.pvc_name {
        manager
            .api
            .persistent_volumes_for_claim(
                &machine.sandbox_namespace,
                name,
                machine.pvc_uid.as_deref(),
            )
            .await
            .map_err(api_pending)?
    } else {
        Vec::new()
    };
    if candidates.len() > 1 {
        return Err(pending("delete_multiple_original_volume_candidates"));
    }
    let candidate = candidates.first().cloned();
    let mut identity = candidate
        .as_ref()
        .map(|pv| current_volume_identity(machine, pvc, pv, progress.volume.as_ref()))
        .transpose()?;
    let exact_name = machine
        .native_pv_name
        .clone()
        .or_else(|| progress.volume.as_ref().map(|v| v.pv_name.clone()))
        .or_else(|| candidate.as_ref().map(|v| v.name.clone()))
        .or_else(|| pvc.and_then(|p| p.volume_name.clone()));
    let exact = if let Some(name) = exact_name {
        manager
            .api
            .persistent_volume(&name)
            .await
            .map_err(api_pending)?
    } else {
        None
    };
    if let Some(pv) = &exact {
        identity = Some(current_volume_identity(
            machine,
            pvc,
            pv,
            identity.as_ref().or(progress.volume.as_ref()),
        )?);
    }
    // Any present exact object must also pass the verifier before a candidate
    // identity is committed. Conflicts keep every prior anchor intact.
    if let Some(identity) = identity {
        let bindings = doc! {
            "native_pv_name": &identity.pv_name,
            "native_pv_uid": &identity.pv_uid,
            "native_pv_reclaim_policy": &identity.reclaim_policy,
        };
        machine.native_pv_name = Some(identity.pv_name.clone());
        machine.native_pv_uid = Some(identity.pv_uid.clone());
        machine.native_pv_reclaim_policy = Some(identity.reclaim_policy.clone());
        progress.volume = Some(identity);
        progress.storage = DeleteStorageDisposition::Provisioned;
        *machine = save(&manager.db, machine, progress, bindings).await?;
    }
    if let Some(volume) = &progress.volume {
        if volume.reclaim_policy != "Delete" {
            progress.storage = DeleteStorageDisposition::Retained;
            return Err(pending("delete_original_volume_retained"));
        }
        if !manager
            .api
            .volume_attachments(&volume.pv_name)
            .await
            .map_err(api_pending)?
            .is_empty()
        {
            return Err(pending("delete_volume_attachment_present"));
        }
    } else if machine.native_pv_uid.is_some()
        || machine.native_pv_name.is_some()
        || pvc.is_some_and(|p| p.volume_name.is_some())
    {
        return Err(pending("delete_original_volume_chain_missing"));
    }
    if !known_storage_obligations_covered(machine, progress) {
        return Err(pending("delete_known_volume_identity_unresolved"));
    }
    Ok(exact.or(candidate))
}

fn recover_resources<'a>(
    manager: &'a Manager,
    machine: ManagedMachine,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(recover_resources_inner(manager, machine))
}

async fn recover_resources_inner(manager: &Manager, mut machine: ManagedMachine) -> AppResult<()> {
    let mut progress = initial_progress(&machine)?;
    // A retained completion is historical evidence until this pass has
    // completed its current inventory. Persist pending before any new effect.
    progress.resources_complete = false;
    retain_known_volume(&machine, &mut progress);
    if machine
        .native_uncertainty
        .as_deref()
        .is_some_and(|r| r != "predecessor_node_unhealthy_or_stale")
    {
        return block(
            manager,
            &machine,
            &mut progress,
            "delete_uncertainty_reason_not_allowed",
        )
        .await;
    }
    let sandbox = manager
        .api
        .get(&machine.sandbox_namespace, &machine.sandbox_name)
        .await
        .map_err(api_pending)?;
    if let Some(sandbox) = &sandbox {
        if !sandbox_matches(&machine, sandbox) {
            return block(
                manager,
                &machine,
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
                &machine,
                &progress.operation_id,
                manager.storage_class.as_deref(),
                false,
                &manager.bootstrap_base_url,
            );
            machine = save(&manager.db, &machine, &mut progress, Document::new()).await?;
            live_before_effect(&manager.db, &machine).await?;
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
                .map_err(api_pending)?;
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
            &machine,
            &mut progress,
            "delete_current_suspension_unconfirmed",
        )
        .await;
    }
    let uid = machine
        .sandbox_uid
        .clone()
        .ok_or_else(|| pending("creation_result_unknown"))?;
    let pvc = discover_original_claim(manager, &mut machine, &mut progress).await?;
    let pods = manager
        .api
        .recovery_pods(
            &machine.sandbox_namespace,
            machine.pod_uid.as_deref(),
            Some(&uid),
            machine.pvc_name.as_deref(),
        )
        .await
        .map_err(api_pending)?;
    if pods.iter().any(|p| {
        !owned(&machine, p)
            || machine
                .pod_uid
                .as_ref()
                .is_some_and(|uid| p.uid.as_ref() != Some(uid))
    }) {
        return block(
            manager,
            &machine,
            &mut progress,
            "delete_pod_identity_conflict",
        )
        .await;
    }
    if machine.resource_state == ResourceState::Deleted && !pods.is_empty() {
        if pods.len() != 1 || !stop_is_valid(&delete_projection(&machine), &progress) {
            return block(
                manager,
                &machine,
                &mut progress,
                "delete_pod_identity_conflict",
            )
            .await;
        }
        let pod = &pods[0];
        if pod.runtime_generation.as_deref()
            != Some(machine.runtime_generation.to_string().as_str())
            || pod.credential_epoch.as_deref()
                != Some(machine.credential_epoch.to_string().as_str())
            || pod.writer_fence.as_deref() != Some(machine.writer_fence.as_str())
            || pod.uid != machine.pod_uid
            || machine
                .native_pod_name
                .as_ref()
                .is_some_and(|name| name != &pod.name)
        {
            return block(
                manager,
                &machine,
                &mut progress,
                "delete_pod_identity_conflict",
            )
            .await;
        }
        progress.resources_complete = false;
        machine = save(&manager.db, &machine, &mut progress, Document::new()).await?;
        live_before_effect(&manager.db, &machine).await?;
        manager
            .api
            .delete_resource_if_identity(
                &machine.sandbox_namespace,
                "pods",
                &pod.name,
                pod.uid
                    .as_deref()
                    .ok_or_else(|| pending("delete_pod_identity_missing"))?,
                pod.resource_version
                    .as_deref()
                    .ok_or_else(|| pending("delete_pod_revision_missing"))?,
            )
            .await
            .map_err(api_pending)?;
        return block(manager, &machine, &mut progress, "delete_pod_gc_pending").await;
    }
    if let Some(name) = &machine.native_pod_name
        && let Some(pod) = manager
            .api
            .resource(&machine.sandbox_namespace, "pods", name)
            .await
            .map_err(api_pending)?
    {
        if pod.uid != machine.pod_uid || !owned(&machine, &pod) {
            return block(
                manager,
                &machine,
                &mut progress,
                "delete_pod_identity_conflict",
            )
            .await;
        }
        return block(manager, &machine, &mut progress, "delete_pod_still_present").await;
    }
    if !pods.is_empty() {
        return block(manager, &machine, &mut progress, "delete_pod_still_present").await;
    }
    // Pending claims and exact persisted PVs share the same current verifier.
    let pv = discover_original_volume(manager, &mut machine, &mut progress, pvc.as_ref()).await?;
    let current_creation_terminal =
        operations::creation_responsibilities_terminal(&manager.db, &machine).await?;
    if machine.resource_state != ResourceState::Deleted {
        progress.creation_terminal = current_creation_terminal;
    }
    if progress.stop.is_none() {
        if super::remote_machine_native::predecessor_stopped(&machine) {
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
                return block(manager, &machine, &mut progress, "creation_result_unknown").await;
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
                    &machine,
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
    progress.blocker = None;
    machine = save(&manager.db, &machine, &mut progress, Document::new()).await?;
    if !stop_is_valid(&delete_projection(&machine), &progress) {
        return block(
            manager,
            &machine,
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
            &machine,
        )
        .await;
    }
    if let Some(s) = &sandbox {
        if s.deletion_timestamp.is_some() {
            return block(
                manager,
                &machine,
                &mut progress,
                "delete_sandbox_gc_pending",
            )
            .await;
        }
        live_before_effect(&manager.db, &machine).await?;
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
            .map_err(api_pending)?;
        return block(
            manager,
            &machine,
            &mut progress,
            "delete_sandbox_gc_pending",
        )
        .await;
    }
    if let Some(pvc) = &pvc {
        if pvc.deletion_timestamp.is_some() {
            return block(
                manager,
                &machine,
                &mut progress,
                "delete_pvc_finalizer_pending",
            )
            .await;
        }
        live_before_effect(&manager.db, &machine).await?;
        manager
            .api
            .delete_resource_if_identity(
                &machine.sandbox_namespace,
                "persistentvolumeclaims",
                &pvc.name,
                pvc.uid
                    .as_deref()
                    .ok_or_else(|| pending("delete_pvc_identity_missing"))?,
                pvc.resource_version
                    .as_deref()
                    .ok_or_else(|| pending("delete_pvc_revision_missing"))?,
            )
            .await
            .map_err(api_pending)?;
        return block(manager, &machine, &mut progress, "delete_pvc_gc_pending").await;
    }
    if pv.is_some() {
        return block(
            manager,
            &machine,
            &mut progress,
            "delete_original_pv_csi_pending",
        )
        .await;
    }
    progress.storage = DeleteStorageDisposition::ControlPlaneReleased;
    progress.resources_complete = true;
    if !current_creation_terminal {
        return block(manager, &machine, &mut progress, "creation_result_unknown").await;
    }
    progress.blocker = None;
    let cleanup = doc! {"cleanup_state":"complete","cleanup_evidence_digest":&progress.evidence_id};
    machine = save(&manager.db, &machine, &mut progress, cleanup).await?;
    if machine.resource_state == ResourceState::Deleted {
        // Only the resource milestone changes. Original stop/accounting and
        // successful operation/slot state remain final and idempotent.
        return Ok(());
    }
    if !finality_ready(&machine, &progress) {
        return block(
            manager,
            &machine,
            &mut progress,
            "delete_remaining_responsibilities_pending",
        )
        .await;
    }
    if super::remote_machine_runtime::require_accounting_final(&manager.db, &machine)
        .await
        .is_err()
    {
        return block(manager, &machine, &mut progress, "accounting_pending").await;
    }
    super::remote_machine_native::finalize_persisted_delete(&manager.db, &machine).await?;
    manager.complete_if_final(&machine).await
}

#[cfg(test)]
#[path = "remote_machine_delete_recovery_tests.rs"]
mod tests;
