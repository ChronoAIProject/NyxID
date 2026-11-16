//! Recovery metadata for replace-all provider plan writes. Existing price rows
//! remain the source of desired economics; removal intents survive row cleanup.
use serde::{Deserialize, Serialize};

pub const PLAN_COLLECTION: &str = "billing_plan_sync";
pub const CHARGE_COLLECTION: &str = "billing_plan_charges";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanCharge {
    #[serde(rename = "_id")]
    pub id: String,
    pub plan_code: String,
    pub metric_code: String,
    /// Mutable catalog prices disappear with their authored field; immutable
    /// runtime/model versions retain historical charges.
    #[serde(default)]
    pub service_id: Option<String>,
    #[serde(default)]
    pub writes_rate_cache: bool,
    pub metric_name: String,
    pub metric_description: String,
    /// None is a durable removal, including when no cache row ever existed.
    pub credits_per_unit: Option<String>,
    #[serde(default)]
    pub repair_attempts: i64,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub repair_next_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub repair_failed_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub repair_reason: Option<String>,
}
