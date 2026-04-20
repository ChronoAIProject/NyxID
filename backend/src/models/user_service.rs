use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::default_request_header::DefaultRequestHeader;

pub const COLLECTION_NAME: &str = "user_services";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserService {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    /// Proxy path slug (e.g., "llm-openai", "my-custom-api")
    pub slug: String,
    /// FK to UserEndpoint
    pub endpoint_id: String,
    /// FK to UserApiKey (None for no-auth auto-connected services)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_id: Option<String>,
    /// "bearer" | "header" | "query" | "basic" | "none"
    pub auth_method: String,
    /// Header name or query param name (e.g., "Authorization", "x-api-key", "key")
    pub auth_key_name: String,
    /// Optional: populated when auto-provisioned from catalog
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_service_id: Option<String>,
    /// Optional: route requests through this node agent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// Failover priority (lower = higher priority, default 0)
    #[serde(default)]
    pub node_priority: i32,
    /// "http" (default) | "ssh"
    #[serde(default = "default_service_type")]
    pub service_type: String,

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_jwt_audience: Option<String>,
    /// Whether to forward the caller's NyxID access token as Authorization: Bearer
    #[serde(default)]
    pub forward_access_token: bool,
    /// Whether to inject a delegation token (X-NyxID-Delegation-Token)
    #[serde(default)]
    pub inject_delegation_token: bool,
    #[serde(default = "default_delegation_token_scope")]
    pub delegation_token_scope: String,

    /// Custom User-Agent header to send to the downstream service.
    /// When set, overrides the client's User-Agent instead of forwarding it.
    /// When None, the client's User-Agent is forwarded as-is (passthrough).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_user_agent: Option<String>,

    /// Per-user default HTTP headers injected on every proxied request
    /// through this service. Layered *above* the catalog
    /// `DownstreamService.default_request_headers`:
    /// the catalog layer lands first, then the user's overrides run with
    /// the same precedence rules (non-overridable wins, overridable yields
    /// to whatever is already set). See
    /// `crate::models::default_request_header` for merge semantics and
    /// denylist rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_request_headers: Option<Vec<DefaultRequestHeader>>,

    pub is_active: bool,

    /// Source tracking for migration idempotency
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// Developer app (OAuth client) ID that triggered this auto-provision.
    /// Set when a private service is auto-provisioned via developer app consent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_app_id: Option<String>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

fn default_service_type() -> String {
    "http".to_string()
}

fn default_identity_propagation_mode() -> String {
    "none".to_string()
}

fn default_delegation_token_scope() -> String {
    "llm:proxy".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "user_services");
    }

    #[test]
    fn bson_roundtrip() {
        let svc = UserService {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            slug: "llm-openai".to_string(),
            endpoint_id: uuid::Uuid::new_v4().to_string(),
            api_key_id: Some(uuid::Uuid::new_v4().to_string()),
            auth_method: "bearer".to_string(),
            auth_key_name: "Authorization".to_string(),
            catalog_service_id: Some("svc-id".to_string()),
            node_id: Some("node-1".to_string()),
            node_priority: 0,
            service_type: "http".to_string(),
            identity_propagation_mode: "headers".to_string(),
            identity_include_user_id: true,
            identity_include_email: true,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: true,
            delegation_token_scope: "llm:proxy".to_string(),
            custom_user_agent: None,
            default_request_headers: None,
            is_active: true,
            source: None,
            source_id: None,
            source_app_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&svc).expect("serialize");
        let restored: UserService = bson::from_document(doc).expect("deserialize");
        assert_eq!(svc.id, restored.id);
        assert_eq!(svc.slug, restored.slug);
        assert_eq!(svc.node_priority, restored.node_priority);
        assert_eq!(restored.service_type, "http");
        assert_eq!(restored.identity_propagation_mode, "headers");
        assert!(restored.identity_include_user_id);
        assert!(!restored.forward_access_token);
        assert!(restored.inject_delegation_token);
        assert!(restored.is_active);
    }

    #[test]
    fn bson_defaults() {
        let svc = UserService {
            id: "id".to_string(),
            user_id: "uid".to_string(),
            slug: "test".to_string(),
            endpoint_id: "ep".to_string(),
            api_key_id: Some("ak".to_string()),
            auth_method: "header".to_string(),
            auth_key_name: "X-API-Key".to_string(),
            catalog_service_id: None,
            node_id: None,
            node_priority: 0,
            service_type: "http".to_string(),
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            custom_user_agent: None,
            default_request_headers: None,
            is_active: true,
            source: None,
            source_id: None,
            source_app_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let mut doc = bson::to_document(&svc).expect("serialize");
        doc.remove("node_priority");
        doc.remove("service_type");
        let restored: UserService = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.node_priority, 0);
        assert_eq!(restored.service_type, "http");
    }

    #[test]
    fn bson_identity_defaults() {
        let svc = UserService {
            id: "id".to_string(),
            user_id: "uid".to_string(),
            slug: "test".to_string(),
            endpoint_id: "ep".to_string(),
            api_key_id: None,
            auth_method: "none".to_string(),
            auth_key_name: String::new(),
            catalog_service_id: None,
            node_id: None,
            node_priority: 0,
            service_type: "http".to_string(),
            identity_propagation_mode: "both".to_string(),
            identity_include_user_id: true,
            identity_include_email: true,
            identity_include_name: true,
            identity_jwt_audience: Some("https://example.com".to_string()),
            forward_access_token: true,
            inject_delegation_token: true,
            delegation_token_scope: "custom:scope".to_string(),
            custom_user_agent: None,
            default_request_headers: None,
            is_active: true,
            source: None,
            source_id: None,
            source_app_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let mut doc = bson::to_document(&svc).expect("serialize");
        // Remove all identity fields to simulate existing documents
        doc.remove("identity_propagation_mode");
        doc.remove("identity_include_user_id");
        doc.remove("identity_include_email");
        doc.remove("identity_include_name");
        doc.remove("identity_jwt_audience");
        doc.remove("forward_access_token");
        doc.remove("inject_delegation_token");
        doc.remove("delegation_token_scope");
        let restored: UserService = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.identity_propagation_mode, "none");
        assert!(!restored.identity_include_user_id);
        assert!(!restored.identity_include_email);
        assert!(!restored.identity_include_name);
        assert!(restored.identity_jwt_audience.is_none());
        assert!(!restored.forward_access_token);
        assert!(!restored.inject_delegation_token);
        assert_eq!(restored.delegation_token_scope, "llm:proxy");
    }

    #[test]
    fn bson_roundtrip_no_api_key() {
        let svc = UserService {
            id: "id".to_string(),
            user_id: "uid".to_string(),
            slug: "auto-svc".to_string(),
            endpoint_id: "ep".to_string(),
            api_key_id: None,
            auth_method: "none".to_string(),
            auth_key_name: String::new(),
            catalog_service_id: Some("cat-1".to_string()),
            node_id: None,
            node_priority: 0,
            service_type: "http".to_string(),
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            custom_user_agent: None,
            default_request_headers: None,
            is_active: true,
            source: None,
            source_id: None,
            source_app_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&svc).expect("serialize");
        assert!(!doc.contains_key("api_key_id"), "None should be skipped");
        let restored: UserService = bson::from_document(doc).expect("deserialize");
        assert!(restored.api_key_id.is_none());
        assert_eq!(restored.auth_method, "none");
    }
}
