use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_tool_results";

/// Ephemeral execution output. Never serialize this model as an API response.
#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantToolResult {
    #[serde(rename = "_id")]
    pub id: String,
    pub version: u8,
    pub user_id: String,
    pub actor_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub api_key_id: String,
    pub key_authority_fence: i64,
    pub agent_id: String,
    pub guest: bool,
    pub size: i64,
    pub boundaries: Vec<i64>,
    #[serde(default)]
    pub encryption_version: Option<u8>,
    #[serde(with = "super::bson_bytes::required")]
    pub data_encrypted: Vec<u8>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
impl std::fmt::Debug for AssistantToolResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AssistantToolResult { [REDACTED] }")
    }
}
