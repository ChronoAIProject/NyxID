use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "channel_reply_deliveries";

/// One logical reply's send evidence. Never contains text, media or credentials.
#[derive(Serialize, Deserialize)]
pub struct ChannelReplyDelivery {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub channel_bot_id: String,
    pub inbound_message_id: String,
    pub expected_parts: u32,
    pub complete: bool,
    pub parts: Vec<ReplyPart>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ReplyPart {
    pub status: PartStatus,
    pub platform_message_ids: Vec<String>,
    pub outbound_message_id: Option<String>,
    pub failure_code: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartStatus {
    NotAttempted,
    /// Persisted before dispatch; also covers process death or a lost response.
    Uncertain,
    Accepted,
    Failed,
}
