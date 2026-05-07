use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "user_api_keys";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserApiKey {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub label: String,

    /// "api_key" | "oauth2" | "bearer" | "basic" | "node_managed" | "ssh_certificate"
    pub credential_type: String,

    // --- Primary credential (encrypted) ---
    /// For api_key/bearer/basic: the raw credential
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub credential_encrypted: Option<Vec<u8>>,

    // --- OAuth2 tokens (encrypted) ---
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub access_token_encrypted: Option<Vec<u8>>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub refresh_token_encrypted: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_scopes: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,

    /// Optional: link to ProviderConfig for OAuth refresh
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_config_id: Option<String>,

    // --- User-owned OAuth app credentials (merged from UserProviderCredentials) ---
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub user_oauth_client_id_encrypted: Option<Vec<u8>>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub user_oauth_client_secret_encrypted: Option<Vec<u8>>,

    /// "active" | "expired" | "revoked" | "failed" | "refresh_failed" | "pending_auth"
    pub status: String,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,

    /// Source tracking for migration: "migration_provider_token" | "migration_connection" | "user_created"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Original record ID from migration (for idempotency)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,

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
        assert_eq!(COLLECTION_NAME, "user_api_keys");
    }

    #[test]
    fn bson_roundtrip_api_key() {
        let key = UserApiKey {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            label: "Production Key".to_string(),
            credential_type: "api_key".to_string(),
            credential_encrypted: Some(vec![1, 2, 3]),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "active".to_string(),
            last_used_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&key).expect("serialize");
        let restored: UserApiKey = bson::from_document(doc).expect("deserialize");
        assert_eq!(key.id, restored.id);
        assert_eq!(key.credential_type, restored.credential_type);
        assert_eq!(key.status, restored.status);
    }

    #[test]
    fn bson_roundtrip_oauth2() {
        let key = UserApiKey {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            label: "GitHub OAuth".to_string(),
            credential_type: "oauth2".to_string(),
            credential_encrypted: None,
            access_token_encrypted: Some(vec![4, 5, 6]),
            refresh_token_encrypted: Some(vec![7, 8, 9]),
            token_scopes: Some("repo read:user".to_string()),
            expires_at: Some(Utc::now()),
            provider_config_id: Some("github-provider-id".to_string()),
            user_oauth_client_id_encrypted: Some(vec![10, 11]),
            user_oauth_client_secret_encrypted: Some(vec![12, 13]),
            status: "active".to_string(),
            last_used_at: Some(Utc::now()),
            error_message: None,
            source: Some("migration_provider_token".to_string()),
            source_id: Some("old-token-id".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&key).expect("serialize");
        let restored: UserApiKey = bson::from_document(doc).expect("deserialize");
        assert_eq!(key.id, restored.id);
        assert_eq!(restored.credential_type, "oauth2");
        assert!(restored.access_token_encrypted.is_some());
        assert!(restored.refresh_token_encrypted.is_some());
        assert!(restored.token_scopes.is_some());
        assert!(restored.expires_at.is_some());
        assert_eq!(restored.source.as_deref(), Some("migration_provider_token"));
        assert_eq!(restored.source_id.as_deref(), Some("old-token-id"));
    }
}
