use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_agents";

/// Bounds for an agent's durable memory.
pub const MAX_MEMORY_NOTES: usize = 50;
pub const MAX_MEMORY_NOTE_CHARS: usize = 500;
pub const MAX_DISPLAY_NAME_CHARS: usize = 40;
pub const MAX_PERSONA_CHARS: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    /// The owner's single personal agent and chief of staff. Full access.
    Nyxbot,
    /// A persistent specialist created by the owner or by NyxBot. Acts only
    /// with its grants and asks NyxBot for anything else.
    Specialist,
}

/// Authority granted to a specialist. Every thread key of the agent mirrors
/// these fields; they are re-applied at every turn start.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentGrants {
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

/// What guests may do with one of a specialist's services. Operations the
/// owner put behind approval stay the owner's at every level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuestAccess {
    /// Reads only: an operation its stored catalog contract marks
    /// read-only, else GET, HEAD or OPTIONS.
    Read,
    /// Reads, creates and acts (GET, HEAD, OPTIONS, POST), never changes or
    /// removes what exists (PUT, PATCH, DELETE, or an operation its spec
    /// marks as deleting or replacing data).
    #[default]
    Use,
    /// Everything the specialist may do with the service.
    All,
}

impl GuestAccess {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Use => "use",
            Self::All => "all",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Self::Read),
            "use" => Some(Self::Use),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

/// One thing the agent chose to remember. Agent-authored, bounded, never
/// secrets; shown to the owner, who can delete it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryNote {
    pub id: String,
    pub text: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// A persistent NyxBot agent. Conversations are its threads (web chats and
/// channel chats); its identity, grants and memory are shared across them.
#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantAgent {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub kind: AgentKind,
    /// Display name. Specialists use a short unique slug-like name.
    pub name: String,
    /// A specialist's role and scope (bounded, shown to the owner).
    #[serde(default)]
    pub description: String,
    /// Optional specialty label for profile routing.
    #[serde(default)]
    pub specialty: Option<String>,
    #[serde(default)]
    pub grants: AgentGrants,
    /// What people other than the owner (guests in the chats a specialist
    /// answers) may do with each granted service, by service ID. A service
    /// without an entry uses [`GuestAccess::Use`]. Beside `grants`, not in it,
    /// so writers of `grants` that predate it never erase it.
    #[serde(default)]
    pub guest_access: BTreeMap<String, GuestAccess>,
    /// Beside grants so older replicas rewriting service grants retain these.
    #[serde(default)]
    pub machine_node_ids: Vec<String>,
    #[serde(default)]
    pub saved_login_ids: Vec<String>,
    /// `user` or `nyxbot`.
    pub created_by: String,
    /// NyxAgent profile for new threads.
    pub model: String,
    /// Where NyxID delivers this agent's events when no thread asked for them.
    #[serde(default)]
    pub home_conversation_id: Option<String>,
    #[serde(default)]
    pub memory: Vec<MemoryNote>,
    /// A friendly name the user chose ("Luna"); `name` stays the @handle.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Personality and tone the user asked for. Style only, never authority.
    #[serde(default)]
    pub persona: Option<String>,
    /// Destroyed agents keep read-only threads and cannot act.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub destroyed_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl AssistantAgent {
    pub fn is_nyxbot(&self) -> bool {
        self.kind == AgentKind::Nyxbot
    }
}

impl std::fmt::Debug for AssistantAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantAgent")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
