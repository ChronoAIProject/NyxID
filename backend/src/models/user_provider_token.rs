use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "user_provider_tokens";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserProviderToken {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub provider_config_id: String,

    /// "oauth2" | "api_key"
    pub token_type: String,

    // --- OAuth2 tokens (encrypted) ---
    pub access_token_encrypted: Option<Vec<u8>>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub token_scopes: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,

    // --- API key (encrypted) ---
    pub api_key_encrypted: Option<Vec<u8>>,

    // --- Status ---
    /// "active" | "expired" | "revoked" | "refresh_failed"
    pub status: String,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_refreshed_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_used_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,

    // --- User metadata ---
    pub label: Option<String>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
