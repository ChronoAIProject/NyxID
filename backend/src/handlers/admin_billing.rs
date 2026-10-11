use crate::models::{
    billing_rate_cache::{BillingRateCache, COLLECTION_NAME as RATES},
    downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
    platform_settings::{
        BillingSettings, COLLECTION_NAME as SETTINGS, PLATFORM_SETTINGS_ID, PlatformSettings,
    },
};
use crate::{
    AppState, errors::AppResult, handlers::admin_helpers::require_admin, mw::auth::AuthUser,
};
use axum::{
    Json,
    extract::{Path, State},
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct BillingConfigResponse {
    pub billing_enabled: bool,
    pub lago_configured: bool,
    pub resale_enabled: bool,
    pub global: Option<BillingSettings>,
    pub precedence: [&'static str; 4],
}

pub async fn get_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<BillingConfigResponse>> {
    require_admin(&state, &auth).await?;
    let global = state
        .db
        .collection::<PlatformSettings>(SETTINGS)
        .find_one(doc! {"_id": PLATFORM_SETTINGS_ID})
        .await?
        .and_then(|s| s.billing);
    Ok(Json(BillingConfigResponse {
        billing_enabled: state.billing.billing_enabled(),
        lago_configured: state.billing.lago_configured(),
        resale_enabled: state.billing.resale_enabled(),
        global,
        precedence: ["env", "global", "service", "legacy/free"],
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
    if let Some(b) = &input.billing {
        if b.default_overdraft_cap_credits.is_some_and(|v| v < 0) {
            return Err(crate::errors::AppError::ValidationError(
                "default overdraft cap must be non-negative".into(),
            ));
        }
    }
    state.db.collection::<PlatformSettings>(SETTINGS).update_one(doc! {"_id": PLATFORM_SETTINGS_ID}, doc! {"$set": {"billing": mongodb::bson::to_bson(&input.billing)?}, "$setOnInsert": {"_id": PLATFORM_SETTINGS_ID}}).upsert(true).await?;
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
    require_admin(&state, &auth).await?;
    let ttl = chrono::Duration::seconds(state.config.billing_rate_cache_ttl_secs);
    let rows = state
        .db
        .collection::<BillingRateCache>(RATES)
        .find(doc! {})
        .await?
        .try_collect::<Vec<_>>()
        .await?;
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
}

pub async fn get_overview(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<BillingServiceResponse>>> {
    require_admin(&state, &auth).await?;
    let rows = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find(doc! {})
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    Ok(Json(
        rows.into_iter()
            .filter(|s| s.billing.is_some())
            .map(|s| BillingServiceResponse {
                id: s.id,
                slug: s.slug,
                name: s.name,
                has_billing: true,
                effective_source: "service",
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
    let service = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id": &id})
        .await?
        .ok_or(crate::errors::AppError::NotFound(
            "service not found".into(),
        ))?;
    Ok(Json(state.billing.sync_service_price(&service).await?))
}
