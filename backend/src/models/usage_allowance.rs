use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::billing_target::BillingTargetKind;
use super::service_billing::BillingMetric;

pub const COLLECTION_NAME: &str = "usage_allowances";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AllowanceRecurrence {
    OneTime,
    Daily,
    Weekly,
    Monthly,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageAllowance {
    #[serde(rename = "_id")]
    pub id: String,
    pub service_id: String,
    pub service_slug: String,
    pub metric: BillingMetric,
    pub quantity: i64,
    pub recurrence: AllowanceRecurrence,
    pub target_kind: BillingTargetKind,
    #[serde(default)]
    pub target_user_ids: Vec<String>,
    pub is_active: bool,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
