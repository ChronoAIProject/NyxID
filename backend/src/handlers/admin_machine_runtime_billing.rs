//! Runtime configuration is a logical Billing SKU, not a catalog credential.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        machine_runtime_billing::MachineRuntimeRate,
        remote_machine::SafeMachineProfileView,
        service_billing::{BillingMetric, PricingSyncStatus},
    },
    mw::auth::AuthUser,
    services::{
        audit_service::{self, AuditActor},
        billing::machine_runtime,
        remote_machine_service,
    },
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct RateView {
    rate_version: String,
    profile_version: String,
    profile: SafeMachineProfileView,
    metric: BillingMetric,
    credits_per_second: String,
    sync_status: PricingSyncStatus,
    retired: bool,
    verified_at: Option<String>,
    created_at: String,
}
impl TryFrom<MachineRuntimeRate> for RateView {
    type Error = AppError;
    fn try_from(rate: MachineRuntimeRate) -> AppResult<Self> {
        let profile =
            crate::services::remote_machine_service::controlled_profile(&rate.profile_version)?;
        Ok(Self {
            rate_version: rate.id,
            profile_version: rate.profile_version,
            profile,
            metric: rate.metric,
            credits_per_second: rate.credits_per_second.to_string(),
            sync_status: rate.sync_status,
            retired: rate.retired,
            verified_at: rate.verified_at.map(|v| v.to_rfc3339()),
            created_at: rate.created_at.to_rfc3339(),
        })
    }
}
#[derive(Serialize)]
pub struct RuntimeBillingView {
    sku: &'static str,
    allowance_service_ref: &'static str,
    allowance_metrics: Vec<BillingMetric>,
    credential_service: bool,
    no_instance_cap: bool,
    paused_storage_free: bool,
    rates: Vec<RateView>,
}
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<RuntimeBillingView>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin_or_operator(&state, &auth, "admin.billing.runtime").await?;
    Ok(Json(RuntimeBillingView {
        sku: machine_runtime::SKU,
        allowance_service_ref: machine_runtime::SKU,
        allowance_metrics: vec![BillingMetric::MachineRunningSeconds],
        credential_service: false,
        no_instance_cap: true,
        paused_storage_free: true,
        rates: state
            .billing
            .list_machine_runtime_rates()
            .await?
            .into_iter()
            .map(TryInto::try_into)
            .collect::<AppResult<_>>()?,
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageRequest {
    pub profile_version: String,
    pub credits_per_second: String,
}
pub async fn stage(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<StageRequest>,
) -> AppResult<Json<RateView>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    remote_machine_service::require_remote_machines_enabled(&state.db, &auth.user_id.to_string())
        .await?;
    let credits_per_second = body.credits_per_second.parse().map_err(|_| {
        AppError::ValidationError("credits_per_second must be an exact decimal price".into())
    })?;
    let rate = state
        .billing
        .stage_machine_runtime_rate(&body.profile_version, credits_per_second)
        .await?;
    audit(&state, &auth, "machine_runtime_rate_staged", &rate.id).await;
    Ok(Json(rate.try_into()?))
}
#[derive(Serialize)]
pub struct SyncView {
    synced: bool,
    rate_version: String,
}
pub async fn sync(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<SyncView>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    remote_machine_service::require_remote_machines_enabled(&state.db, &auth.user_id.to_string())
        .await?;
    let result = state.billing.sync_machine_runtime_rate(&id).await;
    audit(&state, &auth, "machine_runtime_rate_sync_requested", &id).await;
    result?;
    Ok(Json(SyncView {
        synced: true,
        rate_version: id,
    }))
}
pub async fn retire(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    remote_machine_service::require_remote_machines_enabled(&state.db, &auth.user_id.to_string())
        .await?;
    state.billing.retire_machine_runtime_rate(&id).await?;
    audit(&state, &auth, "machine_runtime_rate_retired", &id).await;
    Ok(Json(
        serde_json::json!({"retired":true,"rate_version":id,"historical_contracts_retained":true}),
    ))
}
async fn audit(state: &AppState, auth: &AuthUser, event: &str, id: &str) {
    if audit_service::log_actor_event(
        state.db.clone(),
        &AuditActor::from_auth_user(auth),
        event,
        Some(serde_json::json!({"rate_version":id,"sku":machine_runtime::SKU})),
    )
    .await
    .is_err()
    {
        tracing::error!(event, "Runtime price audit append failed");
    }
}
