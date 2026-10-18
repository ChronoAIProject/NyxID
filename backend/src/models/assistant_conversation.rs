use super::platform_entry_projection::{
    StoredEntryIdentity, StoredSenderIdentity, checked_identity_record, same_version,
};
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
    /// `subagent_settled`, `permission_requested`, `permission_decided`, `skill_request_allowed`,
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
    /// The producer proved `report_to` from a validated source (the causing
    /// turn's snapshot, a card's stored routing, or a carrier with validated
    /// provenance). Unbound (legacy) events never create NyxBot involvement.
    #[serde(default)]
    pub bound: bool,
    #[serde(default)]
    pub report_to: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl AgentEvent {
    /// Mark the event as proved from a validated source. `None` means the
    /// causing work was proven not to report to NyxBot (new owner work).
    pub fn bound_to(mut self, report_to: Option<&str>) -> Self {
        self.bound = true;
        self.report_to = report_to.map(str::to_owned);
        self
    }
}

/// First validated report-snapshot format on delayed carriers (async
/// operations, watches, machine jobs). 0 or absent is legacy and unproven;
/// unknown versions fail closed.
pub const REPORT_PROVENANCE_V1: u8 = 1;

pub fn carried_report_to(report_to: Option<&str>, provenance: u8) -> Option<String> {
    if provenance == REPORT_PROVENANCE_V1 {
        report_to.map(str::to_owned)
    } else {
        None
    }
}

/// The external channel a conversation answers, when it was started by a
/// channel bot through the Agent Event Gateway. Identifiers only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelOrigin {
    #[serde(default)]
    pub thread: Option<Box<super::channel_thread_follow::ThreadTurnBinding>>,
    /// The NyxBot channel (`nyxbot_channels._id`).
    pub nyxbot_channel_id: String,
    /// The chat partition (`nyxbot_threads.partition`): a gateway conversation
    /// ID, or a digest of a direct chat and sender. Opaque.
    pub partition: String,
    /// Canonical platform, e.g. `telegram`.
    pub platform: String,
}

checked_identity_record! {
/// Stable ownership of a personal manager thread. Retained after transport
/// unlink so the owner can continue the thread in NyxID.

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct PlatformThread, RawPlatformThread {
    pub platform: String,
    pub profile_id: String,
    pub manager_bot_id: String,
    #[serde(flatten)]
    pub entry: StoredEntryIdentity,
    pub binding_id: String,
    pub binding_generation: i64,
    pub user_id: String,
    pub nyxbot_agent_id: String,
}
|record| record.platform == super::platform_entry_projection::entry_platform(&record.entry)
    && super::platform_entry_projection::entry_matches(&record.entry, &record.manager_bot_id)
}

/// An event receives fresh admission; it never reuses an ingress receipt.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlatformOriginSource {
    Ingress {
        receipt_id: String,
        claim_id: String,
    },
    ThreadEvent {
        admission_id: String,
    },
}

checked_identity_record! {
/// Exact server-authored execution and delivery proof. Never accepted from an
/// owner request or projected wholesale into an HTTP response.

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct PlatformTurnOrigin, RawPlatformTurnOrigin {
    pub thread: PlatformThread,
    pub manager_authority_epoch: i64,
    pub personal_control_revision: i64,
    #[serde(flatten, with = "super::platform_entry_projection::ingress_sender")]
    pub sender: StoredSenderIdentity,
    pub source: PlatformOriginSource,
}
|record| same_version(&record.thread.entry, &record.sender, None)
}

/// Delivery authority is independent of a later continuation's execution.
/// Web-only watches must not acquire a destination from a newer link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlatformReplyRouting {
    WebOnly,
    Captured {
        origin: Box<PlatformTurnOrigin>,
        turn_id: String,
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        expires_at: DateTime<Utc>,
    },
}

impl std::fmt::Debug for PlatformThread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformThread")
            .field("platform", &self.platform)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PlatformTurnOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformTurnOrigin").finish_non_exhaustive()
    }
}

/// One tool call made with the chat's key during a live turn. Metadata only:
/// the label is a tool identifier, never arguments, results, or secrets.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnActivity {
    #[serde(default)]
    pub machine: Option<Box<super::machine_receipt::MachineReceipt>>,
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub digest: Option<String>,
    /// `running`, `completed`, or `error`.
    pub status: String,
    #[serde(default)]
    pub failure: Option<ToolFailure>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub started_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolFailure {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
}

/// Metadata for an image a tool returned during a turn; the bytes live in
/// `assistant_attachments`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnAttachment {
    /// Whether this tool image may accompany a reply sent to a channel chat.
    #[serde(default)]
    pub chat_delivery: Option<bool>,
    #[serde(default)]
    pub image_input: Option<String>,
    /// None preserves legacy automatic inspection. False requests display only.
    #[serde(default)]
    pub image_inspection: Option<bool>,
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub pages: Option<usize>,
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
pub struct ModelRefusal {
    pub code: String,
    pub attempt_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelStreamFault {
    pub kind: String,
    pub attempt_id: String,
    pub attributable: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineWorkerInstance {
    pub id: String,
    #[serde(default)]
    pub hostname: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveTurn {
    /// Bounded resource handles issued in this turn.
    #[serde(default)]
    pub resource_refs: Vec<String>,
    /// Process incarnation that claimed execution; absent on older replicas' turns.
    #[serde(default)]
    pub worker_instance: Option<EngineWorkerInstance>,
    #[serde(default)]
    pub model_attempt_id: Option<String>,
    #[serde(default)]
    pub model_requests_active: u32,
    #[serde(default)]
    pub model_attribution_uncertain: bool,
    #[serde(default)]
    pub model_refusal: Option<ModelRefusal>,
    #[serde(default)]
    pub model_stream_fault: Option<ModelStreamFault>,
    #[serde(default)]
    pub model_stream_cancellation: Option<ModelStreamFault>,
    #[serde(default)]
    pub waiting_for_capacity: bool,
    /// NyxID-authored display note only; never a transcript/model message.
    #[serde(default)]
    pub capacity_wait_note_origin: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub engine_request_started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub engine_attempts: u32,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub capacity_wait_started_at: Option<DateTime<Utc>>,
    /// Accumulated completed admission waits; excludes execution between requests.
    #[serde(default)]
    pub capacity_wait_ms: u64,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub capacity_wait_deadline: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub capacity_accepted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub capacity_notice_claimed: bool,
    #[serde(default)]
    pub platform_reply: Option<PlatformReplyRouting>,
    /// One platform worker may execute an admitted turn. A lost worker is
    /// settled by orphan recovery; its execution claim is never reopened.
    #[serde(default)]
    pub execution_started: bool,
    #[serde(default)]
    pub machine_scopes: Vec<MachineTurnScope>,
    #[serde(default)]
    pub platform_origin: Option<PlatformTurnOrigin>,
    #[serde(default)]
    pub channel_adapter_origin: Option<super::channel_adapter::TurnAuthority>,
    /// The failed turn this turn retries (owner Retry only).
    #[serde(default)]
    pub retry_of: Option<String>,
    /// New turns count every dispatched tool call, including entries evicted
    /// from the bounded activity display. Absent on legacy turns.
    #[serde(default)]
    pub activity_count: Option<i64>,
    /// Canonical call digests the retried turn completed. A repeated call
    /// with the same tool and arguments is refused (`retry_duplicate_action`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retry_digests: Vec<String>,
    /// The running response, not the previous committed response head.
    #[serde(default)]
    pub running_response: Option<Box<RunningResponse>>,
    /// Server-only event binding for durable channel answer delivery.
    #[serde(default)]
    pub channel_event_id: Option<String>,
    #[serde(default)]
    pub initiating_message_seq: Option<i64>,
    #[serde(default)]
    pub voice_request_id: Option<String>,
    /// Nodes admitted by this turn; Stop uses the durable list across replicas.
    #[serde(default)]
    pub machine_node_ids: Vec<String>,
    #[serde(default)]
    pub continuations: u32,
    #[serde(default)]
    pub activity_total: i64,
    /// Rolling hashes only; never persist tool arguments or results. Reset per continuation.
    #[serde(default)]
    pub tool_progress: ToolProgress,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    /// Worker liveness marker. Absent on turns written before heartbeat support.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub heartbeat_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub trigger_run_id: Option<String>,
    /// Specialist threads: the NyxBot thread this turn reports to, decided
    /// once at admission; ambiguity resolves to `None`.
    #[serde(default)]
    pub report_to: Option<String>,
    /// Group turns: the owner's NyxBot was a member of this personal group at
    /// admission (org groups: never).
    #[serde(default)]
    pub with_nyxbot: bool,
    /// Confirmation continuation claims ignore the original run grace deadline.
    #[serde(default)]
    pub trigger_continuation: bool,
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

/// Runtime identity captured before a machine operation is issued.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineTurnScope {
    pub node_id: String,
    pub runtime_id: String,
}

/// Absent on old rows: their existing title is final until the owner renames it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TitleSource {
    Provisional,
    Generated,
    #[default]
    User,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantConversation {
    #[serde(default)]
    pub platform_thread: Option<PlatformThread>,
    /// Internal conversion fence; typed settlement must preserve it until exact cleanup completes.
    #[serde(default)]
    pub platform_cleanup_pending: bool,
    #[serde(default)]
    pub platform_cleanup_id: Option<String>,
    /// Human opt-in only; command previews are encrypted and expire within 30 days.
    #[serde(default)]
    pub machine_previews: bool,
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub title: String,
    #[serde(default)]
    pub title_source: TitleSource,
    pub model: String,
    #[serde(default)]
    pub access_mode: AccessMode,
    pub nyxagent_session_id: Option<String>,
    /// Server-only instruction state, committed with the upstream session.
    /// Older rows/replicas may omit it. Never expose its marker in a DTO or Debug.
    #[serde(default)]
    pub nyxagent_instruction_binding: Option<InstructionBinding>,
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
    /// Voice task threads are private execution contexts for a visible call.
    /// Trigger automation threads intentionally leave this absent.
    #[serde(default)]
    pub voice_parent_conversation_id: Option<String>,
    /// Stable agent owner binding; the conversation itself belongs to the person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_owner_id: Option<String>,
    /// L1 enrollment cohort. Missing or zero-valued legacy rows are not
    /// eligible evidence; new rows set this only when learning is enabled and
    /// the owner/member has opted in.
    #[serde(default)]
    pub learning_epoch: Option<i64>,
    /// Legacy assignment marker written for old readers only: the NyxBot
    /// thread that assigned the newest work. New code reads the turn snapshot
    /// (`ActiveTurn::report_to`), never this field.
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
    #[serde(default)]
    pub group_request_id: Option<String>,
    /// The newest turn was started by someone other than the owner, e.g. a
    /// member of a group chat. Its tool calls are restricted (NyxBot: none;
    /// specialists: their grants, as far as the owner's guest access for each
    /// service allows). Kept after the turn settles so late tool calls stay
    /// restricted.
    #[serde(default)]
    pub guest_turn: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InstructionBinding {
    pub session_id: String,
    pub fingerprint: String,
    /// None when adopting a legacy session whose initial context had no marker.
    pub marker: Option<String>,
    pub guest: bool,
}

impl std::fmt::Debug for InstructionBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstructionBinding").finish_non_exhaustive()
    }
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

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningResponse {
    /// Key generation that owns this response; server-only.
    #[serde(default)]
    pub credential_api_key_id: String,
    pub response_id: String,
    pub session_id: String,
}

impl std::fmt::Debug for RunningResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningResponse")
            .field("response_id", &self.response_id)
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}
