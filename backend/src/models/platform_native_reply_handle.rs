//! Encrypted provider reply capability. Never expose this stored model in an API.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_native_reply_handles";

#[derive(Clone, Serialize, Deserialize)]
pub struct NativeReplyHandle {
    #[serde(rename = "_id")]
    pub id: String,
    pub receipt_id: String,
    pub binding_digest: String,
    #[serde(with = "crate::models::bson_bytes::required")]
    pub ciphertext: Vec<u8>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for NativeReplyHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeReplyHandle([REDACTED])")
    }
}
