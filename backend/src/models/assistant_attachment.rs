use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_attachments";

/// An image a tool returned during a chat turn. The bytes are envelope-encrypted
/// and follow the runtime tool-image policy (conversation lifetime by default).
#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantAttachment {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(default = "tool_origin")]
    pub origin: String,
    pub user_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub content_type: String,
    pub size: i64,
    #[serde(with = "super::bson_bytes::required")]
    pub data_encrypted: Vec<u8>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl std::fmt::Debug for AssistantAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantAttachment")
            .field("id", &self.id)
            .field("content_type", &self.content_type)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

fn tool_origin() -> String {
    "tool".into()
}
