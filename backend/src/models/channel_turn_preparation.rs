//! Metadata on a NyxBot event; answer bodies remain in assistant_messages.
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Preparation {
    pub version: i32,
    #[serde(default)]
    pub gateway_receipt: Option<super::gateway_reply::ReceiptWait>,
    #[serde(default)]
    pub gateway_request_hash: Option<String>,
    #[serde(default)]
    pub gateway_request_version: Option<i32>,
    pub message_id: String,
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub next: u32,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub notice: Option<super::channel_text_delivery::TextDeliveryNotice>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Receipt {
    pub platform_message_ids: Vec<String>,
    pub outbound_message_id: Option<String>,
}
