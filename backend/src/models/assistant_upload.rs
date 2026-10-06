//! User uploads share the attachment collection and its owner purge lifecycle.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use super::assistant_attachment::COLLECTION_NAME;

#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantUpload {
    #[serde(rename = "_id")]
    pub id: String,
    pub origin: String,
    pub user_id: String,
    pub conversation_id: String,
    pub group_id: Option<String>,
    pub message_id: Option<String>,
    pub label: String,
    pub content_type: String,
    pub size: i64,
    pub pages: usize,
    pub chunks: usize,
    #[serde(with = "super::bson_bytes::required")]
    pub text_encrypted: Vec<u8>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    // Legacy retention timestamp, used only to recover the original binding time.
    #[serde(default, with = "super::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub bound_at: Option<DateTime<Utc>>,
}
