//! Independent CMAEG custom-event channels.
//!
//! These rows deliberately do not reuse `NyxbotChannel`: a custom source has
//! no chat-platform bot, and its producer credential must never become a
//! provider or chat credential.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "nyxbot_event_bindings";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NyxbotEventBinding {
    #[serde(rename = "_id")]
    pub id: String,
    /// Person who created the binding and whose live authority is rechecked
    /// before every provider event.
    pub user_id: String,
    pub agent_id: String,
    /// The agent's owning person or organisation at creation. This is an
    /// identity hint only; ACL is resolved live from the agent on each use.
    pub agent_owner_id: String,
    /// Existing chat row and partition selected by the owner for delivery.
    pub destination_channel_id: String,
    pub destination_partition: String,
    /// `per_event` is the only policy currently emitted. The field is
    /// additive so a later explicit shared policy can be introduced safely.
    #[serde(default = "default_partition_policy")]
    pub partition_policy: String,
    #[serde(default)]
    pub provider_binding_id: Option<String>,
    #[serde(default)]
    pub provider_api_key_id: Option<String>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub provider_key_ciphertext: Option<Vec<u8>>,
    /// CMA's custom-source key. It is returned only once to the owner and is
    /// retained encrypted so a gateway reconciliation can replay the exact
    /// channel definition. It is never returned to a producer through a
    /// provider endpoint.
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub producer_secret_ciphertext: Option<Vec<u8>>,
    #[serde(default = "default_producer_key_id")]
    pub producer_key_id: String,
    #[serde(default)]
    pub gateway_channel_id: Option<String>,
    #[serde(default)]
    pub custom_event_url: Option<String>,
    #[serde(default)]
    pub gateway_record_id: Option<String>,
    #[serde(default)]
    pub gateway_version: Option<i64>,
    /// `preparing`, `ready`, `failed`, or `revoked`.
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub revoked_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

fn default_partition_policy() -> String {
    "per_event".into()
}

fn default_producer_key_id() -> String {
    "nyxid-producer".into()
}

fn default_status() -> String {
    "preparing".into()
}

impl NyxbotEventBinding {
    pub fn active(&self) -> bool {
        self.status == "ready" && self.revoked_at.is_none()
    }
}
