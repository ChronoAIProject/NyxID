//! Server-owned voice lifecycle. Never persist audio, SDP or credentials here.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_voice_sessions";
pub const LEASE_SECONDS: i64 = 15;
pub const START_SECONDS: i64 = 20;
pub const HEARTBEAT_SECONDS: i64 = 30;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Starting,
    Active,
    Closing,
    Closed,
    Failed,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct VoiceSession {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub conversation_id: String,
    pub client_request_id: String,
    pub preferences: super::assistant_voice::VoicePreferences,
    /// Absent on Phase 3 rows; provider identity cannot change during recovery.
    #[serde(default)]
    pub protocol: Option<super::downstream_service::VoiceProtocol>,
    #[serde(default)]
    pub measured_ms: i64,
    pub state: SessionState,
    pub live_slot: bool,
    #[serde(default)]
    pub receipt_message_id: Option<String>,
    #[serde(default)]
    pub purge_requested: bool,
    pub generation: i64,
    pub lease_owner: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub lease_until: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub deadline: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_user_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub heartbeat_at: DateTime<Utc>,
    #[serde(with = "super::bson_datetime::optional")]
    pub closed_at: Option<DateTime<Utc>>,
    pub provider_session_id: Option<String>,
    pub credential_identity: String,
    pub desired_muted: bool,
    #[serde(default)]
    pub input_muted: bool,
    pub end_requested: bool,
    pub end_reason: Option<String>,
    pub observed_seconds: i64,
    pub reserved_until: i64,
    pub final_usage_confirmed: bool,
    #[serde(default)]
    pub billing_finalized: bool,
    pub playback_ms: i64,
    #[serde(default)]
    pub inaudible_until_ms: i64,
    pub playback_revision: i64,
    pub control_revision: i64,
    #[serde(default)]
    pub control_socket_id: Option<String>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub control_socket_until: Option<DateTime<Utc>>,
    pub last_command_id: Option<String>,
    pub last_command_action: Option<String>,
    pub notify_on_completion: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub reconcile_deadline: DateTime<Utc>,
}

impl std::fmt::Debug for VoiceSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceSession { [REDACTED] }")
    }
}
