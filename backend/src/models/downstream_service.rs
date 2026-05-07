use chrono::{DateTime, Utc};
use mongodb::bson::{Document, doc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::models::default_request_header::DefaultRequestHeader;
use crate::models::ssh_auth_mode::{SshAuthMode, default_ssh_auth_mode};
use crate::models::ws_frame_injection::WsFrameInjection;

pub const COLLECTION_NAME: &str = "downstream_services";

/// Structured capability flags describing what a service supports through NyxID proxy.
/// These help AI agents understand the supported interaction patterns without guessing.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ServiceCapabilities {
    #[serde(default)]
    pub supports_proxy_read: bool,
    #[serde(default)]
    pub supports_proxy_write: bool,
    #[serde(default)]
    pub supports_proxy_binary_upload: bool,
    #[serde(default)]
    pub supports_direct_downstream_auth: bool,
    #[serde(default)]
    pub supports_authoring_via_nyx: bool,
    #[serde(default)]
    pub supports_websocket: bool,
    #[serde(default)]
    pub supports_streaming: bool,
}

/// Declarative configuration for a server-side token exchange flow.
///
/// When a service has `auth_method == "token_exchange"`, the proxy uses this
/// config to perform a one-time POST to obtain a short-lived access token,
/// caches the result, and injects it on every outbound request.
///
/// Covers Lark / Feishu tenant tokens, OAuth 2.0 client_credentials, and
/// similar "POST secrets, get a token back" flows without a new auth method
/// per provider. Rare cases that need custom logic (GitHub App JWT signing,
/// AWS SigV4, etc.) still use their own auth methods.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct TokenExchangeConfig {
    /// URL to POST credentials to. Supports `{base_url}` placeholder which
    /// the proxy substitutes with the service's `base_url` at request time.
    /// Example for Lark:
    /// `"{base_url}/open-apis/auth/v3/tenant_access_token/internal"`
    pub endpoint: String,
    /// Wire format for the request body: `"json"` or `"form"`
    /// (application/x-www-form-urlencoded).
    pub request_encoding: String,
    /// Request body template. Any string value starting with `$` is a
    /// placeholder: `"$app_id"` is substituted with the `app_id` field from
    /// the user's credential JSON blob. Literal values pass through as-is.
    ///
    /// Lark example:
    /// `{"app_id": "$app_id", "app_secret": "$app_secret"}`
    ///
    /// OAuth client_credentials example:
    /// `{"grant_type": "client_credentials", "client_id": "$client_id", "client_secret": "$client_secret"}`
    pub request_template: serde_json::Value,
    /// Dot-separated path into the response JSON where the access token lives.
    /// Lark: `"tenant_access_token"`.
    /// OAuth: `"access_token"`.
    /// Nested: `"data.token"`.
    pub token_response_path: String,
    /// Optional dot path to a TTL-in-seconds field in the response. Missing
    /// or absent at runtime falls back to `default_ttl_secs`.
    /// Lark: `"expire"`. OAuth: `"expires_in"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_response_path: Option<String>,
    /// Fallback TTL used when `ttl_response_path` is None or missing.
    pub default_ttl_secs: i64,
    /// How to inject the obtained token on outbound requests:
    /// - `"bearer"` -> `Authorization: Bearer <token>`
    /// - `"bot_bearer"` -> `Authorization: Bot <token>`
    /// - `"token"` -> `Authorization: token <token>` (GitHub style)
    /// - `"header:X-Api-Key"` -> `X-Api-Key: <token>` (custom header)
    pub injection: String,
    /// Optional dot path to an error-code field. Some providers (Lark) return
    /// HTTP 200 with a `{code: N, msg: "..."}` payload where non-zero code
    /// means failure. If the configured field contains a non-zero number or
    /// a non-empty string (other than `"0"`/`"ok"`), the response is rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code_path: Option<String>,
    /// Optional dot path to an error-message field, extracted for logging
    /// when `error_code_path` fires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message_path: Option<String>,
    /// Declared credential fields. Populated so CLI/frontend can render the
    /// correct input form without hard-coding per-provider logic, and so
    /// the backend can validate the user's JSON at create time.
    pub credential_fields: Vec<CredentialFieldSpec>,
}

/// UI/CLI hint for rendering one field of a token-exchange credential form.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct CredentialFieldSpec {
    /// JSON field name in the stored credential (e.g. `"app_id"`,
    /// `"client_secret"`). Must match a `$name` placeholder in
    /// `TokenExchangeConfig::request_template`.
    pub name: String,
    /// Human-readable label for the form input.
    pub label: String,
    /// Optional placeholder hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Render as a password input when true. Also masks the value in logs.
    #[serde(default)]
    pub secret: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SshServiceConfig {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_ssh_auth_mode")]
    pub ssh_auth_mode: SshAuthMode,
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
    /// "public" | "private" -- controls who can see this service in listings
    #[serde(default = "default_visibility")]
    pub visibility: String,
    /// How credentials are injected. Valid values:
    /// - `bearer`: `Authorization: Bearer <credential>`
    /// - `bot_bearer`: `Authorization: Bot <credential>` (Discord-style)
    /// - `header`: custom header named by `auth_key_name`
    /// - `query`: URL query parameter named by `auth_key_name`
    /// - `basic`: HTTP Basic auth, credential is `username:password`
    /// - `body`: merge `{<auth_key_name>: <credential>}` into JSON request body
    ///   (only applies to methods that carry a body: POST/PUT/PATCH)
    /// - `token_exchange`: credential is a JSON blob; the proxy posts it to
    ///   the configured endpoint in `token_exchange_config`, caches the
    ///   resulting token, and injects it per the configured injection format.
    ///   Covers Lark/Feishu tenant tokens, OAuth 2.0 client_credentials, and
    ///   similar exchange flows -- all declarative, no per-provider code.
    /// - `path`: inject credential into URL path (Telegram bot token style)
    /// - `none`: no credential injection
    pub auth_method: String,
    /// Header name, query param name, body field name, or path placeholder
    /// for the credential (depends on `auth_method`).
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

    /// Whether to forward the caller's NyxID access token as Authorization: Bearer
    /// when proxying requests. Used by platform apps that trust NyxID JWTs directly.
    #[serde(default)]
    pub forward_access_token: bool,

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

    // --- Rich metadata for AI agent discovery (issue #148) ---
    /// Public product or docs landing page
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage_url: Option<String>,
    /// Canonical GitHub/source repository URL
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_url: Option<String>,
    /// Issue tracker URL
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issues_url: Option<String>,
    /// Structured capability flags for proxy interaction patterns
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ServiceCapabilities>,
    /// Freeform notes on downstream auth expectations
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_notes: Option<String>,
    /// Important caveats or limitations for agents and CLI users
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_limitations: Option<String>,
    /// Downstream permissions required for key actions (e.g., "ornn:skill:create")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_permissions: Option<Vec<String>>,
    /// URL to examples, starter templates, or skill registry
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub examples_url: Option<String>,
    /// Relevant skill names/paths for AI tools (e.g., "nyxid/ornn", "ornn/authoring")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_skills: Option<Vec<String>>,

    /// Custom User-Agent header to send to the downstream service.
    /// When set, overrides the client's User-Agent instead of forwarding it.
    /// When None, the client's User-Agent is forwarded as-is (passthrough).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_user_agent: Option<String>,

    /// Admin-configured default HTTP headers injected on every proxied
    /// request to this service. Applied after caller headers are filtered
    /// through the forward-allowlist and after identity propagation headers.
    ///
    /// Precedence: non-overridable entries replace any existing value for
    /// that name (including the caller's). Overridable entries only fill
    /// in when the caller (or a higher layer like `UserService` defaults)
    /// has not set that name. See
    /// `crate::models::default_request_header::merge_into_header_list`
    /// for the merge semantics.
    ///
    /// Denylisted header names (`authorization`, `cookie`, `host`,
    /// `x-nyxid-*`, `x-forwarded-*`, `proxy-*`, hop-by-hop headers) are
    /// rejected at update time — see
    /// `crate::models::default_request_header::validate_headers`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_request_headers: Option<Vec<DefaultRequestHeader>>,

    /// WebSocket frame-auth injection rules. Orthogonal to HTTP
    /// `auth_method`; existing documents default to no frame injection.
    #[serde(default)]
    pub ws_frame_injections: Vec<WsFrameInjection>,

    /// Developer app (OAuth client) IDs that grant access to this service.
    /// When set on a private service, users who have consented to any of
    /// these apps will have the service auto-provisioned in their AI Services.
    /// Ignored for public services (they auto-provision for everyone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub developer_app_ids: Option<Vec<String>>,

    /// Declarative token exchange config. Required when `auth_method` is
    /// `token_exchange`, ignored otherwise. See [`TokenExchangeConfig`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_exchange_config: Option<TokenExchangeConfig>,

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

fn default_visibility() -> String {
    "public".to_string()
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
pub mod test_helpers {
    use super::*;

    /// Create a minimal `DownstreamService` for unit tests that need a
    /// valid struct but don't care about specific field values.
    pub fn dummy_service() -> DownstreamService {
        DownstreamService {
            id: "test-id".to_string(),
            name: "Test".to_string(),
            slug: "test".to_string(),
            description: None,
            base_url: "http://localhost".to_string(),
            service_type: "http".to_string(),
            visibility: "public".to_string(),
            auth_method: "none".to_string(),
            auth_key_name: String::new(),
            credential_encrypted: Vec::new(),
            auth_type: None,
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            streaming_supported: false,
            ssh_config: None,
            oauth_client_id: None,
            service_category: "connection".to_string(),
            requires_user_credential: false,
            is_active: true,
            created_by: "test".to_string(),
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: String::new(),
            provider_config_id: None,
            homepage_url: None,
            repository_url: None,
            issues_url: None,
            capabilities: None,
            auth_notes: None,
            known_limitations: None,
            required_permissions: None,
            examples_url: None,
            recommended_skills: None,
            custom_user_agent: None,
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
            developer_app_ids: None,
            token_exchange_config: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
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
            visibility: "public".to_string(),
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
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            provider_config_id: None,
            homepage_url: Some("https://docs.example.com".to_string()),
            repository_url: Some("https://github.com/example/repo".to_string()),
            issues_url: None,
            capabilities: Some(ServiceCapabilities {
                supports_proxy_read: true,
                supports_proxy_write: true,
                ..Default::default()
            }),
            auth_notes: Some("Bearer token required".to_string()),
            known_limitations: None,
            required_permissions: Some(vec!["read:api".to_string()]),
            examples_url: Some("https://github.com/example/repo/tree/main/examples".to_string()),
            recommended_skills: Some(vec!["example/skill".to_string()]),
            custom_user_agent: None,
            default_request_headers: Some(vec![
                crate::models::default_request_header::DefaultRequestHeader {
                    name: "x-openclaw-scopes".to_string(),
                    value: "operator.read,operator.write".to_string(),
                    overridable: false,
                    sensitive: false,
                },
            ]),
            ws_frame_injections: Vec::new(),
            developer_app_ids: None,
            token_exchange_config: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&svc).expect("serialize");
        let restored: DownstreamService = bson::from_document(doc).expect("deserialize");
        assert_eq!(svc.id, restored.id);
        assert_eq!(svc.slug, restored.slug);
        assert_eq!(svc.service_type, restored.service_type);
        assert_eq!(
            svc.default_request_headers,
            restored.default_request_headers
        );
        assert_eq!(svc.service_category, restored.service_category);
        assert_eq!(svc.homepage_url, restored.homepage_url);
        assert_eq!(svc.repository_url, restored.repository_url);
        assert!(restored.capabilities.unwrap().supports_proxy_read);
        assert_eq!(svc.required_permissions, restored.required_permissions);
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
            visibility: "public".to_string(),
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
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            provider_config_id: None,
            homepage_url: None,
            repository_url: None,
            issues_url: None,
            capabilities: None,
            auth_notes: None,
            known_limitations: None,
            required_permissions: None,
            examples_url: None,
            recommended_skills: None,
            custom_user_agent: None,
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
            developer_app_ids: None,
            token_exchange_config: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let mut doc = bson::to_document(&svc).expect("serialize");
        // Remove the fields that have #[serde(default = ...)]
        doc.remove("service_type");
        doc.remove("visibility");
        doc.remove("service_category");
        doc.remove("requires_user_credential");
        doc.remove("identity_propagation_mode");
        doc.remove("forward_access_token");
        doc.remove("inject_delegation_token");
        doc.remove("delegation_token_scope");
        let restored: DownstreamService = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.service_type, "http");
        assert_eq!(restored.visibility, "public");
        assert_eq!(restored.service_category, "connection");
        assert_eq!(restored.identity_propagation_mode, "none");
        assert!(restored.requires_user_credential);
        assert!(!restored.forward_access_token);
        assert!(!restored.inject_delegation_token);
        assert_eq!(restored.delegation_token_scope, "llm:proxy");
    }

    #[test]
    fn ssh_config_roundtrip() {
        let config = SshServiceConfig {
            host: "ssh.internal.example".to_string(),
            port: 22,
            ssh_auth_mode: SshAuthMode::Cert,
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
        assert_eq!(restored.ssh_auth_mode, SshAuthMode::Cert);
        assert!(restored.certificate_auth_enabled);
        assert_eq!(restored.allowed_principals, vec!["ubuntu".to_string()]);
    }
}
