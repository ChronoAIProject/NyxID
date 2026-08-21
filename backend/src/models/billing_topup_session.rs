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

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct BillingTopUpSession {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub idempotency_key: String,
    pub amount_credits: i64,
    pub lago_wallet_id: String,
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
    pub expired_credits_micros: i64,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub credits_expired_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry_void_transaction_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
