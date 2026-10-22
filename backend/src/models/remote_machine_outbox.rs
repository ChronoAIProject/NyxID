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
    /// Set only when claiming a genuinely pending, never attempted effect.
    /// Delivery bookkeeping on legacy claims cannot recreate this authority.
    #[serde(default)]
    pub undispatched_claim_id: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub claimed_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub available_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub submission: Option<MachineSubmission>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// A timeout or worker lease expiry leaves this original request unresolved.
/// The immutable outbox id retains the original responsibility. Each dispatch
/// has its own result so a retry cannot hide an older request still in flight.
#[derive(Clone, Deserialize, Serialize)]
pub struct MachineSubmission {
    pub request_id: String,
    pub runtime_generation: i64,
    pub credential_epoch: i64,
    pub writer_fence: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub started_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub cancelled_before_dispatch: bool,
    pub result_sandbox_uid: Option<String>,
    pub result_resource_version: Option<String>,
    #[serde(default)]
    pub dispatches: Vec<MachineSubmissionDispatch>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct MachineSubmissionDispatch {
    pub id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub started_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub cancelled_before_dispatch: bool,
    pub result_sandbox_uid: Option<String>,
    pub result_resource_version: Option<String>,
}

impl std::fmt::Debug for MachineSubmissionDispatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MachineSubmissionDispatch")
            .field("identity", &"[REDACTED]")
            .field("started_at", &self.started_at)
            .field("completed_at", &self.completed_at)
            .field("cancelled_before_dispatch", &self.cancelled_before_dispatch)
            .finish()
    }
}

impl std::fmt::Debug for MachineSubmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MachineSubmission")
            .field("identity", &"[REDACTED]")
            .field("started_at", &self.started_at)
            .field("completed_at", &self.completed_at)
            .field("cancelled_before_dispatch", &self.cancelled_before_dispatch)
            .finish()
    }
}
