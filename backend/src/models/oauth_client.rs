use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How this client's `allowed_scopes` were determined at creation
/// (NyxID#1222). Durable provenance so future scope-policy changes can
/// distinguish inherited defaults from explicit choices — the missing
/// discriminator that made retroactive scope migrations unsound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScopeProvenance {
    /// The registration/creation request explicitly supplied `scope`.
    Explicit,
    /// The request omitted `scope`; the server default was applied.
    Defaulted,
    /// Row predates provenance tracking; origin is unknowable and the
    /// row must never be widened retroactively.
    #[default]
    UnknownLegacy,
}

pub const COLLECTION_NAME: &str = "oauth_clients";

#[derive(Clone, Serialize, Deserialize)]
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
    /// Provenance of `allowed_scopes` (NyxID#1222); legacy rows without
    /// the field deserialize as `UnknownLegacy` and are never widened.
    #[serde(default)]
    pub scope_provenance: ScopeProvenance,
    /// "authorization_code", "client_credentials", etc.
    pub grant_types: String,
    /// "confidential" or "public"
    pub client_type: String,
    pub is_active: bool,
    /// Space-separated scopes the client can request via token exchange.
    /// Empty string means token exchange is not allowed.
    #[serde(default)]
    pub delegation_scopes: String,
    /// Catalog service slugs this app requests by default at consent time.
    /// Resolved per-user against `UserService.catalog_service_id` when the
    /// consent screen is built; matches are pre-selected. UI hint only --
    /// the stored grant is always the user's explicit selection.
    #[serde(default)]
    pub default_service_catalog_slugs: Vec<String>,
    #[serde(default)]
    pub broker_capability_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation_webhook_url: Option<String>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub revocation_webhook_secret_encrypted: Option<Vec<u8>>,
    #[serde(default)]
    pub connection_webhook_url: Option<String>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub connection_webhook_secret_encrypted: Option<Vec<u8>>,
    #[serde(default)]
    pub connection_webhook_key_id: Option<String>,
    #[serde(default)]
    pub connection_webhook_enabled: bool,
    pub created_by: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for OauthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::redaction::RedactedLen;
        f.debug_struct("OauthClient")
            .field("id", &self.id)
            .field("client_name", &self.client_name)
            .field(
                "client_secret_hash",
                &RedactedLen(self.client_secret_hash.len()),
            )
            .field("redirect_uris", &self.redirect_uris)
            .field("allowed_scopes", &self.allowed_scopes)
            .field("scope_provenance", &self.scope_provenance)
            .field("grant_types", &self.grant_types)
            .field("client_type", &self.client_type)
            .field("is_active", &self.is_active)
            .field("delegation_scopes", &self.delegation_scopes)
            .field(
                "default_service_catalog_slugs",
                &self.default_service_catalog_slugs,
            )
            .field("broker_capability_enabled", &self.broker_capability_enabled)
            .field(
                "revocation_webhook_url",
                &self
                    .revocation_webhook_url
                    .as_ref()
                    .map(|value| RedactedLen(value.len())),
            )
            .field(
                "revocation_webhook_secret_encrypted",
                &self
                    .revocation_webhook_secret_encrypted
                    .as_ref()
                    .map(|value| RedactedLen(value.len())),
            )
            .field(
                "connection_webhook_url",
                &self
                    .connection_webhook_url
                    .as_ref()
                    .map(|value| RedactedLen(value.len())),
            )
            .field(
                "connection_webhook_secret_encrypted",
                &self
                    .connection_webhook_secret_encrypted
                    .as_ref()
                    .map(|value| RedactedLen(value.len())),
            )
            .field("connection_webhook_key_id", &self.connection_webhook_key_id)
            .field(
                "connection_webhook_enabled",
                &self.connection_webhook_enabled,
            )
            .field("created_by", &self.created_by)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
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
            scope_provenance: Default::default(),
            grant_types: "authorization_code".to_string(),
            client_type: "confidential".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            default_service_catalog_slugs: Vec::new(),
            broker_capability_enabled: true,
            revocation_webhook_url: Some("https://client.example.com/cae".to_string()),
            revocation_webhook_secret_encrypted: Some(vec![1, 2, 3]),
            connection_webhook_url: Some("https://client.example.com/connections".to_string()),
            connection_webhook_secret_encrypted: Some(vec![4, 5, 6]),
            connection_webhook_key_id: Some("key_fixture".to_string()),
            connection_webhook_enabled: true,
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
        assert_eq!(
            client.connection_webhook_secret_encrypted,
            restored.connection_webhook_secret_encrypted
        );
        let debug = format!("{client:?}");
        assert!(!debug.contains("abc123"));
        assert!(!debug.contains("client.example.com"));
        assert!(!debug.contains("[1, 2, 3]"));
        assert!(!debug.contains("[4, 5, 6]"));
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
        assert!(restored.default_service_catalog_slugs.is_empty());
    }

    #[test]
    fn bson_roundtrip_default_service_catalog_slugs() {
        let mut client = OauthClient {
            id: "app-client".to_string(),
            client_name: "App".to_string(),
            client_secret_hash: "abc123".to_string(),
            redirect_uris: vec!["http://localhost:3000/callback".to_string()],
            allowed_scopes: "openid".to_string(),
            scope_provenance: Default::default(),
            grant_types: "authorization_code".to_string(),
            client_type: "confidential".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            default_service_catalog_slugs: vec!["openai".to_string(), "lark".to_string()],
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            connection_webhook_url: None,
            connection_webhook_secret_encrypted: None,
            connection_webhook_key_id: None,
            connection_webhook_enabled: false,
            created_by: Some("dev".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&client).expect("serialize");
        let restored: OauthClient = bson::from_document(doc).expect("deserialize");
        assert_eq!(
            restored.default_service_catalog_slugs,
            vec!["openai".to_string(), "lark".to_string()]
        );
        client.default_service_catalog_slugs.clear();
        let doc = bson::to_document(&client).expect("serialize empty");
        let restored: OauthClient = bson::from_document(doc).expect("deserialize empty");
        assert!(restored.default_service_catalog_slugs.is_empty());
    }

    #[test]
    fn bson_roundtrip_no_webhook() {
        let client = OauthClient {
            id: "default-client".to_string(),
            client_name: "Test Client".to_string(),
            client_secret_hash: "abc123".to_string(),
            redirect_uris: vec!["http://localhost:3000/callback".to_string()],
            allowed_scopes: "openid profile email".to_string(),
            scope_provenance: Default::default(),
            grant_types: "authorization_code".to_string(),
            client_type: "confidential".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            default_service_catalog_slugs: Vec::new(),
            broker_capability_enabled: true,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            connection_webhook_url: None,
            connection_webhook_secret_encrypted: None,
            connection_webhook_key_id: None,
            connection_webhook_enabled: false,
            created_by: Some("admin".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&client).expect("serialize");
        let restored: OauthClient = bson::from_document(doc).expect("deserialize");
        assert!(restored.revocation_webhook_url.is_none());
        assert!(restored.revocation_webhook_secret_encrypted.is_none());
    }

    #[test]
    fn debug_redacts_secret_material() {
        let client = OauthClient {
            id: "client".to_string(),
            client_name: "Client".to_string(),
            client_secret_hash: "secret-hash-value".to_string(),
            redirect_uris: vec![],
            allowed_scopes: "openid".to_string(),
            scope_provenance: ScopeProvenance::Explicit,
            grant_types: "authorization_code".to_string(),
            client_type: "confidential".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            default_service_catalog_slugs: vec![],
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: Some(vec![1, 2, 3]),
            connection_webhook_url: None,
            connection_webhook_secret_encrypted: Some(vec![4, 5, 6]),
            connection_webhook_key_id: Some("key_fixture".to_string()),
            connection_webhook_enabled: true,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let debug = format!("{client:?}");
        assert!(!debug.contains("secret-hash-value"));
        assert!(!debug.contains("[1, 2, 3]"));
        assert!(!debug.contains("[4, 5, 6]"));
    }
}
