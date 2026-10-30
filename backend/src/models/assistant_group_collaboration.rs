//! New collaboration payloads and side collections. No legacy enum is extended.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const INVITES: &str = "assistant_group_invites";
pub const AGENT_ACCESS: &str = "assistant_group_agent_access";
pub const TRANSFERS: &str = "assistant_group_transfers";
pub const SLOTS: &str = "assistant_group_execution_slots";
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collaboration {
    pub version: u32,
    pub creator_user_id: String,
    pub membership_revision: i64,
}

/// These variants appear only inside new, optional payloads/collections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Owner,
    OrgMember,
    Shared,
    Guest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Authority {
    pub owner_id: String,
    pub lane: Lane,
    /// Sharing-policy revision or exact group/agent guest opt-in revision.
    pub revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestBinding {
    pub version: u32,
    pub membership_revision: i64,
    /// Captured on queue admission. Revalidate; never silently switch lanes.
    pub authorities: BTreeMap<String, Authority>,
    #[serde(default)]
    pub admitted_agent_ids: Vec<String>,
    #[serde(default)]
    pub refused_agent_ids: Vec<String>,
    #[serde(default)]
    pub published_turn_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageBinding {
    pub version: u32,
    pub original_requester_id: Option<String>,
    pub execution: Option<Authority>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Invite {
    #[serde(rename = "_id")]
    pub id: String,
    pub group_id: String,
    pub issuer_user_id: String,
    pub token_hash: String,
    pub intended_user_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub accepted_at: Option<DateTime<Utc>>,
    pub accepted_by_user_id: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub cancelled_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for Invite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupInvite").finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentAccess {
    #[serde(rename = "_id")]
    pub id: String,
    pub group_id: String,
    pub agent_id: String,
    pub owner_id: String,
    pub added_by_user_id: String,
    pub membership_generation: i64,
    #[serde(default)]
    pub guest_enabled: bool,
    pub consenting_owner_id: Option<String>,
    pub revision: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transfer {
    #[serde(rename = "_id")]
    pub group_id: String,
    pub from_user_id: String,
    pub to_user_id: String,
    pub membership_revision: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

/// One requester/lane slot, distinct from legacy group-thread indexes. Runtime
/// threads use this side binding rather than sharing an owner's home session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionSlot {
    #[serde(rename = "_id")]
    pub id: String,
    pub group_id: String,
    pub agent_id: String,
    pub original_requester_id: String,
    /// `authenticated` or `guest`; owner/shared/org all use requester keys.
    pub lane: String,
    pub conversation_id: Option<String>,
    pub membership_revision: i64,
    pub authority: Authority,
    pub revoked: bool,
}

/// Server-authored binding shared by the hidden thread and its live turn. The
/// ordinary key's group marker selects this binding through the exact thread key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionBinding {
    pub version: u32,
    pub group_id: String,
    pub slot_id: String,
    pub agent_id: String,
    pub original_requester_id: String,
    pub request_id: String,
    pub membership_revision: i64,
    pub authority: Authority,
}
