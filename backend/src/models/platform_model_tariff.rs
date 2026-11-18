//! Immutable economic revisions. Only lifecycle/sync metadata may change.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::service_billing::BillingMetric;

pub const COLLECTION_NAME: &str = "platform_model_tariffs";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TariffLane {
    Byok,
    PlatformKey,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelPriceBinding {
    pub model_id: String,
    /// Derived unit metadata for allowance authoring; never chooses a price.
    #[serde(default)]
    pub allowance_metrics: Vec<BillingMetric>,
    pub variant: BTreeMap<String, String>,
    pub active_tariff_id: Option<String>,
    pub pending_tariff_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CacheTtl {
    FiveMinutes,
    OneHour,
    ThirtyMinutes,
    TwelveHours,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Image,
    Audio,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Selector {
    Input,
    Output,
    Reasoning,
    CacheRead,
    CacheWrite { ttl: CacheTtl },
    ImageInput,
    ImageOutput,
    AudioInput,
    AudioOutput,
    CachedModality { modality: Modality },
    Images,
    Requests,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageFamily {
    Openai,
    Claude,
    Gemini,
    OpenMaas,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UsageContract {
    /// Adapter contract + fixture digest certify the native field mapping.
    pub version: String,
    pub fixture_sha256: String,
    pub family: UsageFamily,
    pub tokens: bool,
    pub cache_read: bool,
    pub cache_write_ttls: Vec<CacheTtl>,
    pub split_reasoning: bool,
    pub input_modalities: Vec<Modality>,
    pub output_modalities: Vec<Modality>,
    /// Gemini tool input is either explicitly billed or explicitly excluded.
    pub bill_tool_input: bool,
    pub images: bool,
    pub requests: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TariffComponent {
    pub key: String,
    pub metric: BillingMetric,
    pub selector: Selector,
    /// Exact Credits/base unit. Zero must also be explicitly marked free.
    pub credits_per_unit: String,
    pub explicit_free: bool,
    /// Conservative verified bound per request, including candidates/tool loops.
    pub max_quantity: i64,
    #[serde(default)]
    pub lago_metric_code: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequestBounds {
    pub max_input_tokens: i64,
    pub max_output_tokens: i64,
    pub max_candidates: i64,
    pub max_tool_iterations: i64,
    pub max_images: i64,
    pub max_request_bytes: i64,
    pub max_response_bytes: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TariffStatus {
    Pending,
    Synced,
    Retired,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelTariff {
    #[serde(rename = "_id")]
    pub id: String,
    pub revision: i64,
    pub service_id: String,
    pub model_id: String,
    pub public_model: String,
    pub lane: TariffLane,
    pub variant: BTreeMap<String, String>,
    pub contract: UsageContract,
    pub bounds: RequestBounds,
    pub components: Vec<TariffComponent>,
    pub source: String,
    pub status: TariffStatus,
    pub sync_error: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// Persisted on the coordinator before any component settlement. Map keys are
/// immutable revision-qualified Lago codes, not units (TTLs can share a unit).
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ModelChargeVector {
    pub native_usage: NativeUsageEvidence,
    pub tariff_id: String,
    pub public_model: String,
    pub quantities: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ModelMeterBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Box<ModelChargeVector>>,
    #[serde(default)]
    pub evidence_digest: Option<String>,
    pub tariff_id: String,
    pub public_model: String,
    pub max_quantity: i64,
}

/// Missing, known zero and inapplicable remain distinct. JSON numbers must be
/// exact nonnegative integers; serde rejects floats/strings/overflow. Adapters
/// supply cumulative final evidence, never deltas or estimated token counts.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NativeUsageEvidence {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub total: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    #[serde(default)]
    pub writes: BTreeMap<CacheTtl, i64>,
    pub reasoning: Option<i64>,
    pub tool_input: Option<i64>,
    pub input_image: Option<i64>,
    pub input_audio: Option<i64>,
    pub output_image: Option<i64>,
    pub output_audio: Option<i64>,
    pub cached_image: Option<i64>,
    pub cached_audio: Option<i64>,
    pub images: Option<i64>,
    pub requests: Option<i64>,
}
