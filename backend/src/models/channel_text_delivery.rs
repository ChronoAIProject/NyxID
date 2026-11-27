use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "channel_text_deliveries";

/// Metadata-only evidence for one immutable multipart plan. No message bodies.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChannelTextDelivery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_grant: Option<super::channel_meta::GrantRef>,
    #[serde(rename = "_id")]
    pub id: String,
    pub logical_key: String,
    pub user_id: String,
    pub source_id: String,
    /// New saved-result plans carry their bot parent on every platform.
    /// Legacy plans require exact source or saved-destination attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_bot_id: Option<String>,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderer_version: Option<u32>,
    pub notice: Option<TextDeliveryNotice>,
    pub parts: Vec<TextPart>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub retry_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TextPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preparation: Option<TextPreparation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formatting_fallback: Option<String>,
    pub state: TextPartState,
    pub attempt_id: Option<String>,
    pub platform_message_id: Option<String>,
    pub outbound_message_id: Option<String>,
}

/// Only Pending is reclaimable. Sending always means a possible provider effect.
#[derive(Clone, Serialize, Deserialize)]
pub struct TextPreparation {
    pub version: u32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub lease_until: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextPartState {
    Pending,
    /// A provider operation may have happened. Never automatically retry.
    Sending,
    Sent,
    Refused,
}

/// Renderer inputs for a stable delivery notice; contains no private content.
#[derive(Clone, Serialize, Deserialize)]
pub struct TextDeliveryNotice {
    pub attempted: u32,
    pub sent: u32,
    pub failed: u32,
    pub unknown: u32,
    pub skipped: u32,
    pub unsupported: bool,
    pub app_visible: bool,
    pub automation: bool,
    pub omitted: u32,
}
