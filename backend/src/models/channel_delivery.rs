use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "channel_delivery_receipts";

/// Accepted provider components of one send. No text, captions, or provider error prose.
#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformSendRecord {
    pub phone_number_id: String,
    pub waba_id: Option<String>,
    pub recipient_id: String,
    pub component_ids: Vec<String>,
    pub expected_components: u32,
    pub complete: bool,
    pub uncertain: bool,
    pub failure_code: Option<u32>,
}

impl std::fmt::Debug for PlatformSendRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PlatformSendRecord([REDACTED])")
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ReceiptTimes {
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub sent_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub delivered_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub read_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub played_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub failed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DeliveryReceipt {
    #[serde(rename = "_id")]
    pub id: String,
    pub identity_key: String,
    pub bot_id: String,
    pub user_id: String,
    pub phone_number_id: String,
    pub waba_id: Option<String>,
    pub recipient_id: String,
    pub platform_message_id: String,
    pub times: ReceiptTimes,
    #[serde(default)]
    pub error_codes: Vec<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for DeliveryReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeliveryReceipt([REDACTED])")
    }
}
