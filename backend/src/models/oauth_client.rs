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
            created_by: Some("admin".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&client).expect("serialize");
        let restored: OauthClient = bson::from_document(doc).expect("deserialize");
        assert_eq!(client.id, restored.id);
        assert_eq!(client.redirect_uris.len(), restored.redirect_uris.len());
        assert_eq!(client.client_type, restored.client_type);
    }
}
