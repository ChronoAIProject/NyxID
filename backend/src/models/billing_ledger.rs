use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::models::service_billing::BillingMetric;
use crate::models::usage_meter::BillingLayer;

pub const COLLECTION_NAME: &str = "billing_ledger";

/// The money-moving billing events recorded on the tamper-evident ledger.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BillingLedgerEventType {
    /// A usage charge was applied to a wallet (settlement first-apply).
    UsageSettled,
    /// A top-up checkout session was created for an owner.
    TopupCreated,
    /// Lago reported paid credits landing on a wallet.
    WalletCredited,
}

/// One append-only, hash-chained billing ledger entry.
///
/// Entries are chained like audit logs (`seq`/`prev_hash`/`entry_hash`
/// HMAC-SHA256): every entry commits to its predecessor, so a mutated,
/// deleted, or inserted row breaks verification at that seq. Rows are
/// never updated after insert.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct BillingLedgerEntry {
    #[serde(rename = "_id")]
    pub id: String,
    pub seq: i64,
    pub prev_hash: String,
    pub entry_hash: String,
    pub event_type: BillingLedgerEventType,
    pub owner_id: String,
    /// Usage meter row id, top-up session id, or Lago customer id,
    /// depending on `event_type`.
    pub reference_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<BillingLayer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<BillingMetric>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<i64>,
    /// Credits moved by this event: the charge for `usage_settled`, the
    /// requested amount for `topup_created`. Absent for balance snapshots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_credits: Option<i64>,
    /// Post-event wallet balance snapshot (`wallet_credited` only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance_credits: Option<i64>,
    /// At-most-once guard for externally delivered events (e.g. the Lago
    /// invoice id behind a `wallet_credited` webhook, which Lago retries).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl BillingLedgerEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UsageSettled => "usage_settled",
            Self::TopupCreated => "topup_created",
            Self::WalletCredited => "wallet_credited",
        }
    }
}
