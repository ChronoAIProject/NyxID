use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "provider_configs";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(rename = "_id")]
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,

    /// "oauth2" | "api_key" | "device_code"
    pub provider_type: String,

    // --- OAuth2 fields (None for api_key providers) ---
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub revocation_url: Option<String>,
    pub default_scopes: Option<Vec<String>>,
    /// NyxID's OAuth client_id for this provider (encrypted)
    pub client_id_encrypted: Option<Vec<u8>>,
    /// NyxID's OAuth client_secret for this provider (encrypted)
    pub client_secret_encrypted: Option<Vec<u8>>,
    #[serde(default)]
    pub supports_pkce: bool,

    // --- Device code flow fields ---
    /// For device_code flow: the URL to request a device code (RFC 8628 step 1)
    /// e.g., "https://auth.openai.com/deviceauth/usercode"
    #[serde(default)]
    pub device_code_url: Option<String>,
    /// For device_code flow: the URL to poll for token exchange (RFC 8628 step 3)
    /// e.g., "https://auth.openai.com/deviceauth/token"
    #[serde(default)]
    pub device_token_url: Option<String>,
    /// For device_code flow: the URL the user visits to enter their code
    /// e.g., "https://auth.openai.com/codex/device"
    #[serde(default)]
    pub device_verification_url: Option<String>,
    /// For device_code flow: legacy field (kept for backward compat)
    #[serde(default)]
    pub hosted_callback_url: Option<String>,

    // --- API key fields ---
    pub api_key_instructions: Option<String>,
    pub api_key_url: Option<String>,

    // --- Display ---
    pub icon_url: Option<String>,
    pub documentation_url: Option<String>,

    pub is_active: bool,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
