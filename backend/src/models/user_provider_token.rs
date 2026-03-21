use std::collections::HashMap;

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
    /// When present, the OAuth connection was minted with user-provided app
    /// credentials owned by this user ID. `None` means provider-level
    /// credentials were used instead.
    #[serde(default)]
    pub credential_user_id: Option<String>,

    /// "oauth2" | "api_key" | "telegram_identity"
    pub token_type: String,

    // --- OAuth2 tokens (encrypted) ---
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub access_token_encrypted: Option<Vec<u8>>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub token_scopes: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,

    // --- API key (encrypted) ---
    #[serde(default, with = "crate::models::bson_bytes::optional")]
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

    /// Arbitrary key-value metadata for provider-specific data.
    /// Used by `telegram_identity` tokens to store `telegram_user_id`,
    /// `username`, `photo_url`, etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,

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
        assert_eq!(COLLECTION_NAME, "user_provider_tokens");
    }

    #[test]
    fn bson_roundtrip_oauth2_token() {
        let token = UserProviderToken {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            provider_config_id: uuid::Uuid::new_v4().to_string(),
            credential_user_id: Some(uuid::Uuid::new_v4().to_string()),
            token_type: "oauth2".to_string(),
            access_token_encrypted: Some(vec![1, 2, 3]),
            refresh_token_encrypted: Some(vec![4, 5, 6]),
            token_scopes: Some("openid email".to_string()),
            expires_at: Some(Utc::now()),
            api_key_encrypted: None,
            status: "active".to_string(),
            last_refreshed_at: Some(Utc::now()),
            last_used_at: None,
            error_message: None,
            label: Some("My Google Token".to_string()),
            metadata: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&token).expect("serialize");
        let restored: UserProviderToken = bson::from_document(doc).expect("deserialize");
        assert_eq!(token.id, restored.id);
        assert_eq!(restored.token_type, "oauth2");
        assert!(restored.expires_at.is_some());
        assert!(restored.last_refreshed_at.is_some());
        assert_eq!(restored.credential_user_id, token.credential_user_id);
    }

    #[test]
    fn bson_roundtrip_api_key_token() {
        let token = UserProviderToken {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            provider_config_id: uuid::Uuid::new_v4().to_string(),
            credential_user_id: None,
            token_type: "api_key".to_string(),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            api_key_encrypted: Some(vec![7, 8, 9]),
            status: "active".to_string(),
            last_refreshed_at: None,
            last_used_at: None,
            error_message: None,
            label: None,
            metadata: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&token).expect("serialize");
        let restored: UserProviderToken = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.token_type, "api_key");
        assert!(restored.api_key_encrypted.is_some());
    }

    #[test]
    fn bson_backward_compat_missing_credential_user_id() {
        let doc = bson::doc! {
            "_id": uuid::Uuid::new_v4().to_string(),
            "user_id": uuid::Uuid::new_v4().to_string(),
            "provider_config_id": uuid::Uuid::new_v4().to_string(),
            "token_type": "oauth2",
            "status": "active",
            "created_at": bson::DateTime::from_chrono(Utc::now()),
            "updated_at": bson::DateTime::from_chrono(Utc::now()),
        };
        let restored: UserProviderToken = bson::from_document(doc).expect("deserialize");
        assert!(restored.credential_user_id.is_none());
    }
}
