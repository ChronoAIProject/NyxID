use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "billing_usage_deliveries";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Prepared,
    Acknowledged,
    Rejected,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeliveryProperties {
    pub quantity: serde_json::Value,
    pub model: Option<String>,
    pub service_code: Option<String>,
    pub layer: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BillingUsageDelivery {
    #[serde(rename = "_id")]
    pub id: String,
    pub transaction_id: String,
    pub owner_id: String,
    pub wallet_id: String,
    pub customer_id: String,
    pub subscription_id: Option<String>,
    pub external_customer_id: Option<String>,
    pub code: String,
    pub event_second: i64,
    pub properties: DeliveryProperties,
    #[serde(default)]
    pub state: DeliveryState,
    pub acknowledgement_sequence: Option<i64>,
    #[serde(default)]
    pub duplicate_acknowledgement: bool,
    #[serde(default)]
    pub rejection_count: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub prepared_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub usage_created_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub acknowledged_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub resolved_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub last_checked_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub last_rejected_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub reconciliation_issue: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
}
