//! Central admin billing surface: environment mirror, global defaults,
//! cached rates, and every billed service's effective prices with sources.
//! Reads are admin/operator; writes are admin-only.

use crate::services::billing::config::{
    self, BillingConfigSource, EffectiveServiceBilling, GlobalBillingPatch, ResolverEnv,
};
use crate::{
    AppState,
    errors::AppResult,
    handlers::admin_helpers::{require_admin, require_admin_or_operator},
    mw::auth::AuthUser,
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct OverdraftCapResponse {
    /// Applied to newly provisioned wallets, top-up provisioning and backfill.
    pub effective: i64,
    pub source: BillingConfigSource,
    /// Stored global value; `None` falls back to the environment default.
    pub global: Option<i64>,
    pub env: i64,
}

#[derive(Debug, Serialize)]
pub struct BillingConfigResponse {
    pub billing_enabled: bool,
    pub lago_configured: bool,
    pub resale_enabled: bool,
    pub default_overdraft_cap_credits: OverdraftCapResponse,
    pub max_default_overdraft_cap_credits: i64,
    /// Read-only deployment values. Secrets are presence flags only.
    pub environment: Vec<ConfigValue>,
}

#[derive(Debug, Serialize)]
pub struct ConfigValue {
    pub key: &'static str,
    pub value: Option<String>,
    pub source: BillingConfigSource,
    pub secret_set: Option<bool>,
}

fn env_value(key: &'static str, value: Option<String>) -> ConfigValue {
    ConfigValue {
        key,
        value,
        source: BillingConfigSource::Env,
        secret_set: None,
    }
}

fn env_secret(key: &'static str, set: bool) -> ConfigValue {
    ConfigValue {
        key,
        value: None,
        source: BillingConfigSource::Env,
        secret_set: Some(set),
    }
}

fn config_response(
    state: &AppState,
    global: Option<&crate::models::platform_settings::BillingSettings>,
) -> BillingConfigResponse {
    let c = &state.config;
    let env_cap = c.billing_default_overdraft_cap_credits;
    let (effective, source) = config::effective_overdraft_cap(global, env_cap);
    BillingConfigResponse {
        billing_enabled: state.billing.billing_enabled(),
        lago_configured: state.billing.lago_configured(),
        resale_enabled: state.billing.resale_enabled(),
        default_overdraft_cap_credits: OverdraftCapResponse {
            effective,
            source,
            global: global.and_then(|g| g.default_overdraft_cap_credits),
            env: env_cap,
        },
        max_default_overdraft_cap_credits: config::MAX_DEFAULT_OVERDRAFT_CAP_CREDITS,
        environment: vec![
            env_value("BILLING_ENABLED", Some(c.billing_enabled.to_string())),
            env_value(
                "BILLING_RESALE_ENABLED",
                Some(c.billing_resale_enabled.to_string()),
            ),
            env_value("LAGO_API_URL", c.lago_api_url.clone()),
            env_value("LAGO_PLAN_CODE", Some(c.lago_plan_code.clone())),
            env_value(
                "LAGO_PAYMENT_PROVIDER_CODE",
                c.lago_payment_provider_code.clone(),
            ),
            env_value(
                "BILLING_RECONCILE_INTERVAL_SECS",
                Some(c.billing_reconcile_interval_secs.to_string()),
            ),
            env_value(
                "BILLING_RATE_CACHE_TTL_SECS",
                Some(c.billing_rate_cache_ttl_secs.to_string()),
            ),
            env_value(
                "BILLING_RESERVATION_ABANDON_SECS",
                Some(c.billing_reservation_abandon_secs.to_string()),
            ),
            env_value(
                "BILLING_DEFAULT_OVERDRAFT_CAP_CREDITS",
                Some(env_cap.to_string()),
            ),
            env_value(
                "BILLING_FAIL_CLOSED",
                Some(c.billing_fail_closed.to_string()),
            ),
            env_value(
                "BILLING_EXACT_CUTOVER_DRAINED",
                Some(c.billing_exact_cutover_drained.to_string()),
            ),
            env_secret("LAGO_API_KEY", c.lago_api_key.is_some()),
            env_secret("LAGO_WEBHOOK_SECRET", c.lago_webhook_secret.is_some()),
            env_secret(
                "BILLING_LEDGER_HMAC_KEY",
                c.billing_ledger_hmac_key.is_some(),
            ),
        ],
    }
}

/// GET /api/v1/admin/billing/config
pub async fn get_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<BillingConfigResponse>> {
    require_admin_or_operator(&state, &auth, "admin.billing.config").await?;
    let global = config::load_settings(&state.db).await?;
    Ok(Json(config_response(&state, global.as_ref())))
}

/// Per-field patch: an absent field keeps its value, `null` clears it to the
/// environment default, a number sets it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateBillingConfigRequest {
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub default_overdraft_cap_credits: Option<Option<i64>>,
}

/// PUT /api/v1/admin/billing/config
pub async fn put_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<UpdateBillingConfigRequest>,
) -> AppResult<Json<BillingConfigResponse>> {
    require_admin(&state, &auth).await?;
    let patch = GlobalBillingPatch {
        default_overdraft_cap_credits: input.default_overdraft_cap_credits,
    };
    let saved = config::update_global_defaults(
        &state.db,
        &patch,
        &crate::services::audit_service::AuditActor::from_auth_user(&auth),
        state.audit_chain_hmac_key.as_slice(),
    )
    .await?;
    Ok(Json(config_response(&state, Some(&saved))))
}

#[derive(Debug, Serialize)]
pub struct BillingRateResponse {
    pub id: String,
    pub lago_metric_code: String,
    pub model: Option<String>,
    /// Exact decimal credits per unit.
    pub credits_per_unit: String,
    pub synced_at: chrono::DateTime<chrono::Utc>,
    pub stale_after: chrono::DateTime<chrono::Utc>,
    pub stale: bool,
    pub retired_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn rate_ttl_secs(state: &AppState) -> AppResult<i64> {
    i64::try_from(state.config.billing_rate_cache_ttl_secs).map_err(|_| {
        crate::errors::AppError::Internal("billing rate cache TTL is too large".into())
    })
}

/// GET /api/v1/admin/billing/rates
pub async fn get_rates(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<BillingRateResponse>>> {
    require_admin_or_operator(&state, &auth, "admin.billing.rates").await?;
    let ttl = chrono::Duration::seconds(rate_ttl_secs(&state)?);
    let now = chrono::Utc::now();
    let rows = config::load_rates(&state.db).await?;
    rows.into_iter()
        .map(|r| {
            let pico = crate::services::billing::amounts::rate_pico(
                r.credits_per_unit_pico,
                r.credits_per_unit_micros,
            );
            let credits = crate::models::credits::Credits::from_pico(pico).map_err(|_| {
                crate::errors::AppError::Internal("cached rate is out of range".into())
            })?;
            Ok(BillingRateResponse {
                id: r.id,
                lago_metric_code: r.lago_metric_code,
                model: r.model,
                credits_per_unit: credits.to_string(),
                synced_at: r.synced_at,
                stale_after: r.synced_at + ttl,
                stale: r.synced_at + ttl < now,
                retired_at: r.retired_at,
            })
        })
        .collect::<AppResult<Vec<_>>>()
        .map(Json)
}

#[derive(Debug, Serialize)]
pub struct BillingServiceResponse {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub is_active: bool,
    /// Unit used by allowances and catalog summaries.
    pub effective_metric: crate::models::service_billing::BillingMetric,
    pub allowance_metrics: Vec<crate::models::service_billing::BillingMetric>,
    pub effective: EffectiveServiceBilling,
}

fn service_rows(
    state: &AppState,
    services: &[crate::models::downstream_service::DownstreamService],
    rates: &std::collections::HashMap<String, crate::models::billing_rate_cache::BillingRateCache>,
) -> AppResult<Vec<BillingServiceResponse>> {
    let env = ResolverEnv {
        billing_enabled: state.billing.billing_enabled(),
        resale_enabled: state.billing.resale_enabled(),
        rate_ttl_secs: rate_ttl_secs(state)?,
        now: chrono::Utc::now(),
        rates,
    };
    Ok(services
        .iter()
        .map(|s| BillingServiceResponse {
            id: s.id.clone(),
            slug: s.slug.clone(),
            name: s.name.clone(),
            is_active: s.is_active,
            effective_metric:
                crate::services::billing::metric_resolution::effective_platform_metric(s),
            allowance_metrics: crate::services::billing::metric_resolution::allowance_metrics(s),
            effective: config::resolve_service_billing(&env, s),
        })
        .collect())
}

async fn service_row(state: &AppState, id: &str) -> AppResult<BillingServiceResponse> {
    let service = config::load_service(&state.db, id)
        .await?
        .ok_or_else(|| crate::errors::AppError::NotFound("service not found".into()))?;
    let services = [service];
    let rates = config::load_rates_for(&state.db, &services).await?;
    Ok(service_rows(state, &services, &rates)?.remove(0))
}

/// GET /api/v1/admin/billing/overview
pub async fn get_overview(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<BillingServiceResponse>>> {
    require_admin_or_operator(&state, &auth, "admin.billing.overview").await?;
    let services = config::load_billed_services(&state.db).await?;
    let rates = config::load_rates_for(&state.db, &services).await?;
    Ok(Json(service_rows(&state, &services, &rates)?))
}

/// GET /api/v1/admin/billing/services/{id}
pub async fn get_service(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<BillingServiceResponse>> {
    require_admin_or_operator(&state, &auth, "admin.billing.service").await?;
    Ok(Json(service_row(&state, &id).await?))
}

#[derive(Debug, Serialize)]
pub struct ResyncResponse {
    pub synced: bool,
    pub service: BillingServiceResponse,
}

/// POST /api/v1/admin/billing/services/{id}/resync
pub async fn resync_service(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<ResyncResponse>> {
    require_admin(&state, &auth).await?;
    let outcome = config::resync_service(
        &state.db,
        &state.billing,
        &crate::services::audit_service::AuditActor::from_auth_user(&auth),
        &id,
    )
    .await?;
    let services = [outcome.service];
    let rates = config::load_rates_for(&state.db, &services).await?;
    Ok(Json(ResyncResponse {
        synced: outcome.synced,
        service: service_rows(&state, &services, &rates)?.remove(0),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BillingOnlyRequest {
    billing: crate::handlers::services::BillingUpdate,
}

/// PUT /api/v1/admin/billing/services/{id}
///
/// Billing-only writes share `PUT /services/{id}`'s write path and audit.
pub async fn update_service_billing(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: crate::telemetry::TelemetryContext,
    Path(service_id): Path<String>,
    Json(body): Json<BillingOnlyRequest>,
) -> AppResult<Json<BillingServiceResponse>> {
    require_admin(&state, &auth_user).await?;
    let body = crate::handlers::services::UpdateServiceRequest::billing_only(body.billing);
    let _service = Box::pin(crate::handlers::services::update_service_inner(
        state.clone(),
        auth_user,
        tele,
        service_id.clone(),
        body,
    ))
    .await?;
    Ok(Json(service_row(&state, &service_id).await?))
}
