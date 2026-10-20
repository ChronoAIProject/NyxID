use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "audit_outbox";
pub const PENDING_COLLECTION_NAME: &str = "audit_pending_events";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuditPrincipal {
    Person {
        id: String,
    },
    Node {
        id: String,
    },
    /// Existing Oracle identity: canonical pool UUID + ':' + bounded worker label.
    OracleWorker {
        id: String,
    },
    ServiceAccount {
        id: String,
    },
    /// A key owner can be a person or org; it is never an inferred human actor.
    ApiKey {
        id: String,
        owner_id: String,
    },
    System {
        actor: SystemAuditActor,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SystemAuditActor {
    Startup,
    Maintenance,
    Reconciliation,
    Expiry,
    Delivery,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuditOwner {
    Person { id: String },
    Org { id: String },
    Platform,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditStage {
    Committed,
    Intent,
    Succeeded,
    Failed,
    Cancelled,
    Uncertain,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessModeScope {
    Workspace,
    Agent,
}

/// Global tri-state policy, distinct from the per-service grant policy.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditApprovalMode {
    WritesOnly,
    All,
    Off,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityFlag {
    MfaEnabled,
    EmailVerified,
    SkipDestructiveConfirmation,
    AllowAutoConnectedServices,
    BrokerEnabled,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceSetKind {
    Services,
    Nodes,
    Agents,
    OrgMembers,
}

/// Construct through audit_outbox_service::scope_ident; deserialized storage
/// values are checked against the same server registry before materialization.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct ScopeIdent(pub(crate) String);

/// Closed security decisions only. No user-authored or provider strings.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuditDetail {
    AccessModeChange {
        scope: AccessModeScope,
        /// None means inherited from workspace, only valid at Agent scope.
        old: Option<super::assistant_settings::AccessMode>,
        new: Option<super::assistant_settings::AccessMode>,
    },
    OrgRoleChange {
        old: super::org_membership::OrgRole,
        new: super::org_membership::OrgRole,
    },
    ApprovalModeChange {
        old: AuditApprovalMode,
        new: AuditApprovalMode,
    },
    ServiceApprovalModeChange {
        old: super::service_approval_config::ApprovalMode,
        new: super::service_approval_config::ApprovalMode,
    },
    SecurityFlagChange {
        flag: SecurityFlag,
        old: bool,
        new: bool,
    },
    ScopeSetChange {
        added: Vec<ScopeIdent>,
        removed: Vec<ScopeIdent>,
        truncated: bool,
    },
    ResourceSetChange {
        #[serde(rename = "resource_kind")]
        kind: ResourceSetKind,
        added: Vec<String>,
        removed: Vec<String>,
        truncated: bool,
    },
    ExpiryChange {
        #[serde(with = "super::bson_datetime::optional")]
        old: Option<DateTime<Utc>>,
        #[serde(with = "super::bson_datetime::optional")]
        new: Option<DateTime<Utc>>,
    },
    /// Mongo modified_count, capped at one million; saturated means the actual
    /// count exceeded that cap. Never a request-supplied or content count.
    SessionRevocationCount { count: u64, saturated: bool },
}

/// An allowlisted projection. No arbitrary event JSON, strings from provider
/// responses, credentials, URLs or user-authored content are accepted here.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SafeAuditEvent {
    pub operation_id: String,
    pub stage: AuditStage,
    pub principal: AuditPrincipal,
    pub owner: AuditOwner,
    pub action: String,
    pub target_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<AuditDetail>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingAuditEvent {
    #[serde(rename = "_id")]
    pub id: String,
    pub event: SafeAuditEvent,
    pub order: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub reserved_at: DateTime<Utc>,
    pub materialized_seq: Option<i64>,
}

/// Durable witness delivery reservation. Existing direct transactional audits
/// materialize immediately; their reservation survives until external ack.
/// This record contains only identities and chain evidence, never event data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditOutbox {
    #[serde(rename = "_id")]
    pub id: String,
    pub chain_id: String,
    pub order: i64,
    pub key_version: u32,
    pub seq: i64,
    pub prev_hash: String,
    pub entry_hash: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub reserved_at: DateTime<Utc>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub witnessed_at: Option<DateTime<Utc>>,
}
