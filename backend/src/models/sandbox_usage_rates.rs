//! Separate v1 shadow-pricing records. Legacy funded rate cards stay unchanged.
use super::credits::Credits;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "sandbox_usage_rates_v1";
pub const POLICIES: &str = "sandbox_usage_policies_v1";
pub const PINS: &str = "sandbox_usage_price_pins_v1";
pub const SCHEMA: &str = "sandbox_usage_pricing_schema_v1";
pub const BUCKET_REVISIONS: &str = "sandbox_usage_shadow_bucket_revisions_v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowFactRef {
    pub stream_id: String,
    pub fact_id: String,
    pub event_id: String,
    pub event_digest: String,
    pub evidence_revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowStreamPosition {
    pub basis: String,
    pub stream_id: String,
    pub validated_sequence: i64,
    pub validated_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowBucketMetric {
    pub metric_code: String,
    pub numerator_pico: String,
    pub divisor: String,
    #[serde(with = "super::credits::whole")]
    pub rounded_candidate: Credits,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowCompletenessRef {
    pub basis: String,
    pub statement_id: String,
    pub statement_digest: String,
    pub validation_status: String,
    pub validation_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowBucketInput {
    pub price_pin: Option<PricePin>,
    pub facts: Vec<ShadowFactRef>,
    #[serde(default)]
    pub completeness: Vec<ShadowCompletenessRef>,
    pub provisional_metrics: Vec<ShadowBucketMetric>,
    pub unresolved: Vec<String>,
    pub rounding: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowBucketRevision {
    #[serde(rename = "_id")]
    pub id: String,
    pub bucket_id: String,
    pub schema_version: u32,
    pub revision: i64,
    pub previous_revision_id: Option<String>,
    pub namespace: String,
    pub sandbox_id: String,
    pub source_epoch: String,
    pub owner_subject: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub minute: DateTime<Utc>,
    pub charge_mode: ChargeMode,
    pub positions: Vec<ShadowStreamPosition>,
    pub input: ShadowBucketInput,
    pub input_digest: String,
    #[serde(with = "super::credits::whole")]
    pub provisional_total: Credits,
    #[serde(with = "super::credits::whole")]
    pub provisional_delta: Credits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeUnit {
    CoreHour,
    GibHour,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageRate {
    pub metric_code: String,
    pub definition_version: String,
    pub charge_unit: ChargeUnit,
    #[serde(with = "super::credits::whole")]
    pub credits_per_unit: Credits,
    /// N3 never claims provider qualification or synchronizes a provider rate.
    pub provider_proof: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageTariff {
    #[serde(rename = "_id")]
    pub id: String,
    pub schema_version: u32,
    pub producer_namespace: String,
    pub version: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub effective_from: DateTime<Utc>,
    pub rates: Vec<UsageRate>,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageScope {
    pub producer_namespace: String,
    pub owner_id: String,
    pub sandbox_id: String,
    pub source_epoch: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Selection {
    Shadow,
    Free,
    Excluded,
    Allocated,
    Measured,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DimensionPolicy {
    pub cpu: Selection,
    pub memory: Selection,
    pub disk: Selection,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyCoverage {
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub covered_until: DateTime<Utc>,
    /// Reference for later cutover verification; this is not charge authorization.
    pub manifest_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsagePolicy {
    #[serde(rename = "_id")]
    pub id: String,
    pub schema_version: u32,
    pub scope_key: String,
    pub scope: UsageScope,
    pub version: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub effective_from: DateTime<Utc>,
    pub dimensions: DimensionPolicy,
    pub legacy_coverage: Option<LegacyCoverage>,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeMode {
    Shadow,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PricePin {
    #[serde(rename = "_id")]
    pub id: String,
    pub schema_version: u32,
    pub scope_key: String,
    pub scope: UsageScope,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub minute: DateTime<Utc>,
    pub charge_mode: ChargeMode,
    pub policy_id: Option<String>,
    pub policy_version: Option<i64>,
    pub dimensions: DimensionPolicy,
    pub legacy_coverage: Option<LegacyCoverage>,
    pub tariff_id: Option<String>,
    pub tariff_version: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
