//! Owner-only HTTP management for managed Remote Nyx Machines.
//!
//! This surface is intentionally a thin adapter over the durable lifecycle
//! services.  It accepts no runtime commands, credentials, Kubernetes
//! identifiers, or caller-selected resource dimensions.  Runtime effects are
//! produced by the manager's durable outbox and reconciler.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header::HeaderName},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        remote_machine::{MachineProfileSpec, SafeMachineProfileView, SafeManagedMachineView},
        remote_machine_operation::{LifecycleAction, SafeMachineOperationView},
    },
    mw::auth::AuthUser,
    services::{remote_machine_operation_service as operation, remote_machine_service as machine},
};

const IDEMPOTENCY_HEADER: HeaderName = HeaderName::from_static("idempotency-key");
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;
pub use crate::models::remote_machine::{DATA_POLICY_VERSION, POLICY_VERSION};

/// The profile is server-owned.  An absent provider image is represented by
/// an explicit unconfigured marker; the Sandbox adapter remains fail-closed
/// until deployment supplies a verified digest; RuntimeClass is optional.
fn fixed_profile() -> MachineProfileSpec {
    let image = std::env::var("NYXID_REMOTE_MACHINE_IMAGE_DIGEST")
        .ok()
        .filter(|value| value.starts_with("sha256:") || value.contains("@sha256:"))
        .unwrap_or_else(|| "unconfigured".to_string());
    machine::fixed_profile(image)
}

fn request_profile(state: &AppState, profile_version: &str) -> AppResult<MachineProfileSpec> {
    machine::controlled_profile(profile_version)?;
    #[cfg(test)]
    if let Some(local) = &state.local_machine_platform {
        machine::validate_fixed_profile(&local.profile)?;
        return Ok(local.profile.clone());
    }
    let _ = state;
    let profile = fixed_profile();
    machine::validate_fixed_profile(&profile)?;
    Ok(profile)
}

fn default_profile_version() -> String {
    nyxid_machine::managed::PROFILE.into()
}
async fn request_platform(
    state: &AppState,
    fresh_storage: bool,
) -> crate::services::remote_machine_manager::PlatformReadiness {
    #[cfg(test)]
    if let Some(local) = &state.local_machine_platform {
        let r = &local.readiness;
        return crate::services::remote_machine_manager::PlatformReadiness {
            configuration_ready: r.configuration_ready,
            manager_started: r.manager_started,
            namespace: r.namespace,
            blockers: r.blockers.clone(),
            physical_acceptance: r.physical_acceptance,
            lifecycle_basis: r.lifecycle_basis,
        };
    }
    if fresh_storage {
        crate::services::remote_machine_manager::platform_readiness_for_provisioning(
            &state.config.base_url,
        )
        .await
    } else {
        crate::services::remote_machine_manager::platform_readiness(&state.config.base_url)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    #[serde(default = "default_profile_version")]
    pub profile_version: String,
    #[serde(default = "crate::models::remote_machine::default_name")]
    pub name: String,
    #[serde(default = "crate::models::remote_machine::default_capabilities")]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub specialist_id: Option<String>,
    /// Whether this request consumes the owner's single free slot.
    pub free_slot: bool,
    pub policy_version: String,
    pub data_policy_version: String,
    pub acknowledge_shared_environment: bool,
    #[serde(default)]
    pub quote_id: Option<String>,
    #[serde(default)]
    pub accept_account_balance: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteRequest {
    #[serde(default = "default_profile_version")]
    pub profile_version: String,
    pub free_slot: bool,
    /// Optional owned, paused Machine whose retained workspace will be resumed.
    #[serde(default)]
    pub machine_id: Option<String>,
}

/// Stable product contract consumed by preflight and the later owner UI.
/// These are disclosures, not provider capability or acceptance evidence.
#[derive(Debug, Serialize)]
pub struct PolicyView {
    pub policy_version: String,
    pub data_policy_version: &'static str,
    pub free_slot_per_personal_account: u32,
    pub free_entitlement_expires: bool,
    pub free_monthly_runtime_limit_seconds: Option<u64>,
    pub paused_workspace_retention: &'static str,
    pub paused_storage_price_credits: &'static str,
    pub workspace_backup_or_restore: bool,
    pub owner_confirmed_delete_required: bool,
    pub cleanup_target_hours: u32,
    pub job_history_days: u32,
    pub operation_history_days: u32,
    pub terminal_output_max_retention_seconds: u32,
    pub shared_environment_mode: &'static str,
    pub cross_client_secret_isolation: bool,
    pub inbound_network_service: bool,
    pub persistent_environment_variables: bool,
    pub service_charges_separate: bool,
    pub concurrent_jobs: u32,
    pub default_command_timeout_seconds: u32,
    pub maximum_command_timeout_seconds: u32,
    pub job_output_bytes: u32,
    pub machine_output_cache_bytes: u32,
}

fn policy_view() -> PolicyView {
    PolicyView {
        policy_version: POLICY_VERSION.into(),
        data_policy_version: DATA_POLICY_VERSION,
        free_slot_per_personal_account: 1,
        free_entitlement_expires: false,
        free_monthly_runtime_limit_seconds: None,
        paused_workspace_retention: "indefinite_until_owner_confirmed_delete",
        paused_storage_price_credits: "0",
        workspace_backup_or_restore: false,
        owner_confirmed_delete_required: true,
        cleanup_target_hours: 24,
        job_history_days: 30,
        operation_history_days: 90,
        terminal_output_max_retention_seconds: 3600,
        shared_environment_mode: "shared_legacy",
        cross_client_secret_isolation: false,
        inbound_network_service: false,
        persistent_environment_variables: false,
        service_charges_separate: true,
        concurrent_jobs: 2,
        default_command_timeout_seconds: 120,
        maximum_command_timeout_seconds: 3600,
        job_output_bytes: 1024 * 1024,
        machine_output_cache_bytes: 32 * 1024 * 1024,
    }
}

fn require_policy_acceptance(body: &CreateRequest) -> AppResult<()> {
    if body.policy_version != POLICY_VERSION
        || body.data_policy_version != DATA_POLICY_VERSION
        || !body.acknowledge_shared_environment
    {
        return Err(AppError::ValidationError(
            "current machine and data policies and shared environment acknowledgement are required"
                .into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ActionRequest {
    #[serde(default)]
    pub profile_version: Option<String>,
    pub action: Option<LifecycleAction>,
    pub expected_revision: Option<i64>,
    #[serde(default)]
    pub data_policy_version: Option<String>,
    #[serde(default)]
    pub acknowledge_data_loss: bool,
    #[serde(default)]
    pub policy_version: Option<String>,
    #[serde(default)]
    pub acknowledge_shared_environment: bool,
    #[serde(default)]
    pub quote_id: Option<String>,
    #[serde(default)]
    pub accept_account_balance: bool,
}

#[derive(Debug, Serialize)]
pub struct MachineResponse {
    pub machine: SafeManagedMachineView,
}

#[derive(Debug, Serialize)]
pub struct MachineListResponse {
    pub machines: Vec<SafeManagedMachineView>,
    pub next_cursor: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct MachineListQuery {
    #[serde(default)]
    pub include_deleted: bool,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct OperationResponse {
    pub operation: SafeMachineOperationView,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteRetryRequest {
    pub expected_revision: i64,
    pub expected_retry_count: i64,
}

#[derive(Debug, Serialize)]
pub struct CreateResponse {
    pub machine: SafeManagedMachineView,
    pub operation: SafeMachineOperationView,
    pub replayed: bool,
}

#[derive(Debug, Serialize)]
pub struct QuoteResponse {
    pub policy_version: String,
    pub profile: SafeMachineProfileView,
    pub free_slot_requested: bool,
    pub paid_admission: &'static str,
    pub pricing_status: &'static str,
    pub valid_until: chrono::DateTime<Utc>,
    pub policy: PolicyView,
    pub quote_id: Option<String>,
    pub rate_version: Option<String>,
    pub credits_per_second: Option<String>,
    pub funding_mode: &'static str,
    pub no_instance_cap: bool,
    pub cumulative_whole_seconds: bool,
    pub paused_storage_free: bool,
    pub platform: crate::services::remote_machine_manager::PlatformReadiness,
    pub quota: crate::services::remote_machine_quota::OwnerQuotaView,
    pub billing_preflight: Option<crate::services::billing::machine_runtime::RuntimeQuotePreflight>,
}

#[derive(Debug, Serialize)]
pub struct EntitlementResponse {
    pub eligible: bool,
    pub free_slot_available: bool,
    pub machine_id: Option<String>,
    pub reason: Option<String>,
    pub policy_version: &'static str,
    pub policy: PolicyView,
    pub platform: crate::services::remote_machine_manager::PlatformReadiness,
    pub quota: crate::services::remote_machine_quota::OwnerQuotaView,
    pub billing_preflight: Option<crate::services::billing::machine_runtime::RuntimeQuotePreflight>,
}

fn platform_error(
    platform: &crate::services::remote_machine_manager::PlatformReadiness,
) -> AppError {
    AppError::Conflict(format!(
        "managed_machine_platform_unavailable:{}",
        platform.blockers.join(",")
    ))
}
async fn require_platform_or_replay(
    state: &AppState,
    owner_id: &str,
    machine_id: Option<&str>,
    action: LifecycleAction,
    key: &str,
    body: &serde_json::Value,
    profile: &MachineProfileSpec,
) -> AppResult<()> {
    // A durable accepted intent is recoverable even if optional platform probes
    // are unavailable now. The service still checks the canonical body digest.
    if machine::has_recorded_request(&state.db, owner_id, machine_id, action, key, body).await? {
        return Ok(());
    }
    let platform = request_platform(state, matches!(action, LifecycleAction::Create)).await;
    if platform.configuration_ready {
        let operation = if matches!(action, LifecycleAction::Resume | LifecycleAction::Rekey) {
            crate::services::remote_machine_quota::Operation::RetainedVolumeResume
        } else {
            crate::services::remote_machine_quota::Operation::Create
        };
        let quota =
            crate::services::remote_machine_quota::owner_preflight(operation, profile).await;
        if quota.status == "insufficient" {
            return Err(AppError::Conflict(
                "managed_machine_quota_insufficient".into(),
            ));
        }
        return Ok(());
    }
    Err(platform_error(&platform))
}

fn require_idempotency_key(headers: &HeaderMap) -> AppResult<String> {
    let value = headers
        .get(&IDEMPOTENCY_HEADER)
        .ok_or_else(|| AppError::ValidationError("Idempotency-Key header is required".into()))?;
    let value = value
        .to_str()
        .map_err(|_| AppError::ValidationError("Idempotency-Key must be valid UTF-8".into()))?
        .trim();
    if value.is_empty() || value.len() > MAX_IDEMPOTENCY_KEY_BYTES {
        return Err(AppError::ValidationError(
            "Idempotency-Key must be between 1 and 256 bytes".into(),
        ));
    }
    Ok(value.to_owned())
}

fn owner(auth: &AuthUser) -> AppResult<String> {
    super::login_client_context::require_first_party_human(auth)?;
    Ok(auth.user_id.to_string())
}

/// POST /api/v1/remote-machine-quotes
pub async fn quote(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<QuoteRequest>,
) -> AppResult<Json<QuoteResponse>> {
    let owner_id = owner(&auth)?;
    machine::require_remote_machines_enabled(&state.db, &owner_id).await?;
    machine::controlled_profile(&body.profile_version)?;
    let platform = request_platform(&state, body.machine_id.is_none()).await;
    if !platform.configuration_ready {
        return Err(platform_error(&platform));
    }
    let (quota_operation, profile, policy_version) =
        if let Some(machine_id) = body.machine_id.as_deref() {
            uuid::Uuid::parse_str(machine_id)
                .map_err(|_| AppError::ValidationError("invalid machine_id".into()))?;
            let current = machine::get_machine(&state.db, &owner_id, machine_id)
                .await?
                .ok_or_else(|| AppError::NotFound("Remote Machine not found".into()))?;
            machine::validate_current_contract(&current)?;
            if body.profile_version != current.profile.profile_version {
                return Err(AppError::ValidationError(
                    "resume profile must match the retained Machine".into(),
                ));
            }
            if current.resource_state != crate::models::remote_machine::ResourceState::Paused
                || current.free_slot != body.free_slot
                || current.pvc_uid.is_none()
            {
                return Err(AppError::Conflict(
                    "retained Machine is not ready for a resume quote".into(),
                ));
            }
            (
                crate::services::remote_machine_quota::Operation::RetainedVolumeResume,
                current.profile,
                current.policy_version,
            )
        } else {
            (
                crate::services::remote_machine_quota::Operation::Create,
                request_profile(&state, &body.profile_version)?,
                POLICY_VERSION.to_owned(),
            )
        };
    if profile.image_digest == "unconfigured" {
        return Err(AppError::Conflict(
            "managed_machine_profile_unavailable_until_provider_configuration".into(),
        ));
    }
    machine::validate_fixed_profile(&profile)?;
    let runtime_quote = if body.free_slot {
        None
    } else {
        let rate = state
            .billing
            .latest_machine_runtime_rate(&profile.profile_version)
            .await?
            .ok_or_else(|| {
                AppError::BillingProviderUnavailable(
                    "no synchronized runtime rate is available for the requested profile".into(),
                )
            })?;
        Some(
            state
                .billing
                .quote_machine_runtime(
                    &owner_id,
                    crate::services::remote_machine_runtime::terms(&profile, &policy_version)?,
                    &rate.id,
                )
                .await?,
        )
    };
    let billing_preflight = state
        .billing
        .machine_runtime_preflight(&owner_id, &profile.profile_version, runtime_quote.as_ref())
        .await
        .ok();
    let quota =
        crate::services::remote_machine_quota::owner_preflight(quota_operation, &profile).await;
    let policy = policy_view();
    Ok(Json(QuoteResponse {
        policy_version,
        profile: SafeMachineProfileView {
            profile_version: profile.profile_version,
            cpu_millis: profile.cpu_millis,
            memory_mib: profile.memory_mib,
            workspace_gib: profile.workspace_gib,
            ephemeral_gib: profile.ephemeral_gib,
        },
        free_slot_requested: body.free_slot,
        paid_admission: if runtime_quote.is_some() {
            "quote_acceptance_required"
        } else {
            "free"
        },
        pricing_status: if runtime_quote.is_some() {
            "synced"
        } else {
            "free"
        },
        valid_until: runtime_quote
            .as_ref()
            .map_or(Utc::now() + chrono::Duration::minutes(15), |q| {
                q.accept_before
            }),
        quote_id: runtime_quote.as_ref().map(|q| q.id.clone()),
        rate_version: runtime_quote.as_ref().map(|q| q.rate_version.clone()),
        credits_per_second: runtime_quote
            .as_ref()
            .map(|q| q.credits_per_second.to_string()),
        funding_mode: "account_balance",
        no_instance_cap: true,
        cumulative_whole_seconds: true,
        paused_storage_free: true,
        policy,
        platform,
        billing_preflight,
        quota,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntitlementQuery {
    #[serde(default = "default_profile_version")]
    pub profile_version: String,
}

/// GET /api/v1/remote-machine-entitlement
pub async fn entitlement(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Query(query): axum::extract::Query<EntitlementQuery>,
) -> AppResult<Json<EntitlementResponse>> {
    let owner_id = owner(&auth)?;
    let profile = request_profile(&state, &query.profile_version)?;
    let result = machine::entitlement(&state.db, &owner_id).await?;
    Ok(Json(EntitlementResponse {
        eligible: result.eligible,
        free_slot_available: result.free_slot_available,
        machine_id: result.machine_id,
        reason: result.reason,
        policy_version: POLICY_VERSION,
        policy: policy_view(),
        platform: request_platform(&state, true).await,
        billing_preflight: state
            .billing
            .machine_runtime_preflight(&owner_id, &profile.profile_version, None)
            .await
            .ok(),
        quota: crate::services::remote_machine_quota::owner_preflight(
            crate::services::remote_machine_quota::Operation::Create,
            &profile,
        )
        .await,
    }))
}

/// POST /api/v1/remote-machines
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(body): Json<CreateRequest>,
) -> AppResult<(StatusCode, Json<CreateResponse>)> {
    let owner_id = owner(&auth)?;
    machine::require_remote_machines_enabled(&state.db, &owner_id).await?;
    let idempotency_key = require_idempotency_key(&headers)?;
    let profile = request_profile(&state, &body.profile_version)?;
    let mut request_body = json!({"profile_version":body.profile_version,"name":body.name,"capabilities":body.capabilities,"specialist_id":body.specialist_id,"free_slot": body.free_slot, "policy_version": body.policy_version,
        "data_policy_version": body.data_policy_version, "acknowledge_shared_environment": body.acknowledge_shared_environment, "quote_id": body.quote_id, "accept_account_balance":body.accept_account_balance});
    crate::services::remote_machine_configuration::normalize_request(&mut request_body)?;
    require_policy_acceptance(&body)?;
    require_platform_or_replay(
        &state,
        &owner_id,
        None,
        LifecycleAction::Create,
        &idempotency_key,
        &request_body,
        &profile,
    )
    .await?;
    let result = machine::create_machine_with_billing(
        &state.db,
        machine::CreateMachineInput {
            owner_id,
            free_slot: body.free_slot,
            profile,
            sandbox_namespace: "nyxid-machine".into(),
            idempotency_key,
            request_body,
            now: None,
        },
        state.billing.clone(),
    )
    .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(CreateResponse {
            machine: crate::services::remote_machine_configuration::safe_view(
                &state.db,
                &result.machine,
            )
            .await?,
            operation: machine::safe_operation_view(&result.operation),
            replayed: result.replayed,
        }),
    ))
}

/// GET /api/v1/remote-machines
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Query(query): axum::extract::Query<MachineListQuery>,
) -> AppResult<Json<MachineListResponse>> {
    let owner_id = owner(&auth)?;
    let page = machine::list_machines_page(
        &state.db,
        &owner_id,
        query.include_deleted,
        query.cursor.as_deref(),
        query.limit.unwrap_or(100),
    )
    .await?;
    let mut views = Vec::with_capacity(page.machines.len());
    for row in &page.machines {
        views.push(crate::services::remote_machine_configuration::safe_view(&state.db, row).await?);
    }
    Ok(Json(MachineListResponse {
        machines: views,
        next_cursor: page.next_cursor,
    }))
}

/// GET /api/v1/remote-machines/{machine_id}
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(machine_id): Path<String>,
) -> AppResult<Json<MachineResponse>> {
    let owner_id = owner(&auth)?;
    let managed = machine::get_machine(&state.db, &owner_id, &machine_id)
        .await?
        .ok_or_else(|| AppError::NotFound("managed machine not found".into()))?;
    Ok(Json(MachineResponse {
        machine: crate::services::remote_machine_configuration::safe_view(&state.db, &managed)
            .await?,
    }))
}

async fn lifecycle_action(
    state: AppState,
    auth: AuthUser,
    headers: HeaderMap,
    Path(machine_id): Path<String>,
    Json(body): Json<ActionRequest>,
    action: LifecycleAction,
) -> AppResult<(StatusCode, Json<OperationResponse>)> {
    let owner_id = owner(&auth)?;
    let expected_revision = body.expected_revision.ok_or_else(|| {
        AppError::ValidationError("expected_revision is required for lifecycle actions".into())
    })?;
    if action == LifecycleAction::Delete
        && (body.data_policy_version.as_deref() != Some(DATA_POLICY_VERSION)
            || !body.acknowledge_data_loss)
    {
        return Err(AppError::ValidationError(
            "delete requires current data policy acknowledgement and irreversible data loss".into(),
        ));
    }
    if matches!(action, LifecycleAction::Resume | LifecycleAction::Rekey)
        && (!body.acknowledge_shared_environment
            || body.policy_version.as_deref() != Some(POLICY_VERSION)
            || body.data_policy_version.as_deref() != Some(DATA_POLICY_VERSION))
    {
        return Err(AppError::ValidationError(
            "saved machine and data policies and shared environment acknowledgement are required"
                .into(),
        ));
    }
    if let Some(requested) = body.action
        && requested != action
    {
        return Err(AppError::ValidationError(
            "action does not match the requested lifecycle endpoint".into(),
        ));
    }
    let idempotency_key = require_idempotency_key(&headers)?;
    let mut request_body = json!({
        "action": action.as_str(),
        "expected_revision": expected_revision,
        "data_policy_version": body.data_policy_version,
        "acknowledge_data_loss": body.acknowledge_data_loss,
        "policy_version": body.policy_version,
        "acknowledge_shared_environment": body.acknowledge_shared_environment,
        "quote_id": body.quote_id, "accept_account_balance":body.accept_account_balance,
    });
    if let Some(version) = body.profile_version {
        request_body["profile_version"] = json!(version);
    }
    if matches!(action, LifecycleAction::Resume | LifecycleAction::Rekey) {
        machine::require_remote_machines_enabled(&state.db, &owner_id).await?;
        let saved = machine::get_machine(&state.db, &owner_id, &machine_id)
            .await?
            .ok_or_else(|| AppError::NotFound("managed machine not found".into()))?;
        machine::validate_current_contract(&saved)?;
        machine::normalize_profile_request(&mut request_body, &saved.profile.profile_version)?;
        require_platform_or_replay(
            &state,
            &owner_id,
            Some(&machine_id),
            action,
            &idempotency_key,
            &request_body,
            &saved.profile,
        )
        .await?;
    }
    let result = machine::request_action_with_billing(
        &state.db,
        machine::LifecycleRequest {
            owner_id,
            machine_id,
            action,
            expected_revision: Some(expected_revision),
            idempotency_key,
            request_body,
            now: None,
        },
        state.billing.clone(),
    )
    .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(OperationResponse {
            operation: machine::safe_operation_view(&result.operation),
        }),
    ))
}

/// DELETE /api/v1/remote-machines/{machine_id}
pub async fn delete(
    state: State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    path: Path<String>,
    body: Json<ActionRequest>,
) -> AppResult<(StatusCode, Json<OperationResponse>)> {
    lifecycle_action(state.0, auth, headers, path, body, LifecycleAction::Delete).await
}

/// POST /api/v1/remote-machines/{machine_id}/rekey
/// Lost local bootstrap recipients require owner-authorized, fenced recovery;
/// an untrusted replacement workload can never rotate its own credentials.
pub async fn rekey(
    state: State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    path: Path<String>,
    body: Json<ActionRequest>,
) -> AppResult<(StatusCode, Json<OperationResponse>)> {
    lifecycle_action(state.0, auth, headers, path, body, LifecycleAction::Rekey).await
}

/// POST /api/v1/remote-machines/{machine_id}/actions
pub async fn action(
    state: State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    path: Path<String>,
    Json(body): Json<ActionRequest>,
) -> AppResult<(StatusCode, Json<OperationResponse>)> {
    let selected = body.action.ok_or_else(|| {
        AppError::ValidationError(
            "action is required and must be pause, resume, rekey, or delete".into(),
        )
    })?;
    if selected == LifecycleAction::Create || selected == LifecycleAction::Reconcile {
        return Err(AppError::ValidationError(
            "only pause, resume, rekey, and delete are owner lifecycle actions".into(),
        ));
    }
    lifecycle_action(state.0, auth, headers, path, Json(body), selected).await
}

/// GET /api/v1/remote-machine-operations/{operation_id}
pub async fn operation(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(operation_id): Path<String>,
) -> AppResult<Json<OperationResponse>> {
    let owner_id = owner(&auth)?;
    let row = operation::get_display_operation(&state.db, &owner_id, &operation_id)
        .await?
        .ok_or_else(|| AppError::NotFound("machine operation not found".into()))?;
    Ok(Json(OperationResponse {
        operation: machine::safe_operation_view(&row),
    }))
}

/// POST /api/v1/remote-machine-operations/{operation_id}/retry-delete
pub async fn retry_delete(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(operation_id): Path<String>,
    Json(body): Json<DeleteRetryRequest>,
) -> AppResult<(StatusCode, Json<OperationResponse>)> {
    let owner_id = owner(&auth)?;
    let key = require_idempotency_key(&headers)?;
    let row = crate::services::remote_machine_delete_retry::retry(
        &state.db,
        &owner_id,
        &operation_id,
        body.expected_revision,
        body.expected_retry_count,
        &key,
    )
    .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(OperationResponse {
            operation: machine::safe_operation_view(&row),
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn idempotency_key_is_required_and_bounded() {
        let mut headers = HeaderMap::new();
        assert!(require_idempotency_key(&headers).is_err());
        headers.insert(
            IDEMPOTENCY_HEADER,
            HeaderValue::from_static("machine-create-1"),
        );
        assert_eq!(
            require_idempotency_key(&headers).unwrap(),
            "machine-create-1"
        );
        headers.insert(
            IDEMPOTENCY_HEADER,
            HeaderValue::from_str(&"x".repeat(257)).unwrap(),
        );
        assert!(require_idempotency_key(&headers).is_err());
    }

    #[test]
    fn profile_does_not_accept_caller_selected_values() {
        let profile = fixed_profile();
        assert_eq!(profile.profile_version, "remote-machine");
        assert_eq!(profile.ephemeral_gib, None);
        assert_eq!(profile.cpu_millis, 2_000);
        assert_eq!(profile.workspace_gib, 20);
    }

    #[test]
    fn management_requires_first_party_human_authority() {
        use crate::mw::auth::AuthMethod;
        let mut auth = crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string());
        for method in [
            AuthMethod::ApiKey,
            AuthMethod::Delegated,
            AuthMethod::ServiceAccount,
            AuthMethod::Relay,
        ] {
            auth.auth_method = method;
            assert!(owner(&auth).is_err());
        }
        auth.auth_method = AuthMethod::AccessToken;
        auth.oauth_client_id = Some("third-party-client".into());
        assert!(owner(&auth).is_err());
        auth.oauth_client_id = None;
        assert!(owner(&auth).is_ok());
        auth.auth_method = AuthMethod::Session;
        assert!(owner(&auth).is_ok());
    }

    #[test]
    fn create_requires_explicit_current_policies_and_shared_environment_acknowledgement() {
        let mut body = CreateRequest {
            profile_version: default_profile_version(),
            name: crate::models::remote_machine::default_name(),
            capabilities: crate::models::remote_machine::default_capabilities(),
            specialist_id: None,
            free_slot: true,
            policy_version: POLICY_VERSION.into(),
            data_policy_version: DATA_POLICY_VERSION.into(),
            acknowledge_shared_environment: true,
            quote_id: None,
            accept_account_balance: false,
        };
        assert!(require_policy_acceptance(&body).is_ok());
        body.acknowledge_shared_environment = false;
        assert!(require_policy_acceptance(&body).is_err());
        body.acknowledge_shared_environment = true;
        body.data_policy_version = "unknown".into();
        assert!(require_policy_acceptance(&body).is_err());
        assert!(serde_json::from_value::<CreateRequest>(json!({"free_slot":true})).is_err());
    }

    #[test]
    fn policy_discloses_indefinite_free_retention_and_separate_service_charges() {
        let policy = serde_json::to_value(policy_view()).unwrap();
        assert_eq!(
            policy["paused_workspace_retention"],
            "indefinite_until_owner_confirmed_delete"
        );
        assert_eq!(policy["paused_storage_price_credits"], "0");
        assert_eq!(
            policy["free_monthly_runtime_limit_seconds"],
            serde_json::Value::Null
        );
        assert_eq!(policy["service_charges_separate"], true);
        assert_eq!(policy["cross_client_secret_isolation"], false);
        assert!(policy.get("paused_expires_at").is_none());
        assert!(policy.get("instance_spend_cap").is_none());
    }

    #[tokio::test]
    async fn rekey_requires_human_cas_idempotency_and_current_policy_before_database_access() {
        use crate::mw::auth::AuthMethod;
        use axum::response::IntoResponse;
        let state = crate::test_utils::test_app_state_no_db().await;
        let mut auth = crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string());
        let machine_id = uuid::Uuid::new_v4().to_string();
        let body = || ActionRequest {
            expected_revision: Some(7),
            data_policy_version: Some(DATA_POLICY_VERSION.into()),
            policy_version: Some(POLICY_VERSION.into()),
            acknowledge_shared_environment: true,
            ..Default::default()
        };
        let mut headers = HeaderMap::new();
        headers.insert(IDEMPOTENCY_HEADER, HeaderValue::from_static("rekey-1"));
        auth.auth_method = AuthMethod::ApiKey;
        let denied = rekey(
            State(state.clone()),
            auth.clone(),
            headers.clone(),
            Path(machine_id.clone()),
            Json(body()),
        )
        .await
        .expect_err("non-human rekey must be denied before database access");
        assert_eq!(denied.into_response().status(), StatusCode::FORBIDDEN);
        auth.auth_method = AuthMethod::Session;
        let mut missing_revision = body();
        missing_revision.expected_revision = None;
        assert!(matches!(
            rekey(
                State(state.clone()),
                auth.clone(),
                headers.clone(),
                Path(machine_id.clone()),
                Json(missing_revision)
            )
            .await,
            Err(AppError::ValidationError(_))
        ));
        let mut missing_policy = body();
        missing_policy.acknowledge_shared_environment = false;
        assert!(matches!(
            rekey(
                State(state.clone()),
                auth.clone(),
                headers.clone(),
                Path(machine_id.clone()),
                Json(missing_policy)
            )
            .await,
            Err(AppError::ValidationError(_))
        ));
        assert!(matches!(
            rekey(
                State(state),
                auth,
                HeaderMap::new(),
                Path(machine_id),
                Json(body())
            )
            .await,
            Err(AppError::ValidationError(_))
        ));
    }
}

/// Safe in-site notifications and timeline, bounded to 100 entries per page.
pub async fn events(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(machine_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<EventQuery>,
) -> AppResult<Json<crate::services::remote_machine_events::EventPage>> {
    let owner_id = owner(&auth)?;
    if machine::get_machine(&state.db, &owner_id, &machine_id)
        .await?
        .is_none()
    {
        return Err(AppError::NotFound("managed machine not found".into()));
    }
    Ok(Json(
        crate::services::remote_machine_events::list_page(
            &state.db,
            &owner_id,
            &machine_id,
            query.before,
            query.cursor.as_deref(),
            query.limit,
        )
        .await?,
    ))
}
#[derive(Deserialize)]
pub struct EventQuery {
    pub before: Option<chrono::DateTime<Utc>>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Deserialize, Default)]
pub struct BillingQuery {
    pub run_id: Option<String>,
    pub offset: Option<u64>,
}
/// GET /api/v1/remote-machines/{machine_id}/billing
pub async fn billing(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(machine_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<BillingQuery>,
) -> AppResult<Json<crate::services::billing::machine_runtime::MachineRuntimeDetail>> {
    let owner_id = owner(&auth)?;
    let managed = machine::get_machine(&state.db, &owner_id, &machine_id)
        .await?
        .ok_or_else(|| AppError::NotFound("managed machine not found".into()))?;
    let run = query
        .run_id
        .or(managed.runtime_run_id)
        .ok_or_else(|| AppError::NotFound("runtime billing contract not found".into()))?;
    let detail = state
        .billing
        .machine_runtime_detail(&owner_id, &run, query.offset.unwrap_or(0).min(100_000))
        .await?;
    if detail.node_id != machine_id {
        return Err(AppError::NotFound(
            "runtime billing contract not found".into(),
        ));
    }
    Ok(Json(detail))
}
#[derive(Deserialize, Default)]
pub struct RunQuery {
    pub offset: Option<u64>,
    pub limit: Option<u32>,
}
#[derive(Serialize)]
pub struct RuntimeRunView {
    pub run_id: String,
    pub rate_version: String,
    pub accepted_credits_per_second: String,
    pub state: crate::models::machine_runtime_billing::MachineRuntimeRunState,
    pub runtime_generation: i64,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}
#[derive(Serialize)]
pub struct RuntimeRunsResponse {
    pub runs: Vec<RuntimeRunView>,
    pub next_offset: Option<u64>,
}
/// GET /api/v1/remote-machines/{machine_id}/billing/runs
pub async fn billing_runs(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(machine_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<RunQuery>,
) -> AppResult<Json<RuntimeRunsResponse>> {
    use crate::models::machine_runtime_billing::{MachineRuntimeRun, RUN_COLLECTION};
    use futures::TryStreamExt;
    let owner_id = owner(&auth)?;
    machine::get_machine(&state.db, &owner_id, &machine_id)
        .await?
        .ok_or_else(|| AppError::NotFound("managed machine not found".into()))?;
    let offset = query.offset.unwrap_or(0).min(100_000);
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let mut rows: Vec<MachineRuntimeRun> = state
        .db
        .collection(RUN_COLLECTION)
        .find(mongodb::bson::doc! {"owner_id":&owner_id,"node_id":&machine_id})
        .sort(mongodb::bson::doc! {"created_at":-1,"_id":-1})
        .skip(offset)
        .limit(i64::from(limit) + 1)
        .await?
        .try_collect()
        .await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    Ok(Json(RuntimeRunsResponse {
        runs: rows
            .into_iter()
            .map(|r| RuntimeRunView {
                run_id: r.id,
                rate_version: r.rate.id,
                accepted_credits_per_second: r.rate.credits_per_second.to_string(),
                state: r.state,
                runtime_generation: r.runtime_generation,
                created_at: r.created_at,
                updated_at: r.updated_at,
            })
            .collect(),
        next_offset: more.then_some(offset + u64::from(limit)),
    }))
}
/// GET /api/v1/remote-machines/{machine_id}/operations
pub async fn operations(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(machine_id): Path<String>,
) -> AppResult<Json<Vec<SafeMachineOperationView>>> {
    let owner_id = owner(&auth)?;
    machine::get_machine(&state.db, &owner_id, &machine_id)
        .await?
        .ok_or_else(|| AppError::NotFound("managed machine not found".into()))?;
    Ok(Json(
        operation::list_operations(&state.db, &owner_id, Some(&machine_id))
            .await?
            .iter()
            .map(machine::safe_operation_view)
            .collect(),
    ))
}
