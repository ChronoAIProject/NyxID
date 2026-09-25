use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_conversations";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessMode {
    #[default]
    Ask,
    Full,
}

/// One tool call made with the chat's key during a live turn. Metadata only:
/// the label is a tool identifier, never arguments, results, or secrets.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnActivity {
    pub id: String,
    pub label: String,
    /// `running`, `completed`, or `error`.
    pub status: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub started_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub ended_at: Option<DateTime<Utc>>,
}

/// Metadata for an image a tool returned during a turn; the bytes live in
/// `assistant_attachments`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnAttachment {
    pub id: String,
    pub content_type: String,
    pub size: i64,
    /// The tool identifier that produced it, never arguments.
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveTurn {
    pub turn_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub stop_requested: bool,
    /// Bounded, oldest first; the newest entries are retained.
    #[serde(default)]
    pub activities: Vec<TurnActivity>,
    /// Bounded; see `MAX_TURN_ATTACHMENTS`.
    #[serde(default)]
    pub attachments: Vec<TurnAttachment>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantConversation {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub title: String,
    pub model: String,
    #[serde(default)]
    pub access_mode: AccessMode,
    pub nyxagent_session_id: Option<String>,
    pub nyxagent_last_response_id: Option<String>,
    pub credential_api_key_id: String,
    pub message_count: i64,
    pub active_turn: Option<ActiveTurn>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub context_reset_at: Option<DateTime<Utc>>,
    pub context_reset_reason: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for AssistantConversation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantConversation")
            .field("id", &self.id)
            .field("message_count", &self.message_count)
            .finish_non_exhaustive()
    }
}
