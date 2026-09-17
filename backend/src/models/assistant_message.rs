use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_messages";

#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
    #[serde(rename = "_id")]
    pub id: String,
    pub conversation_id: String,
    pub user_id: String,
    pub seq: i64,
    pub turn_id: String,
    pub role: String,
    pub text: String,
    pub status: String,
    pub error_code: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl std::fmt::Debug for AssistantMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantMessage")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}
