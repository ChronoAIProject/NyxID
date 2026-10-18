use serde::{Deserialize, Serialize};

pub const SELECTIONS: &str = "platform_personal_setup_selections";

#[derive(Clone, Serialize, Deserialize)]
pub struct PersonalSetupSelection {
    #[serde(rename = "_id")]
    pub binding_id: String,
    pub binding_generation: i64,
    pub user_id: String,
    pub setup_id: String,
    pub setup_revision: i64,
    #[serde(default)]
    pub selection_ref: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub purge_after: chrono::DateTime<chrono::Utc>,
}

/// An owned-resource selector, never rendered text or an account identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "view", rename_all = "snake_case")]
pub enum ManagerManagementView {
    Onboarding,
    Setups,
    Setup { setup_id: String },
    Bots,
    Bot { bot_id: String },
}
