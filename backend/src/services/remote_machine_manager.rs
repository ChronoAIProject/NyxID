//! Durable Sandbox effect consumer and bounded original-resource recovery loop.
use super::{remote_machine_operation_service as operations, remote_machine_service as lifecycle};
use crate::{
    errors::{AppError, AppResult},
    models::{
        remote_machine::{COLLECTION_NAME as MACHINES, ManagedMachine, ResourceState},
        remote_machine_operation::{COLLECTION_NAME as OPERATIONS, LifecycleAction},
        remote_machine_outbox::{EffectKind, MachineEffectOutbox},
    },
};
use chrono::Utc;
use futures::{StreamExt, TryStreamExt};
use mongodb::{
    Database,
    bson::{self, Document, doc},
};
use nyxid_machine_updater::sandbox::{
    ControlledResourceObservation, SandboxApi, SandboxObservation,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

static MANAGER_STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, serde::Serialize)]
pub struct PlatformReadiness {
    pub configuration_ready: bool,
    pub manager_started: bool,
    pub namespace: &'static str,
    pub blockers: Vec<&'static str>,
    pub physical_acceptance: &'static str,
    pub lifecycle_basis: &'static str,
}

/// Configuration is an admission prerequisite, never physical acceptance.
/// Every generation still requires current native identity and lifecycle checks.
pub fn platform_readiness(base_url: &str) -> PlatformReadiness {
    let mut blockers = Vec::new();
    let image = std::env::var("NYXID_REMOTE_MACHINE_IMAGE_DIGEST").unwrap_or_default();
    let digest = image.rsplit_once('@').map(|(_, digest)| digest);
    if !digest.is_some_and(|d| {
        d.len() == 71 && d.starts_with("sha256:") && d[7..].bytes().all(|b| b.is_ascii_hexdigit())
    }) {
        blockers.push("runtime_image_reference_missing_or_invalid");
    }
    if configured_storage_class().is_err() {
        blockers.push("storage_class_configuration_invalid");
    }
    match super::remote_machine_kubernetes_auth::KubernetesConnection::from_environment(
        Duration::from_secs(15),
    ) {
        Ok(connection) => {
            if !connection.credentials.credential_file_available() {
                blockers.push("manager_credential_file_unavailable");
            }
        }
        Err(_) => blockers.push("kubernetes_identity_configuration_invalid"),
    }
    let bootstrap = std::env::var("NYXID_REMOTE_MACHINE_BOOTSTRAP_BASE_URL")
        .unwrap_or_else(|_| base_url.to_owned());
    if !reqwest::Url::parse(&bootstrap).ok().is_some_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    }) {
        blockers.push("bootstrap_origin_invalid");
    }
    let manager_started = MANAGER_STARTED.load(Ordering::Acquire);
    if !manager_started {
        blockers.push("manager_not_started");
    }
    PlatformReadiness {
        configuration_ready: blockers.is_empty(),
        manager_started,
        namespace: "nyxid-machine",
        blockers,
        physical_acceptance: "native_state_only_no_physical_guarantee",
        lifecycle_basis: "agent_sandbox_native_observation",
    }
}

/// Blank/unset means Kubernetes cluster default; never an explicit empty PVC
/// class. A nonempty configured name remains explicit and must be valid.
pub(crate) fn configured_storage_class() -> anyhow::Result<Option<String>> {
    let value = match std::env::var("NYXID_REMOTE_MACHINE_STORAGE_CLASS") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    nyxid_machine_updater::sandbox::validate_storage_class_name(value)?;
    Ok(Some(value.to_owned()))
}

fn sandbox_api_from_environment() -> anyhow::Result<SandboxApi> {
    let connection = super::remote_machine_kubernetes_auth::KubernetesConnection::from_environment(
        Duration::from_secs(20),
    )?;
    SandboxApi::with_credentials_client(connection.base, connection.credentials, connection.client)
}

/// Read-only API validity, not CSI readiness. Used only by hosted provisioning
/// preflight; retained-volume operations do not depend on today's default.
pub(crate) async fn verified_storage_class_from_environment() -> anyhow::Result<String> {
    let configured = configured_storage_class()?;
    sandbox_api_from_environment()?
        .resolve_storage_class(configured.as_deref())
        .await
}

pub async fn platform_readiness_for_provisioning(base_url: &str) -> PlatformReadiness {
    let mut readiness = platform_readiness(base_url);
    if readiness.configuration_ready && verified_storage_class_from_environment().await.is_err() {
        readiness
            .blockers
            .push(if configured_storage_class().ok().flatten().is_some() {
                "storage_class_unavailable_or_unverified"
            } else {
                "default_storage_class_unavailable_or_unverified"
            });
        readiness.configuration_ready = false;
    }
    readiness
}

#[derive(Clone)]
pub struct Manager {
    pub db: Database,
    pub billing: Option<Arc<super::billing::BillingService>>,
    pub api: SandboxApi,
    pub storage_class: Option<String>,
    pub namespace: String,
    pub bootstrap_base_url: String,
}

fn unavailable() -> AppError {
    AppError::Conflict("managed_platform_observation_pending".into())
}
fn api_error(_: anyhow::Error) -> AppError {
    unavailable()
}

/// Lease freshness is separate from Node/owner identity and native stop facts.
fn node_lease_fresh(
    lease: &nyxid_machine_updater::sandbox::NodeLeaseObservation,
    now: chrono::DateTime<Utc>,
) -> bool {
    if lease.uid.as_deref().is_none_or(str::is_empty)
        || lease.resource_version.as_deref().is_none_or(str::is_empty)
        || lease.deletion_timestamp.is_some()
    {
        return false;
    }
    let Some(renewed) = lease
        .renew_time
        .as_deref()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|v| v.with_timezone(&Utc))
    else {
        return false;
    };
    let Some(duration) = lease
        .lease_duration_seconds
        .filter(|v| *v > 0)
        .and_then(chrono::Duration::try_seconds)
    else {
        return false;
    };
    renewed <= now + chrono::Duration::seconds(5)
        && renewed
            .checked_add_signed(duration)
            .is_some_and(|end| end > now)
}

impl Manager {
    /// One bounded pass. Claims are durable and reclaimable after process loss;
    /// API results are observations, never a lifecycle success by themselves.
    pub async fn tick(&self) -> AppResult<()> {
        lifecycle::suspend_ineligible_with_billing(&self.db, self.billing.clone()).await?;
        operations::cancel_undispatched_delete_effects(&self.db).await?;
        for _ in 0..16 {
            let Some(effect) =
                operations::claim_effect(&self.db, Utc::now(), Duration::from_secs(60)).await?
            else {
                break;
            };
            let claim = effect.claim_id.as_deref().ok_or_else(unavailable)?;
            let result = self.apply_effect(&effect).await;
            if result.is_err() && operations::has_fresh_undispatched_claim(&effect) {
                // apply_effect can itself safety-pause before returning an
                // error. Seal that same claim before either acknowledgement
                // exit can discard its final non-dispatch attestation.
                if let Some(current) =
                    lifecycle::get_machine(&self.db, &effect.owner_id, &effect.machine_id).await?
                {
                    Box::pin(operations::cancel_undispatched_claim(
                        &self.db, &current, &effect,
                    ))
                    .await?;
                }
            }
            operations::acknowledge_effect(
                &self.db,
                &effect.id,
                claim,
                result.is_ok(),
                result.as_ref().err().map(|_| "sandbox_effect_pending"),
                Utc::now(),
            )
            .await?;
            if result.is_err() {
                self.progress(&effect.operation_id, "sandbox_effect_pending")
                    .await?;
            }
        }
        let machines = self.observation_candidates().await?;
        // Observation includes large transactional/BSON futures. Poll each
        // bounded observation on the existing Tokio scheduler instead of
        // nesting its entire poll stack below the lifecycle pass. JoinSet
        // owns cancellation: dropping this pass aborts its observation tasks.
        let mut observations = tokio::task::JoinSet::new();
        let mut first_error = None;
        for machine in machines {
            if observations.len() == 8 {
                let result = observations
                    .join_next()
                    .await
                    .ok_or_else(unavailable)?
                    .map_err(|_| unavailable())
                    .and_then(|result| result);
                if let Err(error) = result {
                    first_error.get_or_insert(error);
                }
            }
            let manager = self.clone();
            observations.spawn(async move {
                let result: AppResult<()> = async {
                    // Repair a lost diagnostic append after durable quarantine,
                    // without waiting for Kubernetes connectivity to recover.
                    if machine.native_uncertainty.is_some() {
                        manager.record_native_investigation(&machine).await?;
                    }
                    if !matches!(
                        tokio::time::timeout(Duration::from_secs(45), manager.observe(&machine))
                            .await,
                        Ok(Ok(()))
                    ) && let Some(op) = &machine.active_operation_id
                    {
                        manager.progress(op, "runtime_evidence_pending").await?;
                    }
                    if machine.specialist_assignment_state == "pending" {
                        // The durable pending choice survives a lost WSS response.
                        // Existing assignment policy is applied only once, while Ready.
                        let _ = super::remote_machine_configuration::assign_specialist(
                            &manager.db,
                            &machine.node_id,
                        )
                        .await;
                    }
                    Ok(())
                }
                .await;
                // An unknown machine still moves to the back of its scan
                // share, including diagnostic failures before native I/O.
                manager
                    .db
                    .collection::<Document>(MACHINES)
                    .update_one(
                        doc! {"_id":&machine.node_id},
                        doc! {"$set":{"manager_observed_at":bson::DateTime::now()}},
                    )
                    .await?;
                result
            });
        }
        while let Some(result) = observations.join_next().await {
            if let Err(error) = result.map_err(|_| unavailable()).and_then(|result| result) {
                first_error.get_or_insert(error);
            }
        }
        // Finalized tombstones retain their exact cleanup authority. This
        // independent bounded share catches controller resources that appear
        // after the final transaction without reopening financial state.
        self.reconcile_finalized_cleanup().await?;
        self.inventory().await?;
        super::remote_machine_events::sweep(&self.db, Utc::now()).await?;
        super::remote_machine_reclamation::sweep(&self.db, Utc::now()).await?;
        first_error.map_or(Ok(()), Err)
    }

    /// Give due original deletes their own bounded share. An operation lookup
    /// excludes stale/superseded delete rows and future retries before the
    /// limit, so neither ordinary machines nor old operations can starve them.
    async fn observation_candidates(&self) -> AppResult<Vec<ManagedMachine>> {
        let rows: Vec<Document> = self.db.collection::<Document>(MACHINES).aggregate(vec![
            doc! {"$match":{"resource_state":"deleting","active_operation_id":{"$type":"string"}}},
            doc! {"$lookup":{"from":OPERATIONS,"let":{"operation":"$active_operation_id","machine":"$_id","owner":"$owner_id"},
                "pipeline":[{"$match":{"action":"delete","state":{"$in":["accepted","applying","reconciling"]},
                    "$or":[{"next_attempt_at":bson::Bson::Null},{"next_attempt_at":{"$lte":bson::DateTime::now()}}],
                    "terminal_at":bson::Bson::Null,
                    "$expr":{"$and":[{"$eq":["$_id","$$operation"]},{"$eq":["$machine_id","$$machine"]},{"$eq":["$owner_id","$$owner"]},
                        super::remote_machine_delete_retry::eligible_expression(Utc::now())]}}},
                    {"$project":{"_id":1}}],"as":"due_delete"}},
            doc! {"$match":{"due_delete.0":{"$exists":true}}},
            doc! {"$sort":{"manager_observed_at":1,"_id":1}},
            doc! {"$limit":32_i64},
            doc! {"$unset":"due_delete"},
        ]).max_time(Duration::from_secs(3)).await?.try_collect().await?;
        let mut machines = rows
            .into_iter()
            .map(|row| bson::from_document(row).map_err(|_| unavailable()))
            .collect::<AppResult<Vec<ManagedMachine>>>()?;
        let ordinary: Vec<ManagedMachine> = self
            .db
            .collection(MACHINES)
            .find(doc! {"resource_state":{"$nin":["deleted","deleting"]}})
            .sort(doc! {"manager_observed_at":1,"_id":1})
            .limit(32)
            .max_time(Duration::from_secs(3))
            .await?
            .try_collect()
            .await?;
        machines.extend(ordinary);
        Ok(machines)
    }

    async fn finalized_cleanup_candidates(&self) -> AppResult<Vec<ManagedMachine>> {
        Ok(self.db.collection::<ManagedMachine>(MACHINES)
            .find(doc! {"resource_state":"deleted","sandbox_uid":{"$type":"string"},
                "active_operation_id":{"$type":"string"},"delete_recovery.evidence_version":{"$in":[1_i32,super::remote_machine_delete_recovery::EVIDENCE_VERSION]}})
            .sort(doc! {"manager_observed_at":1,"_id":1}).limit(8)
            .max_time(Duration::from_secs(3)).await?.try_collect().await?)
    }

    async fn reconcile_finalized_cleanup(&self) -> AppResult<()> {
        let machines = self.finalized_cleanup_candidates().await?;
        let mut work = futures::stream::iter(machines).map(|machine| async move {
            // Recovery returns private resource blockers. One blocked tombstone
            // cannot prevent other original scopes from making progress.
            let result = tokio::time::timeout(Duration::from_secs(45),
                super::remote_machine_delete_recovery::advance(self, &machine)).await;
            self.db.collection::<Document>(MACHINES).update_one(
                doc! {"_id":&machine.node_id,"resource_state":"deleted"},
                doc! {"$set":{"manager_observed_at":bson::DateTime::now()}},
            ).await?;
            if !matches!(result, Ok(Ok(())))
                && !matches!(&result, Ok(Err(error)) if super::remote_machine_delete_recovery::observation_error_code(error).is_some())
            {
                tracing::warn!(machine_id=%machine.node_id, "Remote Machine retained-resource cleanup pending");
            }
            Ok::<(), AppError>(())
        }).buffer_unordered(4);
        while let Some(result) = work.next().await {
            result?;
        }
        Ok(())
    }

    /// Funding deadlines do not wait behind slow cleanup/provider requests.
    /// This uses the same durable lifecycle fence and Sandbox adapter; expiry
    /// is only extended after a committed reservation. Capacity delays fail
    /// closed at the old cutoff, without acquiring unfunded responsibility.
    pub async fn funding_tick(&self) -> AppResult<()> {
        // Free safety windows use the same native shutdownTime contract.
        let free: Vec<ManagedMachine> = self.db.collection(MACHINES).find(doc!{"free_slot":true,"resource_state":{"$in":["ready","provisioning","resuming"]}}).sort(doc!{"native_safety_checked_at":1,"_id":1}).limit(16).max_time(Duration::from_secs(3)).await?.try_collect().await?;
        for machine in free {
            if super::remote_machine_runtime::pending_resume_initialization(
                &self.db,
                &machine,
                Utc::now(),
            )
            .await?
            {
                // Keep the stopped predecessor's deadline unchanged. The
                // accepted finite Resume will initialize the next generation.
            } else if machine
                .native_run_not_after
                .is_some_and(|at| at <= Utc::now())
            {
                super::remote_machine_runtime::safety_pause(
                    &self.db,
                    self.billing.clone(),
                    &machine,
                    "runtime_safety_deadline_expired",
                )
                .await?;
            } else {
                super::remote_machine_runtime::ensure_native_deadline(&self.db, &machine, false)
                    .await?;
            }
            self.db
                .collection::<Document>(MACHINES)
                .update_one(
                    super::remote_machine_runtime::maintenance_snapshot_filter(&machine),
                    doc! {"$set":{"native_safety_checked_at":bson::DateTime::now()}},
                )
                .await?;
        }
        let cutoff = Utc::now()
            + chrono::Duration::seconds(super::remote_machine_runtime::renewal_margin_seconds());
        let machines: Vec<ManagedMachine> = self.db.collection(MACHINES)
            .find(doc!{"free_slot":false,"resource_state":{"$in":["ready","provisioning","resuming","reconciling","degraded"]},
                "funded_until":{"$lte":bson::DateTime::from_chrono(cutoff)}})
            .sort(doc!{"funded_until":1,"_id":1}).limit(8)
            .max_time(Duration::from_secs(3)).await?.try_collect().await?;
        let mut work = futures::stream::iter(machines)
            .map(|machine| async move {
                let Some(billing) = self.billing.clone() else {
                    return Ok(());
                };
                let renewed = tokio::time::timeout(
                    Duration::from_secs(15),
                    super::remote_machine_runtime::renew(&self.db, billing, &machine),
                )
                .await;
                let latest = lifecycle::get_machine(&self.db, &machine.owner_id, &machine.node_id)
                    .await?
                    .ok_or_else(unavailable)?;
                let expired = !super::remote_machine_runtime::funding_is_live(&latest, Utc::now());
                let failed_same_window = !matches!(renewed, Ok(Ok(())))
                    && latest.funded_until == machine.funded_until
                    && latest.runtime_generation == machine.runtime_generation;
                if expired || failed_same_window {
                    super::remote_machine_runtime::safety_pause(
                        &self.db,
                        self.billing.clone(),
                        &latest,
                        if expired {
                            "runtime_funding_expired"
                        } else {
                            "runtime_funding_unavailable"
                        },
                    )
                    .await?;
                } else if latest.funded_until != machine.funded_until {
                    // A slow/failed API update cannot roll back the money commit;
                    // the ordinary reconciler repairs this same current intent.
                    let _ = tokio::time::timeout(
                        Duration::from_secs(10),
                        self.observe_resource(&latest),
                    )
                    .await;
                }
                Ok::<(), AppError>(())
            })
            .buffer_unordered(8);
        while let Some(result) = work.next().await {
            result?;
        }
        Ok(())
    }

    pub fn apply_effect<'a>(
        &'a self,
        effect: &'a MachineEffectOutbox,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(self.apply_effect_inner(effect))
    }

    async fn apply_effect_inner(&self, effect: &MachineEffectOutbox) -> AppResult<()> {
        let mut machine = lifecycle::get_machine(&self.db, &effect.owner_id, &effect.machine_id)
            .await?
            .ok_or_else(unavailable)?;
        if effect.kind == EffectKind::InstallProviderDeadline {
            // Superseded optional platform receipt work has no native authority.
            return Ok(());
        }
        if machine.resource_state == ResourceState::Deleted {
            return Ok(());
        }
        if machine.active_operation_id.as_deref() != Some(effect.operation_id.as_str()) {
            // The successor owns desired state. Old lost-response effects are
            // never allowed to resurrect or remove its resource.
            let cancelled =
                operations::cancel_undispatched_claim(&self.db, &machine, effect).await?;
            if !cancelled && operations::has_fresh_undispatched_claim(effect) {
                return Err(unavailable());
            }
            // Legacy unknown deliveries may be acknowledged as inert work;
            // that bookkeeping never discharges their request responsibility.
            return Ok(());
        }
        if machine.resource_state == ResourceState::Deleting
            && effect.kind != EffectKind::DeleteSandbox
        {
            let cancelled =
                operations::cancel_undispatched_claim(&self.db, &machine, effect).await?;
            if !cancelled && operations::has_fresh_undispatched_claim(effect) {
                return Err(unavailable());
            }
            // Legacy unknown deliveries may be acknowledged as inert work;
            // that bookkeeping never discharges their request responsibility.
            return Ok(());
        }
        if effect.kind == EffectKind::DeleteSandbox {
            return super::remote_machine_delete_recovery::advance(self, &machine).await;
        }
        let requires_funding = matches!(
            effect.kind,
            EffectKind::CreateSandbox | EffectKind::ResumeSandbox | EffectKind::RekeySandbox
        ) || (effect.kind == EffectKind::UpdateSandboxDeadline
            && effect.desired_mode == "running");
        if requires_funding && !super::remote_machine_runtime::funding_is_live(&machine, Utc::now())
        {
            super::remote_machine_runtime::safety_pause(
                &self.db,
                self.billing.clone(),
                &machine,
                "runtime_funding_unavailable",
            )
            .await?;
            return Err(unavailable());
        }
        let operation =
            operations::get_operation(&self.db, &machine.owner_id, &effect.operation_id)
                .await?
                .ok_or_else(unavailable)?;
        if matches!(
            operation.action,
            LifecycleAction::Create
                | LifecycleAction::Resume
                | LifecycleAction::Rekey
                | LifecycleAction::Reconcile
        ) && operation.terminal_at.is_none()
            && operation
                .deadline_at
                .unwrap_or(operation.created_at + chrono::Duration::minutes(15))
                <= Utc::now()
        {
            super::remote_machine_runtime::safety_pause(
                &self.db,
                self.billing.clone(),
                &machine,
                "expansion_deadline_exceeded",
            )
            .await?;
            return Err(unavailable());
        }
        if matches!(effect.kind, EffectKind::CreateSandbox) && machine.runtime_generation == 0 {
            machine = lifecycle::advance_runtime_generation(
                &self.db,
                &machine.node_id,
                machine.revision,
                0,
                None,
                None,
            )
            .await?;
        }
        if effect.kind == EffectKind::ResumeSandbox {
            // Capture the predecessor's stop before asking the controller for a
            // successor. Retried resume effects do not increment a second time.
            if operation
                .desired_runtime_generation
                .is_some_and(|generation| machine.runtime_generation < generation)
            {
                machine = lifecycle::advance_runtime_generation(
                    &self.db,
                    &machine.node_id,
                    machine.revision,
                    machine.runtime_generation,
                    None,
                    None,
                )
                .await?;
            }
        }
        if matches!(
            effect.kind,
            EffectKind::SuspendSandbox | EffectKind::RekeySandbox
        ) && machine.sandbox_uid.is_none()
        {
            // A create response may have been lost. Read and bind the original
            // deterministic intent before attempting a stop or delete.
            self.observe_resource(&machine).await?;
            machine = lifecycle::get_machine(&self.db, &machine.owner_id, &machine.node_id)
                .await?
                .ok_or_else(unavailable)?;
        }
        let running = matches!(
            effect.kind,
            EffectKind::CreateSandbox | EffectKind::ResumeSandbox
        ) || (effect.kind == EffectKind::UpdateSandboxDeadline
            && effect.desired_mode == "running")
            || (effect.kind == EffectKind::RekeySandbox
                && operation.desired_runtime_generation == Some(machine.runtime_generation));
        let running_dispatch = if running {
            super::remote_machine_runtime::validate_saved_contract(&self.db, &machine).await?;
            self.require_workspace(&machine).await?;
            machine = super::remote_machine_runtime::mark_submission(&self.db, &machine).await?;
            Some(operations::begin_submission(&self.db, &machine, effect).await?)
        } else {
            None
        };
        let intent = super::remote_machine_adapter::intent(
            &machine,
            &effect.operation_id,
            self.storage_class.as_deref(),
            running,
            &self.bootstrap_base_url,
        );
        if let Some(dispatch) = &running_dispatch
            && !self
                .running_dispatch_is_live(&machine, &effect.id, dispatch)
                .await?
        {
            return Ok(());
        }
        let observation = if let Some(uid) = machine.sandbox_uid.clone() {
            self.api
                .apply_if_uid(&intent, &uid)
                .await
                .map_err(api_error)?
        } else {
            if let Some(current) = self.api.get_for_intent(&intent).await.map_err(api_error)? {
                current
            } else {
                if let Some(dispatch) = &running_dispatch
                    && !self
                        .running_dispatch_is_live(&machine, &effect.id, dispatch)
                        .await?
                {
                    return Ok(());
                }
                self.api.apply(&intent).await.map_err(api_error)?
            }
        };
        if let Some(dispatch) = &running_dispatch {
            operations::finish_submission(&self.db, &effect.id, dispatch, Some(&observation))
                .await?;
        }
        self.record_observation(&machine, &observation).await?;
        self.progress(&effect.operation_id, "runtime_evidence_pending")
            .await?;
        Ok(())
    }

    /// A durable dispatch remains unresolved if the request is cancelled after
    /// it starts. Only a request fenced out before this call is terminal here.
    async fn running_dispatch_is_live(
        &self,
        machine: &ManagedMachine,
        effect: &str,
        dispatch: &str,
    ) -> AppResult<bool> {
        // Recheck after the durable dispatch journal and immediately before
        // Running I/O. A failed read proves only that this attempt did not send;
        // it never resolves earlier dispatch responsibilities.
        if let Err(error) = self.require_resume_node_health(machine).await {
            operations::finish_submission(&self.db, effect, dispatch, None).await?;
            return Err(error);
        }
        let live = lifecycle::managed_machine_for_node(&self.db, &machine.node_id)
            .await?
            .ok_or_else(unavailable)?;
        let allowed = live.owner_id == machine.owner_id
            && live.revision == machine.revision
            && live.active_operation_id == machine.active_operation_id
            && matches!(
                live.resource_state,
                ResourceState::Provisioning
                    | ResourceState::Ready
                    | ResourceState::Resuming
                    | ResourceState::Reconciling
                    | ResourceState::Degraded
            )
            && live.runtime_generation == machine.runtime_generation
            && live.credential_epoch == machine.credential_epoch
            && live.writer_fence == machine.writer_fence
            && live.native_uncertainty.is_none()
            && super::remote_machine_runtime::funding_is_live(&live, Utc::now());
        if !allowed {
            operations::finish_submission(&self.db, effect, dispatch, None).await?;
        }
        Ok(allowed)
    }

    /// Operation labels do not advance Kubernetes generation. Record the exact
    /// next generation only when this UID/RV-preconditioned patch changes spec,
    /// before I/O so a lost response retains the original stop responsibility.
    async fn prepare_delete_suspension(
        &self,
        machine: &ManagedMachine,
        observed: &SandboxObservation,
    ) -> AppResult<ManagedMachine> {
        if machine.resource_state != ResourceState::Deleting
            || (observed.desired_suspended && observed.shutdown_time.is_none())
        {
            return Ok(machine.clone());
        }
        let generation = observed
            .generation
            .filter(|g| *g > 0)
            .and_then(|g| g.checked_add(1))
            .ok_or_else(unavailable)?;
        self.db.collection::<ManagedMachine>(MACHINES).find_one_and_update(
            doc! {"_id":&machine.node_id,"owner_id":&machine.owner_id,"revision":machine.revision,
                "resource_state":"deleting","active_operation_id":&machine.active_operation_id,
                "runtime_generation":machine.runtime_generation,"credential_epoch":machine.credential_epoch,
                "writer_fence":&machine.writer_fence,"sandbox_uid":&machine.sandbox_uid,"writer_admitted":false},
            doc! {"$set":{"delete_suspension_generation":generation,
                "delete_suspension_operation_id":&machine.active_operation_id},"$inc":{"revision":1_i64}},
        ).return_document(mongodb::options::ReturnDocument::After).await?.ok_or_else(unavailable)
    }

    async fn uncertain(&self, machine: &ManagedMachine, reason: &'static str) -> AppResult<()> {
        super::remote_machine_native::quarantine(&self.db, machine, reason).await?;
        if let Some(current) =
            lifecycle::managed_machine_for_node(&self.db, &machine.node_id).await?
            && current.native_uncertainty.is_some()
            && current.runtime_generation == machine.runtime_generation
            && current.writer_fence == machine.writer_fence
        {
            self.record_native_investigation(&current).await?;
        }
        Err(unavailable())
    }

    async fn record_native_investigation(&self, machine: &ManagedMachine) -> AppResult<()> {
        let identity_hash = hex::encode(Sha256::digest(
            format!(
                "native-runtime:{}:{}:{}",
                machine.node_id, machine.runtime_generation, machine.writer_fence
            )
            .as_bytes(),
        ));
        super::remote_machine_events::record_investigation(
            &self.db,
            super::remote_machine_events::InvestigationIdentity {
                kind: "Sandbox",
                namespace: machine.sandbox_namespace.clone(),
                name: Some(machine.sandbox_name.clone()),
                resource_uid: machine.sandbox_uid.clone(),
                sandbox_uid: machine.sandbox_uid.clone(),
                machine_id: Some(machine.node_id.clone()),
                owner_id: Some(machine.owner_id.clone()),
                operation_id: machine.active_operation_id.clone(),
                identity_hash,
            },
            "native_runtime_uncertainty",
        )
        .await
    }

    /// Retained Node identity is a health target across generation initialization,
    /// not a running/stop proof. A successor Pod binds its own Node through the
    /// existing owned-Pod observation CAS.
    async fn require_node_health(
        &self,
        machine: &ManagedMachine,
        name: &str,
        expected_uid: Option<&str>,
    ) -> AppResult<nyxid_machine_updater::sandbox::NodeObservation> {
        let n = self
            .api
            .node(name)
            .await
            .map_err(api_error)?
            .ok_or_else(unavailable)?;
        if n.name != name
            || n.uid
                .as_ref()
                .is_some_and(|uid| expected_uid.is_some_and(|bound| bound != uid.as_str()))
        {
            self.uncertain(machine, "predecessor_node_identity_conflict")
                .await?;
        }
        if !n.ready || n.deletion_timestamp.is_some() {
            self.uncertain(machine, "predecessor_node_unhealthy_or_stale")
                .await?;
        }
        let node_uid = n
            .uid
            .as_deref()
            .filter(|uid| !uid.is_empty())
            .ok_or_else(unavailable)?;
        let lease = self
            .api
            .node_lease(name)
            .await
            .map_err(api_error)?
            .ok_or_else(unavailable)?;
        if lease.owner_identity_conflict
            || lease
                .owner_node_uid
                .as_deref()
                .is_some_and(|uid| uid != node_uid)
            || lease
                .holder_identity
                .as_deref()
                .is_some_and(|holder| holder != name)
            || lease.name != name
            || lease.namespace != "kube-node-lease"
        {
            self.uncertain(machine, "predecessor_node_lease_identity_conflict")
                .await?;
        }
        // A failed/expired health read does not prove an identity conflict or
        // a stop. Leave the durable predecessor and execution cutoff intact.
        if lease.owner_node_uid.as_deref().is_none_or(str::is_empty)
            || lease.holder_identity.as_deref().is_none_or(str::is_empty)
            || !node_lease_fresh(&lease, Utc::now())
        {
            return Err(unavailable());
        }
        Ok(n)
    }

    async fn require_resume_node_health(&self, machine: &ManagedMachine) -> AppResult<()> {
        let operation_id = machine
            .active_operation_id
            .as_deref()
            .ok_or_else(unavailable)?;
        let operation = operations::get_operation(&self.db, &machine.owner_id, operation_id)
            .await?
            .ok_or_else(unavailable)?;
        if operation.action != LifecycleAction::Resume {
            return Ok(());
        }
        if let Some(name) = machine.native_node_name.as_deref() {
            self.require_node_health(machine, name, machine.native_node_uid.as_deref())
                .await?;
        } else if machine.runtime_generation > 1 {
            // An older initialized row without its predecessor binding cannot
            // turn missing health evidence into permission to dispatch Running.
            return Err(unavailable());
        }
        Ok(())
    }

    /// Missing retained PVC must never be repaired by creating an empty disk.
    async fn require_workspace(&self, machine: &ManagedMachine) -> AppResult<()> {
        if machine.native_uncertainty.is_some() {
            return Err(unavailable());
        }
        self.require_resume_node_health(machine).await?;
        if let Some(uid) = &machine.pvc_uid {
            let name = machine.pvc_name.as_deref().ok_or_else(unavailable)?;
            let pvc = self
                .api
                .resource(&machine.sandbox_namespace, "persistentvolumeclaims", name)
                .await
                .map_err(api_error)?;
            if !pvc.as_ref().is_some_and(|p| {
                p.uid.as_ref() == Some(uid)
                    && p.ownership_valid
                    && p.controller_owner_uid == machine.sandbox_uid
                    && p.deletion_timestamp.is_none()
                    && p.machine_id.as_deref() == Some(machine.node_id.as_str())
                    && p.owner_id.as_deref() == Some(machine.owner_id.as_str())
            }) {
                return self.uncertain(machine, "workspace_binding_unknown").await;
            }
        } else if machine.runtime_generation > 1 {
            return self.uncertain(machine, "workspace_binding_missing").await;
        }
        Ok(())
    }

    fn record_observation<'a>(
        &'a self,
        machine: &'a ManagedMachine,
        observed: &'a SandboxObservation,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(self.record_observation_inner(machine, observed, None))
    }

    async fn record_observation_inner(
        &self,
        machine: &ManagedMachine,
        observed: &SandboxObservation,
        delete_snapshot: Option<&mut ManagedMachine>,
    ) -> AppResult<()> {
        use super::remote_machine_native as native;
        use crate::models::remote_machine::NativeRuntimeObservation;
        let mut machine = if delete_snapshot.is_some() {
            machine.clone()
        } else {
            lifecycle::get_machine(&self.db, &machine.owner_id, &machine.node_id)
                .await?
                .ok_or_else(unavailable)?
        };
        if machine.native_uncertainty.is_some() {
            return Err(unavailable());
        }
        if machine.writer_admitted
            && machine
                .native_observation
                .as_ref()
                .is_some_and(|o| o.running && !super::remote_machine_native::fresh(o, Utc::now()))
        {
            return self.uncertain(&machine, "native_observation_expired").await;
        }
        let uid = observed.sandbox_uid.as_deref().ok_or_else(unavailable)?;
        if observed.machine_id.as_deref() != Some(&machine.node_id)
            || observed.owner_id.as_deref() != Some(&machine.owner_id)
            || observed.profile.as_deref() != Some(&machine.profile.profile_version)
            || machine
                .sandbox_uid
                .as_deref()
                .is_some_and(|expected| expected != uid)
        {
            return self.uncertain(&machine, "sandbox_ownership_conflict").await;
        }
        if observed.operation_id != machine.active_operation_id
            || observed.runtime_generation.as_deref()
                != Some(machine.runtime_generation.to_string().as_str())
            || observed.credential_epoch.as_deref()
                != Some(machine.credential_epoch.to_string().as_str())
            || observed.writer_fence.as_deref() != Some(machine.writer_fence.as_str())
        {
            return Err(unavailable()); // late operation; never bind/admit it
        }
        let generation = observed
            .generation
            .filter(|g| *g > 0)
            .ok_or_else(unavailable)?;
        let resource_version = observed
            .resource_version
            .clone()
            .filter(|v| !v.is_empty())
            .ok_or_else(unavailable)?;
        let operation_id = machine
            .active_operation_id
            .clone()
            .ok_or_else(unavailable)?;
        let operation = operations::get_operation(&self.db, &machine.owner_id, &operation_id)
            .await?
            .ok_or_else(unavailable)?;
        let now = Utc::now();
        if operation.machine_id.as_deref() != Some(machine.node_id.as_str())
            || operation.created_at > now
        {
            return Err(unavailable());
        }
        let pods = self
            .api
            .list_owned_resources(&machine.sandbox_namespace, "pods", uid)
            .await
            .map_err(|error| {
                if delete_snapshot.is_some() {
                    super::remote_machine_delete_recovery::api_pending(&machine, error)
                } else {
                    api_error(error)
                }
            })?;
        let pvcs = self
            .api
            .list_owned_resources(&machine.sandbox_namespace, "persistentvolumeclaims", uid)
            .await
            .map_err(|error| {
                if delete_snapshot.is_some() {
                    super::remote_machine_delete_recovery::api_pending(&machine, error)
                } else {
                    api_error(error)
                }
            })?;
        if pods.len() > 1 || pvcs.len() > 1 {
            return self.uncertain(&machine, "multiple_owned_resources").await;
        }
        let pod = pods.first();
        let pvc = pvcs.first();
        for object in pods.iter().chain(pvcs.iter()) {
            if !object.ownership_valid
                || object.controller_owner_uid.as_deref() != Some(uid)
                || object.namespace.as_deref() != Some(machine.sandbox_namespace.as_str())
                || object.machine_id.as_deref() != Some(machine.node_id.as_str())
                || object.owner_id.as_deref() != Some(machine.owner_id.as_str())
                || object.uid.is_none()
                || object.resource_version.is_none()
            {
                return self
                    .uncertain(&machine, "resource_ownership_conflict")
                    .await;
            }
        }
        if let Some(pod) = pod {
            if machine.pod_uid.is_some() && machine.pod_uid != pod.uid {
                return self.uncertain(&machine, "unexpected_pod_replacement").await;
            }
            if pod.deletion_timestamp.is_some() && pod.deletion_grace_period_seconds == Some(0) {
                return self.uncertain(&machine, "known_force_deletion").await;
            }
            if pod.runtime_generation.as_deref()
                != Some(machine.runtime_generation.to_string().as_str())
                || pod.credential_epoch.as_deref()
                    != Some(machine.credential_epoch.to_string().as_str())
                || pod.writer_fence.as_deref() != Some(machine.writer_fence.as_str())
            {
                return self
                    .uncertain(&machine, "pod_runtime_binding_conflict")
                    .await;
            }
        }
        if machine.pvc_uid.is_some() && pvc.is_none() {
            return self.uncertain(&machine, "workspace_disappeared").await;
        }
        if let Some(pvc) = pvc {
            if machine
                .pvc_uid
                .as_ref()
                .is_some_and(|uid| pvc.uid.as_ref() != Some(uid))
                || pvc.deletion_timestamp.is_some()
            {
                return self.uncertain(&machine, "workspace_binding_conflict").await;
            }
            if pod.is_some_and(|p| p.workspace_claim_name.as_deref() != Some(pvc.name.as_str())) {
                return self
                    .uncertain(&machine, "pod_workspace_binding_conflict")
                    .await;
            }
        }
        // Read the previously bound name too: a name-reused foreign Pod is not
        // hidden by an owner-filtered list. NotFound alone is never a stop.
        let tracked_name = machine
            .native_pod_name
            .as_deref()
            .or(observed.tracked_pod_name.as_deref());
        if pod.is_none()
            && let Some(name) = tracked_name
            && self
                .api
                .resource(&machine.sandbox_namespace, "pods", name)
                .await
                .map_err(|error| {
                    if delete_snapshot.is_some() {
                        super::remote_machine_delete_recovery::api_pending(&machine, error)
                    } else {
                        api_error(error)
                    }
                })?
                .is_some()
        {
            return self
                .uncertain(&machine, "predecessor_pod_identity_unknown")
                .await;
        }
        let node_name = pod
            .and_then(|p| p.node_name.as_deref())
            .or(machine.native_node_name.as_deref());
        let node = if let Some(name) = node_name {
            // Before a successor Pod has been bound, the retained Node belongs
            // to the stopped predecessor. A newly observed owned Pod may have
            // been scheduled elsewhere; thereafter its Node UID stays fenced.
            let expected_uid = if pod.is_none() || machine.pod_uid.is_some() {
                machine.native_node_uid.as_deref()
            } else {
                None
            };
            Some(
                self.require_node_health(&machine, name, expected_uid)
                    .await?,
            )
        } else {
            None
        };
        // Capture the selected CSI's PV identity before deleting its PVC. Retain
        // policy never becomes complete merely because the claim disappeared.
        let mut bindings = Document::new();
        if let Some(pvc) = pvc
            && let Some(name) = &pvc.volume_name
        {
            let pv = self
                .api
                .persistent_volume(name)
                .await
                .map_err(|error| {
                    if delete_snapshot.is_some() {
                        super::remote_machine_delete_recovery::api_pending(&machine, error)
                    } else {
                        api_error(error)
                    }
                })?
                .ok_or_else(unavailable)?;
            if pv.claim_uid != pvc.uid
                || pv.claim_namespace.as_deref() != Some(machine.sandbox_namespace.as_str())
                || pv.claim_name.as_deref() != Some(pvc.name.as_str())
                || pv.uid.is_none()
                || pv.deletion_timestamp.is_some()
                || pv.phase.as_deref() != Some("Bound")
                || machine
                    .native_pv_uid
                    .as_ref()
                    .is_some_and(|id| pv.uid.as_ref() != Some(id))
            {
                return self
                    .uncertain(&machine, "pv_claim_ownership_conflict")
                    .await;
            }
            bindings.insert("native_pv_name", name.clone());
            bindings.insert("native_pv_uid", pv.uid);
            bindings.insert("native_pv_reclaim_policy", pv.reclaim_policy);
        }
        let running = !observed.desired_suspended
            && observed.deletion_timestamp.is_none()
            && observed.observed_generation == Some(generation)
            && pod.is_some_and(|p| {
                p.phase.as_deref() == Some("Running")
                    && p.deletion_timestamp.is_none()
                    && p.node_name.is_some()
            })
            && pvc.is_some_and(|p| p.phase.as_deref() == Some("Bound") && p.volume_name.is_some())
            && node.is_some();
        let stopped = observed.desired_suspended
            && observed.suspended
            && observed.observed_generation == Some(generation)
            && pods.is_empty()
            && observed.deletion_timestamp.is_none()
            && (machine.pod_uid.is_none() || node.is_some())
            && (machine.resource_state != ResourceState::Deleting
                || (machine.delete_suspension_operation_id.as_ref() == Some(&operation_id)
                    && machine.delete_suspension_generation == Some(generation))
                || machine.native_observation.as_ref().is_some_and(|previous| {
                    previous.stop_confirmed
                        && previous.operation_id == operation_id
                        && previous.sandbox_generation == generation
                        && native::predecessor_stopped(&machine)
                }));
        if machine.resource_state == ResourceState::Ready && !running {
            if pod.is_some_and(|p| {
                p.phase.as_deref() == Some("Running") && p.deletion_timestamp.is_none()
            }) && observed.observed_generation != Some(generation)
                && !observed.desired_suspended
            {
                return Err(unavailable()); // applied generation pending; old cutoff only
            }
            return self
                .uncertain(&machine, "native_running_state_unknown")
                .await;
        }
        // Binding changes and native facts commit under the same revision CAS.
        if machine.sandbox_uid.is_none() {
            bindings.insert("sandbox_uid", uid);
            machine.sandbox_uid = Some(uid.into());
        }
        if let Some(p) = pod {
            bindings.insert("pod_uid", p.uid.clone());
            machine.pod_uid = p.uid.clone();
            bindings.insert("native_pod_name", p.name.clone());
            bindings.insert("native_node_name", p.node_name.clone());
            if let Some(n) = &node {
                bindings.insert("native_node_uid", n.uid.clone());
            }
        }
        if let Some(p) = pvc {
            bindings.insert("pvc_uid", p.uid.clone());
            bindings.insert("pvc_name", p.name.clone());
        }
        let observation = NativeRuntimeObservation {
            sandbox_uid: uid.into(),
            pod_uid: machine.pod_uid.clone(),
            runtime_generation: machine.runtime_generation,
            credential_epoch: machine.credential_epoch,
            writer_fence: machine.writer_fence.clone(),
            operation_id,
            sandbox_generation: generation,
            sandbox_resource_version: resource_version.clone(),
            observed_at: chrono::DateTime::from_timestamp_millis(now.timestamp_millis())
                .ok_or_else(unavailable)?,
            running,
            suspended: observed.suspended,
            stop_confirmed: stopped,
            run_not_after: observed
                .shutdown_time
                .as_deref()
                .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                .map(|v| v.with_timezone(&Utc)),
            evidence_ref: format!(
                "kubernetes-native:{}:{}:{}",
                uid, generation, resource_version
            ),
        };
        let saved = native::record(&self.db, &machine, &observation, bindings).await?;
        if let Some(snapshot) = delete_snapshot {
            *snapshot = saved;
        }
        Ok(())
    }

    pub(crate) fn observe_resource<'a>(
        &'a self,
        machine: &'a ManagedMachine,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(self.observe_resource_inner(machine, None))
    }

    pub(crate) fn observe_delete_resource<'a>(
        &'a self,
        machine: &'a mut ManagedMachine,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let snapshot = machine.clone();
            self.observe_resource_inner(&snapshot, Some(machine)).await
        })
    }

    async fn observe_resource_inner(
        &self,
        machine: &ManagedMachine,
        mut delete_snapshot: Option<&mut ManagedMachine>,
    ) -> AppResult<()> {
        if let Some(mut observed) = self
            .api
            .get(&machine.sandbox_namespace, &machine.sandbox_name)
            .await
            .map_err(|error| {
                if delete_snapshot.is_some() {
                    super::remote_machine_delete_recovery::api_pending(machine, error)
                } else {
                    api_error(error)
                }
            })?
        {
            // A previously admitted HTTP request can finish after a newer
            // owner transaction. Repair only the current Sandbox desired state;
            // runtime ticket admission separately fences stale Pod templates.
            let mut current = lifecycle::get_machine(&self.db, &machine.owner_id, &machine.node_id)
                .await?
                .ok_or_else(unavailable)?;
            if delete_snapshot.is_some() && current.revision != machine.revision {
                return Err(AppError::Conflict("delete_evidence_raced".into()));
            }
            if current
                .sandbox_uid
                .as_deref()
                .is_some_and(|uid| observed.sandbox_uid.as_deref() != Some(uid))
            {
                return Err(unavailable());
            }
            if observed.machine_id.as_deref() != Some(current.node_id.as_str())
                || observed.owner_id.as_deref() != Some(current.owner_id.as_str())
                || observed.profile.as_deref() != Some(current.profile.profile_version.as_str())
            {
                return Err(unavailable());
            }
            if current.sandbox_uid.is_none() {
                // Recover a lost create response under the original durable
                // allocation intent before applying a later owner stop/delete.
                let original_id = observed.operation_id.as_deref().ok_or_else(unavailable)?;
                let original = operations::get_operation(&self.db, &current.owner_id, original_id)
                    .await?
                    .ok_or_else(unavailable)?;
                let allocation=self.db.collection::<Document>(crate::models::remote_machine_outbox::COLLECTION_NAME)
                    .count_documents(doc!{"operation_id":original_id,"machine_id":&current.node_id,"owner_id":&current.owner_id,"kind":"create_sandbox"}).await?;
                if original.action != LifecycleAction::Create
                    || original.machine_id.as_deref() != Some(current.node_id.as_str())
                    || original.desired_runtime_generation != Some(current.runtime_generation)
                    || allocation != 1
                    || current.runtime_submission_started_at.is_none()
                    || observed.runtime_generation.as_deref()
                        != Some(current.runtime_generation.to_string().as_str())
                    || observed.credential_epoch.as_deref()
                        != Some(current.credential_epoch.to_string().as_str())
                    || observed.writer_fence.as_deref() != Some(current.writer_fence.as_str())
                {
                    return Err(unavailable());
                }
                let uid = observed.sandbox_uid.as_deref().ok_or_else(unavailable)?;
                current=self.db.collection::<ManagedMachine>(MACHINES).find_one_and_update(
                    doc!{"_id":&current.node_id,"revision":current.revision,"sandbox_uid":bson::Bson::Null},
                    doc!{"$set":{"sandbox_uid":uid},"$inc":{"revision":1_i64}}
                ).return_document(mongodb::options::ReturnDocument::After).await?.ok_or_else(unavailable)?;
            }
            let mut running = matches!(
                current.resource_state,
                ResourceState::Provisioning
                    | ResourceState::Ready
                    | ResourceState::Resuming
                    | ResourceState::Reconciling
                    | ResourceState::Degraded
            );
            if running && let Some(operation_id) = &current.active_operation_id {
                let op = operations::get_operation(&self.db, &current.owner_id, operation_id)
                    .await?
                    .ok_or_else(unavailable)?;
                if matches!(op.action, LifecycleAction::Resume | LifecycleAction::Rekey)
                    && op.desired_runtime_generation != Some(current.runtime_generation)
                {
                    // Waiting for the predecessor's physical stop is still a
                    // suspended intent. Reconciliation cannot skip the fence.
                    running = false;
                }
            }
            let funding_matches = observed
                .shutdown_time
                .as_deref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc))
                == if current.free_slot {
                    current.native_run_not_after
                } else {
                    current.funded_until
                };
            let tuple_matches = observed.runtime_generation.as_deref()
                == Some(current.runtime_generation.to_string().as_str())
                && observed.credential_epoch.as_deref()
                    == Some(current.credential_epoch.to_string().as_str())
                && observed.writer_fence.as_deref() == Some(current.writer_fence.as_str());
            if (observed.desired_suspended == running
                || (!running && observed.shutdown_time.is_some())
                || !tuple_matches
                || (running && !funding_matches)
                || observed.operation_id != current.active_operation_id)
                && let (Some(uid), Some(operation_id)) = (
                    current.sandbox_uid.clone(),
                    current.active_operation_id.clone(),
                )
            {
                if running
                    && (!super::remote_machine_runtime::funding_is_live(&current, Utc::now())
                        || current.runtime_generation <= 0)
                {
                    return Err(unavailable());
                }
                if running {
                    super::remote_machine_runtime::validate_saved_contract(&self.db, &current)
                        .await?;
                    self.require_workspace(&current).await?;
                    current =
                        super::remote_machine_runtime::mark_submission(&self.db, &current).await?;
                }
                let running_dispatch = if running {
                    let effect = self.db.collection::<MachineEffectOutbox>(
                        crate::models::remote_machine_outbox::COLLECTION_NAME,
                    ).find_one(doc! {"operation_id":&operation_id,"machine_id":&current.node_id,
                        "owner_id":&current.owner_id,"kind":{"$in":["create_sandbox","resume_sandbox",
                            "rekey_sandbox","reconcile_sandbox","update_sandbox_deadline"]},
                        "$or":[{"desired_mode":"running"},{"kind":"rekey_sandbox"}]})
                        .sort(doc! {"created_at":-1,"_id":-1}).await?.ok_or_else(unavailable)?;
                    let dispatch =
                        operations::begin_submission(&self.db, &current, &effect).await?;
                    Some((effect.id, dispatch))
                } else {
                    None
                };
                let intent = super::remote_machine_adapter::intent(
                    &current,
                    &operation_id,
                    self.storage_class.as_deref(),
                    running,
                    &self.bootstrap_base_url,
                );
                if let Some((effect, dispatch)) = &running_dispatch
                    && !self
                        .running_dispatch_is_live(&current, effect, dispatch)
                        .await?
                {
                    return Ok(());
                }
                observed = if current.resource_state == ResourceState::Deleting {
                    current = self.prepare_delete_suspension(&current, &observed).await?;
                    if let Some(snapshot) = delete_snapshot.as_deref_mut() {
                        *snapshot = current.clone();
                    }
                    self.api
                        .suspend_if_identity(
                            &intent,
                            &uid,
                            observed
                                .resource_version
                                .as_deref()
                                .ok_or_else(unavailable)?,
                        )
                        .await
                        .map_err(|error| {
                            if delete_snapshot.is_some() {
                                super::remote_machine_delete_recovery::api_pending(&current, error)
                            } else {
                                api_error(error)
                            }
                        })?
                } else {
                    self.api
                        .apply_if_uid(&intent, &uid)
                        .await
                        .map_err(api_error)?
                };
                if let Some((effect, dispatch)) = &running_dispatch {
                    operations::finish_submission(&self.db, effect, dispatch, Some(&observed))
                        .await?;
                }
            }
            if let Some(snapshot) = delete_snapshot {
                *snapshot = current.clone();
                Box::pin(self.record_observation_inner(&current, &observed, Some(snapshot)))
                    .await?;
            } else {
                self.record_observation(&current, &observed).await?;
            }
        } else if machine.sandbox_uid.is_some() {
            return self
                .uncertain(machine, "sandbox_disappeared_without_native_stop")
                .await;
        }
        Ok(())
    }

    /// Allocate the lifecycle stage before polling it. Its independent BSON and
    /// accounting stages must not be embedded in each caller's async frame.
    pub fn observe<'a>(
        &'a self,
        machine: &'a ManagedMachine,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(self.observe_inner(machine))
    }

    async fn observe_inner(&self, machine: &ManagedMachine) -> AppResult<()> {
        let current = lifecycle::get_machine(&self.db, &machine.owner_id, &machine.node_id)
            .await?
            .ok_or_else(unavailable)?;
        if current.resource_state == ResourceState::Deleting
            || (current.resource_state == ResourceState::Deleted
                && current.delete_recovery.is_some())
        {
            return super::remote_machine_delete_recovery::advance(self, &current).await;
        }
        if super::remote_machine_runtime::finalize_never_submitted(
            &self.db,
            self.billing.clone(),
            &current,
        )
        .await?
        {
            return self.complete_if_final(&current).await;
        }
        if current.resource_state == ResourceState::Deleted {
            return self.complete_if_final(&current).await;
        }
        if let Some(operation_id) = &current.active_operation_id
            && let Some(op) =
                operations::get_operation(&self.db, &current.owner_id, operation_id).await?
            && matches!(
                op.action,
                LifecycleAction::Create
                    | LifecycleAction::Resume
                    | LifecycleAction::Rekey
                    | LifecycleAction::Reconcile
            )
            && op.terminal_at.is_none()
            && op
                .deadline_at
                .unwrap_or(op.created_at + chrono::Duration::minutes(15))
                <= Utc::now()
        {
            super::remote_machine_runtime::safety_pause(
                &self.db,
                self.billing.clone(),
                &current,
                "expansion_deadline_exceeded",
            )
            .await?;
            return Err(unavailable());
        }
        if !current.free_slot
            && matches!(
                current.resource_state,
                ResourceState::Provisioning
                    | ResourceState::Ready
                    | ResourceState::Degraded
                    | ResourceState::Resuming
                    | ResourceState::Reconciling
            )
        {
            if !super::remote_machine_runtime::funding_is_live(&current, Utc::now()) {
                super::remote_machine_runtime::safety_pause(
                    &self.db,
                    self.billing.clone(),
                    &current,
                    "runtime_funding_expired",
                )
                .await?;
                return Err(unavailable());
            }
            if current.resource_state == ResourceState::Ready {
                let billing = self.billing.clone().ok_or_else(unavailable)?;
                if super::remote_machine_runtime::renew(&self.db, billing, &current)
                    .await
                    .is_err()
                {
                    super::remote_machine_runtime::safety_pause(
                        &self.db,
                        self.billing.clone(),
                        &current,
                        "runtime_funding_unavailable",
                    )
                    .await?;
                    return Err(unavailable());
                }
            }
        }
        self.observe_resource(&current).await?;
        let machine = lifecycle::get_machine(&self.db, &current.owner_id, &current.node_id)
            .await?
            .ok_or_else(unavailable)?;
        if machine.native_uncertainty.is_some() {
            return Err(unavailable());
        }
        let observation = machine
            .native_observation
            .as_ref()
            .ok_or_else(unavailable)?;
        if !machine.free_slot
            && let Err(error) = super::remote_machine_runtime::native_observe_and_settle(
                &self.db,
                self.billing.clone().ok_or_else(unavailable)?,
                &machine,
                observation,
            )
            .await
        {
            if observation.running && machine.resource_state == ResourceState::Ready {
                super::remote_machine_runtime::safety_pause(
                    &self.db,
                    self.billing.clone(),
                    &machine,
                    "native_running_observation_gap",
                )
                .await?;
            }
            if !(machine.resource_state == ResourceState::Deleting && observation.stop_confirmed) {
                return Err(error);
            }
        }
        // Billing may change its state; use the latest lifecycle revision.
        let machine = lifecycle::get_machine(&self.db, &machine.owner_id, &machine.node_id)
            .await?
            .ok_or_else(unavailable)?;
        if matches!(
            machine.resource_state,
            ResourceState::Stopping | ResourceState::Deleting
        ) {
            if !super::remote_machine_native::predecessor_stopped(&machine) {
                return Err(unavailable());
            }
            if machine.resource_state == ResourceState::Deleting {
                return self.finish_native_delete(&machine).await;
            }
            if let Some(id) = &machine.active_operation_id
                && let Some(op) = operations::get_operation(&self.db, &machine.owner_id, id).await?
                && op.action == LifecycleAction::Rekey
            {
                let stop = machine
                    .native_observation
                    .as_ref()
                    .ok_or_else(unavailable)?;
                if machine.free_slot {
                    lifecycle::advance_runtime_generation(
                        &self.db,
                        &machine.node_id,
                        machine.revision,
                        machine.runtime_generation,
                        None,
                        None,
                    )
                    .await?;
                } else {
                    super::remote_machine_runtime::native_replace_stopped(
                        &self.db,
                        self.billing.clone().ok_or_else(unavailable)?,
                        &machine,
                        stop,
                        None,
                    )
                    .await?;
                }
                return Err(unavailable());
            }
            super::remote_machine_native::finalize_stop(&self.db, &machine, None).await?;
        } else if matches!(
            machine.resource_state,
            ResourceState::Provisioning
                | ResourceState::Resuming
                | ResourceState::Reconciling
                | ResourceState::Ready
        ) {
            super::remote_machine_native::activate(&self.db, &machine).await?;
        }
        self.complete_if_final(&machine).await
    }

    async fn finish_native_delete(&self, machine: &ManagedMachine) -> AppResult<()> {
        super::remote_machine_delete_recovery::advance(self, machine).await
    }

    pub(crate) async fn complete_if_final(&self, previous: &ManagedMachine) -> AppResult<()> {
        let Some(op_id) = &previous.active_operation_id else {
            return Ok(());
        };
        let machine = lifecycle::get_machine(&self.db, &previous.owner_id, &previous.node_id)
            .await?
            .ok_or_else(unavailable)?;
        let Some(op) = operations::get_operation(&self.db, &machine.owner_id, op_id).await? else {
            return Err(unavailable());
        };
        let final_state = match op.action {
            LifecycleAction::Create | LifecycleAction::Resume | LifecycleAction::Rekey => {
                machine.resource_state == ResourceState::Ready
            }
            LifecycleAction::Pause => machine.resource_state == ResourceState::Paused,
            LifecycleAction::Delete => machine.resource_state == ResourceState::Deleted,
            LifecycleAction::Reconcile => machine.resource_state == ResourceState::Ready,
        };
        if final_state {
            self.db.collection::<Document>(OPERATIONS).update_one(doc! {"_id":op_id,"state":{"$in":["accepted","applying","reconciling"]}}, doc! {"$set":{"state":"succeeded","cleanup_state":&machine.cleanup_state,"terminal_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),"error_code":bson::Bson::Null,"error_message":bson::Bson::Null,"next_attempt_at":bson::Bson::Null}}).await?;
            super::remote_machine_events::record(
                &self.db,
                &op,
                "completed",
                "Lifecycle operation completed",
                true,
            )
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn progress(&self, operation: &str, code: &str) -> AppResult<()> {
        let mut filter = doc! {"_id":operation,"action":"delete","state":{"$in":["accepted","applying","reconciling"]}};
        if matches!(code, "sandbox_effect_pending" | "runtime_evidence_pending") {
            filter.insert("error_code", doc! {"$in":[bson::Bson::Null,bson::Bson::from("sandbox_effect_pending"),bson::Bson::from("runtime_evidence_pending")]});
        }
        // Only shared admission consumes/schedules a delete attempt. Internal
        // milestones and wrapper errors preserve its budget and last blocker.
        self.db.collection::<Document>(OPERATIONS).update_one(filter, doc! {
            "$set":{"state":"reconciling","error_code":code,
                "error_message":"Awaiting verified lifecycle responsibilities; work data remains retained",
                "updated_at":bson::DateTime::now()},
        }).await?;
        self.db.collection::<Document>(OPERATIONS).update_one(
            doc! {"_id":operation,"action":{"$ne":"delete"},"state":{"$in":["accepted","applying","reconciling"]}}, doc! {
            "$set":{"state":"reconciling","error_code":code,
                "error_message":"Awaiting verified lifecycle responsibilities; work data remains retained",
                "updated_at":bson::DateTime::now()},
            "$max":{"next_attempt_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::seconds(10))},
            "$inc":{"retry_count":1},
        }).await?;
        Ok(())
    }

    pub async fn inventory(&self) -> AppResult<()> {
        use super::remote_machine_events::{InvestigationIdentity, record_investigation};
        let sandboxes = self
            .api
            .list_managed(&self.namespace)
            .await
            .map_err(api_error)?;
        let mut live_sandboxes = HashSet::new();
        for object in sandboxes {
            let matched = if let Some(id) = &object.machine_id {
                self.db.collection::<ManagedMachine>(MACHINES).find_one(doc! {"_id":id,"owner_id":&object.owner_id,"sandbox_namespace":&object.namespace,"sandbox_name":&object.name}).await?
            } else {
                None
            };
            let intent = if let Some(operation) = &object.operation_id {
                self.db.collection::<Document>(OPERATIONS).find_one(doc! {"_id":operation,"machine_id":&object.machine_id,"owner_id":&object.owner_id}).await?
            } else {
                None
            };
            let valid = matched.as_ref().is_some_and(|m| {
                m.resource_state != ResourceState::Deleted
                    && object.namespace == self.namespace
                    && object.profile.as_deref() == Some(&m.profile.profile_version)
                    && object
                        .sandbox_uid
                        .as_deref()
                        .is_some_and(|uid| uuid::Uuid::parse_str(uid).is_ok())
                    && (m.sandbox_uid.is_none() || m.sandbox_uid == object.sandbox_uid)
                    && intent.is_some()
            });
            if valid {
                // Only a live object correlated to a durable intent may explain
                // an allocation still being bound by the normal Manager path.
                live_sandboxes.insert((
                    object.machine_id.clone().unwrap(),
                    object.sandbox_uid.clone().unwrap(),
                ));
            }
            if !valid {
                let identity_hash = hex::encode(Sha256::digest(
                    serde_json::json!(["Sandbox", object.name, object.sandbox_uid])
                        .to_string()
                        .as_bytes(),
                ));
                record_investigation(
                    &self.db,
                    InvestigationIdentity {
                        kind: "Sandbox",
                        namespace: self.namespace.clone(),
                        name: Some(object.name),
                        resource_uid: object.sandbox_uid.clone(),
                        sandbox_uid: object.sandbox_uid,
                        machine_id: object.machine_id,
                        owner_id: object.owner_id,
                        operation_id: object.operation_id,
                        identity_hash,
                    },
                    "sandbox_binding_unmatched",
                )
                .await?;
            }
        }
        // Independent collection reads find retained allocations even when no
        // Sandbox remains. Existing read-only RBAC covers both resource kinds.
        for resource in ["pods", "persistentvolumeclaims"] {
            let objects = self
                .api
                .list_controlled_resources(&self.namespace, resource)
                .await
                .map_err(api_error)?;
            for batch in objects.chunks(100) {
                let ids: Vec<&str> = batch
                    .iter()
                    .filter_map(|o| o.machine_id.as_deref())
                    .collect();
                let operations: Vec<&str> = batch
                    .iter()
                    .filter_map(|o| o.operation_id.as_deref())
                    .collect();
                let machines: HashMap<String, ManagedMachine> = self
                    .db
                    .collection::<ManagedMachine>(MACHINES)
                    .find(doc! {"_id":{"$in":ids}})
                    .limit(100)
                    .max_time(Duration::from_secs(3))
                    .await?
                    .try_collect::<Vec<_>>()
                    .await?
                    .into_iter()
                    .map(|m| (m.node_id.clone(), m))
                    .collect();
                let intents: HashMap<String, Document> = self
                    .db
                    .collection::<Document>(OPERATIONS)
                    .find(doc! {"_id":{"$in":operations}})
                    .limit(100)
                    .max_time(Duration::from_secs(3))
                    .await?
                    .try_collect::<Vec<_>>()
                    .await?
                    .into_iter()
                    .filter_map(|op| op.get_str("_id").ok().map(str::to_owned).map(|id| (id, op)))
                    .collect();
                for object in batch {
                    let machine = object.machine_id.as_ref().and_then(|id| machines.get(id));
                    let intent = object.operation_id.as_ref().and_then(|id| intents.get(id));
                    let live = object
                        .machine_id
                        .as_ref()
                        .zip(object.sandbox_owner_uid.as_ref())
                        .is_some_and(|(machine, uid)| {
                            live_sandboxes.contains(&(machine.clone(), uid.clone()))
                        });
                    if let Some(reason) = inventory_resource_reason(object, machine, intent, live) {
                        record_investigation(
                            &self.db,
                            InvestigationIdentity {
                                kind: object.kind,
                                namespace: self.namespace.clone(),
                                name: object.name.clone(),
                                resource_uid: object.uid.clone(),
                                sandbox_uid: object.sandbox_owner_uid.clone(),
                                machine_id: object.machine_id.clone(),
                                owner_id: object.owner_id.clone(),
                                operation_id: object.operation_id.clone(),
                                identity_hash: object.identity_hash.clone(),
                            },
                            reason,
                        )
                        .await?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn inventory_resource_reason(
    object: &ControlledResourceObservation,
    machine: Option<&ManagedMachine>,
    intent: Option<&Document>,
    live_sandbox: bool,
) -> Option<&'static str> {
    if !object.metadata_valid {
        return Some("resource_metadata_untrusted");
    }
    let Some(machine) = machine.filter(|m| {
        object.owner_id.as_ref() == Some(&m.owner_id)
            && object.namespace.as_ref() == Some(&m.sandbox_namespace)
            && object.profile.as_ref() == Some(&m.profile.profile_version)
    }) else {
        return Some("resource_machine_unmatched");
    };
    if object.kind != "PersistentVolumeClaim"
        && !intent.is_some_and(|op| {
            op.get_str("machine_id").ok() == Some(&machine.node_id)
                && op.get_str("owner_id").ok() == Some(&machine.owner_id)
                && op
                    .get_i64("desired_runtime_generation")
                    .ok()
                    .zip(object.runtime_generation)
                    .is_some_and(|(intended, observed)| {
                        intended > 0
                            && observed >= intended
                            && observed <= machine.runtime_generation.max(intended)
                    })
        })
    {
        return Some("resource_intent_unmatched");
    }
    if machine.resource_state == ResourceState::Deleted {
        return Some("resource_after_finality");
    }
    if machine.sandbox_uid.is_none() {
        return (!live_sandbox).then_some("allocated_resource_unbound");
    }
    if machine.sandbox_uid.as_ref() != object.sandbox_owner_uid.as_ref() {
        return Some("resource_owner_uid_unmatched");
    }
    let expected_uid = if object.kind == "Pod" {
        &machine.pod_uid
    } else {
        &machine.pvc_uid
    };
    if expected_uid.is_none() {
        // A live correlated Sandbox can legitimately be between allocation and
        // durable observation. Its absence never explains an unbound resource.
        return (!live_sandbox).then_some("allocated_resource_unbound");
    }
    if expected_uid != &object.uid
        || (object.kind == "PersistentVolumeClaim" && machine.pvc_name != object.name)
    {
        return Some("resource_uid_unmatched");
    }
    if object.kind == "PersistentVolumeClaim" && machine.resource_state == ResourceState::Paused {
        // Exact retained workspace identity is expected indefinitely, including
        // when its original Sandbox is absent. This grants no writer or cleanup.
        return None;
    }
    (!live_sandbox).then_some("resource_sandbox_missing")
}

impl Manager {
    /// The same constructor used by spawn; provider and RuntimeClass variables
    /// are not read here. Kubernetes credentials remain required; storage may
    /// use the cluster default, verified only before fresh provisioning.
    pub fn from_environment(
        db: Database,
        billing: Option<Arc<super::billing::BillingService>>,
        default_base_url: &str,
    ) -> anyhow::Result<Self> {
        let storage_class = configured_storage_class()?;
        let api = sandbox_api_from_environment()?;
        let bootstrap_base_url = std::env::var("NYXID_REMOTE_MACHINE_BOOTSTRAP_BASE_URL")
            .unwrap_or_else(|_| default_base_url.to_owned());
        let bootstrap_url = reqwest::Url::parse(&bootstrap_base_url)?;
        anyhow::ensure!(
            bootstrap_url.scheme() == "https"
                && bootstrap_url.host_str().is_some()
                && bootstrap_url.username().is_empty()
                && bootstrap_url.password().is_none(),
            "managed bootstrap URL must be https"
        );
        Ok(Manager {
            db,
            billing,
            api,
            storage_class,
            namespace: "nyxid-machine".into(),
            bootstrap_base_url,
        })
    }
}

/// Only configured API access starts this worker. It never chooses a context
/// from kubectl, installs a controller, or deploys platform manifests.
pub fn spawn(state: crate::AppState) {
    let safety_db = state.db.clone();
    let safety_billing = state.billing.clone();
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(Duration::from_secs(10));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticks.tick().await;
            if !matches!(
                tokio::time::timeout(
                    Duration::from_secs(20),
                    super::remote_machine_runtime::maintain_local_safety(
                        &safety_db,
                        safety_billing.clone()
                    )
                )
                .await,
                Ok(Ok(()))
            ) {
                tracing::warn!(
                    "Remote Machine local safety sweep pending; submitted runtime liability remains retained"
                );
            }
        }
    });
    let notification_state = state.clone();
    let notification_db = state.db.clone();
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(Duration::from_secs(60));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticks.tick().await;
            if super::remote_machine_events::sweep(&notification_db, Utc::now())
                .await
                .is_err()
            {
                tracing::warn!("Remote Machine timeline projection pending");
            }
            if super::remote_machine_runtime::recover_existing(
                &notification_db,
                notification_state.billing.clone(),
            )
            .await
            .is_err()
            {
                tracing::warn!("Remote Machine existing accounting recovery pending");
            }
            if super::remote_machine_events::deliver_pending(&notification_state)
                .await
                .is_err()
            {
                tracing::warn!("Remote Machine optional notification delivery pending");
            }
        }
    });
    if std::env::var_os("NYXID_REMOTE_MACHINE_KUBE_API").is_none() {
        return;
    }
    let configured = Manager::from_environment(
        state.db.clone(),
        Some(state.billing.clone()),
        &state.config.base_url,
    );
    let Ok(manager) = configured else {
        tracing::error!("Remote Machine manager configuration invalid; admission remains closed");
        return;
    };
    let manager = Arc::new(manager);
    MANAGER_STARTED.store(true, Ordering::Release);
    let funding_manager = manager.clone();
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(Duration::from_secs(5));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticks.tick().await;
            if funding_manager.funding_tick().await.is_err() {
                tracing::warn!(
                    "Remote Machine funding maintenance pending; existing cutoffs remain authoritative"
                );
            }
        }
    });
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(Duration::from_secs(10));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticks.tick().await;
            if manager.tick().await.is_err() {
                tracing::warn!(
                    "Remote Machine reconciliation pending; no physical finality inferred"
                );
            }
        }
    });
}

/// Self-hosted calls keep their existing query budget. Managed calls additionally
/// enforce the live manager fence before owner-turn execution or lease renewal.
pub async fn require_execution(db: &Database, node: &crate::models::node::Node) -> AppResult<()> {
    let managed = node
        .metadata
        .as_ref()
        .and_then(|m| m.provisioning_source.as_deref())
        .is_some_and(|v| matches!(v, "remote_machine" | "managed_machine" | "agent_sandbox"));
    if !managed {
        return Ok(());
    }
    let machine = lifecycle::managed_machine_for_node(db, &node.id)
        .await?
        .ok_or(AppError::MachineNotAllowed)?;
    if !lifecycle::runtime_fence_allows_writer(
        db,
        &node.id,
        machine.runtime_generation,
        machine.credential_epoch,
        &machine.writer_fence,
    )
    .await?
    {
        return Err(AppError::MachineNotAllowed);
    }
    let eligible = lifecycle::entitlement(db, &node.user_id).await?;
    if !eligible.eligible {
        return Err(AppError::MachineNotAllowed);
    }
    Ok(())
}

pub fn accepts_binding(
    machine: &ManagedMachine,
    binding: &nyxid_machine::managed::Binding,
) -> bool {
    binding.validate().is_ok()
        && machine.node_id == binding.node_id
        && machine.sandbox_uid.as_deref() == Some(binding.sandbox_uid.as_str())
        && machine.pod_uid.as_deref() == Some(binding.pod_uid.as_str())
        && u64::try_from(machine.runtime_generation).ok() == Some(binding.runtime_generation)
        && u64::try_from(machine.credential_epoch).ok() == Some(binding.credential_epoch)
        && machine.writer_fence == binding.writer_fence
        && machine.active_operation_id.as_deref() == Some(binding.operation_id.as_str())
        && machine.bootstrap_delivery_id.as_deref() == Some(binding.delivery_id.as_str())
        && lifecycle::runtime_fence_allows_bootstrap(machine)
}

pub fn accepts_profile(profile: &nyxid_machine::MachineProfile) -> bool {
    profile.version == nyxid_machine::PROTOCOL_VERSION
        && uuid::Uuid::parse_str(&profile.runtime_id).is_ok()
        && profile.enabled()
        && (!profile.computer || profile.browser_enabled())
        && profile.authority_v2()
        && profile.roots == ["/workspace"]
        && (profile.browser_enabled() || (!profile.browser_tools && !profile.saved_login_ready))
        // Browser-only Machines also use the existing cua driver internally.
        // Installed driver tools/readiness do not grant the Computer capability.
        && (profile.browser_enabled()
            || (!profile.computer_ready && profile.computer_tools.is_empty()))
        && profile.commands_isolated == Some(true)
}

pub fn accepts_selected_profile(
    machine: &ManagedMachine,
    profile: &nyxid_machine::MachineProfile,
) -> bool {
    let selected = |name: &str| machine.capabilities.iter().any(|cap| cap == name);
    accepts_profile(profile)
        && profile.shell == selected("shell")
        && profile.files == selected("files")
        && profile.browser_enabled() == selected("browser")
        && profile.computer == selected("computer")
}

pub async fn accepts_node_profile(
    db: &Database,
    node_id: &str,
    profile: &nyxid_machine::MachineProfile,
) -> AppResult<bool> {
    Ok(lifecycle::managed_machine_for_node(db, node_id)
        .await?
        .is_some_and(|machine| accepts_selected_profile(&machine, profile)))
}

/// Native admission permits bootstrap; commercial readiness additionally
/// requires this exact runtime's authenticated, leased WSS capability report.
#[cfg(test)]
pub async fn mark_runtime_online(
    db: &Database,
    node_id: &str,
    binding: &nyxid_machine::managed::Binding,
    connection_id: &str,
) -> AppResult<()> {
    mark_runtime_online_inner(db, node_id, binding, connection_id, None).await
}
pub async fn mark_runtime_online_with_billing(
    db: &Database,
    node_id: &str,
    binding: &nyxid_machine::managed::Binding,
    connection_id: &str,
    billing: Arc<super::billing::BillingService>,
) -> AppResult<()> {
    mark_runtime_online_inner(db, node_id, binding, connection_id, Some(billing)).await
}
async fn mark_runtime_online_inner(
    db: &Database,
    node_id: &str,
    binding: &nyxid_machine::managed::Binding,
    connection_id: &str,
    billing: Option<Arc<super::billing::BillingService>>,
) -> AppResult<()> {
    let machine = lifecycle::managed_machine_for_node(db, node_id)
        .await?
        .ok_or_else(unavailable)?;
    if !accepts_binding(&machine, binding) {
        return Err(unavailable());
    }
    let node = db.collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
        .find_one(doc! {"_id":node_id,"user_id":&machine.owner_id,"is_active":true,"status":"online","connection_owner.connection_id":connection_id,"connection_owner.capabilities_resolved":true,"connection_owner.expires_at":{"$gt":bson::DateTime::now()}}).await?.ok_or_else(unavailable)?;
    if !node
        .machine
        .as_ref()
        .is_some_and(|profile| accepts_selected_profile(&machine, profile))
    {
        return Err(unavailable());
    }
    if !lifecycle::entitlement(db, &machine.owner_id)
        .await?
        .eligible
    {
        return Err(unavailable());
    }
    if machine.resource_state == ResourceState::Ready {
        if machine.specialist_assignment_state == "pending" {
            // Assignment is separately durable/retryable. An optional agent
            // grant must not turn an admitted Machine's WSS into a failure.
            let _ = super::remote_machine_configuration::assign_specialist(db, node_id).await;
        }
        return Ok(());
    }
    let assignment_pending = machine.specialist_assignment_state == "pending";
    let db_owned = db.clone();
    let machine = machine.clone();
    let evidence = format!(
        "wss:{}:{}",
        connection_id,
        machine
            .native_observation
            .as_ref()
            .map(|e| e.evidence_ref.as_str())
            .unwrap_or_default()
    );
    db_owned.client().start_session().await?.start_transaction().write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move |session| {
        let result: AppResult<()> = async {
            lifecycle::ensure_owner_eligible_in_session(&db_owned,session,&machine.owner_id).await?;
            let now=Utc::now();
            if !machine.free_slot {
                super::remote_machine_runtime::ready_in_session(billing.as_deref().ok_or_else(unavailable)?,&db_owned,session,&machine,now,&evidence).await?;
            }
            let result = db_owned.collection::<Document>(MACHINES).update_one(
                doc! {"_id":&machine.node_id,"revision":machine.revision,"writer_admitted":true,"resource_state":{"$in":["provisioning","resuming","reconciling"]}},
                doc! {"$set":{"resource_state":"ready","last_heartbeat_at":bson::DateTime::from_chrono(now),"updated_at":bson::DateTime::from_chrono(now)},"$inc":{"revision":1}}
            ).session(&mut *session).await?;
            if result.matched_count != 1 {return Err(unavailable());}
            Ok(())
        }.await;super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::api_key_mutation_service::map_transaction_error)?;
    if assignment_pending {
        let _ = super::remote_machine_configuration::assign_specialist(db, node_id).await;
    }
    Ok(())
}

#[cfg(test)]
#[path = "remote_machine_manager_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "remote_machine_native_tests.rs"]
mod native_tests;

#[cfg(test)]
#[path = "remote_machine_kubernetes_auth_tests.rs"]
mod kubernetes_auth_tests;
