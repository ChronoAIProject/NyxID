//! One durable, immutable batch at a time, shared by all replicas.
use super::usage_rollup_hourly::UsageRollupHourly;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "usage_rollup_state";
pub const STATE_ID: &str = "hourly-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageRollupBatch {
    pub sequence: i64,
    pub row_ids: Vec<String>,
    pub increments: Vec<UsageRollupHourly>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub claimed_at: DateTime<Utc>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageRollupState {
    #[serde(rename = "_id")]
    pub id: String,
    pub sequence: i64,
    pub batch: Option<UsageRollupBatch>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub rolled_up_through: DateTime<Utc>,
}
