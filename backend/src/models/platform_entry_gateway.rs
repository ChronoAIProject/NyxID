//! Dedicated shared-entry gateway authority survives ordinary NyxBot cleanup.
use super::platform_entry_identity::NativeEntryConfiguration;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_entry_gateway_transports";

#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformEntryGateway {
    #[serde(rename = "_id")]
    pub id: String,
    pub profile_id: String,
    pub configuration: NativeEntryConfiguration,
    pub configured_by: String,
    pub credential_owner_id: String,
    pub route_api_key_id: String,
    pub route_id: String,
    pub provider_api_key_id: String,
    pub callback_url: String,
    pub binding_id: Option<String>,
    pub gateway_base: String,
    pub setup_state: String,
    pub setup_attempt_id: String,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub setup_dispatch_deadline: Option<DateTime<Utc>>,
    pub request_policy: serde_json::Value,
    pub request_hash: String,
    pub creation_fingerprint: String,
    #[serde(with = "crate::models::bson_bytes::required")]
    pub request_ciphertext: Vec<u8>,
    /// prepared, active or retired; no ordinary transport fallback state.
    pub state: String,
    pub revision: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for PlatformEntryGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformEntryGateway")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}
