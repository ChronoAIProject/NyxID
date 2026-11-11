//! Native provider replay. Bodies are bounded and envelope encrypted.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "nyxbot_gateway_responses";

#[derive(Clone, Serialize, Deserialize)]
pub struct GatewayResponse {
    #[serde(rename = "_id")]
    pub id: String,
    pub response_id: String,
    pub user_id: String,
    pub channel_id: String,
    pub binding_id: String,
    pub partition: String,
    pub event_key: String,
    pub request_hash: String,
    /// Internal assistant conversation, bound in the same transaction as its turn.
    pub conversation_id: Option<String>,
    pub terminal: bool,
    pub revision: i64,
    #[serde(with = "crate::models::bson_bytes::required")]
    pub payload_ciphertext: Vec<u8>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
