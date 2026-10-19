//! Permanent operational summaries. No TTL: raw meter retention is independent.
use super::usage_rollup_hourly::{UsageCostPartition, UsageRollupMeasures};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "usage_rollup_daily";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageRollupDaily {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub day: DateTime<Utc>,
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
    /// Preserve historical per-display-group truncation and unknown-rate
    /// masking without adding API keys/ack state to the daily primary key.
    pub cost_partitions: std::collections::BTreeMap<String, UsageCostPartition>,
    /// Derived query accelerators, committed with the integer measures.
    /// The single key permits grouping before any partition expansion;
    /// query_costs retains the hourly tier's compatibility representation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub single_display_key: Option<bson::Document>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_costs: Option<bson::Document>,
    /// Globally ordered batches make a single monotonic fence sufficient;
    /// no unbounded or unsafely evicted applied-batch-id array is needed.
    pub last_batch: i64,
}
