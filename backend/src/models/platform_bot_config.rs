use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_bot_configs";
pub const CLEANUP_COLLECTION: &str = "platform_bot_cleanup";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlatformBotCleanup {
    #[serde(rename = "_id")]
    pub id: String,
    pub platform_config_id: String,
    pub kind: String,
    pub resource_id: String,
    pub owner_user_id: String,
    #[serde(default)]
    pub capture_version: u32,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub async_operation_ids: Vec<String>,
    #[serde(default)]
    pub machine_jobs: Vec<CleanupMachineJob>,
    #[serde(default)]
    pub machine_node_ids: Vec<String>,
    #[serde(default)]
    pub gateway_channel_id: Option<String>,
    #[serde(default)]
    pub gateway_version: Option<i64>,
    pub completed: bool,
    pub claim_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub retry_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CleanupMachineJob {
    pub id: String,
    pub node_id: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlatformExecutionProfile {
    PlatformOnboarding,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlatformBotConfig {
    #[serde(rename = "_id")]
    pub id: String,
    pub channel_bot_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_transport: Option<PlatformChannelTransport>,
    pub manager_bot_id: String,
    pub observation_id: String,
    pub execution_profile: PlatformExecutionProfile,
    pub generation: i64,
    pub enabled: bool,
    #[serde(default)]
    pub guide: Option<GuideConfiguration>,
    /// Legacy inference sponsor. FAQ profiles may use the configuring person as an inert default; no inference is charged.
    pub funding_owner_user_id: String,
    pub configured_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlatformChannelTransport {
    pub platform: String,
    pub platform_bot_id: String,
    pub owner_user_id: String,
    pub ownership_version: i64,
    /// Internal numeric authority handle retained by the original proof format.
    pub authority_id: i64,
    #[serde(default)]
    pub credential_stamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuideConfiguration {
    pub model_service_id: String,
    pub model_id: String,
    pub limits: super::platform_setup::RuntimeLimits,
    #[serde(with = "crate::models::credits::whole")]
    pub anonymous_daily_credits: super::credits::Credits,
    pub platform_turns_per_hour: u32,
    pub platform_concurrent_turns: u32,
}
