use crate::models::credits::Credits;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const COLLECTION_NAME: &str = "billing_topup_sessions";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BillingTopUpStatus {
    Pending,
    CheckoutCreated,
    Failed,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StripeReversalAction {
    ReverseCredits,
    Release,
    WriteOff,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StripeReversalState {
    Pending,
    Applied,
    Refused,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct StripeReversalDecision {
    pub decision_id: String,
    pub action: StripeReversalAction,
    pub credits: i64,
    pub reason: String,
    pub state: StripeReversalState,
    #[serde(default = "assume_reversal_dispatched")]
    pub dispatch_started: bool,
    pub lago_void_transaction_id: Option<String>,
}

fn assume_reversal_dispatched() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct StripeReversal {
    pub provider_id: String,
    pub status: String,
    pub amount_minor: i64,
    pub decision: Option<StripeReversalDecision>,
    #[serde(default)]
    pub finance_correction: Option<StripeFinanceCorrection>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct StripeFinanceCorrection {
    pub reference: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub acknowledged_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct StripePurchase {
    pub account_id: String,
    #[serde(default)]
    pub livemode: bool,
    pub customer_id: String,
    pub currency: String,
    pub subtotal_minor: i64,
    pub unit_amount_minor: i64,
    pub offer_version: String,
    pub product_tax_code: String,
    pub return_url: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub checkout_expires_at: DateTime<Utc>,
    pub checkout_session_id: Option<String>,
    pub payment_intent_id: Option<String>,
    pub stripe_invoice_id: Option<String>,
    pub tax_minor: Option<i64>,
    pub total_minor: Option<i64>,
    #[serde(default)]
    pub expired: bool,
    #[serde(default)]
    pub async_failed: bool,
    #[serde(default)]
    pub reversal_review_required: bool,
    #[serde(default)]
    pub reversal_totals_agree: bool,
    #[serde(default)]
    pub stripe_charge_id: Option<String>,
    #[serde(default)]
    pub refunded_minor: i64,
    #[serde(default)]
    pub reversals: Vec<StripeReversal>,
    #[serde(default)]
    pub used_reversal_decision_ids: Vec<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub payment_observed_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub credits_available_at: Option<DateTime<Utc>>,
}

crate::exact_credit_model! {
    [
        ("expired_credits", "expired_credits_micros"),
    ]
    #[derive(Clone, Debug, Serialize, ToSchema, PartialEq, Eq)]
    pub struct BillingTopUpSession {
        #[serde(rename = "_id")]
        pub id: String,
        pub owner_id: String,
        pub idempotency_key: String,
        pub amount_credits: i64,
        pub lago_wallet_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub stripe_purchase: Option<StripePurchase>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub lago_wallet_transaction_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub lago_invoice_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub payment_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub payment_provider: Option<String>,
        pub status: BillingTopUpStatus,
        #[serde(default, with = "crate::models::bson_datetime::optional")]
        pub paid_at: Option<DateTime<Utc>>,
        #[serde(default, with = "crate::models::bson_datetime::optional")]
        pub credits_expire_at: Option<DateTime<Utc>>,
        #[serde(default)]
        #[serde(with = "crate::models::credits::whole")]
        pub expired_credits: Credits,
        #[serde(default, with = "crate::models::bson_datetime::optional")]
        pub credits_expired_at: Option<DateTime<Utc>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub expiry_void_transaction_id: Option<String>,
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        pub created_at: DateTime<Utc>,
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        pub updated_at: DateTime<Utc>,
    }
}
