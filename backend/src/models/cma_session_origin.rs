//! Server-owned attribution, retained independently of async-watch expiry.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
pub const COLLECTION_NAME: &str = "assistant_cma_session_origins";

#[derive(Clone, Serialize, Deserialize)]
pub struct CmaSessionOrigin {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub service_id: String,
    pub remote_agent_id: String,
    pub connection_binding: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub agent_id: String,
    pub callback_url: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
