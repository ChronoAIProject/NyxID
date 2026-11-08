//! Source references only. Provider capabilities remain encrypted on the event.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "execution_permission_deliveries";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelSource {
    pub version: u32,
    pub event_id: String,
    pub actor_user_id: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub key_id: String,
    pub original_turn_id: String,
    pub channel_id: String,
    pub thread_id: String,
    pub settings_thread_id: String,
    pub channel_revision: i64,
    pub thread_revision: i64,
    pub audience_id: String,
    /// Bounded display-only snapshot; never used for source authorization.
    #[serde(default)]
    pub audience_label: String,
    pub policy_digest: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
impl std::fmt::Debug for ChannelSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChannelSource { [REDACTED] }")
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Delivery {
    #[serde(rename = "_id")]
    pub id: String,
    /// Keyed identity of the original source/task epoch and settled turn.
    pub occurrence_key: String,
    pub version: u32,
    pub user_id: String,
    pub conversation_id: String,
    pub message_id: String,
    pub turn_id: String,
    pub task: super::execution_permission::TaskAdmission,
    pub state: String,
    pub claim_id: Option<String>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
impl std::fmt::Debug for Delivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionDelivery")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}
