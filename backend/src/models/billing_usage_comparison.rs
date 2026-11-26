use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "billing_usage_comparisons";
pub const CONTROL_COLLECTION: &str = "billing_usage_comparison_control";
pub const COVERAGE_ID: &str = "5b543627-0ed6-464e-b6a8-94e99c4d1773";
pub const SCHEDULE_ID: &str = "79de2690-892f-46ca-a714-e7567fef4d5e";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BillingUsageComparison {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub wallet_id: String,
    pub customer_id: String,
    pub subscription_id: String,
    pub acknowledgement_sequence: i64,
    pub coverage_verified: bool,
    #[serde(default)]
    pub coverage_unknown_reason: Option<String>,
    #[serde(default)]
    pub uncertainty_revision: i64,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub uncertain_through: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub warming_since: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub last_acknowledged_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub projection: Option<UsageProjection>,
    #[serde(default)]
    pub comparison: Option<UsageObservation>,
    #[serde(default)]
    pub provider_period: Option<UsagePeriod>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub next_visit_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub retired_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub provider_errors: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageProjection {
    pub start_second: i64,
    pub end_second: i64,
    pub layout: Vec<String>,
    pub generation: String,
    pub uncertainty_revision: i64,
    pub sequence: i64,
    pub rebuild_upper: Option<i64>,
    pub cursor_second: i64,
    pub cursor_sequence: i64,
    pub totals: BTreeMap<String, i64>,
    #[serde(default)]
    pub unverified_origin: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageObservation {
    pub status: String,
    pub reason: String,
    pub generation: String,
    pub sequence: i64,
    pub uncertainty_revision: i64,
    pub start_second: i64,
    pub end_second: i64,
    pub local: BTreeMap<String, String>,
    pub remote: BTreeMap<String, String>,
    pub differences: BTreeMap<String, String>,
    pub fingerprint: String,
    pub first_pass_id: String,
    pub pass_id: String,
    pub stale_drift: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub first_observed_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UsageCoverage {
    pub generation: String,
    pub ready: bool,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub continuous_since: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsagePeriod {
    pub start_second: i64,
    pub end_second: i64,
    pub layout: Vec<String>,
}
