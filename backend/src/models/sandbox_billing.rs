//! CMA lifecycle receipts. Prices remain authored on the existing catalog service.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{credits::Credits, service_billing::BillingMetric};

pub const QUOTE_COLLECTION: &str = "sandbox_billing_quotes";
pub const WINDOW_COLLECTION: &str = "sandbox_billing_windows";
pub const CURSOR_COLLECTION: &str = "sandbox_billing_cursors";
pub const EVIDENCE_COLLECTION: &str = "sandbox_billing_evidence";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SandboxAllocation {
    pub cpu_millis: i64,
    pub memory_mib: i64,
    pub disk_gib: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SandboxUsageQuantity {
    pub cpu_millis_seconds: i64,
    pub memory_mib_seconds: i64,
    pub disk_gib_seconds: i64,
}

/// Accepted evidence of an ordinary service price; never another price authority.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SandboxPriceSnapshot {
    pub metric: BillingMetric,
    pub lago_metric_code: String,
    #[serde(with = "super::credits::whole")]
    pub credits_per_unit: Credits,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SandboxBillingQuote {
    #[serde(rename = "_id")]
    pub id: String,
    pub producer_id: String,
    pub owner_id: String,
    pub sandbox_id: String,
    pub service_id: String,
    pub service_slug: String,
    pub wallet_id: String,
    pub lago_customer_id: String,
    pub lago_subscription_id: String,
    pub plan_code: String,
    pub maximum_allocation: SandboxAllocation,
    pub prices: Vec<SandboxPriceSnapshot>,
    pub accepted_operation_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub accept_before: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub accepted_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SandboxWindowComponent {
    pub usage_row_id: String,
    pub metric: BillingMetric,
    pub reserved_quantity: i64,
    pub finalized_quantity: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SandboxBillingWindow {
    #[serde(rename = "_id")]
    pub id: String,
    pub producer_id: String,
    pub request_id: String,
    pub request_digest: String,
    pub quote_id: String,
    pub owner_id: String,
    pub sandbox_id: String,
    pub runtime_generation: i64,
    /// Whole UTC seconds, matching CMA's retained allocation ledger.
    pub covered_from: i64,
    pub covered_until: i64,
    pub allocation: SandboxAllocation,
    pub components: Vec<SandboxWindowComponent>,
    pub evidence_digest: Option<String>,
    pub first_usage_revision: Option<i64>,
    pub last_usage_revision: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub finalized_at: Option<DateTime<Utc>>,
}

/// Serializes finite reservations for one producer/Sandbox across API replicas.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SandboxBillingCursor {
    #[serde(rename = "_id")]
    pub id: String,
    pub producer_id: String,
    pub sandbox_id: String,
    pub owner_id: String,
    pub quote_id: String,
    pub runtime_generation: i64,
    pub covered_until: i64,
    pub last_window_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// Deduplicates retained producer ledger revisions across adjacent windows.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SandboxLedgerEvidence {
    #[serde(rename = "_id")]
    pub id: String,
    pub producer_id: String,
    pub owner_id: String,
    pub sandbox_id: String,
    pub revision: i64,
    pub payload_digest: String,
    pub started_at: i64,
    pub ended_at: i64,
    pub allocation: SandboxAllocation,
    pub quantity: SandboxUsageQuantity,
}
