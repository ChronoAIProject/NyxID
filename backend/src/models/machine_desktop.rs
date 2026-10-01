use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
pub const COLLECTION_NAME: &str = "machine_desktops";

/// Only session metadata is durable. Frames, input and clipboard never are.
#[derive(Clone, Serialize, Deserialize)]
pub struct MachineDesktop {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub display: nyxid_machine::desktop::Display,
    pub session_id: String,
    pub user_id: String,
    pub conversation_id: Option<String>,
    pub status: String,
    #[serde(default)]
    pub revision: i64,
    pub controller: Option<String>,
    pub reason: Option<String>,
    pub handback_note: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub controller_expires_at: Option<DateTime<Utc>>,
}
impl std::fmt::Debug for MachineDesktop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MachineDesktop")
            .field("node_id", &self.node_id)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}
