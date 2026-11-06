//! Bounded assignment receipts; no instruction, result body or credential data.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
pub const COLLECTION_NAME: &str = "assistant_direct_handoffs";

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub state: String,
    pub assignment: Option<super::assistant_delegated_image::Assignment>,
    pub work: super::trigger_run::AutomationWork,
    pub parent_work: super::trigger_run::AutomationWork,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
