use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_invocations";
pub const TURNS: &str = "assistant_invocation_turns";

/// Admission receipts survive conversation deletion. They contain no input,
/// output, credential, provider binding or arbitrary caller metadata.
#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantInvocation {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub idempotency_key: String,
    pub request_digest: String,
    pub conversation_id: String,
    #[serde(default)]
    pub conversation_ids: Vec<String>,
    pub turn_id: String,
    pub cancel_requested: bool,
    #[serde(default)]
    pub terminal_status: Option<String>,
    pub conversation_deleted: bool,
    #[serde(default)]
    pub continuation_blocked: bool,
    /// Attempt identities retained when external cancellation cannot be confirmed.
    #[serde(default)]
    pub unconfirmed_async_cancellations: Vec<String>,
    pub generation: i64,
    pub turn_count: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InvocationTurn {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub invocation_id: String,
    pub conversation_id: String,
    pub state: String,
    pub result_message_id: Option<String>,
    pub error_code: Option<String>,
    pub hooks_pending: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub settled_at: Option<DateTime<Utc>>,
    /// Captured before settlement clears the active turn. Exact cancellation
    /// may need to stop work after the reply has already been saved.
    #[serde(default)]
    pub machine_scopes: Vec<super::assistant_conversation::MachineTurnScope>,
}

impl std::fmt::Debug for AssistantInvocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AssistantInvocation { [REDACTED] }")
    }
}
impl std::fmt::Debug for InvocationTurn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InvocationTurn { [REDACTED] }")
    }
}
