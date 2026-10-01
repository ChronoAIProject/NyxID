use crate::models::credits::Credits;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::models::service_billing::BillingMetric;

pub const COLLECTION_NAME: &str = "usage_meter";

pub const POOL_RECOVERY_COLLECTION_NAME: &str = "billing_pool_recovery";

/// One bounded scan cursor, independent of request lifetimes and process restarts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolRecoveryCursor {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub lease_until: Option<DateTime<Utc>>,
    pub row_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BillingLayer {
    Platform,
    Resale,
}

impl BillingLayer {
    pub fn as_transaction_suffix(self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::Resale => "resale",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageStatus {
    Reserved,
    Forwarded,
    Finalized,
    Failed,
    Abandoned,
    DeadLetter,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialClass {
    NyxidManagedMaster,
    NyxidPlatformOauthApp,
    UserOwned,
    AgentOverrideUserOwned,
    NodeManaged,
    NoAuth,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct AllowanceReservationAllocation {
    pub allowance_id: String,
    pub period_id: String,
    pub quantity: i64,
}

crate::exact_credit_model! {
    [
        ("amount", "amount_micros"),
    ]
    #[derive(Clone, Debug, Serialize, ToSchema, PartialEq, Eq)]
    pub struct GrantReservationAllocation {
        pub grant_id: String,
        #[serde(with = "crate::models::credits::whole")]
        pub amount: Credits,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct AllowanceConsumptionAllocation {
    pub operation_id: String,
    pub allowance_id: String,
    pub period_id: String,
    pub quantity: i64,
}

crate::exact_credit_model! {
    [
        ("amount", "amount_micros"),
    ]
    #[derive(Clone, Debug, Serialize, ToSchema, PartialEq, Eq)]
    pub struct GrantConsumptionAllocation {
        pub operation_id: String,
        pub grant_id: String,
        #[serde(with = "crate::models::credits::whole")]
        pub amount: Credits,
    }
}

crate::exact_credit_model! {
    [
        ("total_charge", "total_charge_micros"),
        ("allowance_funded", "allowance_funded_micros"),
        ("grant_funded", "grant_funded_micros"),
        ("wallet_funded", "wallet_funded_micros"),
    ]
    #[derive(Clone, Debug, Default, Serialize, ToSchema, PartialEq, Eq)]
    pub struct UsageFunding {
        /// Legacy rows without a reservation must never consume benefits.
        #[serde(default)]
        pub wallet_only: bool,
        /// Cohort committed with this settlement; identifies its rational residue.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub lago_carry_id: Option<String>,
        /// Reservation-time rate used as a settlement fallback when the cache is
        /// temporarily unavailable. Model-specific rates still win at settlement.
        #[serde(default)]
        pub credits_per_unit_micros: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub credits_per_unit_pico: Option<i64>,
        /// Frozen before any funding mutation; crash retries use the same rate.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::models::credits::whole::optional"
        )]
        pub settlement_rate: Option<Credits>,
        #[serde(default)]
        pub allowance_reservations: Vec<AllowanceReservationAllocation>,
        #[serde(default)]
        pub grant_reservations: Vec<GrantReservationAllocation>,
        #[serde(default)]
        pub allowance_consumptions: Vec<AllowanceConsumptionAllocation>,
        #[serde(default)]
        pub grant_consumptions: Vec<GrantConsumptionAllocation>,
        #[serde(default)]
        pub settled: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub settlement_claim_id: Option<String>,
        #[serde(default, with = "crate::models::bson_datetime::optional")]
        pub settlement_claimed_at: Option<DateTime<Utc>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(with = "crate::models::credits::whole::optional")]
        pub wallet_charge_credits: Option<Credits>,
        /// Gross cost of the full finalized quantity at the settlement rate,
        /// including units covered by allowances. Display only; never a wallet debit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(with = "crate::models::credits::whole::optional")]
        pub total_charge: Option<Credits>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub allowance_funded_quantity: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(with = "crate::models::credits::whole::optional")]
        pub allowance_funded: Option<Credits>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(with = "crate::models::credits::whole::optional")]
        pub grant_funded: Option<Credits>,
        /// Exact wallet-funded cost, identical to the wallet debit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[serde(with = "crate::models::credits::whole::optional")]
        pub wallet_funded: Option<Credits>,
        /// Lago quantity funded by the wallet, in millionths of one metered
        /// unit. None preserves legacy whole-quantity event behavior.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub lago_billable_quantity_micros: Option<i64>,
        #[serde(default, with = "crate::models::bson_datetime::optional")]
        pub settled_at: Option<DateTime<Utc>>,
    }
}

/// Additive pool-attempt accounting; historical UsageStatus values remain valid.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct PoolAttemptAccounting {
    pub pool_id: String,
    pub member_id: String,
    pub attempt: u32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub lease_until: DateTime<Utc>,
    #[serde(default)]
    pub outcome: Option<PoolAttemptOutcome>,
    #[serde(default)]
    pub completion_cause: Option<PoolCompletionCause>,
}

/// Transport completion is independent of whether usage is known or settled.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PoolCompletionCause {
    Complete,
    UpstreamBodyFailure,
    UpstreamTimeout,
    CallerCancelled,
    LeaseLost,
}

impl PoolCompletionCause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::UpstreamBodyFailure => "transport_error",
            Self::UpstreamTimeout => "timeout",
            Self::CallerCancelled => "caller_cancelled",
            Self::LeaseLost => "lease_lost",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PoolAttemptOutcome {
    Unsent,
    Rejected,
    Unknown,
    Reported,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct UsageMeterRow {
    #[serde(rename = "_id")]
    pub id: String,
    /// New rows enter the indexed live tail. Missing legacy markers are also
    /// pending; the bounded fold discovers them without a migration.
    #[serde(default = "default_rollup_pending")]
    pub rollup_pending: bool,

    pub transaction_id: String,
    pub billing_request_id: String,
    pub layer: BillingLayer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flush_seq: Option<i64>,
    pub billing_owner_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet_id: Option<String>,
    pub actor_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_slug: Option<String>,
    pub metric: BillingMetric,
    pub lago_metric_code: String,
    pub credential_class: CredentialClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Provider-reported token classes (LLM traffic only; observability,
    /// not priced separately). Follows each provider's own accounting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_breakdown: Option<crate::models::service_billing::TokenBreakdown>,
    #[serde(default)]
    #[serde(with = "crate::models::credits::whole")]
    pub reserved_credits: Credits,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding: Option<UsageFunding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_resale_quantity: Option<i64>,
    /// Crash-recoverable final quantities for all platform components.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_platform_usage: Option<crate::models::service_billing::PlatformUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_attempt: Option<PoolAttemptAccounting>,
    pub status: UsageStatus,
    pub forwarded: bool,
    pub released: bool,
    pub lago_acked: bool,
    pub attempt: i32,
    #[serde(default)]
    pub settlement_attempts: i32,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub settlement_next_retry_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub finalized_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

fn default_rollup_pending() -> bool {
    true
}
