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
