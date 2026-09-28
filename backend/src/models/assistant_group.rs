use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_groups";
pub const MESSAGES_COLLECTION_NAME: &str = "assistant_group_messages";
pub const MAX_MEMBERS: usize = 8;
pub const MAX_NAME_CHARS: usize = 60;
/// Agent-to-agent hand-offs (@mentions in replies) allowed per user message.
pub const HOPS_PER_MESSAGE: i32 = 6;

/// A group chat: the owner plus several of their agents (Grok-Bot style).
/// Each member speaks through its own hidden member thread, so it keeps its
/// own key, grants and memory.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantGroup {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub member_agent_ids: Vec<String>,
    /// Answers user messages that mention no one: NyxBot when it is a member.
    pub lead_agent_id: String,
    /// `user` or `nyxbot`.
    pub created_by: String,
    pub message_count: i64,
    /// Members addressed but not yet started (busy, or the pool was full).
    #[serde(default)]
    pub pending_agent_ids: Vec<String>,
    /// Remaining agent hand-offs since the last user message.
    #[serde(default)]
    pub hops_remaining: i32,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub last_message_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// One message in a group transcript.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupMessage {
    #[serde(rename = "_id")]
    pub id: String,
    pub group_id: String,
    pub user_id: String,
    pub seq: i64,
    /// `user`, `agent` or `notice` (NyxID-authored).
    pub role: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_name: Option<String>,
    pub text: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
