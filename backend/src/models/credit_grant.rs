use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::billing_target::{BillingServiceScope, BillingTargetKind};

pub const COLLECTION_NAME: &str = "credit_grants";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreditGrantStatus {
    Active,
    Consumed,
    Expired,
    Revoked,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreditGrantSettlementLock {
    pub operation_id: String,
    pub usage_row_id: String,
    pub reserved_micros: i64,
    pub consume_micros: i64,
    pub applied: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreditGrantScheduleOrigin {
    pub schedule_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub period_start: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreditGrant {
    #[serde(rename = "_id")]
    pub id: String,
    /// Groups the per-recipient rows created by one admin request.
    pub batch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_origin: Option<CreditGrantScheduleOrigin>,
    pub recipient_user_id: String,
    pub target_kind: BillingTargetKind,
    pub amount_credits: i64,
    /// Exact internal accounting. One credit is 1,000,000 microcredits.
    pub amount_micros: i64,
    pub remaining_micros: i64,
    #[serde(default)]
    pub reserved_micros: i64,
    pub scope: BillingServiceScope,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub granted_by: String,
    pub status: CreditGrantStatus,
    /// Set only after the corresponding `grant_issued` ledger entry is
    /// durable. Pending grants are not spendable until reconciliation closes
    /// this crash window.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub issued_ledgered_at: Option<DateTime<Utc>>,
    /// Set after the terminal `grant_expired` or `grant_revoked` entry is
    /// durable. Consumed grants are journaled per usage settlement instead.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub terminal_ledgered_at: Option<DateTime<Utc>>,
    /// Unspent amount removed by expiry or revocation. Retained so recovery
    /// can reconstruct the exact terminal ledger movement after the spendable
    /// balance has been zeroed.
    #[serde(default)]
    pub terminal_amount_micros: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_settlement: Option<CreditGrantSettlementLock>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub consumed_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub expired_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub revoked_at: Option<DateTime<Utc>>,
}
