//! Sharing is separate from owner grants; old grant writers cannot widen it.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::assistant_agent::{GuestAccess, MemoryNote};

pub const COLLECTION_NAME: &str = "assistant_agent_sharing";
pub const SHARES_COLLECTION_NAME: &str = "assistant_agent_shares";
pub const NOTES_COLLECTION_NAME: &str = "assistant_agent_private_notes";
pub const CONSENTS_COLLECTION_NAME: &str = "assistant_agent_consents";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareRole {
    User,
    Manager,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareSubject {
    Person,
    Org,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentShare {
    #[serde(rename = "_id")]
    pub id: String,
    pub agent_id: String,
    pub owner_id: String,
    pub subject_kind: ShareSubject,
    pub subject_id: String,
    /// The identity label shown to the owner when this share was created.
    /// This is additive and absent on legacy rows; it is never exposed to a
    /// recipient or used as an authorization key.
    #[serde(default)]
    pub subject_email: Option<String>,
    pub role: ShareRole,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AgentSharing {
    /// The agent UUID is also this policy's unique identity.
    #[serde(rename = "_id")]
    pub agent_id: String,
    pub owner_id: String,
    #[serde(default)]
    pub enabled: bool,
    /// Optimistic-concurrency version for every non-no-op policy write.
    #[serde(default)]
    pub version: i64,
    /// Authority revision bound into shared keys, consents and fences. This
    /// changes only when an existing recipient's authority changes (or when
    /// sharing is enabled for the first time).
    #[serde(default)]
    pub revision: i64,
    /// Absence means withheld; do not use GuestAccess::default here.
    #[serde(default)]
    pub shared_access: BTreeMap<String, GuestAccess>,
    #[serde(default)]
    pub memory: Vec<MemoryNote>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for AgentSharing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentSharing")
            .field("version", &self.version)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PrivateNotes {
    #[serde(rename = "_id")]
    pub id: String,
    pub agent_id: String,
    pub actor_user_id: String,
    pub revision: i64,
    #[serde(default)]
    pub notes: Vec<MemoryNote>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentDecision {
    Pending,
    Once,
    Always,
    Skip,
    Consumed,
    Revoked,
}

/// No input body, secret, account token or credential ciphertext is stored here.
#[derive(Clone, Serialize, Deserialize)]
pub struct AgentConsent {
    #[serde(rename = "_id")]
    pub id: String,
    pub actor_user_id: String,
    pub agent_id: String,
    pub owner_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub api_key_id: String,
    pub connection_id: String,
    pub catalog_service_id: String,
    pub policy_revision: i64,
    pub behavior_digest: String,
    pub authority_digest: String,
    pub operation_digest: String,
    /// Stable pending-card identity. New collection only; defaulted for drafts.
    #[serde(default)]
    pub scope_key: String,
    #[serde(default)]
    pub stateless_model: bool,
    #[serde(default)]
    pub operation_summary: String,
    pub decision: ConsentDecision,
    #[serde(default)]
    pub acknowledgement_id: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub claim_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for AgentConsent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentConsent")
            .field("decision", &self.decision)
            .finish_non_exhaustive()
    }
}
