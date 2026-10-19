use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_channel_identities";

/// Server-owned mapping from a channel address to the existing durable proof format.
/// Handles are scoped to one profile and never accepted from client input.
#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformChannelIdentity {
    #[serde(rename = "_id")]
    pub id: String,
    pub profile_id: String,
    pub kind: String,
    pub external_key: String,
    pub handle: i64,
    pub sender_id: String,
    #[serde(default)]
    pub sender_display_name: Option<String>,
    pub conversation_id: String,
    #[serde(default)]
    pub reply_context_encrypted: Vec<u8>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}
