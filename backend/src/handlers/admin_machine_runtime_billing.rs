//! Runtime configuration is a logical Billing SKU, not a catalog credential.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        machine_runtime_billing::MachineRuntimeRate,
        service_billing::{BillingMetric, PricingSyncStatus},
    },
    mw::auth::AuthUser,
    services::{
        audit_service::{self, AuditActor},
        billing::machine_runtime,
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
    metric: BillingMetric,
    credits_per_second: String,
    sync_status: PricingSyncStatus,
    retired: bool,
    verified_at: Option<String>,
    created_at: String,
}
impl From<MachineRuntimeRate> for RateView {
    fn from(rate: MachineRuntimeRate) -> Self {
        Self {
            rate_version: rate.id,
            metric: rate.metric,
            credits_per_second: rate.credits_per_second.to_string(),
            sync_status: rate.sync_status,
            retired: rate.retired,
            verified_at: rate.verified_at.map(|v| v.to_rfc3339()),
            created_at: rate.created_at.to_rfc3339(),
        }
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
            .map(Into::into)
            .collect(),
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageRequest {
    pub credits_per_second: String,
}
pub async fn stage(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<StageRequest>,
) -> AppResult<Json<RateView>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    if body
        .credits_per_second
        .parse::<crate::models::credits::Credits>()?
        != machine_runtime::INITIAL_CREDITS_PER_SECOND.parse()?
    {
        return Err(AppError::ValidationError(
            "The approved fixed runtime price is 0.000040 Credits per second".into(),
        ));
    }
    let rate = state.billing.stage_initial_machine_runtime_rate().await?;
    audit(&state, &auth, "machine_runtime_rate_staged", &rate.id).await;
    Ok(Json(rate.into()))
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
