use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::channel_guest_policy::GuestDataRule;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SetupAudience {
    OwnerOnly,
    Selected { guest_ids: Vec<String> },
    ChannelPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SkillDeclaration {
    pub version: u8,
    pub skill_id: String,
    pub skill_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SkillDeclarationFile {
    pub version: u8,
    pub rules: Vec<GuestDataRule>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetupConfiguration {
    pub audience: Option<SetupAudience>,
    #[serde(default)]
    pub rules: Vec<GuestDataRule>,
    #[serde(default)]
    pub declarations: Vec<SkillDeclaration>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PreparedSetup {
    pub draft_revision: i64,
    pub maintainer_id: String,
    pub ownership_version: i64,
    pub skill_rules: Vec<GuestDataRule>,
    pub skill_selection: Option<crate::services::channel_guest_access::skills::PreparedSelection>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct GuestSetup {
    #[serde(rename = "_id")]
    pub bot_id: String,
    pub owner_id: String,
    pub revision: i64,
    pub draft: Option<SetupConfiguration>,
    #[serde(default)]
    pub prepared: Option<PreparedSetup>,
    #[serde(default)]
    pub pending_activation: bool,
    pub applied: Option<SetupConfiguration>,
    pub applied_agent_id: Option<String>,
    pub applied_binding_id: Option<String>,
    pub policy_revision: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl GuestSetup {
    pub const COLLECTION_NAME: &'static str = "channel_guest_setup";
}
