use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "machine_operations";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleAction {
    Create,
    Pause,
    Resume,
    Rekey,
    Delete,
    Reconcile,
}

impl LifecycleAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Rekey => "rekey",
            Self::Delete => "delete",
            Self::Reconcile => "reconcile",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Accepted,
    Applying,
    Reconciling,
    Succeeded,
    Failed,
}

impl OperationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Applying => "applying",
            Self::Reconciling => "reconciling",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CleanupState {
    NotRequired,
    Pending,
    InProgress,
    Complete,
    Unknown,
}

impl CleanupState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRequired => "not_required",
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Complete => "complete",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeleteRetryState {
    pub attempts: i64,
    #[serde(default)]
    pub claim_id: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub claim_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_request_key_hash: Option<String>,
    #[serde(default)]
    pub last_request_digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MachineOperation {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    #[serde(default)]
    pub machine_id: Option<String>,
    pub action: LifecycleAction,
    pub idempotency_key_hash: String,
    pub request_digest: String,
    pub expected_revision: Option<i64>,
    #[serde(default)]
    pub desired_runtime_generation: Option<i64>,
    pub state: OperationState,
    pub cleanup_state: CleanupState,
    #[serde(default)]
    pub effect_id: Option<String>,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub evidence_digest: Option<String>,
    #[serde(default)]
    pub retry_count: i64,
    #[serde(default)]
    pub delete_retry: Option<DeleteRetryState>,
    #[serde(default, with = "bson_datetime::optional")]
    pub next_attempt_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub deadline_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub terminal_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SafeMachineOperationView {
    pub id: String,
    pub machine_id: Option<String>,
    pub action: LifecycleAction,
    pub state: OperationState,
    pub cleanup_state: CleanupState,
    pub retry_count: i64,
    pub delete_retry_attempts: Option<i64>,
    pub delete_retry_limit: Option<i64>,
    pub delete_retry_stopped: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub evidence_digest: Option<String>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub deadline_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
}
