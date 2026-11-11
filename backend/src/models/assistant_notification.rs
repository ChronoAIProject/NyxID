//! NyxBot Notifications: one durable row per update the acting person did
//! not watch happen. Rows hold identifiers, kind, counts, timestamps,
//! read/resolved state and push state only; preview text is computed at
//! list time from the referenced source after live ownership checks.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_notifications";

/// Closed kinds; stored as `snake_case` strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    AutomationResult,
    AutomationProblem,
    NeedsDecision,
    AgentUpdate,
    ChatUpdate,
    GroupMention,
}

impl NotificationKind {
    pub const ALL: [NotificationKind; 6] = [
        Self::AutomationResult,
        Self::AutomationProblem,
        Self::NeedsDecision,
        Self::AgentUpdate,
        Self::ChatUpdate,
        Self::GroupMention,
    ];

    /// Stable BSON/API spelling, shared with the serde representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AutomationResult => "automation_result",
            Self::AutomationProblem => "automation_problem",
            Self::NeedsDecision => "needs_decision",
            Self::AgentUpdate => "agent_update",
            Self::ChatUpdate => "chat_update",
            Self::GroupMention => "group_mention",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    /// Kinds that keep at most one unread row per (person, source) and
    /// count further activity on it.
    pub const fn coalesced(self) -> bool {
        matches!(
            self,
            Self::AgentUpdate | Self::ChatUpdate | Self::GroupMention
        )
    }
}

/// Closed source types; stored as `snake_case` strings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    #[default]
    Conversation,
    Group,
    /// A channel-bot chat. `id` is its conversation id for personal routes,
    /// or its public provider thread id for organization routes.
    ChannelChat,
    CollaborationGroup,
    Trigger,
}

impl SourceType {
    pub const ALL: [SourceType; 5] = [
        Self::Conversation,
        Self::Group,
        Self::ChannelChat,
        Self::CollaborationGroup,
        Self::Trigger,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Group => "group",
            Self::ChannelChat => "channel_chat",
            Self::CollaborationGroup => "collaboration_group",
            Self::Trigger => "trigger",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

/// Identifiers of what the row is about. Purge paths delete by these
/// nested fields (`source.conversation_id`, `source.group_id`, ...).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NotificationSource {
    #[serde(rename = "type")]
    pub source_type: SourceType,
    /// Conversation, group, channel chat or trigger identity visible to the recipient.
    pub id: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    /// `nyxbot_channels._id` for channel chats.
    #[serde(default)]
    pub channel_id: Option<String>,
    /// Retirement generation observed by the producer.
    #[serde(default)]
    pub channel_epoch: i64,
    #[serde(default)]
    /// Informational provider binding; transport moves do not invalidate activity.
    pub channel_binding_id: Option<String>,
    /// `nyxbot_threads._id` for channel chats (personal chats and
    /// organization public transcripts).
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Organization channel lanes: `org_guest` (audience is the linking
    /// admin; preview from the public transcript) or `org_member`.
    #[serde(default)]
    pub lane: Option<String>,
    #[serde(default)]
    pub trigger_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Automation rows: the trigger run; decision rows: the run the card belongs to.
    #[serde(default)]
    pub run_id: Option<String>,
    /// Decision rows: the acknowledgement card.
    #[serde(default)]
    pub card_id: Option<String>,
    /// Decision rows: the chat key the card was raised under.
    #[serde(default)]
    pub api_key_id: Option<String>,
    /// Latest referenced transcript message (assistant or group message).
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    /// Group message sequence of the latest referenced message.
    #[serde(default)]
    pub seq: Option<i64>,
    /// Linked platform chats (manager bots): the personal binding the thread
    /// was linked through and its generation. Unlink purges by binding id;
    /// list and summary require the thread to still carry this binding.
    #[serde(default)]
    pub platform_binding_id: Option<String>,
    #[serde(default)]
    pub platform_binding_generation: Option<i64>,
}

/// Durable push barrier. `pending -> sending -> sent | failed | unknown |
/// retrying`; `skipped` records a deliberate non-send. A `sending` claim is
/// never resent; only a conclusive retryable refusal schedules `retrying`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PushState {
    pub state: String,
    /// The kind the row had when the push was queued. Preferences and the
    /// payload follow it even after coalescing relabels the row (D9).
    /// Absent on rows written before this field existed.
    #[serde(default)]
    pub kind: Option<NotificationKind>,
    #[serde(default)]
    pub attempt: Option<String>,
    /// Stable code only, never provider prose.
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub claimed_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub first_attempt_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub retry_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub sent_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantNotification {
    /// UUID v4, or UUID v5 for rows with a deterministic identity (one per
    /// run, pause notice or card) so retries and recovery never duplicate.
    #[serde(rename = "_id")]
    pub id: String,
    /// The acting person this row is for. Never a guest, never another person.
    pub user_id: String,
    pub kind: NotificationKind,
    pub source: NotificationSource,
    /// Canonical visible destination; saved chat/conversation ids share one key.
    pub source_key: String,
    pub coalesced: bool,
    /// Needs the person's attention: neither read nor resolved. Indexed with
    /// `source_key` as the unique partial coalescing key.
    pub unread: bool,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub read_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub resolved_at: Option<DateTime<Utc>>,
    /// `decided`, `expired` or `superseded` for decision rows.
    #[serde(default)]
    pub resolution: Option<String>,
    /// Decision rows: the card's timeout; summary and list exclude timed-out
    /// cards without a join and the sweep resolves them.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub card_expires_at: Option<DateTime<Utc>>,
    pub count: i64,
    /// Stable outcome code (`completed`, `failed`, `cancelled`, `skipped`,
    /// `paused`), never text.
    #[serde(default)]
    pub outcome: Option<String>,
    /// Stable reason or error code, never text.
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub push: Option<PushState>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    /// TTL: `updated_at` plus the retention period.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
