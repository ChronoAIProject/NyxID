use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "oauth_states";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OAuthState {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub provider_config_id: String,
    pub code_verifier: Option<String>,
    /// Encrypted device_auth_id (OpenAI) or device_code (RFC 8628) for polling
    #[serde(default)]
    pub device_code_encrypted: Option<String>,
    /// Encrypted user_code needed for OpenAI-style device code polling
    #[serde(default)]
    pub user_code_encrypted: Option<String>,
    /// Polling interval in seconds for device code flow
    #[serde(default)]
    pub poll_interval: Option<i32>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
