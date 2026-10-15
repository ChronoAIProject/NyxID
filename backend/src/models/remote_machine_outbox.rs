use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "machine_effect_outbox";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    CreateSandbox,
    SuspendSandbox,
    ResumeSandbox,
    RekeySandbox,
    DeleteSandbox,
    ReconcileSandbox,
    InstallProviderDeadline,
    UpdateSandboxDeadline,
}

impl EffectKind {
    pub fn for_action(action: super::remote_machine_operation::LifecycleAction) -> Self {
        match action {
            super::remote_machine_operation::LifecycleAction::Create => Self::CreateSandbox,
            super::remote_machine_operation::LifecycleAction::Pause => Self::SuspendSandbox,
            super::remote_machine_operation::LifecycleAction::Resume => Self::ResumeSandbox,
            super::remote_machine_operation::LifecycleAction::Rekey => Self::RekeySandbox,
            super::remote_machine_operation::LifecycleAction::Delete => Self::DeleteSandbox,
            super::remote_machine_operation::LifecycleAction::Reconcile => Self::ReconcileSandbox,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    Pending,
    Claimed,
    Applied,
    Retry,
    Dead,
}

impl EffectState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claimed => "claimed",
            Self::Applied => "applied",
            Self::Retry => "retry",
            Self::Dead => "dead",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MachineEffectOutbox {
    #[serde(rename = "_id")]
    pub id: String,
    pub operation_id: String,
    pub machine_id: String,
    pub owner_id: String,
    pub kind: EffectKind,
    pub effect_digest: String,
    pub namespace: String,
    pub sandbox_name: String,
    pub desired_mode: String,
    pub state: EffectState,
    #[serde(default)]
    pub attempts: i64,
    #[serde(default)]
    pub claim_id: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub claimed_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub available_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
