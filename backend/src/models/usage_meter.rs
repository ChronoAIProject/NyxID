use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::models::service_billing::BillingMetric;

pub const COLLECTION_NAME: &str = "usage_meter";

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
    UserOwned,
    AgentOverrideUserOwned,
    NodeManaged,
    NoAuth,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeferredQuantity {
    TwilioCall {
        account_sid: String,
        call_sid: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct AllowanceReservationAllocation {
    pub allowance_id: String,
    pub period_id: String,
    pub quantity: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct GrantReservationAllocation {
    pub grant_id: String,
    pub amount_micros: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct AllowanceConsumptionAllocation {
    pub operation_id: String,
    pub allowance_id: String,
    pub period_id: String,
    pub quantity: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct GrantConsumptionAllocation {
    pub operation_id: String,
    pub grant_id: String,
    pub amount_micros: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct UsageFunding {
    /// Reservation-time rate used as a settlement fallback when the cache is
    /// temporarily unavailable. Model-specific rates still win at settlement.
    #[serde(default)]
    pub credits_per_unit_micros: i64,
    #[serde(default)]
    pub allowance_reservations: Vec<AllowanceReservationAllocation>,
    #[serde(default)]
    pub grant_reservations: Vec<GrantReservationAllocation>,
    #[serde(default)]
    pub base_fee_grant_reservations: Vec<GrantReservationAllocation>,
    #[serde(default)]
    pub allowance_consumptions: Vec<AllowanceConsumptionAllocation>,
    #[serde(default)]
    pub grant_consumptions: Vec<GrantConsumptionAllocation>,
    #[serde(default)]
    pub base_fee_grant_consumptions: Vec<GrantConsumptionAllocation>,
    #[serde(default)]
    pub settled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement_claim_id: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub settlement_claimed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet_charge_credits: Option<i64>,
    /// Lago quantity funded by the wallet, in millionths of one metered
    /// unit. None preserves legacy whole-quantity event behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lago_billable_quantity_micros: Option<i64>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub settled_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct UsageMeterRow {
    #[serde(rename = "_id")]
    pub id: String,
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
    pub reserved_credits: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding: Option<UsageFunding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<i64>,
    /// Credit-denominated fee snapshotted at reservation time. It is not an
    /// allowance quantity and is therefore funded only by grants and wallet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_fee_micros: Option<i64>,
    #[serde(default)]
    pub base_fee_applied: bool,
    #[serde(default)]
    pub base_fee_applied_credits: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred_quantity: Option<DeferredQuantity>,
    #[serde(default)]
    pub deferred_attempts: i32,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub deferred_next_retry_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_resale_quantity: Option<i64>,
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

#[cfg(test)]
mod tests {
    use super::DeferredQuantity;

    #[test]
    fn deferred_twilio_descriptor_is_tagged_and_contains_only_poll_identity() {
        let value = serde_json::to_value(DeferredQuantity::TwilioCall {
            account_sid: "AC11111111111111111111111111111111".to_string(),
            call_sid: "CA22222222222222222222222222222222".to_string(),
        })
        .expect("serialize deferred quantity");

        assert_eq!(value["type"], "twilio_call");
        assert_eq!(value["account_sid"], "AC11111111111111111111111111111111");
        assert_eq!(value["call_sid"], "CA22222222222222222222222222222222");
        assert_eq!(value.as_object().expect("object").len(), 3);
    }
}
