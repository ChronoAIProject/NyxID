use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "channel_send_claims";

/// Metadata only. The composite ID is `{conversation_id}:{idempotency_key}`.
/// A missing message_id means delivery is in flight or its outcome is uncertain.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChannelSendClaim {
    #[serde(rename = "_id")]
    pub id: String,
    /// Fences completion/release against a replacement after TTL cleanup.
    pub attempt_id: String,
    pub fingerprint: String,
    /// `pending` until a successful send and metadata persistence; then `sent`.
    pub status: String,
    pub message_id: Option<String>,
    pub platform_message_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for ChannelSendClaim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelSendClaim")
            .field("id", &"[REDACTED]")
            .field("completed", &self.message_id.is_some())
            .finish_non_exhaustive()
    }
}
