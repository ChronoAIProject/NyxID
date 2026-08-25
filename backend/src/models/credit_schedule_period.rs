use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::billing_target::{BillingServiceScope, BillingTargetKind};

pub const COLLECTION_NAME: &str = "credit_schedule_periods";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SchedulePeriodStatus {
    Disbursing,
    Complete,
}

/// Derived progress for a schedule walk. Losing this collection may repeat
/// recipient scans, but deterministic grant ids preserve money accuracy.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreditSchedulePeriod {
    #[serde(rename = "_id")]
    pub id: String,
    pub schedule_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub period_start: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub period_end: DateTime<Utc>,
    pub status: SchedulePeriodStatus,
    pub amount_micros: i64,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
    pub target_kind: BillingTargetKind,
    #[serde(default)]
    pub target_user_ids: Vec<String>,
    pub scope: BillingServiceScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_user_id: Option<String>,
    #[serde(default)]
    pub disbursed_count: u64,
    /// An efficiency lease only. The unique deterministic grant id is the
    /// authority for whether a recipient was paid.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
}
