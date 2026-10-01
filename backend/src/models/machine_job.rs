use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "machine_jobs";

/// Server-issued execution authority. No command, output, or gateway token.
#[derive(Clone, Deserialize, Serialize)]
pub struct MachineJob {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub node_id: String,
    #[serde(default)]
    pub runtime_id: String,
    pub conversation_id: String,
    pub api_key_id: String,
    pub agent_id: String,
    pub state: String,
    /// Immutable issue-time service identities. Legacy jobs receive no services.
    #[serde(default)]
    pub services: Vec<DeclaredService>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub finished_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for MachineJob {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MachineJob { [REDACTED] }")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeclaredService {
    pub id: String,
    pub slug: String,
}
