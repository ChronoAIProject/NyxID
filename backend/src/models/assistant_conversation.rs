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

/// The kind of agent a conversation thread belongs to. NyxBot threads act
/// with Full access; specialist threads act with their agent's grants. Rows
/// written before agents existed deserialize as NyxBot threads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRole {
    #[default]
    Orchestrator,
    Subagent,
}

/// Who started a turn. Only NyxBot- and event-started specialist turns report
/// back to NyxBot when they settle; a user's direct chat does not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnOrigin {
    #[default]
    User,
    Orchestrator,
    Event,
    Channel,
    /// A group chat addressed this member (a user message or another member's
    /// @mention). Its reply is posted to the group.
    Group,
    /// Owner-authorized automation, with its own run budget.
    Trigger,
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
    /// Specialist agent the event concerns, when any.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// A queued chat message: its question key (see `question_key`), so the
    /// same question is not queued twice.
    #[serde(default)]
    pub question_key: Option<String>,
    /// A queued chat message: the chats that asked it, which get the answer
    /// of the turn that drains it.
    #[serde(default)]
    pub reply_to: Vec<ChannelOrigin>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// The external channel a conversation answers, when it was started by a
/// channel bot through the Agent Event Gateway. Identifiers only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolProgress {
    pub started: i64,
    pub calls: i64,
    pub digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveTurn {
    /// Nodes admitted by this turn; Stop uses the durable list across replicas.
    #[serde(default)]
    pub machine_node_ids: Vec<String>,
    #[serde(default)]
    pub continuations: u32,
    /// Rolling hashes only; never persist tool arguments or results. Reset per continuation.
    #[serde(default)]
    pub tool_progress: ToolProgress,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub trigger_run_id: Option<String>,
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
    /// The question this turn answers: a digest of its normalized text (so a
    /// repeat is recognized) and a short excerpt (so the agent's other threads
    /// know what is in progress). `None` for event and short messages.
    #[serde(default)]
    pub question_key: Option<String>,
    #[serde(default)]
    pub question: Option<String>,
    /// The chat app chat that asked, for channel turns.
    #[serde(default)]
    pub asked_from: Option<ChannelOrigin>,
    /// Other chats that asked the same question while this turn ran; its
    /// answer is delivered to them too. Bounded.
    #[serde(default)]
    pub also_deliver: Vec<ChannelOrigin>,
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
    /// Denormalized from the owning agent's kind.
    #[serde(default)]
    pub role: AgentRole,
    /// The agent (`assistant_agents._id`) this thread belongs to. `None` on
    /// rows written before agents existed; they belong to the owner's NyxBot.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Dedicated/per-run automation threads never become an agent's home,
    /// including when its previous home was deleted. Explicit home runs leave this false.
    #[serde(default)]
    pub automation_thread: bool,
    /// Specialist threads only: the NyxBot thread that assigned the current
    /// work, which receives its report and permission requests.
    #[serde(default)]
    pub report_to: Option<String>,
    /// Bounded wake-up queue drained by the next event turn.
    #[serde(default)]
    pub pending_events: Vec<AgentEvent>,
    /// Consecutive event turns since the last user message (loop guard).
    #[serde(default)]
    pub event_streak: i32,
    /// Set on threads that answer a channel bot.
    #[serde(default)]
    pub channel: Option<ChannelOrigin>,
    /// An agent's own thread that the owner also reaches from their private
    /// chats: the chat the owner last wrote from, where asynchronous replies
    /// go. Cleared when the owner writes in the NyxID app.
    #[serde(default)]
    pub reply_channel: Option<ChannelOrigin>,
    /// Chats still owed the newest turn's answer (they asked the same
    /// question while it ran); taken once by the settlement hook.
    #[serde(default)]
    pub deliver_also: Vec<ChannelOrigin>,
    /// Set on a member's hidden thread in a group chat: the agent speaks in
    /// the group through it, with its own key and memory.
    #[serde(default)]
    pub group_id: Option<String>,
    /// The newest group message this member has already been given.
    #[serde(default)]
    pub group_seen_seq: i64,
    /// The newest turn was started by someone other than the owner, e.g. a
    /// member of a group chat. Its tool calls are restricted (NyxBot: none;
    /// specialists: their grants, as far as the owner's guest access for each
    /// service allows). Kept after the turn settles so late tool calls stay
    /// restricted.
    #[serde(default)]
    pub guest_turn: bool,
}

impl AssistantConversation {
    pub fn is_subagent(&self) -> bool {
        self.role == AgentRole::Subagent
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
