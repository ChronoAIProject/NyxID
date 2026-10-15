//! Authenticated retained LLM invocation reads alongside the existing usage API.
use crate::models::llm_usage::{
    UsageAdapter, UsageAggregationScope, UsageCompleteness, UsageOutcome, UsageSource,
};
use crate::services::billing::llm_invocations::{self, InvocationQuery};
use crate::{AppState, errors::AppResult, mw::auth::AuthUser};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, utoipa::ToSchema)]
pub struct ObservationResponse {
    pub version: u32,
    pub aggregation_scope: UsageAggregationScope,
    pub adapter: UsageAdapter,
    pub source: UsageSource,
    pub completeness: UsageCompleteness,
    pub outcome: UsageOutcome,
    pub invalid_counts: bool,
    pub reported_response_count: Option<i64>,
    pub unreported_response_count: Option<i64>,
    pub reported_input_tokens: Option<i64>,
    pub reported_output_tokens: Option<i64>,
    pub reported_total_tokens: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub tool_input_tokens: Option<i64>,
    pub cache_write_5m_tokens: Option<i64>,
    pub cache_write_1h_tokens: Option<i64>,
    pub audio_input_tokens: Option<i64>,
    pub audio_output_tokens: Option<i64>,
    pub image_input_tokens: Option<i64>,
    pub image_output_tokens: Option<i64>,
    pub text_input_tokens: Option<i64>,
    pub text_output_tokens: Option<i64>,
    pub cache_read_audio_tokens: Option<i64>,
    pub cache_read_image_tokens: Option<i64>,
    pub cache_read_text_tokens: Option<i64>,
    pub provider_response_id: Option<String>,
    pub resolved_model: Option<String>,
    pub processing_tier: Option<String>,
}
impl From<crate::models::llm_usage::LlmUsageObservation> for ObservationResponse {
    fn from(o: crate::models::llm_usage::LlmUsageObservation) -> Self {
        Self {
            version: o.version,
            aggregation_scope: o.aggregation_scope,
            adapter: o.adapter,
            source: o.source,
            completeness: o.completeness,
            outcome: o.outcome,
            invalid_counts: o.invalid_counts,
            reported_response_count: o.reported_response_count,
            unreported_response_count: o.unreported_response_count,
            reported_input_tokens: o.reported_input_tokens,
            reported_output_tokens: o.reported_output_tokens,
            reported_total_tokens: o.reported_total_tokens,
            input_tokens: o.input_tokens,
            output_tokens: o.output_tokens,
            cache_read_tokens: o.cache_read_tokens,
            cache_write_tokens: o.cache_write_tokens,
            reasoning_tokens: o.reasoning_tokens,
            tool_input_tokens: o.tool_input_tokens,
            cache_write_5m_tokens: o.cache_write_5m_tokens,
            cache_write_1h_tokens: o.cache_write_1h_tokens,
            audio_input_tokens: o.audio_input_tokens,
            audio_output_tokens: o.audio_output_tokens,
            image_input_tokens: o.image_input_tokens,
            image_output_tokens: o.image_output_tokens,
            text_input_tokens: o.text_input_tokens,
            text_output_tokens: o.text_output_tokens,
            cache_read_audio_tokens: o.cache_read_audio_tokens,
            cache_read_image_tokens: o.cache_read_image_tokens,
            cache_read_text_tokens: o.cache_read_text_tokens,
            provider_response_id: o.provider_response_id,
            resolved_model: o.resolved_model,
            processing_tier: o.processing_tier,
        }
    }
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct ChargeResponse {
    pub transaction_id: String,
    pub layer: crate::models::usage_meter::BillingLayer,
    pub metric: crate::models::service_billing::BillingMetric,
    pub quantity: Option<i64>,
    pub settled: bool,
    pub billable: bool,
    pub rate_credits: Option<crate::models::credits::Credits>,
    pub gross_credits: Option<crate::models::credits::Credits>,
    pub allowance_credits: Option<crate::models::credits::Credits>,
    pub grant_credits: Option<crate::models::credits::Credits>,
    pub wallet_credits: Option<crate::models::credits::Credits>,
}
impl From<crate::models::llm_usage::LlmInvocationCharge> for ChargeResponse {
    fn from(o: crate::models::llm_usage::LlmInvocationCharge) -> Self {
        Self {
            transaction_id: o.transaction_id,
            layer: o.layer,
            metric: o.metric,
            quantity: o.quantity,
            settled: o.settled,
            billable: o.billable,
            rate_credits: o.rate_credits,
            gross_credits: o.gross_credits,
            allowance_credits: o.allowance_credits,
            grant_credits: o.grant_credits,
            wallet_credits: o.wallet_credits,
        }
    }
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct InvocationResponse {
    pub id: String,
    pub billing_request_id: String,
    pub actor_user_id: String,
    pub billing_owner_id: String,
    pub api_key_id: Option<String>,
    pub service_id: Option<String>,
    pub user_service_id: Option<String>,
    pub service_slug: Option<String>,
    pub credential_class: crate::models::usage_meter::CredentialClass,
    pub conversation_id: Option<String>,
    pub turn_id: Option<String>,
    pub agent_id: Option<String>,
    pub requested_model: Option<String>,
    pub requested_tier: Option<String>,
    pub pool_id: Option<String>,
    pub attempt: Option<u32>,
    pub observation: ObservationResponse,
    pub charges: Vec<ChargeResponse>,
    pub settlement_complete: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}
impl From<crate::models::llm_usage::LlmInvocation> for InvocationResponse {
    fn from(r: crate::models::llm_usage::LlmInvocation) -> Self {
        Self {
            id: r.id,
            billing_request_id: r.billing_request_id,
            actor_user_id: r.actor_user_id,
            billing_owner_id: r.billing_owner_id,
            api_key_id: r.api_key_id,
            service_id: r.service_id,
            user_service_id: r.user_service_id,
            service_slug: r.service_slug,
            credential_class: r.credential_class,
            conversation_id: r.context.conversation_id,
            turn_id: r.context.turn_id,
            agent_id: r.context.agent_id,
            requested_model: r.context.requested_model,
            requested_tier: r.context.requested_tier,
            pool_id: r.pool_attempt.as_ref().map(|a| a.pool_id.clone()),
            attempt: r.pool_attempt.map(|a| a.attempt),
            observation: r.observation.into(),
            charges: r.charges.into_iter().map(Into::into).collect(),
            settlement_complete: r.settlement_complete,
            created_at: r.created_at,
            expires_at: r.expires_at,
        }
    }
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct InvocationListResponse {
    pub invocations: Vec<InvocationResponse>,
    pub next_cursor: Option<String>,
}
#[derive(Deserialize)]
pub struct DetailQuery {
    pub owner_id: Option<String>,
}

#[utoipa::path(get, path="/api/v1/billing/usage/invocations", params(InvocationQuery), responses((status=200,description="Retained usage invocations",body=InvocationListResponse)), tag="Billing",security(("bearer_auth"=[])))]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<InvocationQuery>,
) -> AppResult<Json<InvocationListResponse>> {
    let (rows, next_cursor) =
        llm_invocations::list(&state.db, &auth.user_id.to_string(), query).await?;
    Ok(Json(InvocationListResponse {
        invocations: rows.into_iter().map(Into::into).collect(),
        next_cursor,
    }))
}
#[utoipa::path(get, path="/api/v1/billing/usage/invocations/{id}", params(("id"=String,Path,description="Invocation identity"),("owner_id"=Option<String>,Query,description="Personal or organization billing owner")), responses((status=200,description="Retained invocation measurements and customer charges",body=InvocationResponse)), tag="Billing",security(("bearer_auth"=[])))]
pub async fn detail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<DetailQuery>,
) -> AppResult<Json<InvocationResponse>> {
    Ok(Json(
        llm_invocations::detail(
            &state.db,
            &auth.user_id.to_string(),
            query.owner_id.as_deref(),
            &id,
        )
        .await?
        .into(),
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn invocation_apis_are_in_openapi_with_dedicated_observation_schema() {
        use utoipa::OpenApi;
        let document = serde_json::to_value(crate::api_docs::ApiDoc::openapi()).unwrap();
        for path in [
            "/api/v1/billing/usage/invocations",
            "/api/v1/billing/usage/invocations/{id}",
        ] {
            assert!(document["paths"][path]["get"]["security"].is_array());
        }
        let fields = &document["components"]["schemas"]["ObservationResponse"]["properties"];
        assert!(fields.get("input_tokens").is_some());
        assert!(fields.get("aggregation_scope").is_some());
        assert!(fields.get("processing_tier").is_some());
    }
}
