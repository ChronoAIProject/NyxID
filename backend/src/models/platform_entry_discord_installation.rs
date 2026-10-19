use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const HISTORY: &str = "platform_discord_control_installation_history";

pub const COLLECTION_NAME: &str = "platform_discord_control_installations";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationState {
    Prepared,
    Dispatched,
    Observed,
}

/// An explicit administrative attempt, independent of subsequent activation.
/// Contains no bot token, signature key, interaction token or user message.
#[derive(Clone, Serialize, Deserialize)]
pub struct DiscordControlInstallation {
    #[serde(rename = "_id")]
    pub id: String,
    pub channel_bot_id: String,
    pub attempt_id: String,
    pub requested_by: String,
    pub credential_fingerprint: String,
    pub platform_bot_id: String,
    pub application_id: String,
    pub schema_digest: String,
    pub owned_command_id: Option<String>,
    pub observed_command_id: Option<String>,
    pub state: InstallationState,
    #[serde(with = "super::bson_datetime::optional")]
    pub dispatch_deadline: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for DiscordControlInstallation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordControlInstallation")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}
