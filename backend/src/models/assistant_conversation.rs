use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_conversations";

/// Retired Ask/Full choice. Every chat now runs with Full access; the field
/// stays readable so legacy rows deserialize and are upgraded on their next turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessMode {
    #[default]
    Ask,
    Full,
}

/// A NyxBot team is one orchestrator plus disposable subagents. Rows written
/// before teams existed deserialize as orchestrators.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRole {
    #[default]
    Orchestrator,
    Subagent,
}

/// Authority the orchestrator delegated to a subagent. The subagent's key
/// mirrors these fields; this copy is re-applied at every turn.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentGrants {
    /// Owner-visible `UserService` IDs.
    #[serde(default)]
    pub service_ids: Vec<String>,
    /// Platform catalog (`DownstreamService`) IDs.
    #[serde(default)]
    pub platform_service_ids: Vec<String>,
    /// Read-only NyxID account tools.
    #[serde(default)]
    pub account_read: bool,
}

/// Who started a turn. Only orchestrator- and event-started subagent turns
/// wake the orchestrator when they settle; a user's direct chat does not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnOrigin {
    #[default]
    User,
    Orchestrator,
    Event,
    Channel,
}

/// A server-authored wake-up item waiting for the agent's next event turn.
/// `text` is NyxID-authored and bounded; it never carries secrets.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentEvent {
    pub id: String,
    /// `subagent_settled`, `permission_requested`, `permission_decided`,
    /// `permission_expired`, `message`.
    pub kind: String,
    pub text: String,
    /// Subagent conversation the event concerns, when any.
    #[serde(default)]
    pub subagent_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// The external channel a conversation answers, when it was started by a
/// channel bot through the Agent Event Gateway. Identifiers only.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelOrigin {
    /// The NyxBot channel (`nyxbot_channels._id`).
    pub nyxbot_channel_id: String,
    /// The chat partition (`nyxbot_threads.partition`): a gateway conversation
    /// ID, or a digest of a direct chat and sender. Opaque.
    pub partition: String,
    /// Canonical platform, e.g. `telegram`.
    pub platform: String,
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
    #[serde(default)]
    pub origin: TurnOrigin,
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
    /// Wake-up events drained into this turn (rendered into its instructions).
    #[serde(default)]
    pub events: Vec<AgentEvent>,
    /// NyxID-authored context for this turn only, e.g. a channel sender's
    /// verified identity. Bounded; never secrets.
    #[serde(default)]
    pub note: Option<String>,
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
    #[serde(default)]
    pub role: AgentRole,
    /// The orchestrator's conversation ID; `None` on orchestrators.
    #[serde(default)]
    pub team_id: Option<String>,
    /// Unique within the team; subagents only.
    #[serde(default)]
    pub agent_name: Option<String>,
    /// Task and scope the orchestrator gave a subagent (bounded).
    #[serde(default)]
    pub charter: Option<String>,
    /// Optional subagent specialty (`research`, `writer`, ...) for profile routing.
    #[serde(default)]
    pub specialty: Option<String>,
    #[serde(default)]
    pub grants: SubagentGrants,
    /// Set when a subagent is destroyed; the transcript is then read-only.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub destroyed_at: Option<DateTime<Utc>>,
    /// Bounded wake-up queue drained by the next event turn.
    #[serde(default)]
    pub pending_events: Vec<AgentEvent>,
    /// Consecutive event turns since the last user message (loop guard).
    #[serde(default)]
    pub event_streak: i32,
    /// Set on conversations that answer a channel bot.
    #[serde(default)]
    pub channel: Option<ChannelOrigin>,
}

impl AssistantConversation {
    pub fn is_subagent(&self) -> bool {
        self.role == AgentRole::Subagent
    }
    /// The orchestrator conversation that owns this row's team.
    pub fn team_root(&self) -> &str {
        self.team_id.as_deref().unwrap_or(&self.id)
    }
}

impl std::fmt::Debug for AssistantConversation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantConversation")
            .field("id", &self.id)
            .field("message_count", &self.message_count)
            .finish_non_exhaustive()
    }
}
