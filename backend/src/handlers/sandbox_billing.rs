//! Trusted producer API for existing service prices and finite prepaid windows.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::sandbox_billing::{
        SandboxAllocation, SandboxBillingQuote, SandboxBillingWindow, SandboxUsageQuantity,
    },
    mw::auth::{AuthMethod, AuthUser},
    services::billing::sandbox_runtime::{
        self, ProducerAuthority, ReserveSandboxWindow, SandboxEvidence, SandboxIntervalEvidence,
    },
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/quotes", post(quote))
        .route("/windows", post(reserve))
        .route("/receipts/{request_id}", get(receipt))
        .route("/windows/{window_id}/evidence", post(evidence))
        .layer(DefaultBodyLimit::max(512 * 1024))
}

async fn authorize(state: &AppState, auth: &AuthUser) -> AppResult<ProducerAuthority> {
    if auth.auth_method != AuthMethod::ServiceAccount {
        return Err(AppError::Forbidden(
            "Sandbox billing requires a service-account token".into(),
        ));
    }
    sandbox_runtime::authorize_producer(
        &state.db,
        state.config.cma_billing_service_account_id.as_deref(),
        &auth.user_id.to_string(),
        &auth.scope,
        auth.token_jti.as_deref().unwrap_or_default(),
    )
    .await
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Allocation {
    cpu_millis: i64,
    memory_mib: i64,
    disk_gib: i64,
}
impl From<Allocation> for SandboxAllocation {
    fn from(value: Allocation) -> Self {
        Self {
            cpu_millis: value.cpu_millis,
            memory_mib: value.memory_mib,
            disk_gib: value.disk_gib,
        }
    }
}
impl From<SandboxAllocation> for Allocation {
    fn from(value: SandboxAllocation) -> Self {
        Self {
            cpu_millis: value.cpu_millis,
            memory_mib: value.memory_mib,
            disk_gib: value.disk_gib,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteRequest {
    owner_id: String,
    sandbox_id: String,
    allocation: Allocation,
}

#[derive(Serialize)]
pub struct PriceResponse {
    metric: String,
    credits_per_unit: String,
}
#[derive(Serialize)]
pub struct QuoteResponse {
    quote_id: String,
    owner_id: String,
    sandbox_id: String,
    service_id: String,
    maximum_allocation: Allocation,
    prices: Vec<PriceResponse>,
    accept_before: String,
}
impl From<SandboxBillingQuote> for QuoteResponse {
    fn from(quote: SandboxBillingQuote) -> Self {
        Self {
            quote_id: quote.id,
            owner_id: quote.owner_id,
            sandbox_id: quote.sandbox_id,
            service_id: quote.service_id,
            maximum_allocation: quote.maximum_allocation.into(),
            accept_before: quote.accept_before.to_rfc3339(),
            prices: quote
                .prices
                .into_iter()
                .map(|price| PriceResponse {
                    metric: price.metric.as_str().into(),
                    credits_per_unit: price.credits_per_unit.to_string(),
                })
                .collect(),
        }
    }
}
async fn quote(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<QuoteRequest>,
) -> AppResult<Json<QuoteResponse>> {
    let producer = authorize(&state, &auth).await?;
    Ok(Json(
        state
            .billing
            .quote_sandbox(
                &producer,
                &input.owner_id,
                &input.sandbox_id,
                input.allocation.into(),
            )
            .await?
            .into(),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowRequest {
    request_id: String,
    quote_id: String,
    owner_id: String,
    sandbox_id: String,
    runtime_generation: i64,
    operation_id: String,
    previous_window_id: Option<String>,
    seconds: i64,
    allocation: Allocation,
}
#[derive(Serialize)]
pub struct ComponentResponse {
    usage_row_id: String,
    metric: String,
    reserved_quantity: i64,
    finalized_quantity: Option<i64>,
}
#[derive(Serialize)]
pub struct WindowResponse {
    window_id: String,
    request_id: String,
    quote_id: String,
    owner_id: String,
    sandbox_id: String,
    runtime_generation: i64,
    covered_from: i64,
    covered_until: i64,
    allocation: Allocation,
    components: Vec<ComponentResponse>,
    evidence_accepted: bool,
}
impl From<SandboxBillingWindow> for WindowResponse {
    fn from(window: SandboxBillingWindow) -> Self {
        Self {
            window_id: window.id,
            request_id: window.request_id,
            quote_id: window.quote_id,
            owner_id: window.owner_id,
            sandbox_id: window.sandbox_id,
            runtime_generation: window.runtime_generation,
            covered_from: window.covered_from,
            covered_until: window.covered_until,
            allocation: window.allocation.into(),
            evidence_accepted: window.evidence_digest.is_some(),
            components: window
                .components
                .into_iter()
                .map(|component| ComponentResponse {
                    usage_row_id: component.usage_row_id,
                    metric: component.metric.as_str().into(),
                    reserved_quantity: component.reserved_quantity,
                    finalized_quantity: component.finalized_quantity,
                })
                .collect(),
        }
    }
}
async fn reserve(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<WindowRequest>,
) -> AppResult<Json<WindowResponse>> {
    let producer = authorize(&state, &auth).await?;
    Ok(Json(
        state
            .billing
            .reserve_sandbox_window(
                &producer,
                ReserveSandboxWindow {
                    request_id: input.request_id,
                    quote_id: input.quote_id,
                    owner_id: input.owner_id,
                    sandbox_id: input.sandbox_id,
                    runtime_generation: input.runtime_generation,
                    operation_id: input.operation_id,
                    previous_window_id: input.previous_window_id,
                    seconds: input.seconds,
                    allocation: input.allocation.into(),
                },
            )
            .await?
            .into(),
    ))
}
async fn receipt(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(request_id): Path<String>,
) -> AppResult<Json<WindowResponse>> {
    let producer = authorize(&state, &auth).await?;
    let window = state
        .billing
        .sandbox_window_receipt(&producer, &request_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Sandbox funding receipt not found".into()))?;
    Ok(Json(window.into()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageQuantity {
    cpu_millis_seconds: i64,
    memory_mib_seconds: i64,
    disk_gib_seconds: i64,
}
impl From<UsageQuantity> for SandboxUsageQuantity {
    fn from(value: UsageQuantity) -> Self {
        Self {
            cpu_millis_seconds: value.cpu_millis_seconds,
            memory_mib_seconds: value.memory_mib_seconds,
            disk_gib_seconds: value.disk_gib_seconds,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntervalRequest {
    revision: i64,
    started_at: i64,
    ended_at: i64,
    allocation: Allocation,
    quantity: UsageQuantity,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRequest {
    owner_id: String,
    sandbox_id: String,
    runtime_generation: i64,
    intervals: Vec<IntervalRequest>,
}
async fn evidence(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(window_id): Path<String>,
    Json(input): Json<EvidenceRequest>,
) -> AppResult<Json<WindowResponse>> {
    let producer = authorize(&state, &auth).await?;
    let intervals = input
        .intervals
        .into_iter()
        .map(|interval| SandboxIntervalEvidence {
            revision: interval.revision,
            started_at: interval.started_at,
            ended_at: interval.ended_at,
            allocation: interval.allocation.into(),
            quantity: interval.quantity.into(),
        })
        .collect();
    Ok(Json(
        state
            .billing
            .finalize_sandbox_window(
                &producer,
                SandboxEvidence {
                    window_id,
                    owner_id: input.owner_id,
                    sandbox_id: input.sandbox_id,
                    runtime_generation: input.runtime_generation,
                    intervals,
                },
            )
            .await?
            .into(),
    ))
}
