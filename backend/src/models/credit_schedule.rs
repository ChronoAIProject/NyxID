use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::billing_target::{BillingServiceScope, BillingTargetKind};

pub const COLLECTION_NAME: &str = "credit_schedules";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleRecurrence {
    Daily,
    Weekly,
    Monthly,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CreditExpiryPolicy {
    EndOfPeriod,
    AfterDays { days: u16 },
    Never,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct SchedulePeriod {
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub start: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub end: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreditSchedule {
    #[serde(rename = "_id")]
    pub id: String,
    pub amount_credits: i64,
    pub amount_micros: i64,
    pub recurrence: ScheduleRecurrence,
    pub expiry: CreditExpiryPolicy,
    pub target_kind: BillingTargetKind,
    #[serde(default)]
    pub target_user_ids: Vec<String>,
    pub scope: BillingServiceScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub is_active: bool,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub last_period_start: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub last_disbursed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub skipped_periods: u64,
}
