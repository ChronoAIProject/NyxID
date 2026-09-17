use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_agent_credentials";

/// NyxAgent binds context to the original key. Never serialize this model to an API.
#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantAgentCredential {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    #[serde(default)]
    pub conversation_id: String,
    pub api_key_id: String,
    #[serde(with = "crate::models::bson_bytes::required")]
    pub key_ciphertext: Vec<u8>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_used_at: DateTime<Utc>,
}

impl std::fmt::Debug for AssistantAgentCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantAgentCredential")
            .field("user_id", &self.user_id)
            .finish_non_exhaustive()
    }
}
