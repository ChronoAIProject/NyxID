//! Durable manager state for the Remote Nyx Machine MVP.
//!
//! This module owns the intent, identity, fences and operation admission. It
//! deliberately does not create Pods/PVCs or execute commands; the Sandbox
//! adapter consumes the outbox effects produced here.

use std::{collections::BTreeMap, sync::Arc};

#[cfg(test)]
use crate::models::remote_machine::RuntimeEvidence;
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{self, Document, doc},
    options::{IndexOptions, ReturnDocument},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    crypto::token::hash_token,
    errors::{AppError, AppResult},
    models::{
        machine_free_slot::{FreeSlotState, MachineFreeSlot},
        remote_machine::{
            AuthorizationState, BillingState, COLLECTION_NAME as MACHINES, MachineProfileSpec,
            ManagedMachine, ResourceState, SafeMachineProfileView, SafeManagedMachineView,
        },
        remote_machine_operation::{
            COLLECTION_NAME as OPERATIONS, CleanupState, LifecycleAction, MachineOperation,
            OperationState, SafeMachineOperationView,
        },
        remote_machine_outbox::{
            COLLECTION_NAME as OUTBOX, EffectKind, EffectState, MachineEffectOutbox,
        },
    },
};

use super::{api_key_mutation_service, remote_machine_operation_service};

/// Require the acting person's rollout flag for new HTTP admission only.
/// Callers retain their human, role and owner checks. Shared lifecycle and
/// billing consumers must remain usable for accepted work after flag disable.
pub async fn require_remote_machines_enabled(db: &Database, actor_id: &str) -> AppResult<()> {
    if !super::feature_flag_service::personal_flag_enabled(
        db,
        actor_id,
        super::feature_flag_service::REMOTE_MACHINES_FLAG_KEY,
    )
    .await?
    {
        return Err(AppError::Forbidden(
            "Remote Machines are not enabled for this account".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn append_owner_audit(
    db: &Database,
    session: &mut mongodb::ClientSession,
    owner_id: &str,
    event: &str,
    machine_id: &str,
    operation_id: &str,
) -> AppResult<()> {
    let key = super::audit_service::audit_chain_hmac_key();
    #[cfg(test)]
    let key = key.or(Some(&[2u8; 32]));
    let key = key.ok_or_else(|| AppError::Internal("audit chain key is unavailable".into()))?;
    super::audit_service::log_actor_event_in_session(
        db,
        session,
        key,
        &super::audit_service::AuditActor {
            user_id: owner_id.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        event,
        json!({"machine_id":machine_id,"operation_id":operation_id}),
    )
    .await
}

pub const FREE_SLOTS: &str = crate::models::machine_free_slot::COLLECTION_NAME;

async fn supersede_operation(
    db: &Database,
    session: &mut mongodb::ClientSession,
    operation_id: Option<&str>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if let Some(operation_id) = operation_id {
        db.collection::<Document>(OPERATIONS).update_one(
            doc! {"_id": operation_id, "state": {"$in": ["accepted", "applying", "reconciling"]}},
            doc! {"$set": {"state": "failed", "error_code": "operation_superseded", "error_message": "A newer lifecycle intent superseded this operation", "terminal_at": bson::DateTime::from_chrono(now), "updated_at": bson::DateTime::from_chrono(now)}}
        ).session(session).await?;
    }
    Ok(())
}

fn owner_ineligibility(user: Option<&Document>) -> Option<&'static str> {
    let Some(user) = user else {
        return Some("owner_not_found");
    };
    if user
        .get("user_type")
        .is_some_and(|kind| !matches!(kind, bson::Bson::String(kind) if kind == "person"))
    {
        return Some("organization_not_eligible");
    }
    if user
        .get("deletion_requested_at")
        .is_some_and(|value| !matches!(value, bson::Bson::Null))
    {
        return Some("account_deletion_pending");
    }
    if !user.get_bool("is_active").unwrap_or(false) {
        return Some("account_inactive");
    }
    if user.get_bool("machine_restricted").unwrap_or(false) {
        return Some("machine_security_restriction");
    }
    None
}

pub(crate) async fn ensure_owner_eligible_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    owner_id: &str,
) -> AppResult<()> {
    // Touch the live owner row so disable/delete/restriction writes conflict
    // with this admission transaction instead of racing a snapshot-only read.
    let result = db.collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id": owner_id, "is_active": true, "$or": [{"user_type":"person"}, {"user_type":{"$exists":false}}], "deletion_requested_at": bson::Bson::Null, "machine_restricted":{"$ne":true}}, doc! {"$inc":{"machine_lifecycle_admission_fence":1_i64}})
        .session(session).await?;
    if result.matched_count != 1 {
        return Err(AppError::Forbidden(
            "managed-machine owner qualification is unavailable".into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct CreateMachineInput {
    pub owner_id: String,
    pub free_slot: bool,
    pub profile: MachineProfileSpec,
    pub sandbox_namespace: String,
    pub idempotency_key: String,
    pub request_body: Value,
    pub now: Option<DateTime<Utc>>,
}

impl CreateMachineInput {
    #[cfg(test)]
    pub fn fixed(owner_id: &str, free_slot: bool, image_digest: &str) -> Self {
        Self {
            owner_id: owner_id.into(),
            free_slot,
            profile: fixed_profile(image_digest),
            sandbox_namespace: "nyxid-machine".into(),
            idempotency_key: "preview-key".into(),
            request_body: json!({"profile":nyxid_machine::managed::PROFILE,"free_slot":free_slot, "policy_version":crate::models::remote_machine::POLICY_VERSION, "data_policy_version":crate::models::remote_machine::DATA_POLICY_VERSION, "acknowledge_shared_environment":true}),
            now: None,
        }
    }

    pub fn preview(&self, node_id: &str, now: DateTime<Utc>) -> ManagedMachine {
        let sandbox_name =
            super::remote_machine_adapter::SandboxIntent::deterministic_name(node_id);
        ManagedMachine {
            node_id: node_id.into(),
            owner_id: self.owner_id.clone(),
            name: crate::models::remote_machine::default_name(),
            capabilities: crate::models::remote_machine::default_capabilities(),
            specialist_id: None,
            specialist_assignment_state: "none".into(),
            specialist_assignment_error: None,
            policy_version: crate::models::remote_machine::POLICY_VERSION.into(),
            data_policy_version: crate::models::remote_machine::DATA_POLICY_VERSION.into(),
            deletion_basis: None,
            profile: self.profile.clone(),
            sandbox_namespace: self.sandbox_namespace.clone(),
            sandbox_name,
            sandbox_uid: None,
            pvc_name: None,
            pvc_uid: None,
            pod_uid: None,
            resource_state: ResourceState::Provisioning,
            billing_state: if self.free_slot {
                BillingState::Free
            } else {
                BillingState::Pending
            },
            authorization_state: AuthorizationState::Pending,
            writer_admitted: false,
            revision: 0,
            runtime_generation: 0,
            credential_epoch: 0,
            writer_fence: Uuid::new_v4().to_string(),
            bootstrap_epoch: 0,
            bootstrap_delivery_id: None,
            bootstrap_digest: None,
            active_operation_id: None,
            free_slot: self.free_slot,
            runtime_run_id: None,
            funded_until: None,
            provider_coverage: None,
            native_observation: None,
            native_uncertainty: None,
            native_pod_name: None,
            native_node_name: None,
            native_node_uid: None,
            native_pv_name: None,
            native_pv_uid: None,
            native_pv_reclaim_policy: None,
            native_run_not_after: None,
            runtime_submission_started_at: None,
            delete_recovery: None,
            delete_suspension_generation: None,
            delete_suspension_operation_id: None,
            cleanup_state: CleanupState::NotRequired.as_str().into(),
            stop_reason: None,
            cleanup_evidence_digest: None,
            last_runtime_evidence: None,
            last_runtime_observation: None,
            last_heartbeat_at: None,
            eligibility_checked_at: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CreateMachineResult {
    pub machine: ManagedMachine,
    pub operation: MachineOperation,
    pub replayed: bool,
}

#[derive(Clone, Debug)]
pub struct LifecycleRequest {
    pub owner_id: String,
    pub machine_id: String,
    pub action: LifecycleAction,
    pub expected_revision: Option<i64>,
    pub idempotency_key: String,
    pub request_body: Value,
    pub now: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct LifecycleResult {
    #[cfg(test)]
    pub machine: ManagedMachine,
    pub operation: MachineOperation,
    #[cfg(test)]
    pub replayed: bool,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub struct RuntimeObservationInput {
    pub machine_id: String,
    pub sandbox_uid: String,
    pub pod_uid: Option<String>,
    pub pvc_name: Option<String>,
    pub pvc_uid: Option<String>,
    pub ready: bool,
    pub physically_stopped: bool,
    pub source: String,
    pub now: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(test)]
pub enum BootstrapDeliveryResult {
    Accepted,
    Replay,
    RekeyRequired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineEntitlement {
    pub eligible: bool,
    pub free_slot_available: bool,
    pub machine_id: Option<String>,
    pub reason: Option<String>,
}

/// Create manager collections and all uniqueness/fence indexes.
pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    let machines = db.collection::<Document>(MACHINES);
    machines
        .create_index(
            IndexModel::builder()
                .keys(doc! {"owner_id":1,"created_at":-1,"_id":-1})
                .build(),
        )
        .await?;
    machines
        .create_index(
            IndexModel::builder()
                .keys(doc! {"owner_id": 1, "resource_state": 1})
                .build(),
        )
        .await?;
    machines
        .create_index(
            IndexModel::builder()
                .keys(doc! {"resource_state":1,"manager_observed_at":1,"_id":1})
                .build(),
        )
        .await?;
    machines
        .create_index(
            IndexModel::builder()
                .keys(doc! {"free_slot":1,"resource_state":1,"funded_until":1,"_id":1})
                .build(),
        )
        .await?;
    machines
        .create_index(
            IndexModel::builder()
                .keys(doc! {"sandbox_namespace": 1, "sandbox_name": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<Document>(FREE_SLOTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"machine_id": 1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! {"machine_id": {"$type": "string"}})
                        .build(),
                )
                .build(),
        )
        .await?;
    remote_machine_operation_service::ensure_indexes(db).await
}

/// Canonical digest for an idempotent mutation. Object keys are sorted and the
/// raw idempotency key is included, so the same key with a different body can
/// be rejected rather than accidentally replaying another operation.
#[cfg(test)]
pub fn canonical_idempotency_digest(
    owner_id: &str,
    action: &str,
    machine_id: Option<&str>,
    idempotency_key: &str,
    body: &Value,
) -> String {
    let canonical = canonical_json(json!({
        "owner_id": owner_id,
        "action": action,
        "machine_id": machine_id,
        "idempotency_key": idempotency_key,
        "body": body,
    }));
    hex::encode(Sha256::digest(
        serde_json::to_vec(&canonical).expect("canonical JSON is serializable"),
    ))
}

fn request_digest(body: &Value) -> String {
    let canonical = canonical_json(body.clone());
    hex::encode(Sha256::digest(
        serde_json::to_vec(&canonical).expect("canonical JSON is serializable"),
    ))
}

fn request_digest_matches(existing: &str, body: &Value) -> bool {
    if existing == request_digest(body) {
        return true;
    }
    // Only the additive profile field may be absent from a historical full
    // intent. All choices, accepted quote and policies remain digest-bound.
    // Callers also validate the saved canonical Machine before returning replay.
    if body.get("profile_version").and_then(Value::as_str) != Some(nyxid_machine::managed::PROFILE)
    {
        return false;
    }
    let mut historical = body.clone();
    historical
        .as_object_mut()
        .expect("profile field requires an object")
        .remove("profile_version");
    existing == request_digest(&historical)
}

/// Owner acceptance is required at the Remote boundary, including exact retries.
/// The generic Billing service continues to own financial contract validation.
fn validate_request_acceptance(action: LifecycleAction, body: &Value) -> AppResult<()> {
    use crate::models::remote_machine::{DATA_POLICY_VERSION, POLICY_VERSION};
    for field in ["profile", "profile_version"] {
        if let Some(value) = body.get(field) {
            let version = value.as_str().ok_or_else(|| {
                AppError::ValidationError(
                    "profile_version must name a controlled Machine profile".into(),
                )
            })?;
            controlled_profile(version)?;
        }
    }
    if ["cpu_millis", "memory_mib", "workspace_gib", "ephemeral_gib"]
        .iter()
        .any(|field| body.get(field).is_some())
    {
        return Err(AppError::ValidationError(
            "Machine resources must come from the controlled profile".into(),
        ));
    }
    if matches!(
        action,
        LifecycleAction::Create | LifecycleAction::Resume | LifecycleAction::Rekey
    ) && (body.get("policy_version").and_then(Value::as_str) != Some(POLICY_VERSION)
        || body.get("data_policy_version").and_then(Value::as_str) != Some(DATA_POLICY_VERSION)
        || body
            .get("acknowledge_shared_environment")
            .and_then(Value::as_bool)
            != Some(true))
    {
        return Err(AppError::ValidationError(
            "current machine and data policies and shared environment acknowledgement are required"
                .into(),
        ));
    }
    if action == LifecycleAction::Delete
        && (body.get("data_policy_version").and_then(Value::as_str) != Some(DATA_POLICY_VERSION)
            || body.get("acknowledge_data_loss").and_then(Value::as_bool) != Some(true))
    {
        return Err(AppError::ValidationError(
            "current data policy and irreversible data loss acknowledgement are required".into(),
        ));
    }
    Ok(())
}

pub fn validate_current_contract(machine: &ManagedMachine) -> AppResult<()> {
    validate_fixed_profile(&machine.profile)?;
    if machine.policy_version != crate::models::remote_machine::POLICY_VERSION
        || machine.data_policy_version != crate::models::remote_machine::DATA_POLICY_VERSION
    {
        return Err(AppError::ValidationError(
            "unsupported managed-machine policy".into(),
        ));
    }
    Ok(())
}

/// Read-only preflight exception for durable retries during platform outages.
/// The normal mutation function performs its complete replay validation again.
pub async fn has_recorded_request(
    db: &Database,
    owner_id: &str,
    machine_id: Option<&str>,
    action: LifecycleAction,
    idempotency_key: &str,
    body: &Value,
) -> AppResult<bool> {
    let mut normalized_body = body.clone();
    if matches!(
        action,
        LifecycleAction::Create | LifecycleAction::Resume | LifecycleAction::Rekey
    ) {
        validate_request_acceptance(action, &normalized_body)?;
        normalize_profile_request(&mut normalized_body, nyxid_machine::managed::PROFILE)?;
    }
    let body = &normalized_body;
    let mut filter = doc! {"owner_id":owner_id,"action":action.as_str(),"idempotency_key_hash":idempotency_key_hash(idempotency_key)};
    if let Some(id) = machine_id {
        filter.insert("machine_id", id);
    }
    let Some(existing) = db
        .collection::<MachineOperation>(OPERATIONS)
        .find_one(filter)
        .await?
    else {
        return Ok(false);
    };
    if !request_digest_matches(&existing.request_digest, body) {
        return Err(AppError::Conflict(
            "idempotency_key_reused_with_different_request".into(),
        ));
    }
    if existing.request_digest != request_digest(body) {
        let id = existing.machine_id.as_deref().ok_or_else(|| {
            AppError::Conflict("historical Machine intent has no resource binding".into())
        })?;
        let saved = get_machine(db, owner_id, id)
            .await?
            .ok_or_else(|| AppError::NotFound("managed machine not found".into()))?;
        super::remote_machine_runtime::validate_saved_contract(db, &saved).await?;
    }
    Ok(true)
}

fn idempotency_key_hash(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

fn canonical_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonical_json).collect()),
        Value::Object(values) => {
            let mut values: BTreeMap<_, _> = values.into_iter().collect();
            Value::Object(
                values
                    .iter_mut()
                    .map(|(key, value)| (key.clone(), canonical_json(value.take())))
                    .collect(),
            )
        }
        other => other,
    }
}

pub fn safe_view(machine: &ManagedMachine) -> SafeManagedMachineView {
    SafeManagedMachineView {
        node_id: machine.node_id.clone(),
        name: machine.name.clone(),
        capabilities: machine.capabilities.clone(),
        specialist_id: machine.specialist_id.clone(),
        specialist_assignment_state: machine.specialist_assignment_state.clone(),
        specialist_assignment_error: machine.specialist_assignment_error.clone(),
        policy_version: machine.policy_version.clone(),
        data_policy_version: machine.data_policy_version.clone(),
        profile: SafeMachineProfileView {
            profile_version: machine.profile.profile_version.clone(),
            cpu_millis: machine.profile.cpu_millis,
            memory_mib: machine.profile.memory_mib,
            workspace_gib: machine.profile.workspace_gib,
            ephemeral_gib: machine.profile.ephemeral_gib,
        },
        sandbox_namespace: machine.sandbox_namespace.clone(),
        sandbox_name: machine.sandbox_name.clone(),
        sandbox_uid: machine.sandbox_uid.clone(),
        pvc_name: machine.pvc_name.clone(),
        pvc_uid: machine.pvc_uid.clone(),
        pod_uid: machine.pod_uid.clone(),
        resource_state: machine.resource_state,
        billing_state: machine.billing_state,
        authorization_state: machine.authorization_state,
        revision: machine.revision,
        runtime_generation: machine.runtime_generation,
        credential_epoch: machine.credential_epoch,
        active_operation_id: machine.active_operation_id.clone(),
        cleanup_state: machine.cleanup_state.clone(),
        stop_reason: machine.stop_reason.clone(),
        free_slot: machine.free_slot,
        runtime_run_id: machine.runtime_run_id.clone(),
        funded_until: machine.funded_until,
        last_heartbeat_at: machine.last_heartbeat_at,
        updated_at: machine.updated_at,
    }
}

pub fn safe_operation_view(operation: &MachineOperation) -> SafeMachineOperationView {
    let retry_stopped = super::remote_machine_delete_retry::stopped(operation, Utc::now());
    let safe_code = operation.error_code.as_deref().map(|code| match code {
        "sandbox_effect_pending"
        | "runtime_evidence_pending"
        | "account_inactive"
        | "owner_not_found"
        | "organization_not_eligible"
        | "account_deletion_pending"
        | "machine_security_restriction" => code,
        "operation_superseded"
        | "runtime_funding_unavailable"
        | "runtime_funding_expired"
        | "expansion_deadline_exceeded"
        | "cleanup_overdue" => code,
        _ => "operation_reconciling",
    });
    SafeMachineOperationView {
        id: operation.id.clone(),
        machine_id: operation.machine_id.clone(),
        action: operation.action,
        state: operation.state,
        cleanup_state: operation.cleanup_state,
        retry_count: operation.retry_count,
        delete_retry_attempts: (operation.action == LifecycleAction::Delete)
            .then(|| super::remote_machine_delete_retry::attempts(operation).min(super::remote_machine_delete_retry::LIMIT)),
        delete_retry_limit: (operation.action == LifecycleAction::Delete)
            .then_some(super::remote_machine_delete_retry::LIMIT),
        delete_retry_stopped: retry_stopped,
        error_code: safe_code.map(str::to_string),
        error_message: safe_code.map(|code| match code {
            "runtime_funding_unavailable" | "runtime_funding_expired" => "Runtime funding could not be renewed. Your workspace is retained; restore available funds or billing availability, then review a new quote and resume manually.",
            "expansion_deadline_exceeded" => "Provisioning exceeded its deadline and was stopped. Your existing workspace is retained; review the platform status before resuming.",
            "cleanup_overdue" => "Deletion remains pending beyond the 24-hour target. Platform operators have been notified; physical cleanup and accounting must finish before completion.",
            _ => "The operation requires reconciliation. Check its cleanup and resource state before retrying.",
        }.into()),
        evidence_digest: operation.evidence_digest.as_ref().map(|reference| hex::encode(Sha256::digest(reference.as_bytes()))),
        next_attempt_at: if retry_stopped { None } else { operation.next_attempt_at },
        deadline_at: operation.deadline_at,
        created_at: operation.created_at,
        updated_at: operation.updated_at,
        terminal_at: operation.terminal_at,
    }
}

#[cfg(test)]
pub async fn create_machine(
    db: &Database,
    input: CreateMachineInput,
) -> AppResult<CreateMachineResult> {
    create_machine_inner(db, input, None).await
}

pub async fn create_machine_with_billing(
    db: &Database,
    input: CreateMachineInput,
    billing: Arc<super::billing::BillingService>,
) -> AppResult<CreateMachineResult> {
    create_machine_inner(db, input, Some(billing)).await
}

async fn create_machine_inner(
    db: &Database,
    mut input: CreateMachineInput,
    billing: Option<Arc<super::billing::BillingService>>,
) -> AppResult<CreateMachineResult> {
    let choices = super::remote_machine_configuration::normalize_request(&mut input.request_body)?;
    validate_request_acceptance(LifecycleAction::Create, &input.request_body)?;
    validate_fixed_profile(&input.profile)?;
    normalize_profile_request(&mut input.request_body, &input.profile.profile_version)?;
    if input.request_body.get("free_slot").and_then(Value::as_bool) != Some(input.free_slot) {
        return Err(AppError::ValidationError(
            "Machine funding intent mismatch".into(),
        ));
    }
    if input.sandbox_namespace != "nyxid-machine" {
        return Err(AppError::ValidationError(
            "unsupported managed-machine namespace".into(),
        ));
    }
    if input.owner_id.trim().is_empty() || input.idempotency_key.trim().is_empty() {
        return Err(AppError::ValidationError(
            "owner and idempotency key are required".into(),
        ));
    }
    let key_hash = idempotency_key_hash(&input.idempotency_key);
    let body_digest = request_digest(&input.request_body);
    if let Some(existing) = db
        .collection::<MachineOperation>(OPERATIONS)
        .find_one(doc! {
            "owner_id": &input.owner_id,
            "action": LifecycleAction::Create.as_str(),
            "idempotency_key_hash": &key_hash,
        })
        .await?
    {
        if !request_digest_matches(&existing.request_digest, &input.request_body) {
            return Err(AppError::Conflict(
                "idempotency_key_reused_with_different_request".into(),
            ));
        }
        let machine_id = existing
            .machine_id
            .as_deref()
            .ok_or_else(|| AppError::Internal("create operation is missing machine id".into()))?;
        let machine = db
            .collection::<ManagedMachine>(MACHINES)
            .find_one(doc! {"_id": machine_id, "owner_id": &input.owner_id})
            .await?
            .ok_or_else(|| {
                AppError::Internal("idempotent machine operation lost its machine".into())
            })?;
        super::remote_machine_runtime::validate_saved_contract(db, &machine).await?;
        return Ok(CreateMachineResult {
            machine,
            operation: existing,
            replayed: true,
        });
    }

    // New allocations use the deployment configuration; exact current retries
    // above retain their saved image and RuntimeClass even during an outage.
    validate_fixed_profile(&input.profile)?;
    if input.profile != fixed_profile(&input.profile.image_digest) {
        return Err(AppError::ValidationError(
            "new machine must use the configured profile".into(),
        ));
    }

    super::remote_machine_configuration::validate_specialist(
        db,
        &input.owner_id,
        choices.specialist_id.as_deref(),
    )
    .await?;
    let now = input.now.unwrap_or_else(Utc::now);
    let machine_id = Uuid::new_v4().to_string();
    let operation_id = Uuid::new_v4().to_string();
    let effect_id = Uuid::new_v4().to_string();
    let mut machine = input.preview(&machine_id, now);
    machine.name = choices.name;
    machine.capabilities = choices.capabilities;
    machine.specialist_id = choices.specialist_id;
    machine.specialist_assignment_state = if machine.specialist_id.is_some() {
        "pending"
    } else {
        "none"
    }
    .into();
    machine.active_operation_id = Some(operation_id.clone());
    let prepared = super::remote_machine_runtime::prepare_start(
        billing.as_deref(),
        &input.owner_id,
        input.free_slot,
        &input.request_body,
        &input.profile,
        crate::models::remote_machine::POLICY_VERSION,
        now,
    )
    .await?;
    let operation = MachineOperation {
        id: operation_id.clone(),
        owner_id: input.owner_id.clone(),
        machine_id: Some(machine_id.clone()),
        action: LifecycleAction::Create,
        idempotency_key_hash: key_hash.clone(),
        request_digest: body_digest.clone(),
        expected_revision: None,
        desired_runtime_generation: Some(1),
        state: OperationState::Accepted,
        cleanup_state: CleanupState::NotRequired,
        effect_id: Some(effect_id.clone()),
        error_code: None,
        error_message: None,
        evidence_digest: None,
        retry_count: 0,
        delete_retry: None,
        next_attempt_at: Some(now),
        deadline_at: Some(now + chrono::Duration::minutes(15)),
        created_at: now,
        updated_at: now,
        terminal_at: None,
    };
    let effect = MachineEffectOutbox {
        id: effect_id,
        operation_id: operation_id.clone(),
        machine_id: machine_id.clone(),
        owner_id: input.owner_id.clone(),
        kind: EffectKind::CreateSandbox,
        effect_digest: request_digest(&json!({"machine_id":machine_id,"action":"create"})),
        namespace: machine.sandbox_namespace.clone(),
        sandbox_name: machine.sandbox_name.clone(),
        desired_mode: "running".into(),
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
    };
    let mut slot = MachineFreeSlot {
        owner_id: input.owner_id.clone(),
        machine_id: Some(machine_id.clone()),
        operation_id: Some(operation_id.clone()),
        state: FreeSlotState::Reserved,
        revision: 1,
        release_evidence_digest: None,
        created_at: now,
        updated_at: now,
        released_at: None,
    };
    let db_owned = db.clone();
    let input_owned = input.clone();
    let mut machine_owned = machine.clone();
    let operation_owned = operation.clone();
    let effect_owned = effect.clone();
    let billing_owned = billing.clone();
    let tx_result = db_owned
        .client()
        .start_session()
        .await?
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<ManagedMachine> = async {
                ensure_owner_eligible_in_session(&db_owned, session, &input_owned.owner_id).await?;
                let operations = db_owned.collection::<MachineOperation>(OPERATIONS);
                if let Some(existing) = operations
                    .find_one(doc! {"owner_id": &input_owned.owner_id, "action": "create", "idempotency_key_hash": &key_hash})
                    .session(&mut *session)
                    .await?
                {
                    if !request_digest_matches(&existing.request_digest, &input_owned.request_body) {
                        return Err(AppError::Conflict("idempotency_key_reused_with_different_request".into()));
                    }
                    return Err(AppError::Conflict("idempotent_create_race".into()));
                }
                if input_owned.free_slot {
                    let slots = db_owned.collection::<MachineFreeSlot>(FREE_SLOTS);
                    if let Some(existing) = slots
                        .find_one(doc! {"_id": &input_owned.owner_id})
                        .session(&mut *session)
                        .await?
                    {
                        if existing.state != FreeSlotState::Released {
                            return Err(AppError::Conflict("free_machine_slot_already_occupied".into()));
                        }
                        let updated = slots
                            .update_one(
                                doc! {"_id": &input_owned.owner_id, "state": "released", "revision": existing.revision},
                                doc! {"$set": {"machine_id": &machine_owned.node_id, "operation_id": &operation_owned.id, "state": "reserved", "release_evidence_digest": bson::Bson::Null, "updated_at": bson::DateTime::from_chrono(machine_owned.updated_at), "released_at": bson::Bson::Null}, "$inc": {"revision": 1}},
                            )
                            .session(&mut *session)
                            .await?;
                        if updated.modified_count != 1 {
                            return Err(AppError::Conflict("free_machine_slot_changed".into()));
                        }
                    } else {
                        slots.insert_one(&slot).session(&mut *session).await?;
                    }
                } else {
                    slot.machine_id = None;
                    slot.operation_id = None;
                    slot.state = FreeSlotState::Released;
                }
                if let Some(prepared) = &prepared {
                    let window = billing_owned.as_ref().ok_or_else(|| AppError::BillingProviderUnavailable("runtime billing unavailable".into()))?
                        .accept_machine_quote_in_session(session, prepared, super::billing::machine_runtime::AcceptRuntimeQuote {
                            owner_id: &machine_owned.owner_id, node_id: &machine_owned.node_id,
                            operation_id: &operation_owned.id, runtime_generation: 1,
                            terms: &super::remote_machine_runtime::terms(&machine_owned.profile, &machine_owned.policy_version)?,
                        }).await?;
                    machine_owned.runtime_run_id = Some(window.run_id);
                    machine_owned.funded_until = Some(window.covered_until);
                    machine_owned.billing_state = BillingState::Ready;
                }
                // Reserve the stable Node identity in the same transaction as
                // the Machine intent. Bootstrap later fills the sealed auth
                // material; this row is offline and cannot authenticate.
                let node_exists = db_owned
                    .collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
                    .find_one(doc! {"_id": &machine_owned.node_id})
                    .session(&mut *session)
                    .await?;
                if node_exists.is_some() {
                    return Err(AppError::Conflict("managed_node_identity_collision".into()));
                }
                let now_node = machine_owned.created_at;
                let reservation_secret = Zeroizing::new(format!("managed-reservation:{}", Uuid::new_v4()));
                let reservation_signing = Zeroizing::new(format!("signing-reservation:{}", Uuid::new_v4()));
                let node = crate::models::node::Node {
                    id: machine_owned.node_id.clone(),
                    user_id: machine_owned.owner_id.clone(),
                    name: machine_owned.name.clone(),
                    status: crate::models::node::NodeStatus::Offline,
                    auth_token_hash: hash_token(&reservation_secret),
                    signing_secret_encrypted: None,
                    signing_secret_hash: hash_token(&reservation_signing),
                    last_heartbeat_at: None,
                    connected_at: None,
                    metadata: Some(crate::models::node::NodeMetadata {
                        agent_version: None,
                        os: Some("linux".into()),
                        arch: Some("unknown".into()),
                        ip_address: None,
                        provisioning_source: Some("remote_machine".into()),
                    }),
                    metrics: Default::default(),
                    connection_owner: None,
                    machine: Some(Default::default()),
                    machine_confirm: Default::default(),
                    allow_single_user_saved_logins: false,
                    is_active: false,
                    created_at: now_node,
                    updated_at: now_node,
                };
                db_owned
                    .collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
                    .insert_one(&node)
                    .session(&mut *session)
                    .await?;
                db_owned
                    .collection::<ManagedMachine>(MACHINES)
                    .insert_one(&machine_owned)
                    .session(&mut *session)
                    .await?;
                db_owned
                    .collection::<MachineOperation>(OPERATIONS)
                    .insert_one(&operation_owned)
                    .session(&mut *session)
                    .await?;
                db_owned
                    .collection::<MachineEffectOutbox>(OUTBOX)
                    .insert_one(&effect_owned)
                    .session(&mut *session)
                    .await?;
                append_owner_audit(&db_owned, session, &input_owned.owner_id, "machine_create_accepted", &machine_owned.node_id, &operation_owned.id).await?;
                Ok(machine_owned.clone())
            }
            .await;
            api_key_mutation_service::transaction_result(result)
        })
        .await;
    match tx_result {
        Ok(machine) => Ok(CreateMachineResult {
            machine,
            operation,
            replayed: false,
        }),
        Err(error) => {
            let mapped = api_key_mutation_service::map_transaction_error(error);
            // A duplicate-key or retried transaction can lose the race after
            // its initial read. Resolve the durable idempotency row before
            // returning a resource conflict to the retrying caller.
            if db
                .collection::<MachineOperation>(OPERATIONS)
                .find_one(doc! {
                    "owner_id": &input.owner_id,
                    "action": "create",
                    "idempotency_key_hash": idempotency_key_hash(&input.idempotency_key),
                })
                .await?
                .is_some()
            {
                Box::pin(create_machine_inner(db, input, billing)).await
            } else {
                Err(mapped)
            }
        }
    }
}

/// Resolve the one currently saleable profile without deployment defaults.
/// New resource tuples must get a distinct identity; this mapping never changes.
pub fn controlled_profile(profile_version: &str) -> AppResult<SafeMachineProfileView> {
    if !nyxid_machine::managed::is_supported_profile(profile_version) {
        return Err(AppError::ValidationError(
            "managed_machine_profile_unavailable".into(),
        ));
    }
    Ok(SafeMachineProfileView {
        profile_version: profile_version.into(),
        cpu_millis: nyxid_machine::managed::PROFILE_CPU_MILLIS,
        memory_mib: nyxid_machine::managed::PROFILE_MEMORY_MIB,
        workspace_gib: nyxid_machine::managed::PROFILE_WORKSPACE_GIB,
        ephemeral_gib: None,
    })
}

pub fn resolve_profile(
    profile_version: &str,
    image_digest: impl Into<String>,
) -> AppResult<MachineProfileSpec> {
    let controlled = controlled_profile(profile_version)?;
    Ok(MachineProfileSpec {
        profile_version: controlled.profile_version,
        image_digest: image_digest.into(),
        runtime_class: None,
        cpu_millis: controlled.cpu_millis,
        memory_mib: controlled.memory_mib,
        workspace_gib: controlled.workspace_gib,
        ephemeral_gib: controlled.ephemeral_gib,
    })
}

/// Normalize only the current controlled profile, never obsolete quote identities.
/// This value participates in the full durable intent digest.
pub fn normalize_profile_request(body: &mut Value, expected: &str) -> AppResult<()> {
    controlled_profile(expected)?;
    if let Some(value) = body.get("profile_version")
        && value.as_str() != Some(expected)
    {
        return Err(AppError::ValidationError(
            "Machine request profile does not match the accepted profile".into(),
        ));
    }
    body.as_object_mut()
        .ok_or_else(|| AppError::ValidationError("Invalid Machine request".into()))?
        .insert("profile_version".into(), json!(expected));
    Ok(())
}

/// Internal callers have the same fixed-profile boundary as HTTP callers.
/// Allocation knobs cannot be smuggled through a custom DTO or replay body.
pub fn validate_fixed_profile(profile: &MachineProfileSpec) -> AppResult<()> {
    let mut expected = resolve_profile(&profile.profile_version, &profile.image_digest)?;
    // A retained current Machine binds its own RuntimeClass, not today's default.
    expected.runtime_class = profile.runtime_class.clone();
    if profile != &expected
        || profile.image_digest.trim().is_empty()
        || profile.runtime_class.as_ref().is_some_and(|name| {
            name.is_empty()
                || name.len() > 253
                || !name.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && label.as_bytes()[0].is_ascii_alphanumeric()
                        && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                        && label
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                })
        })
    {
        return Err(AppError::ValidationError(
            "unsupported managed-machine profile".into(),
        ));
    }
    Ok(())
}

pub fn fixed_profile(image_digest: impl Into<String>) -> MachineProfileSpec {
    let mut profile = resolve_profile(nyxid_machine::managed::PROFILE, image_digest)
        .expect("the compiled Machine profile exists");
    profile.runtime_class = std::env::var("NYXID_REMOTE_MACHINE_RUNTIME_CLASS")
        .ok()
        .filter(|value| !value.trim().is_empty());
    profile
}

#[cfg(test)]
pub async fn request_action(db: &Database, input: LifecycleRequest) -> AppResult<LifecycleResult> {
    request_action_inner(db, input, None).await
}
pub async fn request_action_with_billing(
    db: &Database,
    input: LifecycleRequest,
    billing: Arc<super::billing::BillingService>,
) -> AppResult<LifecycleResult> {
    request_action_inner(db, input, Some(billing)).await
}
async fn request_action_inner(
    db: &Database,
    mut input: LifecycleRequest,
    billing: Option<Arc<super::billing::BillingService>>,
) -> AppResult<LifecycleResult> {
    if input.owner_id.trim().is_empty()
        || input.idempotency_key.trim().is_empty()
        || input.idempotency_key.len() > 256
        || matches!(
            input.action,
            LifecycleAction::Create | LifecycleAction::Reconcile
        )
    {
        return Err(AppError::ValidationError(
            "invalid lifecycle request".into(),
        ));
    }
    if input.expected_revision.is_none() {
        return Err(AppError::ValidationError(
            "expected_revision is required for lifecycle actions".into(),
        ));
    }
    validate_request_acceptance(input.action, &input.request_body)?;
    if matches!(
        input.action,
        LifecycleAction::Resume | LifecycleAction::Rekey
    ) {
        normalize_profile_request(&mut input.request_body, nyxid_machine::managed::PROFILE)?;
    }
    let body_digest = request_digest(&input.request_body);
    let key_hash = idempotency_key_hash(&input.idempotency_key);
    if let Some(existing) = db
        .collection::<MachineOperation>(OPERATIONS)
        .find_one(doc! {"owner_id": &input.owner_id, "machine_id": &input.machine_id, "action": input.action.as_str(), "idempotency_key_hash": &key_hash})
        .await?
    {
        if !request_digest_matches(&existing.request_digest, &input.request_body) {
            return Err(AppError::Conflict("idempotency_key_reused_with_different_request".into()));
        }
        let machine = get_machine(db, &input.owner_id, &input.machine_id).await?.ok_or_else(|| AppError::NotFound("managed machine not found".into()))?;
        super::remote_machine_runtime::validate_saved_contract(db, &machine).await?;
        return Ok(LifecycleResult { #[cfg(test)] machine, operation: existing, #[cfg(test)] replayed: true });
    }
    let now = input.now.unwrap_or_else(Utc::now);
    let operation_id = Uuid::new_v4().to_string();
    let effect_id = Uuid::new_v4().to_string();
    let prior = get_machine(db, &input.owner_id, &input.machine_id)
        .await?
        .ok_or_else(|| AppError::NotFound("managed machine not found".into()))?;
    validate_current_contract(&prior)?;
    let prepared = if input.action == LifecycleAction::Resume {
        super::remote_machine_runtime::prepare_start(
            billing.as_deref(),
            &input.owner_id,
            prior.free_slot,
            &input.request_body,
            &prior.profile,
            &prior.policy_version,
            now,
        )
        .await?
    } else {
        None
    };
    let revoked_auth_hash = hash_token(&Uuid::new_v4().to_string());
    let revoked_signing_hash = hash_token(&Uuid::new_v4().to_string());
    let db_owned = db.clone();
    let input_owned = input.clone();
    let billing_owned = billing.clone();
    let result = db_owned
        .client()
        .start_session()
        .await?
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<LifecycleResult> = async {
                let machine = db_owned
                    .collection::<ManagedMachine>(MACHINES)
                    .find_one(doc! {"_id": &input_owned.machine_id, "owner_id": &input_owned.owner_id})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NotFound("managed machine not found".into()))?;
                validate_current_contract(&machine)?;
                if let Some(expected) = input_owned.expected_revision
                    && machine.revision != expected
                {
                    return Err(AppError::Conflict("managed_machine_revision_conflict".into()));
                }
                if machine.resource_state == ResourceState::Deleted {
                    return Err(AppError::Conflict("managed_machine_is_deleted".into()));
                }
                if input_owned.action == LifecycleAction::Pause
                    && !matches!(
                        machine.resource_state,
                        ResourceState::Ready | ResourceState::Degraded
                    )
                {
                    return Err(AppError::Conflict("managed_machine_is_not_running".into()));
                }
                if input_owned.action == LifecycleAction::Resume
                    && machine.resource_state != ResourceState::Paused
                {
                    return Err(AppError::Conflict("managed_machine_is_not_paused".into()));
                }
                if input_owned.action == LifecycleAction::Rekey && (machine.runtime_generation == 0 || machine.resource_state == ResourceState::Deleting) {
                    return Err(AppError::Conflict("managed_machine_cannot_rekey_before_runtime_or_during_delete".into()));
                }
                if matches!(input_owned.action, LifecycleAction::Resume | LifecycleAction::Rekey) {
                    ensure_owner_eligible_in_session(&db_owned, session, &input_owned.owner_id).await?;
                }
                let next = next_resource_state(input_owned.action);
                let operation = MachineOperation {
                    id: operation_id.clone(),
                    owner_id: input_owned.owner_id.clone(),
                    machine_id: Some(input_owned.machine_id.clone()),
                    action: input_owned.action,
                    idempotency_key_hash: key_hash.clone(),
                    request_digest: body_digest.clone(),
                    expected_revision: input_owned.expected_revision,
                    desired_runtime_generation: Some(if matches!(input_owned.action, LifecycleAction::Resume | LifecycleAction::Rekey) { machine.runtime_generation + 1 } else { machine.runtime_generation }),
                    state: OperationState::Accepted,
                    cleanup_state: if matches!(input_owned.action, LifecycleAction::Delete | LifecycleAction::Pause | LifecycleAction::Rekey) { CleanupState::Pending } else { CleanupState::NotRequired },
                    effect_id: Some(effect_id.clone()),
                    error_code: None,
                    error_message: None,
                    evidence_digest: None,
                    retry_count: 0, delete_retry: None,
                    next_attempt_at: Some(now),
                    deadline_at: Some(now + if input_owned.action == LifecycleAction::Delete { chrono::Duration::hours(24) } else { chrono::Duration::minutes(15) }),
                    created_at: now,
                    updated_at: now,
                    terminal_at: None,
                };
                let kind = EffectKind::for_action(input_owned.action);
                let outbox = MachineEffectOutbox {
                    id: effect_id.clone(),
                    operation_id: operation_id.clone(),
                    machine_id: machine.node_id.clone(),
                    owner_id: machine.owner_id.clone(),
                    kind,
                    effect_digest: request_digest(&json!({"machine_id":machine.node_id,"action":input_owned.action.as_str(),"revision":machine.revision + 1})),
                    namespace: machine.sandbox_namespace.clone(),
                    sandbox_name: machine.sandbox_name.clone(),
                    desired_mode: if matches!(input_owned.action, LifecycleAction::Pause | LifecycleAction::Delete | LifecycleAction::Rekey) { "suspended".into() } else { "running".into() },
                    state: EffectState::Pending,
                    attempts: 0,
                    claim_id: None, undispatched_claim_id: None,
                    claimed_until: None,
                    last_error: None,
                    available_at: Some(now),
                    submission: None,
                    created_at: now,
                    updated_at: now,
                };
                if input_owned.action == LifecycleAction::Delete && machine.deletion_basis.is_none() {
                    let basis = crate::models::remote_machine::MachineDeletionBasis {
                        owner_id: machine.owner_id.clone(),
                        source: crate::models::remote_machine::MachineDeletionSource::MachineOwner,
                        policy_version: machine.data_policy_version.clone(),
                        accepted_at: now,
                    };
                    db_owned.collection::<Document>(MACHINES).update_one(
                        doc! {"_id":&machine.node_id,"owner_id":&input_owned.owner_id,"revision":machine.revision},
                        doc! {"$set":{"deletion_basis":bson::to_bson(&basis).map_err(|_|AppError::Internal("Machine deletion basis encoding failed".into()))?}}
                    ).session(&mut *session).await?;
                }
                let mut update = db_owned
                    .collection::<ManagedMachine>(MACHINES)
                    .find_one_and_update(
                        doc! {"_id": &input_owned.machine_id, "owner_id": &input_owned.owner_id, "revision": machine.revision},
                            doc! {"$set": {"resource_state": next.as_str(), "writer_admitted": false, "stop_reason": match input_owned.action { LifecycleAction::Pause => Some("owner_pause"), LifecycleAction::Delete => Some("owner_delete"), LifecycleAction::Rekey => Some("owner_rekey"), _ => None }, "active_operation_id": &operation_id, "updated_at": bson::DateTime::from_chrono(now), "cleanup_state": operation.cleanup_state.as_str()}, "$inc": {"revision": 1}},
                    )
                    .return_document(ReturnDocument::After)
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::Conflict("managed_machine_revision_conflict".into()))?;
                if let Some(prepared) = &prepared {
                    let window = billing_owned.as_ref().ok_or_else(|| AppError::BillingProviderUnavailable("runtime billing unavailable".into()))?
                        .accept_machine_quote_in_session(session, prepared, super::billing::machine_runtime::AcceptRuntimeQuote {
                            owner_id: &machine.owner_id, node_id: &machine.node_id, operation_id: &operation_id,
                            runtime_generation: machine.runtime_generation + 1,
                            terms: &super::remote_machine_runtime::terms(&machine.profile, &machine.policy_version)?,
                        }).await?;
                    db_owned.collection::<Document>(MACHINES).update_one(doc! {"_id": &machine.node_id, "revision": update.revision},
                        doc! {"$set": {"runtime_run_id": &window.run_id, "funded_until": bson::DateTime::from_chrono(window.covered_until), "billing_state":"ready"}}).session(&mut *session).await?;
                    update.runtime_run_id = Some(window.run_id); update.funded_until = Some(window.covered_until); update.billing_state = BillingState::Ready;
                }
                if matches!(input_owned.action, LifecycleAction::Pause | LifecycleAction::Delete | LifecycleAction::Rekey) {
                    super::remote_machine_runtime::stop_in_session(&db_owned,billing_owned.as_deref(), session, &machine, input_owned.action == LifecycleAction::Rekey).await?;
                }
                db_owned.collection::<MachineOperation>(OPERATIONS).insert_one(&operation).session(&mut *session).await?;
                db_owned.collection::<MachineEffectOutbox>(OUTBOX).insert_one(&outbox).session(&mut *session).await?;
                supersede_operation(&db_owned, session, machine.active_operation_id.as_deref(), now).await?;
                if matches!(input_owned.action, LifecycleAction::Pause | LifecycleAction::Delete | LifecycleAction::Rekey) {
                    // Existing execution admission increments this same Node
                    // fence, so it cannot commit concurrently across a stop.
                    db_owned.collection::<Document>(crate::models::node::COLLECTION_NAME)
                        .update_one(doc! {"_id": &machine.node_id}, doc! {"$set": {"is_active": false, "status": "offline", "connection_owner": bson::Bson::Null, "updated_at": bson::DateTime::from_chrono(now)}, "$inc": {"machine_authority_admission_fence": 1}})
                        .session(&mut *session).await?;
                }
                if input_owned.action == LifecycleAction::Rekey {
                    db_owned.collection::<Document>(crate::models::node::COLLECTION_NAME)
                        .update_one(doc! {"_id": &machine.node_id}, doc! {"$set": {"auth_token_hash": &revoked_auth_hash, "signing_secret_hash": &revoked_signing_hash, "signing_secret_encrypted": bson::Bson::Null}})
                        .session(&mut *session).await?;
                    db_owned.collection::<Document>(MACHINES).update_one(doc! {"_id": &machine.node_id}, doc! {"$set": {"bootstrap_delivery_id": bson::Bson::Null, "bootstrap_digest": bson::Bson::Null}}).session(&mut *session).await?;
                    update.bootstrap_delivery_id = None;
                    update.bootstrap_digest = None;
                }
                append_owner_audit(&db_owned, session, &input_owned.owner_id, &format!("machine_{}_accepted", input_owned.action.as_str()), &machine.node_id, &operation_id).await?;
                Ok(LifecycleResult { #[cfg(test)] machine: update, operation, #[cfg(test)] replayed: false })
            }.await;
            api_key_mutation_service::transaction_result(result)
        })
        .await;
    match result {
        Ok(result) => Ok(result),
        Err(error) => {
            if db.collection::<MachineOperation>(OPERATIONS).find_one(doc! {
                "owner_id": &input.owner_id, "machine_id": &input.machine_id,
                "action": input.action.as_str(), "idempotency_key_hash": idempotency_key_hash(&input.idempotency_key),
            }).await?.is_some() {
                Box::pin(request_action_inner(db, input, billing)).await
            } else {
                Err(api_key_mutation_service::map_transaction_error(error))
            }
        }
    }
}

pub async fn get_machine(
    db: &Database,
    owner_id: &str,
    machine_id: &str,
) -> AppResult<Option<ManagedMachine>> {
    Ok(db
        .collection::<ManagedMachine>(MACHINES)
        .find_one(doc! {"_id": machine_id, "owner_id": owner_id})
        .await?)
}

#[cfg(test)]
pub async fn list_machines(db: &Database, owner_id: &str) -> AppResult<Vec<ManagedMachine>> {
    Ok(db
        .collection::<ManagedMachine>(MACHINES)
        .find(doc! {"owner_id": owner_id, "resource_state": {"$ne": "deleted"}})
        .sort(doc! {"created_at": -1})
        .await?
        .try_collect()
        .await?)
}

pub struct MachinePage {
    pub machines: Vec<ManagedMachine>,
    pub next_cursor: Option<String>,
}

/// Owner-only seek page includes retained tombstones only on explicit request.
/// IDs are seek positions, never authority; owner/state filters precede limit.
pub async fn list_machines_page(
    db: &Database,
    owner_id: &str,
    include_deleted: bool,
    cursor: Option<&str>,
    limit: u32,
) -> AppResult<MachinePage> {
    let limit = limit.clamp(1, 100) as usize;
    let mut filter = doc! {"owner_id":owner_id};
    if !include_deleted {
        filter.insert("resource_state", doc! {"$ne":"deleted"});
    }
    if let Some(cursor) = cursor {
        let invalid = || AppError::ValidationError("invalid machine cursor".into());
        if cursor.len() > 80 {
            return Err(invalid());
        }
        let (timestamp, id) = cursor.split_once('|').ok_or_else(invalid)?;
        let at = DateTime::parse_from_rfc3339(timestamp)
            .map_err(|_| invalid())?
            .with_timezone(&Utc);
        let uuid = Uuid::parse_str(id).map_err(|_| invalid())?;
        if uuid.get_version_num() != 4 || uuid.to_string() != id {
            return Err(invalid());
        }
        filter.insert(
            "$or",
            bson::to_bson(&vec![
                doc! {"created_at":{"$lt":bson::DateTime::from_chrono(at)}},
                doc! {"created_at":bson::DateTime::from_chrono(at),"_id":{"$lt":id}},
            ])
            .map_err(|_| invalid())?,
        );
    }
    let mut machines: Vec<ManagedMachine> = db
        .collection(MACHINES)
        .find(filter)
        .sort(doc! {"created_at":-1,"_id":-1})
        .limit((limit + 1) as i64)
        .max_time(std::time::Duration::from_secs(3))
        .await?
        .try_collect()
        .await?;
    let more = machines.len() > limit;
    machines.truncate(limit);
    let next_cursor = if more {
        machines.last().map(|m| {
            format!(
                "{}|{}",
                m.created_at
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                m.node_id
            )
        })
    } else {
        None
    };
    Ok(MachinePage {
        machines,
        next_cursor,
    })
}

pub async fn managed_machine_for_node(
    db: &Database,
    node_id: &str,
) -> AppResult<Option<ManagedMachine>> {
    Ok(db
        .collection::<ManagedMachine>(MACHINES)
        .find_one(doc! {"_id": node_id})
        .await?)
}

pub async fn guard_legacy_node_mutation(
    db: &Database,
    node_id: &str,
    action: &str,
) -> AppResult<()> {
    if managed_machine_for_node(db, node_id).await?.is_some() {
        return if action == "transfer" {
            Err(AppError::Conflict("managed_transfer_unsupported".into()))
        } else {
            Err(AppError::Conflict(
                "managed_node_mutation_requires_lifecycle".into(),
            ))
        };
    }
    // A managed Node row without its manager mapping is a safety incident,
    // never an invitation to fall back to the self-hosted mutator.
    if let Some(node) = db
        .collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
        .find_one(doc! {"_id": node_id})
        .await?
    {
        let source = node
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.provisioning_source.as_deref());
        if source.is_some_and(|value| {
            matches!(
                value,
                "remote_machine" | "managed_machine" | "agent_sandbox"
            )
        }) {
            return Err(AppError::Conflict(
                "managed_node_mapping_missing_reconciliation_required".into(),
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RuntimeIdentity<'a> {
    pub generation: i64,
    pub credential_epoch: i64,
    pub writer_fence: &'a str,
}

pub fn can_write_runtime(
    state: ResourceState,
    authorization: AuthorizationState,
    current: RuntimeIdentity<'_>,
    expected: RuntimeIdentity<'_>,
) -> bool {
    state == ResourceState::Ready
        && authorization == AuthorizationState::Active
        && current == expected
}

pub async fn runtime_fence_allows_writer(
    db: &Database,
    machine_id: &str,
    runtime_generation: i64,
    credential_epoch: i64,
    writer_fence: &str,
) -> AppResult<bool> {
    let machine = managed_machine_for_node(db, machine_id).await?;
    Ok(machine.is_some_and(|machine| {
        has_current_verified_runtime_evidence(&machine)
            && super::remote_machine_runtime::funding_is_live(&machine, Utc::now())
            && can_write_runtime(
                machine.resource_state,
                machine.authorization_state,
                RuntimeIdentity {
                    generation: machine.runtime_generation,
                    credential_epoch: machine.credential_epoch,
                    writer_fence: &machine.writer_fence,
                },
                RuntimeIdentity {
                    generation: runtime_generation,
                    credential_epoch,
                    writer_fence,
                },
            )
    }))
}

pub fn has_current_verified_runtime_evidence(machine: &ManagedMachine) -> bool {
    validate_current_contract(machine).is_ok()
        && machine.writer_admitted
        && super::remote_machine_runtime::execution_coverage_is_live(machine, Utc::now())
        && machine
            .native_observation
            .as_ref()
            .is_some_and(|observation| {
                super::remote_machine_native::current_running(machine, observation, Utc::now())
            })
}

pub fn runtime_fence_allows_bootstrap(machine: &ManagedMachine) -> bool {
    has_current_verified_runtime_evidence(machine)
        && machine.authorization_state == AuthorizationState::Active
        && matches!(
            machine.resource_state,
            ResourceState::Provisioning
                | ResourceState::Resuming
                | ResourceState::Reconciling
                | ResourceState::Ready
        )
        && super::remote_machine_runtime::funding_is_live(machine, Utc::now())
}

pub async fn advance_runtime_generation(
    db: &Database,
    machine_id: &str,
    expected_revision: i64,
    expected_generation: i64,
    pod_uid: Option<&str>,
    now: Option<DateTime<Utc>>,
) -> AppResult<ManagedMachine> {
    let now = now.unwrap_or_else(Utc::now);
    let current = managed_machine_for_node(db, machine_id)
        .await?
        .ok_or(AppError::MachineNotAllowed)?;
    if current.native_uncertainty.is_some() {
        return Err(AppError::Conflict(
            "native_runtime_investigation_required".into(),
        ));
    }
    if expected_generation > 0
        && !super::remote_machine_runtime::has_never_submitted_stop(&current)
        && !super::remote_machine_native::predecessor_stopped(&current)
    {
        return Err(AppError::Conflict(
            "native_predecessor_stop_required".into(),
        ));
    }
    if !current.free_slot {
        // Paid create/resume fund the future generation in their acceptance
        // transaction; internal replacement uses the dedicated atomic path.
        let admitted = db.collection::<crate::models::machine_runtime_billing::MachineRuntimeRun>(crate::models::machine_runtime_billing::RUN_COLLECTION)
            .count_documents(doc! {"_id":&current.runtime_run_id,"node_id":machine_id,"runtime_generation":expected_generation+1,"state":"accepted"}).await?;
        if admitted != 1 || !super::remote_machine_runtime::funding_is_live(&current, now) {
            return Err(AppError::Conflict(
                "replacement_requires_generation_funding".into(),
            ));
        }
    }
    db.collection::<ManagedMachine>(MACHINES)
        .find_one_and_update(
            doc! {"_id": machine_id, "revision": expected_revision, "runtime_generation": expected_generation, "resource_state": {"$in": ["provisioning", "resuming", "reconciling", "stopping", "ready", "degraded"]}},
            doc! {"$set": {"pod_uid": pod_uid, "native_observation": bson::Bson::Null, "native_pod_name": bson::Bson::Null, "native_node_name": bson::Bson::Null, "native_node_uid": bson::Bson::Null, "native_run_not_after": bson::Bson::Null, "writer_fence": Uuid::new_v4().to_string(), "writer_admitted": false, "runtime_submission_started_at": bson::Bson::Null, "bootstrap_delivery_id": bson::Bson::Null, "bootstrap_digest": bson::Bson::Null, "updated_at": bson::DateTime::from_chrono(now), "resource_state": ResourceState::Resuming.as_str()}, "$inc": {"revision": 1, "runtime_generation": 1, "credential_epoch": 1}},
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Conflict("runtime_generation_or_revision_conflict".into()))
}

#[cfg(test)]
pub async fn apply_runtime_observation(
    db: &Database,
    input: RuntimeObservationInput,
) -> AppResult<ManagedMachine> {
    let now = input.now.unwrap_or_else(Utc::now);
    let machine = managed_machine_for_node(db, &input.machine_id)
        .await?
        .ok_or_else(|| AppError::NotFound("managed machine not found".into()))?;
    if let Some(existing) = machine.sandbox_uid.as_deref()
        && existing != input.sandbox_uid
    {
        return Err(AppError::Conflict(
            "sandbox_uid_mismatch_reconciling".into(),
        ));
    }
    if input.pod_uid.is_some()
        && machine.pod_uid.is_some()
        && machine.pod_uid != input.pod_uid
        && machine.runtime_generation > 0
    {
        return Err(AppError::Conflict(
            "pod_uid_changed_requires_new_runtime_generation".into(),
        ));
    }
    if machine.pvc_uid.is_some() && input.pvc_uid.is_some() && machine.pvc_uid != input.pvc_uid {
        return Err(AppError::Conflict("pvc_uid_mismatch_reconciling".into()));
    }
    // Raw Sandbox/Pod conditions are observations only. A caller cannot turn
    // an arbitrary `ready=true` into an execution admission; the verified
    // provider evidence path below is the only Ready transition.
    // Observation cannot skip a requested lifecycle transition. In
    // particular, `Ready`/`Suspended` conditions never prove a stop or open a
    // writer; finality APIs consume verified provider evidence separately.
    let observation_ready = input.ready && input.pod_uid.is_some();
    let state = if machine.resource_state == ResourceState::Ready
        && observation_ready
        && has_current_verified_runtime_evidence(&machine)
    {
        ResourceState::Ready
    } else {
        observation_state(machine.resource_state)
    };
    let evidence = RuntimeEvidence {
        sandbox_uid: input.sandbox_uid.clone(),
        pod_uid: input.pod_uid.clone().or_else(|| machine.pod_uid.clone()),
        runtime_generation: machine.runtime_generation,
        credential_epoch: machine.credential_epoch,
        writer_fence: machine.writer_fence.clone(),
        ready: input.ready,
        physically_stopped: input.physically_stopped,
        observed_at: now,
        source: input.source,
        provider_evidence_ref: None,
    };
    let mut observed = doc! {"sandbox_uid": &input.sandbox_uid, "resource_state": state.as_str(), "last_runtime_observation": bson::to_bson(&evidence).map_err(|_| mongodb::error::Error::custom("evidence serialization"))?, "last_heartbeat_at": bson::DateTime::from_chrono(now), "updated_at": bson::DateTime::from_chrono(now)};
    if !observation_ready {
        observed.insert("writer_admitted", false);
    }
    // A missing controller resource does not prove prior writer or volume
    // removal. Remember their UIDs until verified stop/cleanup finality.
    if let Some(pod_uid) = input.pod_uid {
        observed.insert("pod_uid", pod_uid);
    }
    if let Some(pvc_name) = input.pvc_name {
        observed.insert("pvc_name", pvc_name);
    }
    if let Some(pvc_uid) = input.pvc_uid {
        observed.insert("pvc_uid", pvc_uid);
    }
    db.collection::<ManagedMachine>(MACHINES)
        .find_one_and_update(
            doc! {"_id": &input.machine_id, "revision": machine.revision},
            doc! {"$set": observed, "$inc": {"revision": 1}},
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Conflict("managed_machine_revision_conflict".into()))
}

/// Finalize an intent that was never submitted to the Sandbox API. The
/// manager advances generation atomically *before* external creation, so a
/// free generation-zero row with no bound runtime/volume can be proven local.
/// This is not a controller `NotFound` shortcut for an allocated runtime.
pub async fn finalize_unallocated_delete(
    db: &Database,
    machine_id: &str,
) -> AppResult<Option<ManagedMachine>> {
    let db_owned = db.clone();
    let machine_id = machine_id.to_string();
    let revoked_auth_hash = hash_token(&Uuid::new_v4().to_string());
    let revoked_signing_hash = hash_token(&Uuid::new_v4().to_string());
    db_owned.client().start_session().await?.start_transaction().write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move |session| {
        let result: AppResult<Option<ManagedMachine>> = async {
            let machine = db_owned.collection::<ManagedMachine>(MACHINES).find_one(doc! {
                "_id": &machine_id, "resource_state": "deleting", "runtime_generation": 0,
                "sandbox_uid": bson::Bson::Null, "pod_uid": bson::Bson::Null, "pvc_uid": bson::Bson::Null,
                "pvc_name": bson::Bson::Null, "native_pv_name": bson::Bson::Null, "native_pv_uid": bson::Bson::Null,
                "runtime_submission_started_at": bson::Bson::Null, "bootstrap_delivery_id": bson::Bson::Null,
                "native_uncertainty": bson::Bson::Null, "writer_admitted": false,
            }).session(&mut *session).await?;
            let Some(machine) = machine else { return Ok(None) };
            if !deletion_authorized_in_session(&db_owned, session, &machine_id).await? {
                return Err(AppError::Conflict("machine_owner_deletion_basis_required".into()));
            }
            if !super::remote_machine_operation_service::creation_responsibilities_terminal_in_session(&db_owned,session,&machine).await? {return Ok(None);}
            super::remote_machine_runtime::require_accounting_final_in_session(&db_owned,session,&machine).await?;
            let node = db_owned.collection::<Document>(crate::models::node::COLLECTION_NAME).find_one(doc! {"_id": &machine_id, "is_active": false, "signing_secret_encrypted": bson::Bson::Null}).session(&mut *session).await?;
            if node.is_none() { return Ok(None); }
            let Some(operation_id) = machine.active_operation_id.as_deref() else { return Ok(None) };
            let operation = db_owned.collection::<Document>(OPERATIONS).find_one(doc! {"_id": operation_id, "machine_id": &machine_id, "action": "delete", "state": {"$in": ["accepted", "applying", "reconciling"]}}).session(&mut *session).await?;
            if operation.is_none() { return Ok(None); }
            let now = Utc::now();
            let proof = request_digest(&json!({"proof":"never_submitted","machine_id":machine_id,"revision":machine.revision}));
            let finalized = db_owned.collection::<ManagedMachine>(MACHINES).find_one_and_update(
                doc! {"_id": &machine_id, "revision": machine.revision, "resource_state": "deleting", "runtime_generation": 0},
                doc! {"$set": {"resource_state":"deleted","writer_admitted":false,"authorization_state":"revoked","billing_state":"closed","cleanup_state":"complete","cleanup_evidence_digest":&proof,"updated_at":bson::DateTime::from_chrono(now)},"$inc":{"revision":1}},
            ).return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(|| AppError::Conflict("unallocated_delete_revision_conflict".into()))?;
            let slot = db_owned.collection::<Document>(FREE_SLOTS).update_one(
                doc! {"_id": &machine.owner_id, "machine_id": &machine_id, "state": {"$in":["reserved","active","releasing"]}},
                doc! {"$set":{"state":"released","release_evidence_digest":&proof,"released_at":bson::DateTime::from_chrono(now),"updated_at":bson::DateTime::from_chrono(now)},"$inc":{"revision":1}},
            ).session(&mut *session).await?;
            if machine.free_slot && slot.matched_count != 1 { return Err(AppError::Conflict("free_slot_cleanup_binding_missing".into())); }
            db_owned.collection::<Document>(crate::models::node::COLLECTION_NAME).update_one(doc! {"_id": &machine_id}, doc! {"$set":{"auth_token_hash":&revoked_auth_hash,"signing_secret_hash":&revoked_signing_hash,"signing_secret_encrypted":bson::Bson::Null,"is_active":false,"status":"offline","connection_owner":bson::Bson::Null,"updated_at":bson::DateTime::from_chrono(now)},"$inc":{"machine_authority_admission_fence":1}}).session(&mut *session).await?;
            db_owned.collection::<Document>(OPERATIONS).update_one(doc! {"_id":operation_id}, doc! {"$set":{"state":"succeeded","cleanup_state":"complete","evidence_digest":&proof,"terminal_at":bson::DateTime::from_chrono(now),"updated_at":bson::DateTime::from_chrono(now)}}).session(&mut *session).await?;
            let key = super::audit_service::audit_chain_hmac_key();
            #[cfg(test)]
            let key = key.or(Some(&[2u8;32]));
            let key = key.ok_or_else(|| AppError::Internal("audit chain key is unavailable".into()))?;
            super::audit_chain_service::append_chained_entry_in_session(&db_owned, session, crate::models::audit_log::AuditLog {
                id: Uuid::new_v4().to_string(), user_id: None, event_type:"machine_unallocated_delete_proven".into(),
                event_data: Some(json!({"machine_id":machine_id,"operation_id":operation_id,"proof":"generation_zero_never_submitted"})),
                ip_address:None,user_agent:None,api_key_id:None,api_key_name:None,seq:None,prev_hash:None,entry_hash:None,created_at:now,
            }, key).await?;
            Ok(Some(finalized))
        }.await;
        api_key_mutation_service::transaction_result(result)
    }).await.map_err(api_key_mutation_service::map_transaction_error)
}

#[cfg(test)]
pub async fn accept_bootstrap_delivery(
    db: &Database,
    machine_id: &str,
    runtime_generation: i64,
    credential_epoch: i64,
    delivery_id: &str,
    sealed_digest: &str,
) -> AppResult<BootstrapDeliveryResult> {
    let machine = managed_machine_for_node(db, machine_id)
        .await?
        .ok_or_else(|| AppError::NotFound("managed machine not found".into()))?;
    if machine.runtime_generation != runtime_generation
        || machine.credential_epoch != credential_epoch
    {
        return Ok(BootstrapDeliveryResult::RekeyRequired);
    }
    if machine.bootstrap_delivery_id.as_deref() == Some(delivery_id)
        && machine.bootstrap_digest.as_deref() == Some(sealed_digest)
    {
        return Ok(BootstrapDeliveryResult::Replay);
    }
    if machine.bootstrap_delivery_id.is_some() {
        return Ok(BootstrapDeliveryResult::RekeyRequired);
    }
    let changed = db.collection::<ManagedMachine>(MACHINES).update_one(doc! {"_id": machine_id, "revision": machine.revision, "runtime_generation": runtime_generation, "credential_epoch": credential_epoch, "bootstrap_delivery_id": null}, doc! {"$set": {"bootstrap_delivery_id": delivery_id, "bootstrap_digest": sealed_digest, "updated_at": bson::DateTime::now()}, "$inc": {"bootstrap_epoch": 1, "revision": 1}}).await?;
    if changed.modified_count == 1 {
        Ok(BootstrapDeliveryResult::Accepted)
    } else {
        Err(AppError::Conflict("bootstrap_delivery_race".into()))
    }
}

#[cfg(test)]
pub async fn rekey_bootstrap(
    db: &Database,
    machine_id: &str,
    expected_revision: i64,
    now: Option<DateTime<Utc>>,
) -> AppResult<ManagedMachine> {
    let now = now.unwrap_or_else(Utc::now);
    let db_owned = db.clone();
    let machine_id = machine_id.to_string();
    let revoked_auth_hash = hash_token(&Uuid::new_v4().to_string());
    let revoked_signing_hash = hash_token(&Uuid::new_v4().to_string());
    let writer_fence = Uuid::new_v4().to_string();
    db_owned.client().start_session().await?.start_transaction().write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move |session| {
        let result: AppResult<ManagedMachine> = async {
            let machine = db_owned.collection::<ManagedMachine>(MACHINES)
                .find_one_and_update(
                    doc! {"_id": &machine_id, "revision": expected_revision, "resource_state": {"$nin": ["deleting", "deleted"]}},
                    doc! {"$set": {"bootstrap_delivery_id": bson::Bson::Null, "bootstrap_digest": bson::Bson::Null, "writer_fence": &writer_fence, "writer_admitted": false, "authorization_state": "pending", "resource_state": "reconciling", "last_runtime_evidence": bson::Bson::Null, "updated_at": bson::DateTime::from_chrono(now)}, "$inc": {"bootstrap_epoch": 1, "credential_epoch": 1, "revision": 1}},
                ).return_document(ReturnDocument::After).session(&mut *session).await?
                .ok_or_else(|| AppError::Conflict("bootstrap_rekey_revision_conflict".into()))?;
            let revoked = db_owned.collection::<Document>(crate::models::node::COLLECTION_NAME)
                .update_one(doc! {"_id": &machine_id}, doc! {"$set": {"auth_token_hash": &revoked_auth_hash, "signing_secret_hash": &revoked_signing_hash, "signing_secret_encrypted": bson::Bson::Null, "is_active": false, "status": "offline", "connection_owner": bson::Bson::Null, "updated_at": bson::DateTime::from_chrono(now)}, "$inc": {"machine_authority_admission_fence": 1_i64}})
                .session(&mut *session).await?;
            if revoked.matched_count != 1 {
                return Err(AppError::Conflict("managed_node_identity_missing".into()));
            }
            Ok(machine)
        }.await;
        api_key_mutation_service::transaction_result(result)
    }).await.map_err(api_key_mutation_service::map_transaction_error)
}

/// Return the live owner qualification and free-slot state. This reads the
/// existing `users` row; it does not create a parallel principal/entitlement
/// registry. Creation repeats the same checks inside its transaction.
pub async fn entitlement(db: &Database, owner_id: &str) -> AppResult<MachineEntitlement> {
    let user = db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": owner_id})
        .await?;
    if let Some(reason) = owner_ineligibility(user.as_ref()) {
        return Ok(MachineEntitlement {
            eligible: false,
            free_slot_available: false,
            machine_id: None,
            reason: Some(reason.into()),
        });
    }
    let slot = db
        .collection::<MachineFreeSlot>(FREE_SLOTS)
        .find_one(doc! {"_id": owner_id})
        .await?;
    let free_slot_available = slot
        .as_ref()
        .is_none_or(|slot| slot.state == FreeSlotState::Released);
    Ok(MachineEntitlement {
        eligible: true,
        free_slot_available,
        machine_id: if free_slot_available {
            None
        } else {
            slot.and_then(|slot| slot.machine_id)
        },
        reason: None,
    })
}

/// Bounded, fair account-safety sweep. A lost account qualification closes
/// authority and queues a physical pause. Only a durable, policy-bound owner
/// deletion basis permits irreversible cleanup, including already paused disks.
/// Funding/grant expiry alone never deletes storage or expires the free slot.
#[cfg(test)]
pub async fn suspend_ineligible(db: &Database) -> AppResult<u64> {
    suspend_ineligible_with_billing(db, None).await
}
pub async fn suspend_ineligible_with_billing(
    db: &Database,
    billing: Option<Arc<super::billing::BillingService>>,
) -> AppResult<u64> {
    let machines: Vec<ManagedMachine> = db
        .collection::<ManagedMachine>(MACHINES)
        .find(doc! {"resource_state": {"$ne": "deleted"}})
        .sort(doc! {"eligibility_checked_at": 1, "_id": 1})
        .limit(64)
        .await?
        .try_collect()
        .await?;
    let mut stopped = 0;
    for machine in machines {
        db.collection::<Document>(MACHINES)
            .update_one(
                doc! {"_id":&machine.node_id},
                doc! {"$set":{"eligibility_checked_at":bson::DateTime::now()}},
            )
            .await?;
        let user = db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id": &machine.owner_id})
            .await?;
        let unauthorized_replay = machine.resource_state == ResourceState::Deleting
            && !deletion_authorized(db, &machine.node_id).await?;
        if machine.resource_state == ResourceState::Deleting && !unauthorized_replay {
            continue;
        }
        let reason = if unauthorized_replay {
            Some("owner_deletion_basis_missing")
        } else {
            owner_ineligibility(user.as_ref())
        };
        let deletion = has_owner_deletion_basis(&machine);
        if reason.is_none()
            || (!deletion
                && matches!(
                    machine.resource_state,
                    ResourceState::Paused | ResourceState::Stopping | ResourceState::Failed
                )
                && machine.authorization_state == AuthorizationState::Revoked
                && !machine.writer_admitted)
        {
            db.collection::<Document>(MACHINES)
                .update_one(
                    doc! {"_id": &machine.node_id},
                    doc! {"$set": {"eligibility_checked_at": bson::DateTime::now()}},
                )
                .await?;
            continue;
        }
        let db_owned = db.clone();
        let operation_id = Uuid::new_v4().to_string();
        let effect_id = Uuid::new_v4().to_string();
        let billing_owned = billing.clone();
        let changed = db_owned.client().start_session().await?.start_transaction().write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move |session| {
            let result: AppResult<bool> = async {
                let current = db_owned.collection::<ManagedMachine>(MACHINES)
                    .find_one(doc! {"_id": &machine.node_id, "resource_state": {"$ne": "deleted"}})
                    .session(&mut *session).await?;
                let Some(current) = current else { return Ok(false) };
                let user = db_owned.collection::<Document>(crate::models::user::COLLECTION_NAME)
                    .find_one(doc! {"_id": &current.owner_id}).session(&mut *session).await?;
                let unauthorized_replay = current.resource_state == ResourceState::Deleting
                    && !deletion_authorized_in_session(&db_owned, session, &current.node_id).await?;
                if current.resource_state == ResourceState::Deleting && !unauthorized_replay { return Ok(false); }
                let Some(reason) = (if unauthorized_replay { Some("owner_deletion_basis_missing") } else { owner_ineligibility(user.as_ref()) }) else { return Ok(false) };
                let deletion=has_owner_deletion_basis(&current);
                if !deletion && matches!(current.resource_state,ResourceState::Paused|ResourceState::Stopping|ResourceState::Failed) && current.authorization_state == AuthorizationState::Revoked && !current.writer_admitted {return Ok(false);}
                if let Some(user)=&user {
                    let touched=db_owned.collection::<Document>(crate::models::user::COLLECTION_NAME).update_one(
                        doc!{"_id":&current.owner_id,"is_active":user.get_bool("is_active").unwrap_or(false),
                            "deletion_requested_at":user.get("deletion_requested_at").cloned().unwrap_or(bson::Bson::Null)},
                        doc!{"$inc":{"machine_lifecycle_safety_fence":1_i64}}).session(&mut *session).await?;
                    if touched.matched_count!=1{return Err(AppError::Conflict("owner_safety_state_changed".into()));}
                }
                let action=if deletion {LifecycleAction::Delete} else {LifecycleAction::Pause};
                let now = Utc::now();
                let operation = MachineOperation {
                    id: operation_id.clone(), owner_id: current.owner_id.clone(), machine_id: Some(current.node_id.clone()),
                    action,
                    idempotency_key_hash: format!("system:{}", idempotency_key_hash(&if unauthorized_replay {format!("account-safety:retained-recovery:{}:{}",current.runtime_generation,current.active_operation_id.as_deref().unwrap_or("unbound"))} else {format!("account-safety:{}:{}",action.as_str(),current.runtime_generation)})),
                    request_digest: request_digest(&json!({"source":"account_safety","generation":current.runtime_generation})),
                    expected_revision: Some(current.revision), desired_runtime_generation: Some(current.runtime_generation),
                    state: OperationState::Accepted, cleanup_state: CleanupState::Pending, effect_id: Some(effect_id.clone()),
                    error_code: Some(reason.into()), error_message: None, evidence_digest: None, retry_count: 0, delete_retry: None,
                    next_attempt_at: Some(now), deadline_at: Some(now+chrono::Duration::hours(24)), created_at: now, updated_at: now, terminal_at: None,
                };
                let outbox = MachineEffectOutbox {
                    id: effect_id.clone(), operation_id: operation_id.clone(), machine_id: current.node_id.clone(), owner_id: current.owner_id.clone(),
                    kind: EffectKind::for_action(action), effect_digest: request_digest(&json!({"machine_id":current.node_id,"safety_generation":current.runtime_generation})),
                    namespace: current.sandbox_namespace.clone(), sandbox_name: current.sandbox_name.clone(), desired_mode: "suspended".into(),
                    state: EffectState::Pending, attempts: 0, claim_id: None, undispatched_claim_id: None, claimed_until: None, last_error: None, available_at: Some(now),
                    submission: None, created_at: now, updated_at: now,
                };
                let updated = db_owned.collection::<Document>(MACHINES)
                    .update_one(doc! {"_id": &current.node_id, "revision": current.revision}, doc! {"$set": {"resource_state":if deletion {"deleting"} else {"stopping"}, "stop_reason": reason, "authorization_state": "revoked", "writer_admitted": false, "active_operation_id": &operation_id, "cleanup_state": "pending", "eligibility_checked_at": bson::DateTime::from_chrono(now), "updated_at": bson::DateTime::from_chrono(now)}, "$inc": {"revision": 1}})
                    .session(&mut *session).await?;
                if updated.matched_count != 1 { return Err(AppError::Conflict("safety_pause_revision_conflict".into())); }
                db_owned.collection::<Document>(crate::models::node::COLLECTION_NAME)
                    .update_one(doc! {"_id": &current.node_id}, doc! {"$set": {"is_active":false,"status":"offline","connection_owner":bson::Bson::Null,"updated_at":bson::DateTime::from_chrono(now)},"$inc":{"machine_authority_admission_fence":1}})
                    .session(&mut *session).await?;
                super::remote_machine_runtime::stop_in_session(&db_owned,billing_owned.as_deref(), session, &current, false).await?;
                db_owned.collection::<MachineOperation>(OPERATIONS).insert_one(operation).session(&mut *session).await?;
                db_owned.collection::<MachineEffectOutbox>(OUTBOX).insert_one(outbox).session(&mut *session).await?;
                supersede_operation(&db_owned, session, current.active_operation_id.as_deref(), now).await?;
                let key = super::audit_service::audit_chain_hmac_key();
                #[cfg(test)]
                let key = key.or(Some(&[2u8; 32]));
                let key = key.ok_or_else(|| AppError::Internal("audit chain key is unavailable".into()))?;
                super::audit_chain_service::append_chained_entry_in_session(&db_owned, session, crate::models::audit_log::AuditLog {
                    id: Uuid::new_v4().to_string(), user_id: None, event_type: if deletion {"machine_account_deletion_cleanup"} else {"machine_account_safety_pause"}.into(),
                    event_data: Some(json!({"owner_id":current.owner_id,"machine_id":current.node_id,"operation_id":operation_id,"reason":reason})),
                    ip_address: None, user_agent: None, api_key_id: None, api_key_name: None, seq: None, prev_hash: None, entry_hash: None, created_at: now,
                }, key).await?;
                Ok(true)
            }.await;
            api_key_mutation_service::transaction_result(result)
        }).await.map_err(api_key_mutation_service::map_transaction_error)?;
        stopped += u64::from(changed);
    }
    Ok(stopped)
}

/// Replay uses the same durable consent fence as new lifecycle admission.
/// Current producers commit the basis with acceptance before effects. Audit
/// history and account absence cannot reconstruct or invent deletion consent.
pub async fn deletion_authorized(db: &Database, machine_id: &str) -> AppResult<bool> {
    let db = db.clone();
    let machine_id = machine_id.to_owned();
    db.client()
        .start_session()
        .await?
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            api_key_mutation_service::transaction_result(
                deletion_authorized_in_session(&db, session, &machine_id).await,
            )
        })
        .await
        .map_err(api_key_mutation_service::map_transaction_error)
}

pub(super) async fn deletion_authorized_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    machine_id: &str,
) -> AppResult<bool> {
    let Some(machine) = db
        .collection::<ManagedMachine>(MACHINES)
        .find_one(doc! {"_id":machine_id,"resource_state":"deleting"})
        .session(&mut *session)
        .await?
    else {
        return Ok(false);
    };
    Ok(has_owner_deletion_basis(&machine))
}

pub(crate) fn has_owner_deletion_basis(machine: &ManagedMachine) -> bool {
    use crate::models::remote_machine::MachineDeletionSource;
    machine.deletion_basis.as_ref().is_some_and(|basis| {
        basis.owner_id == machine.owner_id
            && match basis.source {
                MachineDeletionSource::AccountOwner => {
                    basis.policy_version
                        == super::admin_user_service::ACCOUNT_DELETION_POLICY_VERSION
                }
                MachineDeletionSource::MachineOwner => {
                    basis.policy_version == crate::models::remote_machine::DATA_POLICY_VERSION
                        && machine.data_policy_version
                            == crate::models::remote_machine::DATA_POLICY_VERSION
                }
            }
    })
}

pub fn next_resource_state(action: LifecycleAction) -> ResourceState {
    match action {
        LifecycleAction::Create => ResourceState::Provisioning,
        LifecycleAction::Pause => ResourceState::Stopping,
        LifecycleAction::Resume => ResourceState::Resuming,
        LifecycleAction::Rekey => ResourceState::Stopping,
        LifecycleAction::Delete => ResourceState::Deleting,
        LifecycleAction::Reconcile => ResourceState::Reconciling,
    }
}

#[cfg(test)]
pub fn observation_state(current: ResourceState) -> ResourceState {
    match current {
        ResourceState::Deleting => ResourceState::Deleting,
        ResourceState::Stopping => ResourceState::Stopping,
        ResourceState::Paused => ResourceState::Paused,
        ResourceState::Resuming => ResourceState::Resuming,
        _ => ResourceState::Reconciling,
    }
}
