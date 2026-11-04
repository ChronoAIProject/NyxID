use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_direct_handoffs";

/// Receipt for one admitted specialist turn. It carries notification authority,
/// never the coordinator's execution/trigger authority.
#[derive(Clone, Serialize, Deserialize)]
pub struct DirectHandoff {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub run_id: String,
    pub parent_thread_id: String,
    pub parent_turn_id: String,
    pub child_thread_id: String,
    pub child_turn_id: String,
    pub agent_id: String,
    pub invocation_id: Option<String>,
    pub state: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub retry_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
