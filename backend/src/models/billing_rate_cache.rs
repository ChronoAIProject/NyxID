use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const COLLECTION_NAME: &str = "billing_rate_cache";

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct BillingRateCache {
    #[serde(rename = "_id")]
    pub id: String,
    pub lago_metric_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub credits_per_unit_micros: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits_per_unit_pico: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub synced_at: DateTime<Utc>,
}

impl BillingRateCache {
    pub fn cache_id(lago_metric_code: &str, model: Option<&str>) -> String {
        format!("{}:{}", lago_metric_code, model.unwrap_or("*"))
    }
}
