use chrono::{DateTime, Utc};
use mongodb::bson::{Document, doc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "downstream_services";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SshServiceConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub certificate_auth_enabled: bool,
    #[serde(default = "default_certificate_ttl_minutes")]
    pub certificate_ttl_minutes: u32,
    #[serde(default)]
    pub allowed_principals: Vec<String>,
    #[serde(
        default,
        with = "crate::models::bson_bytes::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub ca_private_key_encrypted: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_public_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownstreamService {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    /// Base URL of the downstream service.
    /// For SSH services this is derived as `ssh://host:port`.
    pub base_url: String,
    /// "http" | "ssh"
    #[serde(default = "default_service_type")]
    pub service_type: String,
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
    #[serde(
        default,
        alias = "api_spec_url",
        skip_serializing_if = "Option::is_none"
    )]
    pub openapi_spec_url: Option<String>,
    /// URL to an AsyncAPI spec describing this service's streaming interfaces
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asyncapi_spec_url: Option<String>,
    /// Whether this service supports SSE or other streaming responses.
    #[serde(default)]
    pub streaming_supported: bool,
    /// SSH tunnel configuration for first-class SSH services.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_config: Option<SshServiceConfig>,
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

    /// Whether to inject a delegation token (X-NyxID-Delegation-Token)
    /// when proxying requests to this service via MCP or REST proxy.
    /// The token allows the service to call NyxID APIs on behalf of the user.
    #[serde(default)]
    pub inject_delegation_token: bool,
    /// Space-separated scopes for the injected delegation token.
    #[serde(default = "default_delegation_scope")]
    pub delegation_token_scope: String,

    /// Optional link to a ProviderConfig for auto-seeded LLM services.
    /// When set, this service was auto-created for the provider's API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_config_id: Option<String>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// Match HTTP services while remaining compatible with legacy documents created
/// before `service_type` was introduced.
pub fn legacy_http_service_type_filter() -> Document {
    doc! {
        "$or": [
            { "service_type": "http" },
            { "service_type": { "$exists": false } },
        ],
    }
}

fn default_service_type() -> String {
    "http".to_string()
}

fn default_service_category() -> String {
    "connection".to_string()
}

fn default_certificate_ttl_minutes() -> u32 {
    30
}

fn default_identity_propagation_mode() -> String {
    "none".to_string()
}

fn default_true() -> bool {
    true
}

fn default_delegation_scope() -> String {
    "llm:proxy".to_string()
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
        assert_eq!(default_service_type(), "http");
        assert_eq!(default_service_category(), "connection");
        assert_eq!(default_certificate_ttl_minutes(), 30);
        assert_eq!(default_identity_propagation_mode(), "none");
        assert!(default_true());
    }

    #[test]
    fn legacy_http_service_type_filter_matches_missing_field() {
        let filter = legacy_http_service_type_filter();
        let clauses = filter.get_array("$or").expect("or clause");

        assert_eq!(clauses.len(), 2);
        assert_eq!(
            clauses[0]
                .as_document()
                .expect("http clause")
                .get_str("service_type")
                .expect("service_type"),
            "http"
        );
        assert!(
            !clauses[1]
                .as_document()
                .expect("legacy clause")
                .get_document("service_type")
                .expect("exists clause")
                .get_bool("$exists")
                .expect("exists value")
        );
    }

    #[test]
    fn bson_roundtrip() {
        let svc = DownstreamService {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Test Service".to_string(),
            slug: "test-service".to_string(),
            description: Some("A test service".to_string()),
            base_url: "https://api.example.com".to_string(),
            service_type: "http".to_string(),
            auth_method: "header".to_string(),
            auth_key_name: "Authorization".to_string(),
            credential_encrypted: vec![1, 2, 3],
            auth_type: Some("bearer".to_string()),
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            streaming_supported: false,
            ssh_config: None,
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
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            provider_config_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&svc).expect("serialize");
        let restored: DownstreamService = bson::from_document(doc).expect("deserialize");
        assert_eq!(svc.id, restored.id);
        assert_eq!(svc.slug, restored.slug);
        assert_eq!(svc.service_type, restored.service_type);
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
            service_type: "http".to_string(),
            auth_method: "header".to_string(),
            auth_key_name: "Authorization".to_string(),
            credential_encrypted: vec![1],
            auth_type: None,
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            streaming_supported: false,
            ssh_config: None,
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
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            provider_config_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let mut doc = bson::to_document(&svc).expect("serialize");
        // Remove the fields that have #[serde(default = ...)]
        doc.remove("service_type");
        doc.remove("service_category");
        doc.remove("requires_user_credential");
        doc.remove("identity_propagation_mode");
        doc.remove("inject_delegation_token");
        doc.remove("delegation_token_scope");
        let restored: DownstreamService = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.service_type, "http");
        assert_eq!(restored.service_category, "connection");
        assert_eq!(restored.identity_propagation_mode, "none");
        assert!(restored.requires_user_credential);
        assert!(!restored.inject_delegation_token);
        assert_eq!(restored.delegation_token_scope, "llm:proxy");
    }

    #[test]
    fn ssh_config_roundtrip() {
        let config = SshServiceConfig {
            host: "ssh.internal.example".to_string(),
            port: 22,
            certificate_auth_enabled: true,
            certificate_ttl_minutes: 30,
            allowed_principals: vec!["ubuntu".to_string()],
            ca_private_key_encrypted: Some(vec![1, 2, 3]),
            ca_public_key: Some("ssh-ed25519 AAAATEST ssh-ca".to_string()),
        };

        let doc = bson::to_document(&config).expect("serialize");
        let restored: SshServiceConfig = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.host, "ssh.internal.example");
        assert_eq!(restored.port, 22);
        assert!(restored.certificate_auth_enabled);
        assert_eq!(restored.allowed_principals, vec!["ubuntu".to_string()]);
    }
}
