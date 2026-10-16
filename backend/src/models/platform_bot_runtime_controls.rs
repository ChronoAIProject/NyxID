use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{credits::Credits, platform_personal_binding::PersonalActiveTurn};

pub const COLLECTION_NAME: &str = "platform_bot_runtime_controls";
pub const STOP_EFFECTS: &str = "platform_personal_stop_effects";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicRuntimePolicy {
    pub sponsor_owner_user_id: String,
    pub nyxagent_profile: String,
    #[serde(with = "crate::models::credits::whole")]
    pub total_daily_credits: Credits,
    #[serde(with = "crate::models::credits::whole")]
    pub sender_daily_credits: Credits,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlatformBotRuntimeControls {
    #[serde(rename = "_id")]
    pub id: String,
    pub contract_version: u32,
    pub platform_config_id: String,
    pub manager_bot_id: String,
    pub telegram_bot_id: i64,
    pub manager_authority_epoch: i64,
    pub personal_control_revision: i64,
    pub public_policy_revision: i64,
    pub personal_enabled: bool,
    pub public_enabled: bool,
    pub emergency_stopped: bool,
    pub retired: bool,
    pub public_policy: Option<PublicRuntimePolicy>,
    pub effect_revision: i64,
    pub cleanup_pending: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactMachineJob {
    pub node_id: String,
    pub runtime_id: String,
    pub job_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactMachineScope {
    pub node_id: String,
    pub runtime_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopSettlementState {
    Pending,
    Settled,
    AlreadySettled,
    ConversationDeleted,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineCancelState {
    Cancelled,
    AlreadyFinished,
    NodeUnavailable,
    Unsupported,
    RuntimeChanged,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MachineJobProgress {
    pub job: ExactMachineJob,
    pub outcome: MachineCancelState,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MachineScopeProgress {
    pub scope: ExactMachineScope,
    pub outcome: MachineCancelState,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalStopProgress {
    pub settlement: StopSettlementState,
    pub machine_jobs: Vec<MachineJobProgress>,
    pub machine_scopes: Vec<MachineScopeProgress>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformPersonalStopEffect {
    #[serde(rename = "_id")]
    pub id: String,
    pub active_turn: PersonalActiveTurn,
    pub machine_jobs: Vec<ExactMachineJob>,
    pub machine_scopes: Vec<ExactMachineScope>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub stop_requested_at: DateTime<Utc>,
    pub progress: Option<PersonalStopProgress>,
    pub completed: bool,
    pub claim_id: Option<String>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub retry_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl std::fmt::Debug for PlatformPersonalStopEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformPersonalStopEffect")
            .field("completed", &self.completed)
            .finish_non_exhaustive()
    }
}
