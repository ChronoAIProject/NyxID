use crate::models::platform_settings::BillingSettings;
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
pub struct BillingConfigResponse {
    pub billing_enabled: bool,
    pub lago_configured: bool,
    pub resale_enabled: bool,
    pub global: Option<BillingSettings>,
    pub precedence: [&'static str; 4],
    pub environment: Vec<ConfigValue>,
}

#[derive(Debug, Serialize)]
pub struct ConfigValue {
    pub key: &'static str,
    pub value: Option<String>,
    pub source: &'static str,
    pub secret_set: Option<bool>,
}

pub async fn get_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<BillingConfigResponse>> {
    require_admin_or_operator(&state, &auth, "admin.billing.config").await?;
    let global = crate::services::billing::config::load_settings(&state.db).await?;
    Ok(Json(BillingConfigResponse {
        billing_enabled: state.billing.billing_enabled(),
        lago_configured: state.billing.lago_configured(),
        resale_enabled: state.billing.resale_enabled(),
        global,
        precedence: ["env", "global", "service", "legacy/free"],
        environment: vec![
            ConfigValue {
                key: "LAGO_API_URL",
                value: state.config.lago_api_url.clone(),
                source: "env",
                secret_set: None,
            },
            ConfigValue {
                key: "LAGO_PLAN_CODE",
                value: Some(state.config.lago_plan_code.clone()),
                source: "env",
                secret_set: None,
            },
            ConfigValue {
                key: "LAGO_PAYMENT_PROVIDER_CODE",
                value: state.config.lago_payment_provider_code.clone(),
                source: "env",
                secret_set: None,
            },
            ConfigValue {
                key: "BILLING_RECONCILE_INTERVAL_SECS",
                value: Some(state.config.billing_reconcile_interval_secs.to_string()),
                source: "env",
                secret_set: None,
            },
            ConfigValue {
                key: "BILLING_RATE_CACHE_TTL_SECS",
                value: Some(state.config.billing_rate_cache_ttl_secs.to_string()),
                source: "env",
                secret_set: None,
            },
            ConfigValue {
                key: "BILLING_RESERVATION_ABANDON_SECS",
                value: Some(state.config.billing_reservation_abandon_secs.to_string()),
                source: "env",
                secret_set: None,
            },
            ConfigValue {
                key: "BILLING_DEFAULT_OVERDRAFT_CAP_CREDITS",
                value: Some(
                    state
                        .config
                        .billing_default_overdraft_cap_credits
                        .to_string(),
                ),
                source: "env",
                secret_set: None,
            },
            ConfigValue {
                key: "BILLING_FAIL_CLOSED",
                value: Some(state.config.billing_fail_closed.to_string()),
                source: "env",
                secret_set: None,
            },
            ConfigValue {
                key: "BILLING_EXACT_CUTOVER_DRAINED",
                value: Some(state.config.billing_exact_cutover_drained.to_string()),
                source: "env",
                secret_set: None,
            },
            ConfigValue {
                key: "LAGO_API_KEY",
                value: None,
                source: "env",
                secret_set: Some(state.config.lago_api_key.is_some()),
            },
            ConfigValue {
                key: "LAGO_WEBHOOK_SECRET",
                value: None,
                source: "env",
                secret_set: Some(state.config.lago_webhook_secret.is_some()),
            },
            ConfigValue {
                key: "BILLING_LEDGER_HMAC_KEY",
                value: None,
                source: "env",
                secret_set: Some(std::env::var_os("BILLING_LEDGER_HMAC_KEY").is_some()),
            },
        ],
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateConfigRequest {
    pub billing: Option<BillingSettings>,
}

pub async fn put_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<UpdateConfigRequest>,
) -> AppResult<Json<BillingConfigResponse>> {
    require_admin(&state, &auth).await?;
    if input
        .billing
        .as_ref()
        .and_then(|b| b.default_overdraft_cap_credits)
        .is_some_and(|v| v < 0)
    {
        return Err(crate::errors::AppError::ValidationError(
            "default overdraft cap must be non-negative".into(),
        ));
    }
    crate::services::billing::config::save_settings_and_audit(
        &state.db,
        input.billing.clone(),
        &crate::services::audit_service::AuditActor::from_auth_user(&auth),
    )
    .await?;
    get_config(State(state), auth).await
}

#[derive(Debug, Serialize)]
pub struct BillingRateResponse {
    pub id: String,
    pub metric: String,
    pub model: Option<String>,
    pub credits_per_unit_micros: i64,
    pub synced_at: chrono::DateTime<chrono::Utc>,
    pub retired_at: Option<chrono::DateTime<chrono::Utc>>,
    pub stale: bool,
}

pub async fn get_rates(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<BillingRateResponse>>> {
    require_admin_or_operator(&state, &auth, "admin.billing.rates").await?;
    let ttl_secs = i64::try_from(state.config.billing_rate_cache_ttl_secs).map_err(|_| {
        crate::errors::AppError::ValidationError("billing rate cache TTL is too large".into())
    })?;
    let ttl = chrono::Duration::seconds(ttl_secs);
    let rows = crate::services::billing::config::load_rates(&state.db).await?;
    let mut rows = rows;
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Json(
        rows.into_iter()
            .map(|r| BillingRateResponse {
                id: r.id,
                metric: r.lago_metric_code,
                model: r.model,
                credits_per_unit_micros: r.credits_per_unit_micros,
                synced_at: r.synced_at,
                retired_at: r.retired_at,
                stale: r.synced_at + ttl < chrono::Utc::now(),
            })
            .collect(),
    ))
}

#[derive(Debug, Serialize)]
pub struct BillingServiceResponse {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub has_billing: bool,
    pub effective_source: &'static str,
    pub effective_metric: String,
    pub allowance_metrics: Vec<String>,
    pub sync_status: Option<String>,
    pub sync_error: Option<String>,
    pub primary_source: &'static str,
    pub component_sources: Vec<String>,
}

pub async fn get_overview(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<BillingServiceResponse>>> {
    require_admin_or_operator(&state, &auth, "admin.billing.overview").await?;
    let rows = crate::services::billing::config::load_services(&state.db).await?;
    let global = crate::services::billing::config::load_settings(&state.db).await?;
    Ok(Json(
        rows.into_iter()
            .filter(|s| s.billing.is_some())
            .map(|s| {
                let effective = crate::services::billing::config::effective_config(
                    s.billing.as_ref(),
                    global.as_ref(),
                    None,
                    Some(state.config.billing_default_overdraft_cap_credits),
                );
                let source = match effective.lane_source {
                    crate::services::billing::config::BillingConfigSource::Env => "env",
                    crate::services::billing::config::BillingConfigSource::Global => "global",
                    crate::services::billing::config::BillingConfigSource::Service => "service",
                    crate::services::billing::config::BillingConfigSource::Legacy => "legacy",
                    crate::services::billing::config::BillingConfigSource::Free => "free",
                };
                BillingServiceResponse {
                    id: s.id,
                    slug: s.slug,
                    name: s.name,
                    has_billing: true,
                    effective_source: source,
                    effective_metric:
                        crate::services::billing::metric_resolution::effective_platform_metric(&s)
                            .slug()
                            .to_string(),
                    allowance_metrics:
                        crate::services::billing::metric_resolution::allowance_metrics(&s)
                            .into_iter()
                            .map(|m| m.slug().to_string())
                            .collect(),
                    sync_status: s
                        .billing
                        .as_ref()
                        .and_then(|b| b.byok_pricing.as_ref().or(b.platform_key_pricing.as_ref()))
                        .map(|p| format!("{:?}", p.sync_status).to_lowercase()),
                    sync_error: s
                        .billing
                        .as_ref()
                        .and_then(|b| b.byok_pricing.as_ref().or(b.platform_key_pricing.as_ref()))
                        .and_then(|p| p.sync_error.clone()),
                    primary_source: source,
                    component_sources: s
                        .billing
                        .as_ref()
                        .map(|b| {
                            b.byok_pricing
                                .as_ref()
                                .or(b.platform_key_pricing.as_ref())
                                .map(|p| {
                                    crate::services::billing::config::effective_component_sources(p)
                                        .into_iter()
                                        .map(|c| format!("{:?}", c.source).to_lowercase())
                                        .collect()
                                })
                                .unwrap_or_default()
                        })
                        .unwrap_or_default(),
                }
            })
            .collect(),
    ))
}

pub async fn resync_service(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<bool>> {
    require_admin(&state, &auth).await?;
    let service = crate::services::billing::config::load_service(&state.db, &id)
        .await?
        .ok_or(crate::errors::AppError::NotFound(
            "service not found".into(),
        ))?;
    let synced = state.billing.sync_service_price(&service).await?;
    crate::services::audit_service::log_actor_event(
        state.db.clone(),
        &crate::services::audit_service::AuditActor::from_auth_user(&auth),
        "billing_service_resynced",
        Some(serde_json::json!({"service_id": id, "synced": synced})),
    )
    .await?;
    Ok(Json(synced))
}

pub async fn update_service_billing(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: crate::telemetry::TelemetryContext,
    Path(service_id): Path<String>,
    Json(body): Json<crate::handlers::services::UpdateServiceRequest>,
) -> AppResult<Json<crate::handlers::services::ServiceResponse>> {
    require_admin(&state, &auth_user).await?;
    if body.billing.is_none()
        || body.name.is_some()
        || body.base_url.is_some()
        || body.is_active.is_some()
        || body.description.is_some()
        || body.visibility.is_some()
        || body.service_category.is_some()
        || body.offering_kind.is_some()
        || body.topics.is_some()
        || body.destination_targets.is_some()
        || body.credential.is_some()
    {
        return Err(crate::errors::AppError::ValidationError(
            "billing update must contain only billing".into(),
        ));
    }
    crate::handlers::services::update_service_inner(state, auth_user, tele, service_id, body).await
}
