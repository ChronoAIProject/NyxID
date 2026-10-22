//! Managed lifecycle's binding to the existing exact runtime Billing core.
//! All resource admission and deadline changes are fenced with the lifecycle row.
use super::{
    api_key_mutation_service,
    billing::{BillingService, machine_runtime::PreparedRuntimeWindow},
    remote_machine_service as lifecycle,
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        machine_runtime_billing::{
            MachineRuntimeRun, MachineRuntimeRunState, MachineRuntimeTerms, RUN_COLLECTION,
        },
        node::Node,
        remote_machine::{
            BillingState, COLLECTION_NAME as MACHINES, MachineProfileSpec, ManagedMachine,
            NativeRuntimeObservation, ResourceState,
        },
    },
};
use chrono::{DateTime, Duration, Utc};
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};
use std::sync::Arc;

/// Engineering windows are configurable but are not production stop-margin evidence.
pub fn window_seconds() -> i64 {
    std::env::var("NYXID_REMOTE_MACHINE_FUNDING_WINDOW_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(120)
        .clamp(60, 300)
}
pub fn renewal_margin_seconds() -> i64 {
    std::env::var("NYXID_REMOTE_MACHINE_FUNDING_RENEWAL_MARGIN_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(45)
        .clamp(20, window_seconds() / 2)
}
pub fn terms(profile: &MachineProfileSpec, policy: &str) -> AppResult<MachineRuntimeTerms> {
    super::remote_machine_service::validate_fixed_profile(profile)?;
    if policy != crate::models::remote_machine::POLICY_VERSION {
        return Err(AppError::ValidationError(
            "unsupported managed-machine policy".into(),
        ));
    }
    let digest = profile
        .image_digest
        .rsplit_once('@')
        .map_or(profile.image_digest.as_str(), |(_, digest)| digest);
    if !digest.starts_with("sha256:")
        || digest.len() != 71
        || !digest[7..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(AppError::ValidationError(
            "a pinned runtime image digest is required".into(),
        ));
    }
    Ok(MachineRuntimeTerms {
        profile_version: profile.profile_version.clone(),
        image_digest: digest.to_string(),
        policy_version: policy.into(),
    })
}
pub async fn prepare_start(
    billing: Option<&BillingService>,
    owner: &str,
    free: bool,
    request: &serde_json::Value,
    profile: &MachineProfileSpec,
    policy: &str,
    now: DateTime<Utc>,
) -> AppResult<Option<PreparedRuntimeWindow>> {
    lifecycle::validate_fixed_profile(profile)?;
    let mut normalized_request = request.clone();
    lifecycle::normalize_profile_request(&mut normalized_request, &profile.profile_version)?;
    if free {
        return Ok(None);
    }
    let expected_terms = terms(profile, policy)?;
    let quote = request
        .get("quote_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::ValidationError("paid create or resume requires an accepted quote_id".into())
        })?;
    if request
        .get("accept_account_balance")
        .and_then(|v| v.as_bool())
        != Some(true)
    {
        return Err(AppError::ValidationError(
            "paid runtime requires explicit account_balance and no-instance-cap acceptance".into(),
        ));
    }
    let billing = billing.ok_or_else(|| {
        AppError::BillingProviderUnavailable("runtime billing unavailable".into())
    })?;
    // Lifecycle BSON dates have millisecond precision. Make interval boundaries
    // canonical before reserving so a later renewal uses exactly the same instant.
    let now = DateTime::from_timestamp_millis(now.timestamp_millis())
        .ok_or_else(|| AppError::ValidationError("runtime time out of range".into()))?;
    let prepared = billing
        .prepare_machine_start(
            owner,
            quote,
            &uuid::Uuid::new_v4().to_string(),
            now,
            now + Duration::seconds(window_seconds()),
        )
        .await?;
    if prepared.terms() != Some(&expected_terms) {
        return Err(AppError::Conflict(
            "runtime quote does not match the Machine profile and terms".into(),
        ));
    }
    Ok(Some(prepared))
}

fn validate_run_profile(machine: &ManagedMachine, run: &MachineRuntimeRun) -> AppResult<()> {
    if run.terms != terms(&machine.profile, &machine.policy_version)?
        || run.rate.profile_version != machine.profile.profile_version
        || run.node_id != machine.node_id
        || run.owner_id != machine.owner_id
    {
        return Err(AppError::MachineNotAllowed);
    }
    Ok(())
}
/// Read-only admission fence for persisted resources and their current contract.
/// Accounting can still settle its frozen snapshot independently of resource admission.
pub async fn validate_saved_contract(db: &Database, machine: &ManagedMachine) -> AppResult<()> {
    lifecycle::validate_current_contract(machine)?;
    if machine.free_slot {
        return Ok(());
    }
    let run_id = machine
        .runtime_run_id
        .as_deref()
        .ok_or(AppError::MachineNotAllowed)?;
    let run = db
        .collection::<MachineRuntimeRun>(RUN_COLLECTION)
        .find_one(doc! {"_id":run_id,"node_id":&machine.node_id,"owner_id":&machine.owner_id})
        .await?
        .ok_or(AppError::MachineNotAllowed)?;
    validate_run_profile(machine, &run)
}

pub fn funding_is_live(machine: &ManagedMachine, now: DateTime<Utc>) -> bool {
    if machine.free_slot {
        return machine.billing_state == BillingState::Free
            || machine.billing_state == BillingState::Suspended;
    }
    machine.billing_state == BillingState::Ready
        && machine.runtime_run_id.is_some()
        && machine.funded_until.is_some_and(|deadline| deadline > now)
}

/// A requested or funded extension is not execution authority. The current
/// Kubernetes observation must include the applied Sandbox shutdownTime; only
/// a new observation can extend the signed Node window. This is control-plane
/// enforcement, not an independent physical shutdown bound.
pub fn effective_execution_cutoff(machine: &ManagedMachine) -> Option<DateTime<Utc>> {
    let observation = machine.native_observation.as_ref()?;
    if validate_native_observation(machine, observation).is_err()
        || !observation.running
        || observation.suspended
        || observation.stop_confirmed
        || !native_observation_is_fresh(observation, Utc::now())
    {
        return None;
    }
    let observed = observation.run_not_after?;
    let requested = if machine.free_slot {
        machine.native_run_not_after?
    } else {
        machine.funded_until?
    };
    Some(observed.min(requested))
}

pub const NATIVE_OBSERVATION_MAX_AGE_SECONDS: i64 = 60;

fn native_observation_is_fresh(observation: &NativeRuntimeObservation, now: DateTime<Utc>) -> bool {
    observation.observed_at <= now + Duration::seconds(5)
        && observation.observed_at >= now - Duration::seconds(NATIVE_OBSERVATION_MAX_AGE_SECONDS)
}

fn validate_native_observation(
    machine: &ManagedMachine,
    observation: &NativeRuntimeObservation,
) -> AppResult<()> {
    if machine.native_uncertainty.is_some()
        || machine.sandbox_uid.as_deref() != Some(observation.sandbox_uid.as_str())
        || machine.pod_uid != observation.pod_uid
        || machine.runtime_generation != observation.runtime_generation
        || machine.credential_epoch != observation.credential_epoch
        || machine.writer_fence != observation.writer_fence
        || machine.active_operation_id.as_deref() != Some(observation.operation_id.as_str())
        || observation.runtime_generation <= 0
        || observation.credential_epoch <= 0
        || observation.sandbox_generation <= 0
        || observation.sandbox_uid.is_empty()
        || observation.writer_fence.is_empty()
        || observation.operation_id.is_empty()
        || observation.running && observation.pod_uid.as_deref().is_none_or(str::is_empty)
        || observation.sandbox_resource_version.is_empty()
        || observation.evidence_ref.is_empty()
        || observation.running && (observation.suspended || observation.stop_confirmed)
        || observation.stop_confirmed && !observation.suspended
    {
        return Err(AppError::Conflict(
            "native_runtime_observation_mismatch".into(),
        ));
    }
    Ok(())
}
pub fn execution_coverage_is_live(machine: &ManagedMachine, now: DateTime<Utc>) -> bool {
    funding_is_live(machine, now) && effective_execution_cutoff(machine).is_some_and(|at| at > now)
}

pub async fn signed_node_window(
    db: &Database,
    encryption_keys: &crate::crypto::aes::EncryptionKeys,
    node_id: &str,
    binding: &nyxid_machine::managed::Binding,
) -> AppResult<nyxid_machine::managed::SignedRuntimeWindow> {
    let machine = lifecycle::managed_machine_for_node(db, node_id)
        .await?
        .ok_or(AppError::MachineNotAllowed)?;
    if !super::remote_machine_manager::accepts_binding(&machine, binding) {
        return Err(AppError::MachineNotAllowed);
    }
    let cutoff = effective_execution_cutoff(&machine)
        .filter(|at| *at > Utc::now())
        .ok_or(AppError::MachineNotAllowed)?;
    let observation = machine
        .native_observation
        .as_ref()
        .ok_or(AppError::MachineNotAllowed)?;
    let key = super::node_service::get_node_signing_secret(db, encryption_keys, node_id).await?;
    nyxid_machine::managed::SignedRuntimeWindow::sign(
        nyxid_machine::managed::RuntimeWindow {
            binding: binding.clone(),
            control_revision: u64::try_from(observation.sandbox_generation)
                .map_err(|_| AppError::MachineNotAllowed)?,
            run_not_after_ms: cutoff.timestamp_millis(),
            issued_at_ms: Utc::now().timestamp_millis(),
        },
        &key,
    )
    .map_err(|_| AppError::MachineNotAllowed)
}

/// Extend the controller-native shutdownTime through the existing Sandbox
/// outbox. The desired value is durable; applied admission still comes only
/// from a matching native observation and never from this queue submission.
pub async fn queue_native_deadline_in_session(
    db: &Database,
    session: &mut ClientSession,
    machine: &ManagedMachine,
    stop: bool,
    now: DateTime<Utc>,
) -> AppResult<bool> {
    use crate::models::remote_machine_outbox::{
        COLLECTION_NAME as OUTBOX, EffectKind, EffectState, MachineEffectOutbox,
    };
    let stopping = matches!(
        machine.resource_state,
        ResourceState::Stopping
            | ResourceState::Deleting
            | ResourceState::Paused
            | ResourceState::Deleted
            | ResourceState::Failed
    );
    if stop != stopping || machine.native_uncertainty.is_some() && !stop {
        return Err(AppError::MachineNotAllowed);
    }
    if !stop
        && (!funding_is_live(machine, now)
            || machine
                .native_observation
                .as_ref()
                .and_then(|o| o.run_not_after)
                .is_some_and(|at| at <= now))
    {
        // Late API recovery cannot revive a generation whose admitted window
        // expired. Its owner resumes a newly fenced generation after stop.
        return Err(AppError::MachineNotAllowed);
    }
    let cutoff = if stop {
        now
    } else if machine.free_slot {
        if let Some(desired) = machine.native_run_not_after {
            if desired <= now {
                return Err(AppError::MachineNotAllowed);
            }
            // Do not race ahead of the controller's application of an earlier
            // deadline; preserve its finite pending responsibility.
            if desired > now + Duration::seconds(renewal_margin_seconds()) {
                return Ok(false);
            }
            let applied = machine
                .native_observation
                .as_ref()
                .and_then(|o| o.run_not_after)
                .is_some_and(|at| at >= desired);
            if !applied && machine.runtime_submission_started_at.is_some() {
                return Ok(false);
            }
        }
        now + Duration::seconds(window_seconds())
    } else {
        machine.funded_until.ok_or(AppError::MachineNotAllowed)?
    };
    let cutoff = DateTime::from_timestamp_millis(cutoff.timestamp_millis())
        .ok_or(AppError::MachineNotAllowed)?;
    let operation_id = machine
        .active_operation_id
        .clone()
        .ok_or(AppError::MachineNotAllowed)?;
    let digest = format!(
        "native-shutdown:{}:{}:{}:{}",
        machine.runtime_generation,
        machine.credential_epoch,
        stop,
        cutoff.timestamp_millis()
    );
    if db
        .collection::<Document>(OUTBOX)
        .count_documents(doc! {
            "machine_id":&machine.node_id,"operation_id":&operation_id,"effect_digest":&digest,
            "kind":"update_sandbox_deadline"
        })
        .session(&mut *session)
        .await?
        > 0
    {
        return Ok(false);
    }
    let changed = db.collection::<Document>(MACHINES).update_one(doc!{
        "_id":&machine.node_id,"revision":machine.revision,
        "runtime_generation":machine.runtime_generation,"credential_epoch":machine.credential_epoch,
        "active_operation_id":&operation_id,"resource_state":machine.resource_state.as_str()
    },doc!{"$set":{"native_run_not_after":bson::DateTime::from_chrono(cutoff)},
        "$inc":{"native_deadline_fence":1_i64}}).session(&mut *session).await?;
    if changed.matched_count != 1 {
        return Err(AppError::Conflict("native_deadline_request_raced".into()));
    }
    db.collection::<MachineEffectOutbox>(OUTBOX)
        .insert_one(MachineEffectOutbox {
            id: uuid::Uuid::new_v4().to_string(),
            operation_id,
            machine_id: machine.node_id.clone(),
            owner_id: machine.owner_id.clone(),
            kind: EffectKind::UpdateSandboxDeadline,
            effect_digest: digest,
            namespace: machine.sandbox_namespace.clone(),
            sandbox_name: machine.sandbox_name.clone(),
            desired_mode: if stop { "suspended" } else { "running" }.into(),
            state: EffectState::Pending,
            attempts: 0,
            claim_id: None,
            undispatched_claim_id: None,
            claimed_until: None,
            last_error: None,
            available_at: Some(now),
            submission: None,
            created_at: now,
            updated_at: now,
        })
        .session(&mut *session)
        .await?;
    Ok(true)
}

pub async fn ensure_native_deadline(
    db: &Database,
    machine: &ManagedMachine,
    stop: bool,
) -> AppResult<()> {
    let db_owned = db.clone();
    let snapshot = machine.clone();
    db_owned
        .client()
        .start_session()
        .await?
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                let current = db_owned
                    .collection::<ManagedMachine>(MACHINES)
                    .find_one(doc! {
                        "_id":&snapshot.node_id,"runtime_generation":snapshot.runtime_generation,
                        "credential_epoch":snapshot.credential_epoch,
                        "active_operation_id":&snapshot.active_operation_id
                    })
                    .session(&mut *session)
                    .await?
                    .ok_or(AppError::MachineNotAllowed)?;
                if !stop {
                    lifecycle::ensure_owner_eligible_in_session(
                        &db_owned,
                        session,
                        &current.owner_id,
                    )
                    .await?;
                }
                queue_native_deadline_in_session(&db_owned, session, &current, stop, Utc::now())
                    .await?;
                Ok(())
            }
            .await;
            api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(api_key_mutation_service::map_transaction_error)
}

#[cfg(test)]
pub(crate) async fn install_test_coverage(
    db: &Database,
    machine: &ManagedMachine,
    cutoff: DateTime<Utc>,
) -> AppResult<ManagedMachine> {
    // Explicit Kubernetes observation fixture, never a fake signed provider.
    let observation = NativeRuntimeObservation {
        sandbox_uid: machine
            .sandbox_uid
            .clone()
            .ok_or(AppError::MachineNotAllowed)?,
        pod_uid: machine.pod_uid.clone(),
        runtime_generation: machine.runtime_generation,
        credential_epoch: machine.credential_epoch,
        writer_fence: machine.writer_fence.clone(),
        operation_id: machine
            .active_operation_id
            .clone()
            .ok_or(AppError::MachineNotAllowed)?,
        sandbox_generation: machine
            .native_observation
            .as_ref()
            .map_or(1, |o| o.sandbox_generation + 1),
        sandbox_resource_version: uuid::Uuid::new_v4().to_string(),
        observed_at: DateTime::from_timestamp_millis(Utc::now().timestamp_millis())
            .ok_or(AppError::MachineNotAllowed)?,
        running: true,
        suspended: false,
        stop_confirmed: false,
        run_not_after: Some(cutoff),
        evidence_ref: format!("kubernetes-fixture:{}", uuid::Uuid::new_v4()),
    };
    db.collection::<Document>(MACHINES).update_one(doc!{"_id":&machine.node_id},doc!{"$set":{
        "native_observation":bson::to_bson(&observation).map_err(|_| AppError::MachineNotAllowed)?,
        "native_run_not_after":bson::DateTime::from_chrono(cutoff),
        "native_uncertainty":bson::Bson::Null,
        "provider_coverage":bson::Bson::Null
    }}).await?;
    lifecycle::managed_machine_for_node(db, &machine.node_id)
        .await?
        .ok_or(AppError::MachineNotAllowed)
}
pub fn is_managed(node: &Node) -> bool {
    node.metadata
        .as_ref()
        .and_then(|m| m.provisioning_source.as_deref())
        .is_some_and(|v| matches!(v, "remote_machine" | "managed_machine" | "agent_sandbox"))
}
#[derive(Clone, Debug, Default)]
pub struct RuntimeFence {
    pub runtime_generation: Option<u64>,
    pub credential_epoch: Option<u64>,
    pub funding_cutoff: Option<DateTime<Utc>>,
}
fn execution_fence(machine: &ManagedMachine) -> AppResult<RuntimeFence> {
    lifecycle::validate_current_contract(machine)?;
    if machine.resource_state != ResourceState::Ready
        || machine.authorization_state != crate::models::remote_machine::AuthorizationState::Active
        || !lifecycle::has_current_verified_runtime_evidence(machine)
        || !execution_coverage_is_live(machine, Utc::now())
    {
        return Err(AppError::MachineNotAllowed);
    }
    Ok(RuntimeFence {
        runtime_generation: Some(
            u64::try_from(machine.runtime_generation).map_err(|_| AppError::MachineNotAllowed)?,
        ),
        credential_epoch: Some(
            u64::try_from(machine.credential_epoch).map_err(|_| AppError::MachineNotAllowed)?,
        ),
        funding_cutoff: effective_execution_cutoff(machine),
    })
}
/// No managed collection read for self-hosted Nodes.
#[cfg(test)]
pub async fn require_execution_cutoff(
    db: &Database,
    node: &Node,
) -> AppResult<Option<DateTime<Utc>>> {
    if !is_managed(node) {
        return Ok(None);
    }
    let machine = lifecycle::managed_machine_for_node(db, &node.id)
        .await?
        .ok_or(AppError::MachineNotAllowed)?;
    if machine.owner_id != node.user_id
        || !lifecycle::entitlement(db, &node.user_id).await?.eligible
    {
        return Err(AppError::MachineNotAllowed);
    }
    Ok(execution_fence(&machine)?.funding_cutoff)
}
/// Admission/renewal calls this in the same transaction as the source grant fence.
pub fn fence_execution_in_session<'a>(
    db: &'a Database,
    node: &'a Node,
    session: &'a mut ClientSession,
) -> futures::future::BoxFuture<'a, AppResult<RuntimeFence>> {
    Box::pin(async move {
        if !is_managed(node) {
            return Ok(RuntimeFence::default());
        }
        let machine = db
            .collection::<ManagedMachine>(MACHINES)
            .find_one(doc! {"_id": &node.id, "owner_id": &node.user_id})
            .session(&mut *session)
            .await?
            .ok_or(AppError::MachineNotAllowed)?;
        let fence = execution_fence(&machine)?;
        lifecycle::ensure_owner_eligible_in_session(db, session, &machine.owner_id).await?;
        if !machine.free_slot {
            let run = db.collection::<MachineRuntimeRun>(RUN_COLLECTION).find_one(doc! {"_id": &machine.runtime_run_id, "node_id": &machine.node_id, "owner_id": &machine.owner_id, "runtime_generation": machine.runtime_generation, "state":"running"}).session(&mut *session).await?.ok_or(AppError::MachineNotAllowed)?;
            validate_run_profile(&machine, &run)?;
            if run.covered_until_ns < Utc::now().timestamp_nanos_opt().unwrap_or(i64::MAX) {
                return Err(AppError::MachineNotAllowed);
            }
        }
        let changed = db.collection::<Document>(MACHINES).update_one(doc! {"_id": &machine.node_id, "revision": machine.revision, "resource_state":"ready", "writer_admitted":true},
        doc! {"$inc":{"execution_admission_fence":1_i64}}).session(session).await?;
        if changed.matched_count != 1 {
            return Err(AppError::MachineNotAllowed);
        }
        Ok(fence)
    })
}
pub async fn stop_in_session(
    db: &Database,
    billing: Option<&BillingService>,
    session: &mut ClientSession,
    machine: &ManagedMachine,
    replacement: bool,
) -> AppResult<()> {
    // The lifecycle transaction already revokes local execution and queues its
    // Sandbox suspend effect. Funding holds remain until native stop observation
    // and exact settlement; no signed-provider side effect is required.
    if machine.free_slot {
        return Ok(());
    }
    let run = machine
        .runtime_run_id
        .as_deref()
        .ok_or_else(|| AppError::Conflict("runtime_contract_missing".into()))?;
    let contract = db
        .collection::<MachineRuntimeRun>(RUN_COLLECTION)
        .find_one(doc! {"_id":run,"node_id":&machine.node_id,"owner_id":&machine.owner_id})
        .session(&mut *session)
        .await?
        .ok_or(AppError::MachineNotAllowed)?;
    if contract.runtime_generation != machine.runtime_generation
        && !(contract.runtime_generation == machine.runtime_generation + 1
            && contract.ready_ns.is_none()
            && machine.active_operation_id.as_deref() == Some(contract.operation_id.as_str()))
    {
        return Err(AppError::Conflict(
            "runtime_stop_contract_generation_mismatch".into(),
        ));
    }
    billing
        .ok_or_else(|| AppError::BillingProviderUnavailable("runtime billing unavailable".into()))?
        .request_machine_runtime_stop_in_session(
            session,
            run,
            contract.runtime_generation,
            replacement,
        )
        .await
}
pub async fn require_accounting_final(db: &Database, machine: &ManagedMachine) -> AppResult<()> {
    if machine.free_slot {
        return Ok(());
    }
    if db
        .collection::<MachineRuntimeRun>(RUN_COLLECTION)
        .count_documents(
            doc! {"_id": &machine.runtime_run_id, "node_id": &machine.node_id, "state":"closed"},
        )
        .await?
        != 1
    {
        return Err(AppError::Conflict(
            "runtime_accounting_finality_pending".into(),
        ));
    }
    Ok(())
}
pub async fn require_accounting_final_in_session(
    db: &Database,
    session: &mut ClientSession,
    machine: &ManagedMachine,
) -> AppResult<()> {
    if machine.free_slot {
        return Ok(());
    }
    if db
        .collection::<MachineRuntimeRun>(RUN_COLLECTION)
        .count_documents(
            doc! {"_id": &machine.runtime_run_id, "node_id": &machine.node_id, "state":"closed"},
        )
        .session(session)
        .await?
        != 1
    {
        return Err(AppError::Conflict(
            "runtime_accounting_finality_pending".into(),
        ));
    }
    Ok(())
}
/// Ready is established by the authenticated Node WSS handshake, after native
/// bootstrap admission. The exact binding is touched with the Billing state.
pub async fn ready_in_session(
    billing: &BillingService,
    db: &Database,
    session: &mut ClientSession,
    machine: &ManagedMachine,
    ready_at: DateTime<Utc>,
    evidence: &str,
) -> AppResult<()> {
    if machine.free_slot {
        return Ok(());
    }
    if !funding_is_live(machine, ready_at)
        || !lifecycle::has_current_verified_runtime_evidence(machine)
    {
        return Err(AppError::MachineNotAllowed);
    }
    let run_id = machine
        .runtime_run_id
        .as_deref()
        .ok_or(AppError::MachineNotAllowed)?;
    let run = db
        .collection::<MachineRuntimeRun>(RUN_COLLECTION)
        .find_one(doc! {"_id":run_id})
        .session(&mut *session)
        .await?
        .ok_or(AppError::MachineNotAllowed)?;
    if run.ready_ns.is_none() {
        billing
            .prove_machine_ready_in_session(
                session,
                run_id,
                machine.runtime_generation,
                ready_at,
                evidence,
            )
            .await?;
    }
    Ok(())
}
/// Consume only the exact native observation already accepted by the Manager.
/// A stop confirmation records an upper observation bound without claiming a
/// physical terminal instant. It does not advance billable running evidence.
pub async fn native_observe_and_settle(
    db: &Database,
    billing: Arc<BillingService>,
    machine: &ManagedMachine,
    observation: &NativeRuntimeObservation,
) -> AppResult<()> {
    validate_native_observation(machine, observation)?;
    if machine.native_observation.as_ref() != Some(observation) {
        return Err(AppError::Conflict(
            "native_runtime_observation_not_persisted".into(),
        ));
    }
    if machine.free_slot {
        return Ok(());
    }
    if observation.running && !native_observation_is_fresh(observation, Utc::now()) {
        return Err(AppError::Conflict(
            "native_runtime_observation_stale".into(),
        ));
    }
    let run_id = machine
        .runtime_run_id
        .clone()
        .ok_or_else(|| AppError::Conflict("runtime_contract_missing".into()))?;
    let db_owned = db.clone();
    let snapshot = machine.clone();
    let observation = observation.clone();
    let generation = observation.runtime_generation;
    let stopped = observation.stop_confirmed;
    let run_owned = run_id.clone();
    let billing_tx = billing.clone();
    db_owned.client().start_session().await?.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                let mut update = doc!{"$inc":{"billing_evidence_fence":1_i64}};
                if stopped { update.insert("$set", doc!{"writer_admitted":false}); }
                let changed = db_owned.collection::<Document>(MACHINES).update_one(doc!{
                    "_id":&snapshot.node_id,"runtime_generation":generation,
                    "credential_epoch":snapshot.credential_epoch,"writer_fence":&snapshot.writer_fence,
                    "active_operation_id":&observation.operation_id,"native_uncertainty":bson::Bson::Null,
                    "native_observation":bson::to_bson(&observation).map_err(|_|AppError::MachineNotAllowed)?
                }, update).session(&mut *session).await?;
                if changed.matched_count != 1 {
                    return Err(AppError::Conflict("native_runtime_evidence_raced".into()));
                }
                let run = db_owned.collection::<MachineRuntimeRun>(RUN_COLLECTION)
                    .find_one(doc!{"_id":&run_owned,"node_id":&snapshot.node_id,"owner_id":&snapshot.owner_id,
                        "runtime_generation":generation})
                    .session(&mut *session).await?.ok_or(AppError::MachineNotAllowed)?;
                if run.state == MachineRuntimeRunState::Closed { return Ok(()); }
                if stopped && run.stop_ns.is_none() {
                    // Only a live, explicit rekey may retain the accepted run
                    // for replacement. Unexpected controller replacement is
                    // handled as uncertainty by the lifecycle Manager.
                    let rekey = db_owned.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
                        .count_documents(doc!{"_id":&observation.operation_id,"action":"rekey","terminal_at":bson::Bson::Null})
                        .session(&mut *session).await? == 1;
                    billing_tx.request_machine_runtime_stop_in_session(session, &run_owned, generation, rekey).await?;
                    billing_tx.prove_machine_stop_observed_in_session(session, &run_owned, generation,
                        observation.observed_at, &observation.evidence_ref).await?;
                } else if observation.running && run.ready_ns.is_some() && run.stop_ns.is_none() {
                    let through = observation.observed_at.timestamp_nanos_opt().ok_or(AppError::MachineNotAllowed)?;
                    if let Some(previous) = run.proven_until_ns {
                        if through < previous {
                            return Err(AppError::Conflict("native_runtime_observation_replayed".into()));
                        }
                        if through.saturating_sub(previous) > NATIVE_OBSERVATION_MAX_AGE_SECONDS * 1_000_000_000 {
                            return Err(AppError::Conflict("native_runtime_running_gap_unconfirmed".into()));
                        }
                    }
                    if run.proven_until_ns.is_none_or(|previous| through > previous) {
                        billing_tx.prove_machine_running_in_session(session, &run_owned, generation,
                            observation.observed_at, &observation.evidence_ref).await?;
                    }
                }
                for _ in 0..8 {
                    if billing_tx.finalize_machine_interval_in_session(session, &run_owned).await?.is_none() { break; }
                }
                Ok(())
            }.await;
            api_key_mutation_service::transaction_result(result)
        }).await.map_err(api_key_mutation_service::map_transaction_error)?;
    billing.settle_machine_runtime(&run_id, 8).await?;
    let run = db
        .collection::<MachineRuntimeRun>(RUN_COLLECTION)
        .find_one(doc! {"_id":&run_id})
        .await?
        .ok_or(AppError::MachineNotAllowed)?;
    if stopped && !run.replacement_requested {
        let db_owned = db.clone();
        db_owned
            .client()
            .start_session()
            .await?
            .start_transaction()
            .write_concern(mongodb::options::WriteConcern::majority())
            .and_run2(async move |session| {
                api_key_mutation_service::transaction_result(
                    billing
                        .close_machine_runtime_in_session(session, &run_id)
                        .await,
                )
            })
            .await
            .map_err(api_key_mutation_service::map_transaction_error)?;
    }
    Ok(())
}

/// Consume only an already persisted, original-delete stop. It cannot produce
/// running/Ready evidence, extend coverage, rekey or authorize replacement.
pub(crate) async fn settle_persisted_delete(
    db: &Database,
    billing: Arc<BillingService>,
    machine: &ManagedMachine,
) -> AppResult<()> {
    let progress = machine
        .delete_recovery
        .as_ref()
        .ok_or_else(|| AppError::Conflict("delete_evidence_missing".into()))?;
    if !super::remote_machine_delete_recovery::stop_is_valid(machine, progress) {
        return Err(AppError::Conflict("delete_stop_evidence_mismatch".into()));
    }
    if machine.free_slot {
        return Ok(());
    }
    let stop = progress
        .stop
        .as_ref()
        .ok_or(AppError::MachineNotAllowed)?
        .clone();
    let run_id = machine
        .runtime_run_id
        .clone()
        .ok_or_else(|| AppError::Conflict("runtime_contract_missing".into()))?;
    let db_owned = db.clone();
    let snapshot = machine.clone();
    let progress = progress.clone();
    let billing_tx = billing.clone();
    let run_owned = run_id.clone();
    db_owned.client().start_session().await?.start_transaction().write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move |session| {
        let result:AppResult<()>=async {
            if !lifecycle::deletion_authorized_in_session(&db_owned,session,&snapshot.node_id).await? {return Err(AppError::MachineNotAllowed);}
            let changed=db_owned.collection::<Document>(MACHINES).update_one(doc!{"_id":&snapshot.node_id,"revision":snapshot.revision,
                "resource_state":"deleting","writer_admitted":false,"active_operation_id":&progress.operation_id,
                "runtime_generation":progress.runtime_generation,"credential_epoch":progress.credential_epoch,"writer_fence":&progress.writer_fence,
                "native_uncertainty":&progress.original_uncertainty,"delete_recovery.evidence_version":super::remote_machine_delete_recovery::EVIDENCE_VERSION,"delete_recovery":bson::to_bson(&progress).map_err(|_|AppError::MachineNotAllowed)?},
                doc!{"$inc":{"billing_evidence_fence":1_i64}}).session(&mut *session).await?;
            if changed.matched_count!=1{return Err(AppError::Conflict("delete_stop_evidence_raced".into()));}
            if stop.kind == crate::models::remote_machine::DeleteStopKind::ControlPlaneWorkloadAbsent
                && !super::remote_machine_operation_service::delete_creation_responsibilities_terminal_in_session(&db_owned,session,&snapshot).await?
            {
                return Err(AppError::Conflict("creation_result_unknown".into()));
            }
            let run=db_owned.collection::<MachineRuntimeRun>(RUN_COLLECTION).find_one(doc!{"_id":&run_owned,"node_id":&snapshot.node_id,"owner_id":&snapshot.owner_id,"runtime_generation":progress.runtime_generation}).session(&mut *session).await?.ok_or(AppError::MachineNotAllowed)?;
            if run.state==MachineRuntimeRunState::Closed{return Ok(());}
            if run.stop_ns.is_none() {
                billing_tx.request_machine_runtime_stop_in_session(session,&run_owned,progress.runtime_generation,false).await?;
                billing_tx.prove_machine_stop_observed_in_session(session,&run_owned,progress.runtime_generation,stop.observed_at,&stop.evidence_ref).await?;
            }
            for _ in 0..8 {if billing_tx.finalize_machine_interval_in_session(session,&run_owned).await?.is_none(){break;}}
            Ok(())
        }.await;api_key_mutation_service::transaction_result(result)
    }).await.map_err(api_key_mutation_service::map_transaction_error)?;
    billing.settle_machine_runtime(&run_id, 8).await?;
    db.client()
        .start_session()
        .await?
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            api_key_mutation_service::transaction_result(
                billing
                    .close_machine_runtime_in_session(session, &run_id)
                    .await,
            )
        })
        .await
        .map_err(api_key_mutation_service::map_transaction_error)
}

/// Returns the committed cutoff. It never renews a stopped or accepted-but-not-ready run.
pub async fn renew(
    db: &Database,
    billing: Arc<BillingService>,
    machine: &ManagedMachine,
) -> AppResult<()> {
    if machine.free_slot || machine.resource_state != ResourceState::Ready {
        return Ok(());
    }
    let now = Utc::now();
    if !execution_coverage_is_live(machine, now) {
        return Err(AppError::MachineNotAllowed);
    }
    let cutoff = machine.funded_until.ok_or(AppError::MachineNotAllowed)?;
    if cutoff > now + Duration::seconds(renewal_margin_seconds()) {
        return Ok(());
    }
    if cutoff <= now {
        return Err(AppError::MachineNotAllowed);
    }
    let run_id = machine
        .runtime_run_id
        .as_deref()
        .ok_or(AppError::MachineNotAllowed)?;
    let prepared = billing
        .prepare_machine_next_window(
            &machine.owner_id,
            run_id,
            &uuid::Uuid::new_v4().to_string(),
            cutoff,
            cutoff + Duration::seconds(window_seconds()),
        )
        .await?;
    if prepared.terms() != Some(&terms(&machine.profile, &machine.policy_version)?) {
        return Err(AppError::MachineNotAllowed);
    }
    let db_owned = db.clone();
    let machine = machine.clone();
    db_owned.client().start_session().await?.start_transaction().write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move |session| {
        let result: AppResult<()> = async {
            lifecycle::ensure_owner_eligible_in_session(&db_owned, session, &machine.owner_id).await?;
            let changed=db_owned.collection::<Document>(MACHINES).update_one(doc! {"_id":&machine.node_id,"revision":machine.revision,"resource_state":"ready","writer_admitted":true,"funded_until":bson::DateTime::from_chrono(cutoff)},doc! {"$inc":{"revision":1_i64}}).session(&mut *session).await?;
            if changed.matched_count!=1 {return Err(AppError::Conflict("runtime_renewal_fenced".into()));}
            let funded=billing.reserve_machine_next_window_in_session(session,&prepared).await?;
            db_owned.collection::<Document>(MACHINES).update_one(doc! {"_id":&machine.node_id,"revision":machine.revision+1},doc! {"$set":{"funded_until":bson::DateTime::from_chrono(funded.covered_until),"updated_at":bson::DateTime::from_chrono(now)}}).session(&mut *session).await?;
            let mut funded_machine = machine.clone(); funded_machine.revision += 1; funded_machine.funded_until = Some(funded.covered_until);
            queue_native_deadline_in_session(&db_owned, session, &funded_machine, false, now).await?;
            Ok(())
        }.await;api_key_mutation_service::transaction_result(result)
    }).await.map_err(api_key_mutation_service::map_transaction_error)
}

/// Durable system pause for deadline/funding failure; it never deletes a workspace.
pub async fn safety_pause(
    db: &Database,
    billing: Option<Arc<BillingService>>,
    machine: &ManagedMachine,
    reason: &str,
) -> AppResult<()> {
    use crate::models::{
        remote_machine_operation::{
            COLLECTION_NAME as OPERATIONS, CleanupState, LifecycleAction, MachineOperation,
            OperationState,
        },
        remote_machine_outbox::{
            COLLECTION_NAME as OUTBOX, EffectKind, EffectState, MachineEffectOutbox,
        },
    };
    if !(matches!(
        machine.resource_state,
        ResourceState::Provisioning
            | ResourceState::Ready
            | ResourceState::Degraded
            | ResourceState::Resuming
            | ResourceState::Reconciling
    ) || (machine.resource_state == ResourceState::Stopping
        && reason == "expansion_deadline_exceeded"))
    {
        return Ok(());
    }
    let db_owned = db.clone();
    let machine = machine.clone();
    let reason = reason.to_owned();
    let operation_id = uuid::Uuid::new_v4().to_string();
    let effect_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    db_owned.client().start_session().await?.start_transaction().write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move |session| {
        let result: AppResult<()> = async {
            let changed=db_owned.collection::<Document>(MACHINES).update_one(doc! {"_id":&machine.node_id,"revision":machine.revision,"resource_state":{"$in":["provisioning","ready","degraded","resuming","reconciling","stopping"]}},doc! {"$set":{"resource_state":"stopping","stop_reason":&reason,"writer_admitted":false,"active_operation_id":&operation_id,"cleanup_state":"pending","updated_at":bson::DateTime::from_chrono(now)},"$inc":{"revision":1_i64}}).session(&mut *session).await?;
            if changed.matched_count!=1{return Err(AppError::Conflict("runtime_safety_stop_raced".into()));}
            stop_in_session(&db_owned,billing.as_deref(),session,&machine,false).await?;
            db_owned.collection::<Document>(crate::models::node::COLLECTION_NAME).update_one(doc! {"_id":&machine.node_id},doc! {"$set":{"is_active":false,"status":"offline","connection_owner":bson::Bson::Null},"$inc":{"machine_authority_admission_fence":1_i64}}).session(&mut *session).await?;
            if let Some(prior)=&machine.active_operation_id {
                db_owned.collection::<Document>(OPERATIONS).update_one(doc! {"_id":prior,"terminal_at":bson::Bson::Null},doc! {"$set":{"state":"failed","error_code":&reason,"terminal_at":bson::DateTime::from_chrono(now),"updated_at":bson::DateTime::from_chrono(now)}}).session(&mut *session).await?;
            }
            let op=MachineOperation{id:operation_id.clone(),owner_id:machine.owner_id.clone(),machine_id:Some(machine.node_id.clone()),action:LifecycleAction::Pause,
                idempotency_key_hash:format!("system:{}:{}",reason,machine.revision),request_digest:format!("{}:{}",reason,machine.revision),expected_revision:Some(machine.revision),desired_runtime_generation:Some(machine.runtime_generation),
                state:OperationState::Accepted,cleanup_state:CleanupState::Pending,effect_id:Some(effect_id.clone()),error_code:Some(reason.clone()),error_message:None,evidence_digest:None,retry_count:0,delete_retry:None,next_attempt_at:Some(now),deadline_at:Some(now+Duration::hours(24)),created_at:now,updated_at:now,terminal_at:None};
            let effect=MachineEffectOutbox{id:effect_id.clone(),operation_id:operation_id.clone(),machine_id:machine.node_id.clone(),owner_id:machine.owner_id.clone(),kind:EffectKind::SuspendSandbox,effect_digest:format!("{}:{}",reason,machine.revision),namespace:machine.sandbox_namespace.clone(),sandbox_name:machine.sandbox_name.clone(),desired_mode:"suspended".into(),state:EffectState::Pending,attempts:0,claim_id:None,undispatched_claim_id:None,claimed_until:None,last_error:None,available_at:Some(now),submission:None,created_at:now,updated_at:now};
            db_owned.collection::<MachineOperation>(OPERATIONS).insert_one(op).session(&mut *session).await?;
            db_owned.collection::<MachineEffectOutbox>(OUTBOX).insert_one(effect).session(&mut *session).await?;
            lifecycle::append_owner_audit(&db_owned,session,&machine.owner_id,"machine_runtime_safety_pause",&machine.node_id,&operation_id).await?;
            Ok(())
        }.await;api_key_mutation_service::transaction_result(result)
    }).await.map_err(api_key_mutation_service::map_transaction_error)
}

/// Explicit owner rekey retains the accepted continuous-run rate only after
/// native predecessor stop and settlement. Unexpected controller replacement
/// cannot use this path. Funding and the fresh credential/runtime generation
/// commit together; an owner pause always wins the lifecycle fence.
pub async fn native_replace_stopped(
    db: &Database,
    billing: Arc<BillingService>,
    machine: &ManagedMachine,
    proof: &NativeRuntimeObservation,
    pod_uid: Option<&str>,
) -> AppResult<ManagedMachine> {
    use crate::models::remote_machine_operation::{
        COLLECTION_NAME as OPERATIONS, LifecycleAction, MachineOperation,
    };
    validate_native_observation(machine, proof)?;
    if !proof.stop_confirmed || machine.native_observation.as_ref() != Some(proof) {
        return Err(AppError::Conflict(
            "replacement_requires_native_stop".into(),
        ));
    }
    let active = if let Some(id) = &machine.active_operation_id {
        db.collection::<MachineOperation>(OPERATIONS)
            .find_one(doc! {"_id":id})
            .await?
    } else {
        None
    };
    let owner_rekey = active
        .as_ref()
        .is_some_and(|op| op.action == LifecycleAction::Rekey && op.terminal_at.is_none());
    if !owner_rekey {
        return Err(AppError::Conflict(
            "runtime_replacement_not_authorized".into(),
        ));
    }
    if machine.free_slot {
        return lifecycle::advance_runtime_generation(
            db,
            &machine.node_id,
            machine.revision,
            machine.runtime_generation,
            pod_uid,
            None,
        )
        .await;
    }
    let db_owned = db.clone();
    let snapshot = machine.clone();
    let billing_tx = billing.clone();
    db_owned
        .client()
        .start_session()
        .await?
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                lifecycle::ensure_owner_eligible_in_session(&db_owned, session, &snapshot.owner_id)
                    .await?;
                let changed = db_owned
                    .collection::<Document>(MACHINES)
                    .update_one(
                        doc! {"_id":&snapshot.node_id,"revision":snapshot.revision},
                        doc! {"$set":{"writer_admitted":false},"$inc":{"replacement_fence":1_i64}},
                    )
                    .session(&mut *session)
                    .await?;
                if changed.matched_count != 1 {
                    return Err(AppError::Conflict("runtime_replacement_raced".into()));
                }
                stop_in_session(&db_owned, Some(&billing_tx), session, &snapshot, true).await?;
                Ok(())
            }
            .await;
            api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(api_key_mutation_service::map_transaction_error)?;
    native_observe_and_settle(db, billing.clone(), machine, proof).await?;
    let run_id = machine
        .runtime_run_id
        .as_deref()
        .ok_or(AppError::MachineNotAllowed)?;
    let now = DateTime::from_timestamp_millis(Utc::now().timestamp_millis() + 1)
        .ok_or(AppError::MachineNotAllowed)?;
    let prepared = billing
        .prepare_machine_replacement(
            &machine.owner_id,
            run_id,
            &uuid::Uuid::new_v4().to_string(),
            now,
            now + Duration::seconds(window_seconds()),
        )
        .await?;
    let operation_id = active
        .as_ref()
        .ok_or(AppError::MachineNotAllowed)?
        .id
        .clone();
    let db_owned = db.clone();
    let machine = machine.clone();
    let pod_uid = pod_uid.map(str::to_owned);
    db_owned.client().start_session().await?.start_transaction().write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move |session|{
        let result:AppResult<ManagedMachine>=async{
            lifecycle::ensure_owner_eligible_in_session(&db_owned,session,&machine.owner_id).await?;
            let changed=db_owned.collection::<Document>(MACHINES).update_one(doc!{"_id":&machine.node_id,"revision":machine.revision,"runtime_generation":machine.runtime_generation},doc!{"$inc":{"revision":1_i64}}).session(&mut *session).await?;
            if changed.matched_count!=1{return Err(AppError::Conflict("runtime_replacement_raced".into()));}
            let funded=billing.reserve_machine_replacement_in_session(session,&prepared,machine.runtime_generation+1).await?;
            let updated=db_owned.collection::<ManagedMachine>(MACHINES).find_one_and_update(doc!{"_id":&machine.node_id,"revision":machine.revision+1},doc!{"$set":{
                "runtime_generation":machine.runtime_generation+1,"credential_epoch":machine.credential_epoch+1,"writer_fence":uuid::Uuid::new_v4().to_string(),"writer_admitted":false,
                "resource_state":"resuming","stop_reason":bson::Bson::Null,"runtime_submission_started_at":bson::Bson::Null,"billing_state":"ready","active_operation_id":&operation_id,"pod_uid":&pod_uid,
                "native_observation":bson::Bson::Null,"native_pod_name":bson::Bson::Null,"native_node_name":bson::Bson::Null,"native_node_uid":bson::Bson::Null,"native_run_not_after":bson::Bson::Null,"native_uncertainty":bson::Bson::Null,"provider_coverage":bson::Bson::Null,"bootstrap_delivery_id":bson::Bson::Null,"bootstrap_digest":bson::Bson::Null,"funded_until":bson::DateTime::from_chrono(funded.covered_until),"updated_at":bson::DateTime::from_chrono(now)
            }}).return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await?.ok_or(AppError::MachineNotAllowed)?;
            Ok(updated)
        }.await;api_key_mutation_service::transaction_result(result)
    }).await.map_err(api_key_mutation_service::map_transaction_error)
}

#[cfg(test)]
#[path = "remote_machine_runtime_tests.rs"]
mod tests;

/// Recovery is independent of provider configuration and new-admission flags.
/// Only already durable evidence is consumed; unknown tails retain their holds.
pub async fn recover_existing(db: &Database, billing: Arc<BillingService>) -> AppResult<()> {
    use futures::TryStreamExt;
    let runs: Vec<MachineRuntimeRun> = db
        .collection(RUN_COLLECTION)
        .find(doc! {"state":{"$ne":"closed"}})
        .sort(doc! {"lifecycle_recovery_checked_at":1,"_id":1})
        .limit(32)
        .max_time(std::time::Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    for run in runs {
        let db_owned = db.clone();
        let run_id = run.id.clone();
        let billing_tx = billing.clone();
        let finalized = db_owned
            .client()
            .start_session()
            .await?
            .start_transaction()
            .write_concern(mongodb::options::WriteConcern::majority())
            .and_run2(async move |session| {
                let result: AppResult<()> = async {
                    for _ in 0..8 {
                        if billing_tx
                            .finalize_machine_interval_in_session(session, &run_id)
                            .await?
                            .is_none()
                        {
                            break;
                        }
                    }
                    Ok(())
                }
                .await;
                api_key_mutation_service::transaction_result(result)
            })
            .await;
        if finalized.is_ok() {
            let _ = billing.settle_machine_runtime(&run.id, 8).await;
            if run.state == MachineRuntimeRunState::Stopped && !run.replacement_requested {
                let db_owned = db.clone();
                let run_id = run.id.clone();
                let billing_tx = billing.clone();
                let _ = db_owned
                    .client()
                    .start_session()
                    .await?
                    .start_transaction()
                    .write_concern(mongodb::options::WriteConcern::majority())
                    .and_run2(async move |session| {
                        api_key_mutation_service::transaction_result(
                            billing_tx
                                .close_machine_runtime_in_session(session, &run_id)
                                .await,
                        )
                    })
                    .await;
            }
        }
        db.collection::<Document>(RUN_COLLECTION)
            .update_one(
                doc! {"_id":&run.id},
                doc! {"$set":{"lifecycle_recovery_checked_at":bson::DateTime::now()}},
            )
            .await?;
    }
    Ok(())
}

/// Local authority safety survives missing or broken platform configuration.
/// It can stop admission and settle a proven never-submitted generation, but
/// never infers physical stop for work whose API intent may have been sent.
pub async fn maintain_local_safety(db: &Database, billing: Arc<BillingService>) -> AppResult<()> {
    lifecycle::suspend_ineligible_with_billing(db, Some(billing.clone())).await?;
    use crate::models::remote_machine_operation::{
        COLLECTION_NAME as OPS, LifecycleAction, MachineOperation,
    };
    use futures::TryStreamExt;
    let machines:Vec<ManagedMachine>=db.collection(MACHINES)
        .find(doc!{"resource_state":{"$in":["provisioning","ready","degraded","resuming","reconciling","stopping","deleting"]}})
        .sort(doc!{"local_safety_checked_at":1,"_id":1}).limit(32)
        .max_time(std::time::Duration::from_secs(3)).await?.try_collect().await?;
    for mut machine in machines {
        // Advance the fair cursor even when this row has an unresolved
        // accounting defect; it must not starve every later deadline.
        db.collection::<Document>(MACHINES)
            .update_one(
                doc! {"_id":&machine.node_id},
                doc! {"$set":{"local_safety_checked_at":bson::DateTime::now()}},
            )
            .await?;
        let now = Utc::now();
        let mut reason = None;
        if !machine.free_slot && machine.funded_until.is_none_or(|cutoff| cutoff <= now) {
            reason = Some("runtime_funding_expired");
        }
        if let Some(id) = &machine.active_operation_id
            && let Some(operation) = db
                .collection::<MachineOperation>(OPS)
                .find_one(doc! {"_id":id})
                .await?
            && operation.terminal_at.is_none()
            && matches!(
                operation.action,
                LifecycleAction::Create
                    | LifecycleAction::Resume
                    | LifecycleAction::Rekey
                    | LifecycleAction::Reconcile
            )
            && operation
                .deadline_at
                .unwrap_or(operation.created_at + Duration::minutes(15))
                <= now
        {
            reason = Some("expansion_deadline_exceeded");
        }
        if let Some(reason) = reason {
            safety_pause(db, Some(billing.clone()), &machine, reason).await?;
            machine = lifecycle::managed_machine_for_node(db, &machine.node_id)
                .await?
                .ok_or(AppError::MachineNotAllowed)?;
        }
        finalize_never_submitted(db, Some(billing.clone()), &machine).await?;
        if machine.resource_state == ResourceState::Deleting {
            lifecycle::finalize_unallocated_delete(db, &machine.node_id).await?;
        }
    }
    Ok(())
}

pub fn has_never_submitted_stop(machine: &ManagedMachine) -> bool {
    machine.runtime_submission_started_at.is_none()
        && machine.bootstrap_delivery_id.is_none()
        && !machine.writer_admitted
        && machine.pod_uid.is_none()
        && machine.last_runtime_evidence.as_ref().is_some_and(|e| {
            e.source == "lifecycle-never-submitted"
                && e.physically_stopped
                && e.runtime_generation == machine.runtime_generation
                && e.credential_epoch == machine.credential_epoch
                && e.writer_fence == machine.writer_fence
        })
}
/// Must precede every external Running intent. Once marked, timeouts and missing
/// resources cannot use local no-submission proof. The first free native lease
/// is persisted before rendering its Sandbox intent; later retries retain it.
pub async fn mark_submission(db: &Database, machine: &ManagedMachine) -> AppResult<ManagedMachine> {
    let now = Utc::now();
    if !funding_is_live(machine, now)
        || machine.native_uncertainty.is_some()
        || machine
            .native_observation
            .as_ref()
            .and_then(|o| o.run_not_after)
            .is_some_and(|at| at <= now)
    {
        return Err(AppError::MachineNotAllowed);
    }
    let cutoff = if machine.free_slot {
        machine
            .native_run_not_after
            .unwrap_or(now + Duration::seconds(window_seconds()))
    } else {
        machine.funded_until.ok_or(AppError::MachineNotAllowed)?
    };
    if cutoff <= now {
        return Err(AppError::MachineNotAllowed);
    }
    db.collection::<ManagedMachine>(MACHINES).find_one_and_update(doc!{
        "_id":&machine.node_id,"revision":machine.revision,"runtime_generation":machine.runtime_generation,
        "active_operation_id":&machine.active_operation_id,"native_uncertainty":bson::Bson::Null,
        "resource_state":{"$in":["provisioning","resuming","reconciling","ready","degraded"]}
    }, doc!{"$set":{
        "runtime_submission_started_at":bson::DateTime::from_chrono(machine.runtime_submission_started_at.unwrap_or(now)),
        "native_run_not_after":bson::DateTime::from_chrono(cutoff)
    }}).return_document(mongodb::options::ReturnDocument::After).await?
        .ok_or_else(|| AppError::Conflict("runtime_submission_fenced".into()))
}
/// A funded generation whose Running request was never submitted has no physical
/// process to stop. The same lifecycle write fence used before API submission is
/// held while proving its zero runtime; this is not a Pod NotFound inference.
pub async fn finalize_never_submitted(
    db: &Database,
    billing: Option<Arc<BillingService>>,
    machine: &ManagedMachine,
) -> AppResult<bool> {
    if !matches!(
        machine.resource_state,
        ResourceState::Stopping | ResourceState::Deleting
    ) || machine.writer_admitted
        || machine.bootstrap_delivery_id.is_some()
    {
        return Ok(false);
    }
    let contract = if machine.free_slot {
        None
    } else {
        Some(
            db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
                .find_one(doc! {"_id":&machine.runtime_run_id,"node_id":&machine.node_id})
                .await?
                .ok_or(AppError::MachineNotAllowed)?,
        )
    };
    let never_submitted = if let Some(run) = &contract {
        run.ready_ns.is_none()
            && ((run.runtime_generation == machine.runtime_generation + 1)
                || (run.runtime_generation == machine.runtime_generation
                    && machine.runtime_submission_started_at.is_none()
                    && machine.pod_uid.is_none()))
    } else {
        machine.runtime_generation == 0
            && machine.runtime_submission_started_at.is_none()
            && machine.sandbox_uid.is_none()
            && machine.pvc_uid.is_none()
    };
    if !never_submitted {
        return Ok(false);
    }
    if let Some(run) = &contract {
        let billing = billing.clone().ok_or(AppError::MachineNotAllowed)?;
        let db_owned = db.clone();
        let run = run.clone();
        let run_id = run.id.clone();
        let snapshot = machine.clone();
        let billing_tx = billing.clone();
        db_owned.client().start_session().await?.start_transaction().write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move|session|{
            let result:AppResult<()>=async{
                let touched=db_owned.collection::<Document>(MACHINES).update_one(doc!{"_id":&snapshot.node_id,"revision":snapshot.revision,"resource_state":{"$in":["stopping","deleting"]}},doc!{"$inc":{"never_submitted_evidence_fence":1_i64}}).session(&mut *session).await?;
                if touched.matched_count!=1{return Err(AppError::Conflict("unsubmitted_runtime_raced".into()));}
                if snapshot.resource_state == ResourceState::Deleting
                    && !super::remote_machine_operation_service::delete_creation_responsibilities_terminal_in_session(&db_owned,session,&snapshot).await? {
                    return Err(AppError::Conflict("creation_result_unknown".into()));
                }
                let current=db_owned.collection::<MachineRuntimeRun>(RUN_COLLECTION).find_one(doc!{"_id":&run.id}).session(&mut *session).await?.ok_or(AppError::MachineNotAllowed)?;
                if current.stop_ns.is_none(){billing_tx.prove_machine_stopped_in_session(session,&run.id,run.runtime_generation,Utc::now(),&format!("never-submitted:{}:{}",snapshot.node_id,run.id)).await?;}
                for _ in 0..8{if billing_tx.finalize_machine_interval_in_session(session,&run.id).await?.is_none(){break;}}
                Ok(())
            }.await;api_key_mutation_service::transaction_result(result)
        }).await.map_err(api_key_mutation_service::map_transaction_error)?;
        billing.settle_machine_runtime(&run_id, 8).await?;
        let db_owned = db.clone();
        db_owned
            .client()
            .start_session()
            .await?
            .start_transaction()
            .write_concern(mongodb::options::WriteConcern::majority())
            .and_run2(async move |session| {
                api_key_mutation_service::transaction_result(
                    billing
                        .close_machine_runtime_in_session(session, &run_id)
                        .await,
                )
            })
            .await
            .map_err(api_key_mutation_service::map_transaction_error)?;
    }
    if machine.resource_state == ResourceState::Deleting {
        return Ok(false);
    }
    let generation = contract
        .as_ref()
        .map_or(machine.runtime_generation, |r| r.runtime_generation);
    let epoch = machine.credential_epoch
        + if generation > machine.runtime_generation {
            1
        } else {
            0
        };
    let fence = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let evidence = crate::models::remote_machine::RuntimeEvidence {
        sandbox_uid: machine.sandbox_uid.clone().unwrap_or_default(),
        pod_uid: None,
        runtime_generation: generation,
        credential_epoch: epoch,
        writer_fence: fence.clone(),
        ready: false,
        physically_stopped: true,
        observed_at: now,
        source: "lifecycle-never-submitted".into(),
        provider_evidence_ref: Some(format!(
            "never-submitted:{}:{}",
            machine.node_id, generation
        )),
    };
    let db_owned = db.clone();
    let machine = machine.clone();
    db_owned.client().start_session().await?.start_transaction().write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move|session|{
        let result:AppResult<bool>=async{
            let changed=db_owned.collection::<Document>(MACHINES).update_one(doc!{"_id":&machine.node_id,"revision":machine.revision,"resource_state":"stopping"},doc!{"$set":{"resource_state":"paused","billing_state":"suspended","cleanup_state":"not_required","runtime_generation":generation,"credential_epoch":epoch,"writer_fence":&fence,"pod_uid":bson::Bson::Null,"runtime_submission_started_at":bson::Bson::Null,"last_runtime_evidence":bson::to_bson(&evidence).map_err(|_|AppError::Internal("runtime evidence encoding".into()))?,"updated_at":bson::DateTime::from_chrono(now)},"$inc":{"revision":1_i64}}).session(&mut *session).await?;
            if changed.matched_count!=1 {return Ok(false);}
            if let Some(id)=&machine.active_operation_id {
                db_owned.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME).update_one(doc!{"_id":id,"action":"pause","terminal_at":bson::Bson::Null},doc!{"$set":{"state":"succeeded","cleanup_state":"not_required","terminal_at":bson::DateTime::from_chrono(now),"updated_at":bson::DateTime::from_chrono(now)}}).session(&mut *session).await?;
                db_owned.collection::<Document>(crate::models::remote_machine_outbox::COLLECTION_NAME).update_many(doc!{"operation_id":id},doc!{"$set":{"state":"applied","claimed_until":bson::Bson::Null,"claim_id":bson::Bson::Null,"updated_at":bson::DateTime::from_chrono(now)}}).session(&mut *session).await?;
            }
            Ok(true)
        }.await;api_key_mutation_service::transaction_result(result)
    }).await.map_err(api_key_mutation_service::map_transaction_error)
}
