//! Durable voice admission metadata. Audio and credentials never enter these rows.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const REQUESTS: &str = "assistant_voice_requests";
pub const COLLECTION_NAME: &str = REQUESTS;
pub const MAX_QUEUED: u64 = 10;
pub const MAX_CALL_SECONDS: i64 = 1800;
pub const IDLE_SECONDS: i64 = 180;
pub const IDLE_WARNING_SECONDS: i64 = 15;
pub const FINALIZATION_HOURS: i64 = 24;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VoiceInputMode {
    #[default]
    PushToTalk,
    Automatic,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VoiceKeySource {
    Platform,
    Own,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VoicePreferences {
    pub service_id: String,
    pub connection_id: Option<String>,
    pub key_source: VoiceKeySource,
    pub model: String,
    pub voice: Option<String>,
    #[serde(default)]
    pub input_mode: VoiceInputMode,
    pub language: Option<String>,
    #[serde(default)]
    pub notify_on_completion: bool,
}
impl std::fmt::Debug for VoicePreferences {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoicePreferences { [REDACTED] }")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestState {
    Queued,
    Claimed,
    AwaitingConfirmation,
    Completed,
    Cancelled,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct VoiceRequest {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub conversation_id: String,
    /// Hidden automation thread used for this request's independent turn.
    #[serde(default)]
    pub task_conversation_id: Option<String>,
    pub session_id: String,
    /// Provider delegation ID or server acknowledgement ID; never client text.
    pub source_id: String,
    pub message_id: String,
    pub message_seq: i64,
    pub turn_id: String,
    pub state: RequestState,
    #[serde(default)]
    pub pending_acknowledgement_ids: Vec<String>,
    /// A continuation remains bound to the exact key and requesting turn.
    pub acknowledgement_id: Option<String>,
    pub credential_api_key_id: Option<String>,
    pub parent_turn_id: Option<String>,
    /// Continuations retain the original task's cancellation identity.
    #[serde(default)]
    pub root_request_id: Option<String>,
    /// Visible-thread result summary, populated once the hidden task settles.
    #[serde(default)]
    pub result_message_id: Option<String>,
    /// Number of bounded restart replays already attempted for this request.
    #[serde(default)]
    pub recovery_replays: u8,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
impl std::fmt::Debug for VoiceRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceRequest { [REDACTED] }")
    }
}

pub const WINDOWS: &str = "assistant_voice_windows";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VoiceWindow {
    #[serde(rename = "_id")]
    pub id: String,
    pub billing_request_id: String,
    pub session_id: String,
    pub user_id: String,
    /// Binds retries to the same resolved owner, service, credential class and rates.
    pub context_identity: String,
    pub start_second: i64,
    pub reserved_seconds: i64,
    /// Only server/provider-observed duration; no audio-byte estimation.
    pub observed_seconds: i64,
    pub sealed: bool,
    pub settled: bool,
    pub uncertain: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub deadline: DateTime<Utc>,
}
