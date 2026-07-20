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
    #[serde(default)]
    pub reserved_credits: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<i64>,
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
