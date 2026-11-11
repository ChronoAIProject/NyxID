//! Explicit execution consent, stored only as an optional ApprovalGrant extension.
//! No credentials, caller-authored task authority, or new policy language.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::agent_operation_scope::AgentOperationScope;

pub const VERSION: u32 = 1;
/// Deliberately rejected by the legacy v1 approval-scope parser.
pub const LEGACY_SCOPE: &str = "execution:v1";
/// 9999-12-31T23:59:59Z. Storage/TTL sentinel, never permission validity.
pub const PERSISTENT_TTL_SECONDS: i64 = 253_402_300_799;
pub const TERMINAL_RETENTION_SECONDS: i64 = 30 * 24 * 60 * 60;

/// Only operation permissions carry this record. Unknown versions never dispatch.
#[derive(Clone, Serialize, Deserialize)]
pub struct ExecutionUse {
    pub version: u32,
    pub operation_id: String,
    pub prepared_digest: String,
    pub claim_id: String,
    pub state: ExecutionUseState,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub claimed_at: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub dispatched_at: Option<DateTime<Utc>>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for ExecutionUse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionUse")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionUseState {
    Claimed,
    Dispatched,
    Completed,
    NotSent,
    Unknown,
}

/// Unknown versions remain readable for inspection/revocation but cannot cover
/// execution. Future versions must use their own payload, not reinterpret v1.
#[derive(Clone, Serialize, Deserialize)]
pub struct ExecutionPermission {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v1: Option<ExecutionContract>,
}

impl std::fmt::Debug for ExecutionPermission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionPermission")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionContract {
    pub subject: Subject,
    pub binding: ConnectionBinding,
    pub context: ExecutionContext,
    pub operations: AgentOperationScope,
    /// Existing provider policy; requires fresh adapter evidence in addition to
    /// operation/input coverage. Generic input rules never prove Drive ancestry.
    pub resource_policy: Option<nyxid_permissions::Policy>,
    pub validity: Validity,
    pub satisfies: Vec<ExecutionFloor>,
    pub revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subject {
    pub actor_user_id: String,
    pub principal: Principal,
    pub lane: ExecutionLane,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Principal {
    Agent(String),
    ApiKey(String),
    OauthClient(String),
    ServiceAccount(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLane {
    Owner,
    OrgMember,
    Shared,
    Guest,
}

/// Refreshable token bytes/expiry are intentionally absent. This is the exact
/// selected connection and credential replacement identity, not a catalog alias.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionBinding {
    pub connection_id: String,
    pub owner_user_id: String,
    pub credential_id: Option<String>,
    pub credential_epoch: i64,
    pub authority_digest: String,
    pub behavior_digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionContext {
    pub audience: Audience,
    pub conversation: ConversationScope,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Audience {
    Private { user_id: String },
    Group { group_id: String, revision: i64 },
    Channel { binding_id: String, revision: i64 },
}

/// Cross-conversation reuse must be explicit, never inferred from a missing ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConversationScope {
    Exact { conversation_id: String },
    SameAudience,
}

/// Server-only reference to an admitted root. Possession never authenticates a
/// caller or proves a live task; the evaluator also needs trusted root evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRef {
    pub root_kind: TaskRootKind,
    pub root_id: String,
}

/// Persisted on existing roots and their server-admitted descendants only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskAdmission {
    /// Verified original channel source, inherited only through server task lineage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_source: Option<Box<super::execution_permission_channel::ChannelSource>>,
    pub version: u32,
    pub task: TaskRef,
    pub actor_user_id: String,
    pub audience: Audience,
    pub epoch: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub hard_deadline: DateTime<Utc>,
    pub cancelled: bool,
    pub parent_turn_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskRootKind {
    Message,
    Invocation,
    GroupRequest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Validity {
    Operation {
        operation_id: String,
        prepared_digest: String,
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        expires_at: DateTime<Utc>,
    },
    Task {
        task: TaskRef,
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        hard_deadline: DateTime<Utc>,
    },
    Window {
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        starts_at: DateTime<Utc>,
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        ends_at: DateTime<Utc>,
    },
    Persistent,
}

/// Closed v1 floor vocabulary. Only the first three may be satisfied by this
/// permission. All remaining gates/claims retain their separate contracts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionFloor {
    AccessModeRisk,
    PersonalServiceApproval,
    SharedConnectionConsent,
    WebhookAutomation,
    MachineConfirm,
    SavedLoginEachSignIn,
    DelegationWidening,
    /// Existing org approval policy is not personal execution consent.
    OrgServiceApproval,
    OrgInstruction,
    OrgMandatoryReview,
    ExactClaim,
    DurableClaim,
    Guest,
}
