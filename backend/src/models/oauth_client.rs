use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "oauth_clients";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OauthClient {
    #[serde(rename = "_id")]
    pub id: String,
    pub client_name: String,
    /// Hashed client secret (SHA-256)
    pub client_secret_hash: String,
    /// Allowed redirect URIs
    pub redirect_uris: Vec<String>,
    /// Space-separated allowed scopes
    pub allowed_scopes: String,
    /// "authorization_code", "client_credentials", etc.
    pub grant_types: String,
    /// "confidential" or "public"
    pub client_type: String,
    pub is_active: bool,
    /// Space-separated scopes the client can request via token exchange.
    /// Empty string means token exchange is not allowed.
    #[serde(default)]
    pub delegation_scopes: String,
    #[serde(default)]
    pub broker_capability_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation_webhook_url: Option<String>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub revocation_webhook_secret_encrypted: Option<Vec<u8>>,
    pub created_by: Option<String>,
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
        assert_eq!(COLLECTION_NAME, "oauth_clients");
    }

    #[test]
    fn bson_roundtrip() {
        let client = OauthClient {
            id: "default-client".to_string(),
            client_name: "Test Client".to_string(),
            client_secret_hash: "abc123".to_string(),
            redirect_uris: vec!["http://localhost:3000/callback".to_string()],
            allowed_scopes: "openid profile email".to_string(),
            grant_types: "authorization_code".to_string(),
            client_type: "confidential".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            broker_capability_enabled: true,
            revocation_webhook_url: Some("https://client.example.com/cae".to_string()),
            revocation_webhook_secret_encrypted: Some(vec![1, 2, 3]),
            created_by: Some("admin".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&client).expect("serialize");
        let restored: OauthClient = bson::from_document(doc).expect("deserialize");
        assert_eq!(client.id, restored.id);
        assert_eq!(client.redirect_uris.len(), restored.redirect_uris.len());
        assert_eq!(client.client_type, restored.client_type);
        assert_eq!(
            client.broker_capability_enabled,
            restored.broker_capability_enabled
        );
        assert_eq!(
            client.revocation_webhook_url,
            restored.revocation_webhook_url
        );
        assert_eq!(
            client.revocation_webhook_secret_encrypted,
            restored.revocation_webhook_secret_encrypted
        );
    }

    #[test]
    fn bson_default_for_legacy_doc() {
        let now = Utc::now();
        let doc = bson::doc! {
            "_id": "legacy-client",
            "client_name": "Legacy Client",
            "client_secret_hash": "abc123",
            "redirect_uris": ["http://localhost:3000/callback"],
            "allowed_scopes": "openid profile email",
            "grant_types": "authorization_code",
            "client_type": "confidential",
            "is_active": true,
            "delegation_scopes": "",
            "created_by": "admin",
            "created_at": bson::DateTime::from_chrono(now),
            "updated_at": bson::DateTime::from_chrono(now),
        };

        let restored: OauthClient = bson::from_document(doc).expect("deserialize legacy doc");
        assert!(!restored.broker_capability_enabled);
        assert!(restored.revocation_webhook_url.is_none());
        assert!(restored.revocation_webhook_secret_encrypted.is_none());
    }

    #[test]
    fn bson_roundtrip_no_webhook() {
        let client = OauthClient {
            id: "default-client".to_string(),
            client_name: "Test Client".to_string(),
            client_secret_hash: "abc123".to_string(),
            redirect_uris: vec!["http://localhost:3000/callback".to_string()],
            allowed_scopes: "openid profile email".to_string(),
            grant_types: "authorization_code".to_string(),
            client_type: "confidential".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            broker_capability_enabled: true,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            created_by: Some("admin".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&client).expect("serialize");
        let restored: OauthClient = bson::from_document(doc).expect("deserialize");
        assert!(restored.revocation_webhook_url.is_none());
        assert!(restored.revocation_webhook_secret_encrypted.is_none());
    }
}
