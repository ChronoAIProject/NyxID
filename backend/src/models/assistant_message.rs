use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_messages";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageAuthor {
    Owner,
    Guest,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
    /// Metadata-only handoff to manager delivery, including fresh event replies.
    #[serde(default)]
    pub platform_delivery_recovery_queued: bool,
    #[serde(default)]
    pub platform_reply: Option<super::assistant_conversation::PlatformReplyRouting>,
    /// Retained after settlement for exact transport delivery and recovery.
    #[serde(default)]
    pub platform_origin: Option<super::assistant_conversation::PlatformTurnOrigin>,
    #[serde(default)]
    pub author: Option<MessageAuthor>,
    /// Guidance delivered to an existing response, never a new instruction turn.
    #[serde(default)]
    pub steering: Option<Box<Steering>>,
    #[serde(default)]
    pub voice: Option<VoiceTranscript>,
    /// Voice input is visible in history but is not execution context until claimed.
    #[serde(default)]
    pub execution_pending: bool,
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
    /// Sanitized upstream code retained separately from the stable NyxID class.
    #[serde(default)]
    pub upstream_error_code: Option<String>,
    /// Metadata-only failure diagnostics; never contains prompt or credential data.
    #[serde(default)]
    pub error_detail: Option<AssistantErrorDetail>,
    #[serde(default)]
    pub turn_metadata: Option<TurnMetadata>,
    /// Specialist identity for bounded access-denial notices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specialist_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    /// Tool calls observed during the turn that produced this reply (assistant rows only).
    #[serde(default)]
    pub activities: Vec<super::assistant_conversation::TurnActivity>,
    #[serde(default)]
    pub activity_total: i64,
    /// Images tools returned during the turn that produced this reply.
    #[serde(default)]
    pub attachments: Vec<super::assistant_conversation::TurnAttachment>,
    /// What started the turn this message belongs to. Authorship is separate:
    /// owner retry requires an explicitly owner-authored user input.
    #[serde(default)]
    pub origin: Option<super::assistant_conversation::TurnOrigin>,
    /// A user message written in a chat app: its platform.
    #[serde(default)]
    pub via: Option<String>,
    /// Failed turn this turn retries, on its user input and assistant reply.
    #[serde(default)]
    pub retry_of: Option<String>,
    /// A failed reply can be consumed by one retry only. Stored on the source
    /// reply in the same transaction that claims its successor turn.
    #[serde(default)]
    pub retry_claimed_by: Option<String>,
    /// None means a legacy turn whose bounded activity list is insufficient
    /// evidence for automatic recovery.
    #[serde(default)]
    pub activity_count: Option<i64>,
    /// Owner decision that caused this browser continuation turn.
    #[serde(default)]
    pub continuation_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_to: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantErrorDetail {
    #[serde(default)]
    pub attempts: Option<u32>,
    #[serde(default)]
    pub capacity_wait_ms: Option<u64>,
    pub stage: String,
    pub http_status: Option<u16>,
    pub upstream_request_id: Option<String>,
    pub response_id: Option<String>,
    pub continuations: u32,
    pub turn_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub occurred_at: DateTime<Utc>,
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

#[derive(Clone, Serialize, Deserialize)]
pub struct VoiceFragment {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct VoiceTranscript {
    #[serde(default)]
    pub fragments: Vec<VoiceFragment>,
    pub session_id: String,
    pub segment_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub sealed: bool,
    pub complete: bool,
    pub delivery: String,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub backend_message_id: Option<String>,
}

/// Durable delivery receipt. Text stays on its transcript row, never in Debug.
#[derive(Clone, Serialize, Deserialize)]
pub struct Steering {
    #[serde(default)]
    pub finalized: bool,
    pub client_request_id: String,
    pub response_id: String,
    pub session_id: String,
    /// `applied`, `may_not_have_applied`, or `refused`.
    pub outcome: String,
    pub code: Option<String>,
    pub http_status: u16,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TurnMetadata {
    pub attempts: Option<u32>,
    pub capacity_wait_ms: Option<u64>,
}
