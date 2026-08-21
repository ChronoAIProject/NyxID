use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "usage_allowance_periods";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AllowanceSettlementLock {
    pub operation_id: String,
    pub usage_row_id: String,
    pub reserved_quantity: i64,
    pub consume_quantity: i64,
    pub applied: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageAllowancePeriod {
    #[serde(rename = "_id")]
    pub id: String,
    pub allowance_id: String,
    pub owner_user_id: String,
    pub total_quantity: i64,
    #[serde(default)]
    pub consumed_quantity: i64,
    #[serde(default)]
    pub reserved_quantity: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub period_start: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub period_end: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_settlement: Option<AllowanceSettlementLock>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
