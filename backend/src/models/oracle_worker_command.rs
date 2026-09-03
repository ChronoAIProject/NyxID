use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "oracle_worker_commands";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OracleWorkerCommandKind {
    Drain,
    Resume,
    Restart,
    RelaunchBrowser,
    Relogin,
    Upgrade,
    SessionImport,
}

impl OracleWorkerCommandKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Drain => "drain",
            Self::Resume => "resume",
            Self::Restart => "restart",
            Self::RelaunchBrowser => "relaunch_browser",
            Self::Relogin => "relogin",
            Self::Upgrade => "upgrade",
            Self::SessionImport => "session_import",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OracleWorkerCommandStatus {
    Queued,
    Delivered,
    Succeeded,
    Failed,
    Expired,
    /// Withdrawn by a manager before the worker executed it.
    Cancelled,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OracleWorkerCommand {
    #[serde(rename = "_id")]
    pub id: String,
    pub pool_id: String,
    pub worker_label: String,
    pub kind: OracleWorkerCommandKind,
    pub status: OracleWorkerCommandStatus,
    pub created_by_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_capability: Option<String>,
    #[serde(default)]
    pub delivery_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_sha256: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub delivered_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub delivery_lease_expires_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub deadline_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for OracleWorkerCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OracleWorkerCommand")
            .field("id", &self.id)
            .field("pool_id", &self.pool_id)
            .field("worker_label", &self.worker_label)
            .field("kind", &self.kind)
            .field("status", &self.status)
            .field("delivery_count", &self.delivery_count)
            .field("result_code", &self.result_code)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_kind_wire_names_are_stable() {
        assert_eq!(
            OracleWorkerCommandKind::RelaunchBrowser.as_str(),
            "relaunch_browser"
        );
        assert_eq!(
            OracleWorkerCommandKind::SessionImport.as_str(),
            "session_import"
        );
    }
}
