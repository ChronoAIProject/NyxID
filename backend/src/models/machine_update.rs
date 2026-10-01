use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const ATTEMPTS_COLLECTION_NAME: &str = "machine_update_attempts";

pub const COLLECTION_NAME: &str = "machine_updates";

/// One owner policy and current update attempt per node. Audit retains history.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MachineUpdate {
    #[serde(rename = "_id")]
    pub node_id: String,
    pub user_id: String,
    pub automatic: bool,
    pub attempt_id: Option<String>,
    pub requested_by: Option<String>,
    pub previous_version: Option<String>,
    pub target_version: Option<String>,
    pub phase: String,
    pub code: Option<String>,
    pub notify_pending: bool,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub requested_at: Option<DateTime<Utc>>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub deadline: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl MachineUpdate {
    pub fn new(node_id: &str, owner: &str) -> Self {
        Self {
            node_id: node_id.into(),
            user_id: owner.into(),
            automatic: false,
            attempt_id: None,
            requested_by: None,
            previous_version: None,
            target_version: None,
            phase: "idle".into(),
            code: None,
            notify_pending: false,
            requested_at: None,
            deadline: None,
            updated_at: Utc::now(),
        }
    }

    pub fn pending(&self) -> bool {
        matches!(
            self.phase.as_str(),
            "manual_step" | "queued" | "verifying" | "downloading" | "restarting"
        )
    }
}

/// Terminal snapshot retained while reconnect watches may still be delivered.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MachineUpdateAttempt {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub state: MachineUpdate,
}
