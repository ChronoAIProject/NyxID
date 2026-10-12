use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Metadata-only snapshot of the reply a vote is about (addendum A1). Every
/// field defaults, so rows written before a field existed, or partial rows,
/// still deserialize. Never holds reply, prompt, comment or tool-argument text.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FeedbackReplyContext {
    pub surface: Option<String>,
    pub agent_id: Option<String>,
    pub agent_kind: Option<String>,
    pub model: Option<String>,
    pub access_mode: Option<String>,
    pub reply_status: Option<String>,
    pub error_code: Option<String>,
    pub origin: Option<String>,
    pub via: Option<String>,
    pub turn_id: Option<String>,
    pub reply_seq: Option<i64>,
    #[serde(with = "super::bson_datetime::optional")]
    pub reply_created_at: Option<DateTime<Utc>>,
    pub latency_ms: Option<i64>,
    pub tool_calls: u32,
    pub tool_errors: u32,
    pub tool_labels: Vec<String>,
    pub machine_used: bool,
    pub attachments: u32,
    pub steered: bool,
    pub voice: bool,
    pub reply_chars: u32,
    pub context_reset_since_reply: bool,
    pub org_group: bool,
}

pub const COLLECTION_NAME: &str = "assistant_message_feedback";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantMessageFeedback {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    pub message_id: String,
    pub turn_id: Option<String>,
    pub vote: String,
    pub comment: Option<String>,
    #[serde(default)]
    pub context: Option<FeedbackReplyContext>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
