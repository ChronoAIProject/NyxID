//! Runtime CMA rate configuration on the existing admin billing surface.
use super::admin_helpers::{require_admin, require_admin_or_operator};
use crate::{
    AppState,
    errors::AppResult,
    models::{sandbox_rate_card::SandboxRateCard, service_billing::PricingSyncStatus},
    mw::auth::AuthUser,
    services::{audit_service::AuditActor, billing::sandbox_rates},
};
use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize)]
pub struct RateCardResponse {
    id: String,
    version: i64,
    effective_from: DateTime<Utc>,
    rates: BTreeMap<String, String>,
    sync_status: PricingSyncStatus,
    created_at: DateTime<Utc>,
    created_by: Option<String>,
}
impl From<SandboxRateCard> for RateCardResponse {
    fn from(card: SandboxRateCard) -> Self {
        Self {
            id: card.id,
            version: card.version,
            effective_from: card.effective_from,
            rates: card
                .prices
                .into_iter()
                .map(|p| (p.metric.as_str().into(), p.credits_per_unit.to_string()))
                .collect(),
            sync_status: card.sync_status,
            created_at: card.created_at,
            created_by: card.created_by,
        }
    }
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<RateCardResponse>>> {
    require_admin_or_operator(&state, &auth, "admin.billing.cma_rates").await?;
    Ok(Json(
        sandbox_rates::list(&state.db)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishRequest {
    expected_version: i64,
    effective_from: DateTime<Utc>,
    rates: BTreeMap<String, String>,
}

pub async fn publish(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<PublishRequest>,
) -> AppResult<Json<RateCardResponse>> {
    require_admin(&state, &auth).await?;
    let card = sandbox_rates::publish(
        &state.db,
        &state.config.lago_plan_code,
        sandbox_rates::PublishRateCard {
            expected_version: input.expected_version,
            effective_from: input.effective_from,
            rates: input.rates,
        },
        &AuditActor::from_auth_user(&auth),
        state.audit_chain_hmac_key.as_slice(),
    )
    .await?;
    // The durable pending card is retried by the existing reconciler. Publishing
    // cannot claim failure after the configuration and chained audit commit.
    Ok(Json(card.into()))
}
