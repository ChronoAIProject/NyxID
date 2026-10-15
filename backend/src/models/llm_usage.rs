//! Additive measurements, independent of the quantities used by current prices.
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const COLLECTION_NAME: &str = "llm_usage_invocations";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmInvocation {
    #[serde(rename = "_id")]
    pub id: String,
    pub billing_request_id: String,
    pub actor_user_id: String,
    pub billing_owner_id: String,
    pub api_key_id: Option<String>,
    pub service_id: Option<String>,
    pub user_service_id: Option<String>,
    pub service_slug: Option<String>,
    pub credential_class: super::usage_meter::CredentialClass,
    pub context: LlmInvocationContext,
    pub observation: LlmUsageObservation,
    pub pool_attempt: Option<super::usage_meter::PoolAttemptAccounting>,
    pub charges: Vec<LlmInvocationCharge>,
    pub settlement_complete: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmInvocationCharge {
    pub transaction_id: String,
    pub layer: super::usage_meter::BillingLayer,
    pub metric: super::service_billing::BillingMetric,
    pub quantity: Option<i64>,
    pub settled: bool,
    pub billable: bool,
    #[serde(default, with = "crate::models::credits::whole::optional")]
    pub rate_credits: Option<super::credits::Credits>,
    #[serde(default, with = "crate::models::credits::whole::optional")]
    pub gross_credits: Option<super::credits::Credits>,
    #[serde(default, with = "crate::models::credits::whole::optional")]
    pub allowance_credits: Option<super::credits::Credits>,
    #[serde(default, with = "crate::models::credits::whole::optional")]
    pub grant_credits: Option<super::credits::Credits>,
    #[serde(default, with = "crate::models::credits::whole::optional")]
    pub wallet_credits: Option<super::credits::Credits>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageAdapter {
    Openai,
    Anthropic,
    Gemini,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    ProviderReported,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageCompleteness {
    Complete,
    Partial,
    #[default]
    #[serde(other)]
    Missing,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageOutcome {
    Succeeded,
    Interrupted,
    Failed,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageAggregationScope {
    #[default]
    Invocation,
    RealtimeConnection,
    #[serde(other)]
    Unknown,
}

/// Missing fields are unknown, including unknown cache/modality intersections.
/// Subsets must never be added to the four normalized classes as extra charges.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(default)]
pub struct LlmUsageObservation {
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

/// All assistant references originate in authenticated, server-loaded authority.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(default)]
pub struct LlmInvocationContext {
    pub conversation_id: Option<String>,
    pub turn_id: Option<String>,
    pub agent_id: Option<String>,
    pub requested_model: Option<String>,
    pub requested_tier: Option<String>,
}
