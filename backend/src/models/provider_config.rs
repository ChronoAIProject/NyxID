use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "provider_configs";

fn default_credential_mode() -> String {
    "admin".to_string()
}

fn default_token_endpoint_auth_method() -> String {
    "client_secret_post".to_string()
}

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
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub client_id_encrypted: Option<Vec<u8>>,
    /// NyxID's OAuth client_secret for this provider (encrypted)
    #[serde(default, with = "crate::models::bson_bytes::optional")]
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
    /// "admin" | "user" | "both" -- controls where OAuth client credentials come from
    #[serde(default = "default_credential_mode")]
    pub credential_mode: String,
    /// How client credentials are sent to the token endpoint:
    /// "client_secret_post" (form body, default) | "client_secret_basic" (HTTP Basic Auth)
    #[serde(default = "default_token_endpoint_auth_method")]
    pub token_endpoint_auth_method: String,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "provider_configs");
    }

    #[test]
    fn bson_roundtrip_oauth2() {
        let config = ProviderConfig {
            id: uuid::Uuid::new_v4().to_string(),
            slug: "google".to_string(),
            name: "Google".to_string(),
            description: Some("Google OAuth2".to_string()),
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://accounts.google.com/o/oauth2/v2/auth".to_string()),
            token_url: Some("https://oauth2.googleapis.com/token".to_string()),
            revocation_url: None,
            default_scopes: Some(vec!["openid".to_string(), "email".to_string()]),
            client_id_encrypted: Some(vec![1, 2, 3]),
            client_secret_encrypted: Some(vec![4, 5, 6]),
            supports_pkce: true,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            created_by: "admin".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&config).expect("serialize");
        let restored: ProviderConfig = bson::from_document(doc).expect("deserialize");
        assert_eq!(config.slug, restored.slug);
        assert_eq!(config.provider_type, restored.provider_type);
        assert!(restored.supports_pkce);
        assert_eq!(restored.credential_mode, "admin");
    }

    #[test]
    fn bson_roundtrip_api_key() {
        let config = ProviderConfig {
            id: uuid::Uuid::new_v4().to_string(),
            slug: "anthropic".to_string(),
            name: "Anthropic".to_string(),
            description: None,
            provider_type: "api_key".to_string(),
            authorization_url: None,
            token_url: None,
            revocation_url: None,
            default_scopes: None,
            client_id_encrypted: None,
            client_secret_encrypted: None,
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: Some("Get your key at...".to_string()),
            api_key_url: Some("https://console.anthropic.com".to_string()),
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            created_by: "admin".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&config).expect("serialize");
        let restored: ProviderConfig = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.provider_type, "api_key");
        assert!(restored.api_key_instructions.is_some());
    }

    #[test]
    fn credential_mode_defaults_to_admin() {
        // Simulate a document without credential_mode (backward compat)
        let doc = bson::doc! {
            "_id": "test-id",
            "slug": "test",
            "name": "Test",
            "provider_type": "api_key",
            "supports_pkce": false,
            "is_active": true,
            "created_by": "admin",
            "created_at": bson::DateTime::from_chrono(Utc::now()),
            "updated_at": bson::DateTime::from_chrono(Utc::now()),
        };
        let restored: ProviderConfig = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.credential_mode, "admin");
    }
}
