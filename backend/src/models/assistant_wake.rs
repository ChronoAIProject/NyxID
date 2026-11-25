//! References for durable continuations. Content is rendered at turn admission.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WakeReference {
    Watch(Box<WatchWake>),
    GroupFollower {
        group_id: String,
        since_seq: i64,
        upper_seq: i64,
    },
    ChannelHealth {
        channel_id: String,
        agent_id: String,
        reason: String,
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        observed_at: DateTime<Utc>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WatchClaim {
    pub id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WatchWake {
    pub watch_id: String,
    pub claim_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub ready_at: DateTime<Utc>,
    pub watch_kind: String,
    pub source_id: Option<String>,
    pub bot_id: Option<String>,
    pub channel_id: Option<String>,
    /// Server-computed at hosted settlement; never supplied by the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_callback: Option<String>,
    pub agent_id: Option<String>,
    pub reason: Option<String>,
    pub source_status: Option<String>,
    pub source_turn_id: Option<String>,
    pub source_connect_link_id: Option<String>,
    pub conversation_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
