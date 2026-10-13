use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

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

#[derive(Clone, Serialize, Deserialize)]
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

impl fmt::Debug for AssistantMessageFeedback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssistantMessageFeedback")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("conversation_id", &self.conversation_id)
            .field("group_id", &self.group_id)
            .field("message_id", &self.message_id)
            .field("turn_id", &self.turn_id)
            .field("vote", &self.vote)
            .field("comment_present", &self.comment.is_some())
            .field("comment_len", &self.comment.as_ref().map(String::len))
            .field("context_present", &self.context.is_some())
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_comment() {
        let debug = format!(
            "{:?}",
            AssistantMessageFeedback {
                id: "feedback".into(),
                user_id: "user".into(),
                conversation_id: Some("conversation".into()),
                group_id: None,
                message_id: "message".into(),
                turn_id: Some("turn".into()),
                vote: "down".into(),
                comment: Some("sample feedback comment".into()),
                context: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }
        );
        assert!(!debug.contains("sample feedback comment"));
        assert!(debug.contains("comment_present: true"));
    }
}
