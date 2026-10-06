//! Permanent operational summaries. No TTL: raw meter retention is independent.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "usage_rollup_hourly";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageRollupHourly {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub hour: DateTime<Utc>,
    pub actor: String,
    pub owner: String,
    pub service_id: Option<String>,
    pub service_slug: Option<String>,
    pub class: String,
    pub metric: String,
    pub code: String,
    pub layer: String,
    pub model: Option<String>,
    pub billable: bool,
    pub exact: bool,
    #[serde(flatten)]
    pub measures: UsageRollupMeasures,
    /// Preserve historical display grouping and unknown-rate
    /// masking without adding API keys/ack state to the hourly primary key.
    pub cost_partitions: std::collections::BTreeMap<String, UsageCostPartition>,
    /// Derived query accelerators, committed with the exact measures. The
    /// decimal mirror avoids per-document conversions in Mongo's group stage;
    /// the single key permits grouping before any partition expansion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub single_display_key: Option<bson::Document>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_costs: Option<bson::Document>,
    /// Missing on older hourly-only summaries; the daily bootstrap discovers
    /// them through its additive pending index. New folds set this to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_pending: Option<bool>,
    /// Globally ordered batches make a single monotonic fence sufficient;
    /// no unbounded or unsafely evicted applied-batch-id array is needed.
    pub last_batch: i64,
}

crate::exact_credit_model! {
    [
        ("legacy_grant_cost", "legacy_grant"),
        ("gross_cost", "gross_cost_micros"),
        ("wallet_cost", "wallet_cost_micros"),
        ("grant_cost", "grant_cost_micros"),
        ("allowance_cost", "allowance_cost_micros"),
    ]
    #[derive(Clone, Debug, Serialize)]
    pub struct UsageRollupMeasures {
        pub quantity: i64,
        pub events: i64,
        pub requests: i64,
        pub exact_cost_events: i64,
        pub legacy_cost_events: i64,
        pub legacy_quantity: i64,
        pub legacy_allowance_quantity: i64,
        #[serde(with = "crate::models::credits::whole")]
        pub legacy_grant_cost: crate::models::credits::Credits,
        #[serde(with = "crate::models::credits::whole")]
        pub gross_cost: crate::models::credits::Credits,
        #[serde(with = "crate::models::credits::whole")]
        pub wallet_cost: crate::models::credits::Credits,
        #[serde(with = "crate::models::credits::whole")]
        pub grant_cost: crate::models::credits::Credits,
        #[serde(with = "crate::models::credits::whole")]
        pub allowance_cost: crate::models::credits::Credits,
        pub prompt_tokens: i64,
        pub completion_tokens: i64,
        pub cached_tokens: i64,
        pub cache_creation_tokens: i64,
        /// Absent on summaries folded before audio metering.
        #[serde(default)]
        pub audio_input_tokens: i64,
        #[serde(default)]
        pub audio_output_tokens: i64,
        pub rows_folded: i64,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageCostPartition {
    pub key: bson::Document,
    #[serde(flatten)]
    pub measures: UsageRollupMeasures,
}
