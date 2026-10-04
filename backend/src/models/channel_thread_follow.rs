//! Additive metadata in nyxbot_threads; no transcript or provider credentials.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ThreadFollow {
    pub threads: Option<String>,
    pub has_thread_history: bool,
    pub record_scope: Option<String>,
    pub parent_chat_id: Option<String>,
    pub settings_chat_id: Option<String>,
    pub thread_identity_version: Option<u32>,
    pub thread_kind: Option<super::channel_thread::ThreadKind>,
    pub thread_root_id: Option<String>,
    pub thread_scope_id: Option<String>,
    pub follow_state: Option<String>,
    pub follow_revision: i64,
    pub binding_generation: i64,
    pub binding_channel_generation: i64,
    pub bound_agent_id: Option<String>,
    pub follow_started_by: Option<String>,
    pub follow_stop_reason: Option<String>,
    pub follow_error_code: Option<String>,
    pub context_status: Option<String>,
    pub context_message_count: u32,
    pub follow_busy_count: i64,
    pub follow_drop_count: i64,
    #[serde(with = "super::bson_datetime::optional")]
    pub follow_started_at: Option<DateTime<Utc>>,
    #[serde(with = "super::bson_datetime::optional")]
    pub follow_last_admitted_at: Option<DateTime<Utc>>,
    #[serde(with = "super::bson_datetime::optional")]
    pub follow_expires_at: Option<DateTime<Utc>>,
    #[serde(with = "super::bson_datetime::optional")]
    pub follow_stopped_at: Option<DateTime<Utc>>,
    #[serde(with = "super::bson_datetime::optional")]
    pub follow_opening_expires_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for ThreadFollow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ThreadFollow([REDACTED])")
    }
}

/// Server-authored per-event authority. Never accepted from an HTTP turn DTO.
/// A queued event keeps its own source; newer messages cannot retarget it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadTurnBinding {
    pub child_id: String,
    pub source_message_id: String,
    pub sender_id: String,
    pub guest: bool,
    pub revision: i64,
    pub generation: i64,
    pub channel_generation: i64,
    pub agent_id: String,
    /// Already admitted to the bounded owner queue; subscription-only stop
    /// does not revoke it. Live ownership, policy and generation still apply.
    #[serde(default)]
    pub queued: bool,
}

impl std::fmt::Debug for ThreadTurnBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ThreadTurnBinding([REDACTED])")
    }
}
