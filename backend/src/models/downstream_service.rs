use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "downstream_services";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownstreamService {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    /// Base URL of the downstream service (e.g. https://api.example.com)
    pub base_url: String,
    /// How credentials are injected: "header", "query", "body"
    pub auth_method: String,
    /// Header name or query param name for the credential
    pub auth_key_name: String,
    /// Encrypted master credential for this service
    #[serde(with = "crate::models::bson_bytes::required")]
    pub credential_encrypted: Vec<u8>,
    /// Original auth type as selected by the admin (e.g., "api_key", "oauth2", "oidc", "basic", "bearer").
    /// Preserves the user's intent, while `auth_method` is the resolved injection method.
    #[serde(default)]
    pub auth_type: Option<String>,
    /// URL to an OpenAPI / Swagger spec describing this service's API
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_spec_url: Option<String>,
    /// Associated OAuth client ID (set when auth_method is "oidc")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,

    /// "provider" | "connection" | "internal"
    /// - provider: OIDC services where NyxID is the identity provider (not user-connectable)
    /// - connection: external services users connect to with their own credentials
    /// - internal: internal services using master credential (users just enable access)
    #[serde(default = "default_service_category")]
    pub service_category: String,

    /// Whether this service requires per-user credentials to connect.
    /// true for connection services, false for internal/provider services.
    #[serde(default = "default_true")]
    pub requires_user_credential: bool,

    pub is_active: bool,
    pub created_by: String,

    // --- Identity propagation config ---
    /// "none" | "headers" | "jwt" | "both"
    #[serde(default = "default_identity_propagation_mode")]
    pub identity_propagation_mode: String,
    #[serde(default)]
    pub identity_include_user_id: bool,
    #[serde(default)]
    pub identity_include_email: bool,
    #[serde(default)]
    pub identity_include_name: bool,
    /// Custom JWT audience for identity assertions (defaults to service base_url)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_jwt_audience: Option<String>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

fn default_service_category() -> String {
    "connection".to_string()
}

fn default_identity_propagation_mode() -> String {
    "none".to_string()
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "downstream_services");
    }

    #[test]
    fn default_values() {
        assert_eq!(default_service_category(), "connection");
        assert_eq!(default_identity_propagation_mode(), "none");
        assert!(default_true());
    }

    #[test]
    fn bson_roundtrip() {
        let svc = DownstreamService {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Test Service".to_string(),
            slug: "test-service".to_string(),
            description: Some("A test service".to_string()),
            base_url: "https://api.example.com".to_string(),
            auth_method: "header".to_string(),
            auth_key_name: "Authorization".to_string(),
            credential_encrypted: vec![1, 2, 3],
            auth_type: Some("bearer".to_string()),
            api_spec_url: None,
            oauth_client_id: None,
            service_category: "connection".to_string(),
            requires_user_credential: true,
            is_active: true,
            created_by: "admin".to_string(),
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&svc).expect("serialize");
        let restored: DownstreamService = bson::from_document(doc).expect("deserialize");
        assert_eq!(svc.id, restored.id);
        assert_eq!(svc.slug, restored.slug);
        assert_eq!(svc.service_category, restored.service_category);
    }

    #[test]
    fn bson_deserialize_applies_defaults() {
        // Serialize a full struct, then remove default fields from the doc,
        // and verify they get their defaults on deserialization.
        let svc = DownstreamService {
            id: "test-id".to_string(),
            name: "Svc".to_string(),
            slug: "svc".to_string(),
            description: None,
            base_url: "https://example.com".to_string(),
            auth_method: "header".to_string(),
            auth_key_name: "Authorization".to_string(),
            credential_encrypted: vec![1],
            auth_type: None,
            api_spec_url: None,
            oauth_client_id: None,
            service_category: "connection".to_string(),
            requires_user_credential: true,
            is_active: true,
            created_by: "admin".to_string(),
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let mut doc = bson::to_document(&svc).expect("serialize");
        // Remove the fields that have #[serde(default = ...)]
        doc.remove("service_category");
        doc.remove("requires_user_credential");
        doc.remove("identity_propagation_mode");
        let restored: DownstreamService = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.service_category, "connection");
        assert_eq!(restored.identity_propagation_mode, "none");
        assert!(restored.requires_user_credential);
    }
}
