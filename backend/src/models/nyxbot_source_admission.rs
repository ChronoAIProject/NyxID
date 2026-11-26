//! Versioned local trigger policy for a personal Discord bot connection.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "ids",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum IdentityRule<T> {
    Deny,
    Open,
    Allowlist(Vec<T>),
}

/// The provider's exact conversation kind and ID. A parent channel entry
/// grants only that channel; a thread has its own independent entry.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationId {
    pub id: String,
    pub kind: ConversationKind,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationKind {
    Private,
    Group,
    Channel,
    Thread,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SourceAdmissionInput")]
pub struct DiscordSourceAdmission {
    pub schema_version: i32,
    /// Local rule revision, independent of the CMA channel version.
    pub revision: i64,
    pub enabled: bool,
    /// Platform identity verified for the enclosing connection before authoring.
    pub bot_id: String,
    pub senders: IdentityRule<String>,
    pub chats: IdentityRule<ConversationId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAdmissionInput {
    pub schema_version: i32,
    pub revision: i64,
    pub enabled: bool,
    pub bot_id: String,
    pub senders: IdentityRule<String>,
    pub chats: IdentityRule<ConversationId>,
}
