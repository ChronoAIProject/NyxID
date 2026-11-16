use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "billing_usage_comparisons";

/// Capture metadata only. No projection or numeric comparison is stored yet.
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
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub warming_since: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub last_acknowledged_at: Option<DateTime<Utc>>,
}
