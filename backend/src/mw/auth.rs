use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, FromRequestParts, OriginalUri},
    http::{HeaderMap, Method, header, request::Parts},
    middleware::Next,
    response::IntoResponse,
};
use base64::Engine as _;
use mongodb::bson::doc;
use uuid::Uuid;

use crate::AppState;
use crate::crypto::jwt;
use crate::crypto::token::hash_token;
use crate::errors::AppError;
use crate::models::api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as API_KEYS};
use crate::models::service_account::{COLLECTION_NAME as SERVICE_ACCOUNTS, ServiceAccount};
use crate::models::service_account_token::{COLLECTION_NAME as SA_TOKENS, ServiceAccountToken};
use crate::models::session::{COLLECTION_NAME as SESSIONS, Session};
use crate::models::user::{COLLECTION_NAME as USERS, User};

/// Authenticated user extracted from session cookie or Bearer token.
///
/// This acts as an Axum extractor: handlers that include `AuthUser` in their
/// parameters will automatically reject unauthenticated requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethod {
    /// Browser session cookie
    Session,
    /// Bearer access token (JWT)
    AccessToken,
    /// Channel relay access token (JWT with `relay: true`), minted per inbound
    /// bot message and shipped to the client callback URL as `X-NyxID-User-Token`.
    /// Accepted only on proxy/LLM surfaces (rejected elsewhere by
    /// `reject_relay_tokens`); inherits the originating agent key's service/node
    /// allowlist and does NOT bypass approval enforcement (only `Session` does).
    Relay,
    /// X-API-Key header
    ApiKey,
    /// Service account client credentials
    ServiceAccount,
    /// Delegated access token
    Delegated,
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub session_id: Option<Uuid>,
    /// Space-separated scopes from the access token or API key (empty for session auth).
    pub scope: String,
    /// If this is a delegated request, the OAuth client_id of the acting service.
    pub acting_client_id: Option<String>,
    /// Registered OAuth client that received this ordinary access token.
    pub oauth_client_id: Option<String>,
    /// JWT ID for online delegated-authority validation. `None` for non-JWT
    /// authentication contexts.
    pub token_jti: Option<String>,
    pub verified_catalog_grant:
        Option<crate::services::catalog_delegation_service::VerifiedCatalogGrant>,
    /// Resource-owner user ID used for approval/notification decisions.
    /// For service-account auth this points to the SA owner; otherwise `None`.
    pub approval_owner_user_id: Option<String>,
    /// How the user authenticated this request.
    pub auth_method: AuthMethod,
    /// If true, key can access ALL of the user's external services (default for non-API-key auth).
    pub allow_all_services: bool,
    /// If true, key can route through ALL of the user's nodes (default for non-API-key auth).
    pub allow_all_nodes: bool,
    /// List of UserService IDs this key can access (only checked when allow_all_services is false).
    pub allowed_service_ids: Vec<String>,
    /// RFC 8707 resource URI restrictions carried by OAuth bearer tokens.
    pub resource_uris: Option<Vec<String>>,
    /// List of Node IDs this key can route through (only checked when allow_all_nodes is false).
    pub allowed_node_ids: Vec<String>,
    /// API key ID when auth_method == ApiKey (for agent identity tracking)
    pub api_key_id: Option<String>,
    /// Human-readable API key name (for audit logs)
    pub api_key_name: Option<String>,
    /// Immutable security class copied from the verified API key. Non-API-key
    /// authentication contexts use `general`.
    pub api_key_purpose: ApiKeyPurpose,
    /// Per-agent rate limit (from ApiKey), None = use user-level defaults
    pub rate_limit_per_second: Option<u32>,
    pub rate_limit_burst: Option<u32>,
    /// Client IP captured at extraction time (from X-Forwarded-For, X-Real-IP, or
    /// the TCP peer address). Used to enrich audit log entries.
    pub ip_address: Option<String>,
    /// Client User-Agent header captured at extraction time. Used to enrich audit
    /// log entries.
    pub user_agent: Option<String>,
}

/// Extract the client IP from common reverse-proxy headers, falling back to the
/// TCP peer address available via `ConnectInfo`.
///
/// Lookup order: `X-Forwarded-For` (first hop), `X-Real-IP`, then the peer
/// socket address.
fn extract_request_ip(parts: &Parts) -> Option<String> {
    if let Some(forwarded) = parts
        .headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(forwarded);
    }

    if let Some(real_ip) = parts
        .headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(real_ip);
    }

    parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip().to_string())
}

/// Extract the User-Agent header.
fn extract_request_user_agent(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

impl AuthUser {
    /// Resource owner whose approval settings should be consulted.
    pub fn effective_approval_owner_user_id(&self) -> String {
        self.approval_owner_user_id
            .clone()
            .unwrap_or_else(|| self.user_id.to_string())
    }

    /// User ID whose services should be considered for proxy resolution.
    ///
    /// Service-account tokens use the service account UUID as the authenticated
    /// subject for audit/requester attribution, but proxy resources are owned
    /// by the service account's effective owner. For all non-ServiceAccount auth
    /// methods (Session, AccessToken, ApiKey) this returns `user_id.to_string()`,
    /// identical to prior behavior.
    pub fn proxy_resolution_user_id(&self) -> String {
        if self.auth_method == AuthMethod::ServiceAccount
            && let Some(owner) = &self.approval_owner_user_id
        {
            return owner.clone();
        }

        self.user_id.to_string()
    }

    /// Canonical requester type used in approval request and grant records.
    /// Session callers never enter approval flow.
    pub fn approval_requester_type(&self) -> Option<&'static str> {
        match self.auth_method {
            AuthMethod::ApiKey => Some("api_key"),
            AuthMethod::Delegated => Some("delegated"),
            AuthMethod::ServiceAccount => Some("service_account"),
            AuthMethod::AccessToken => Some("access_token"),
            AuthMethod::Relay => Some("relay"),
            AuthMethod::Session => None,
        }
    }

    /// Canonical requester ID used in approval request and grant records.
    /// Delegated tokens use acting client_id; all others use token subject.
    pub fn approval_requester_id(&self) -> String {
        self.acting_client_id
            .clone()
            .unwrap_or_else(|| self.user_id.to_string())
    }

    pub fn has_scope(&self, expected: &str) -> bool {
        scope_contains(&self.scope, expected)
    }

    pub fn verified_catalog_grant(
        &self,
    ) -> Option<&crate::services::catalog_delegation_service::VerifiedCatalogGrant> {
        self.verified_catalog_grant.as_ref()
    }

    pub fn can_use_rest_proxy(&self) -> bool {
        matches!(self.auth_method, AuthMethod::Session)
            || self.has_scope(PROXY_SCOPE)
            || self.has_scope(WIDE_PROXY_SCOPE)
    }

    pub fn can_use_llm_proxy(&self) -> bool {
        matches!(self.auth_method, AuthMethod::Session) || scope_allows_llm_proxy(&self.scope)
    }

    pub fn ensure_rest_proxy_access(&self) -> Result<(), AppError> {
        if self.can_use_rest_proxy() {
            return Ok(());
        }

        Err(AppError::Forbidden(format!(
            "Missing required scope for proxy access. Expected one of: {PROXY_SCOPE}, {WIDE_PROXY_SCOPE}"
        )))
    }

    pub fn ensure_llm_proxy_access(&self) -> Result<(), AppError> {
        if self.can_use_llm_proxy() {
            return Ok(());
        }

        Err(AppError::Forbidden(format!(
            "Missing required scope for LLM proxy access. Expected one of: {PROXY_SCOPE}, {WIDE_PROXY_SCOPE}, {LLM_PROXY_SCOPE}"
        )))
    }

    pub fn can_write(&self) -> bool {
        !matches!(self.auth_method, AuthMethod::ApiKey)
            || self.has_scope(WRITE_SCOPE)
            || self.has_scope(ADMIN_SCOPE)
    }

    pub fn ensure_write_scope(&self) -> Result<(), AppError> {
        if self.can_write() {
            return Ok(());
        }
        Err(AppError::Forbidden(
            "write or admin scope required for this operation".to_string(),
        ))
    }

    pub fn ensure_management_write_scope(
        &self,
        method: &Method,
        path: &str,
    ) -> Result<(), AppError> {
        if matches!(self.auth_method, AuthMethod::ApiKey)
            && api_key_management_write_requires_scope(method, path)
        {
            self.ensure_write_scope()?;
        }
        Ok(())
    }
}

/// Name of the session cookie.
pub const SESSION_COOKIE_NAME: &str = "nyx_session";

/// Name of the access token cookie.
pub const ACCESS_TOKEN_COOKIE_NAME: &str = "nyx_access_token";

/// Scope that grants management write access (create, update, delete, rotate).
pub const WRITE_SCOPE: &str = "write";

/// Scope that grants full admin access (implies write).
pub const ADMIN_SCOPE: &str = "admin";

/// Scope that grants standard NyxID proxy access.
pub const PROXY_SCOPE: &str = "proxy";

/// Scope that grants broad delegated/service-account proxy access.
pub const WIDE_PROXY_SCOPE: &str = "proxy:*";

/// Scope that grants access to the LLM gateway.
pub const LLM_PROXY_SCOPE: &str = "llm:proxy";

/// Scope that grants read-only access to user account management resources.
pub const ACCOUNT_READ_SCOPE: &str = "account:read";

/// Scope that grants the delegated connected-service catalog read.
pub const MCP_CATALOG_READ_SCOPE: &str = "mcp:catalog:read";

/// Delegation scopes that may be configured on downstream and user services.
/// OAuth-client delegation validation intentionally uses a narrower list.
pub const SERVICE_DELEGATION_SCOPES: &[&str] = &[
    LLM_PROXY_SCOPE,
    WIDE_PROXY_SCOPE,
    "llm:status",
    ACCOUNT_READ_SCOPE,
    "sandbox:execute",
];

const DELEGATED_ENDPOINT_FORBIDDEN: &str = "Delegated tokens cannot access this endpoint";

fn scope_contains(scopes: &str, expected: &str) -> bool {
    scopes.split_whitespace().any(|scope| scope == expected)
}

pub fn scope_allows_rest_proxy(scopes: &str) -> bool {
    scope_contains(scopes, PROXY_SCOPE) || scope_contains(scopes, WIDE_PROXY_SCOPE)
}

pub fn scope_allows_llm_proxy(scopes: &str) -> bool {
    scope_allows_rest_proxy(scopes) || scope_contains(scopes, LLM_PROXY_SCOPE)
}

fn ensure_api_key_purpose_route(api_key: &ApiKey, path: &str) -> Result<(), AppError> {
    if api_key.purpose != ApiKeyPurpose::ScheduledInvocation {
        return Ok(());
    }
    let concrete_proxy_route =
        path_matches_prefix(path, "/api/v1/proxy") && path != "/api/v1/proxy/services";
    if concrete_proxy_route {
        return Ok(());
    }
    Err(AppError::DurableGrantMismatch(
        "scheduled_invocation API keys are restricted to durable proxy execution routes"
            .to_string(),
    ))
}

fn api_key_management_write_requires_scope(method: &Method, path: &str) -> bool {
    if !matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) || !path_matches_prefix(path, "/api/v1")
    {
        return false;
    }

    ![
        "/api/v1/channel-events",
        "/api/v1/channel-relay",
        "/api/v1/delegation",
        "/api/v1/llm",
        "/api/v1/platform-ops",
        "/api/v1/proxy",
        "/api/v1/ssh",
        "/api/v1/approvals/exact-service",
    ]
    .iter()
    .any(|prefix| path_matches_prefix(path, prefix))
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn contains_percent_encoded_slash(path: &str) -> bool {
    path.as_bytes().windows(3).any(|window| {
        window[0] == b'%' && window[1] == b'2' && window[2].eq_ignore_ascii_case(&b'f')
    })
}

/// Return normalized path segments below `/api/v1` without applying the
/// management-route percent-encoding guard.
fn api_v1_path_segments_unchecked(path: &str) -> Option<Vec<&str>> {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    segments
        .starts_with(&["api", "v1"])
        .then(|| segments.into_iter().skip(2).collect())
}

/// Return normalized path segments below `/api/v1`.
///
/// Duplicate and trailing slashes are collapsed by dropping empty segments.
/// Query strings are ignored. Percent-encoded slashes are rejected instead of
/// decoded so a path cannot change management route class after authorization.
fn api_v1_path_segments(path: &str) -> Option<Vec<&str>> {
    let path_without_query = path.split_once('?').map_or(path, |(path, _)| path);
    if contains_percent_encoded_slash(path_without_query) {
        return None;
    }

    api_v1_path_segments_unchecked(path)
}

/// Native delegated routes retain their existing behavior for every method and
/// scope. These routes are checked before the account-read management rule.
fn is_delegated_native_path(path: &str) -> bool {
    let Some(segments) = api_v1_path_segments_unchecked(path) else {
        return false;
    };

    if matches!(
        segments.first().copied(),
        Some("llm" | "delegation" | "proxy" | "demo" | "channel-relay" | "channel-events")
    ) {
        return true;
    }

    matches!(segments.as_slice(), ["approvals", "requests", _, "status"])
        || segments.starts_with(&["approvals", "exact-service"])
}

/// Return true for management GET classes that delegated account reads must
/// never reach. Matching is segment-based so adjacent names such as
/// `/nodesXYZ` and `/administer` remain ordinary default-allowed GET paths.
fn delegated_read_denied_path(path: &str) -> bool {
    let Some(segments) = api_v1_path_segments(path) else {
        return true;
    };

    if matches!(
        segments.first().copied(),
        Some(
            "admin"
                | "ssh"
                | "assistant"
                | "auth"
                | "devices"
                | "cli-pairings"
                | "services"
                | "mcp"
                | "webhooks"
                | "integrations"
                | "billing"
                | "oracle"
                | "channel-bots"
                | "channel-conversations"
                | "connect-links"
                | "platform-ops"
        )
    ) {
        return true;
    }

    // Node WebSocket transport and both pending-credential URL shapes are
    // protocols that deliver or advance one-time credential material.
    if matches!(segments.as_slice(), ["nodes", "ws"])
        || (segments.first() == Some(&"nodes")
            && (segments.get(1) == Some(&"credentials") || segments.get(2) == Some(&"credentials")))
    {
        return true;
    }

    // Org invites expose a redeemable bearer nonce. Keep ordinary org
    // inventory readable while denying both the mounted collection route and
    // the reserved item shape should a GET handler be added later.
    if matches!(
        segments.as_slice(),
        ["orgs", _, "invites"] | ["orgs", _, "invites", _]
    ) {
        return true;
    }

    // Provider inventory and credential-presence reads stay available. OAuth
    // initiation/callback GETs mutate protocol state or deliver authorization
    // material and therefore require explicit exclusions within the family.
    matches!(
        segments.as_slice(),
        ["providers", "callback"]
            | ["providers", _, "callback"]
            | ["providers", _, "connect", "oauth"]
    )
}

fn header_contains_token(headers: &HeaderMap, name: &header::HeaderName, expected: &str) -> bool {
    headers.get_all(name).iter().any(|value| {
        value.to_str().ok().is_some_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case(expected))
        })
    })
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    header_contains_token(headers, &header::CONNECTION, "upgrade")
        && header_contains_token(headers, &header::UPGRADE, "websocket")
}

fn delegated_request_allowed(
    method: &Method,
    path: &str,
    headers: &HeaderMap,
    scopes: &str,
) -> bool {
    if is_delegated_native_path(path) {
        return true;
    }

    if matches!(
        api_v1_path_segments(path).as_deref(),
        Some(["mcp", "config"])
    ) {
        return method == Method::GET
            && scope_contains(scopes, MCP_CATALOG_READ_SCOPE)
            && !is_websocket_upgrade(headers);
    }

    method == Method::GET
        && scope_contains(scopes, ACCOUNT_READ_SCOPE)
        && !delegated_read_denied_path(path)
        && !is_websocket_upgrade(headers)
}

fn delegated_path_class(path: &str) -> String {
    api_v1_path_segments(path)
        .and_then(|segments| segments.first().map(|segment| (*segment).to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

fn ensure_delegated_claim_consistency(claims: &jwt::Claims) -> Result<(), AppError> {
    if (claims.delegated == Some(true)) != claims.act.is_some() {
        return Err(AppError::Unauthorized(
            "Invalid delegated token claims".to_string(),
        ));
    }
    Ok(())
}

async fn validate_dpop_bound_access(
    parts: &Parts,
    state: &AppState,
    expected_jkt: &str,
) -> Result<(), AppError> {
    let proof = parts
        .headers
        .get("dpop")
        .ok_or_else(|| AppError::Unauthorized("DPoP proof required".to_string()))?
        .to_str()
        .map_err(|_| AppError::Unauthorized("invalid DPoP proof".to_string()))?;
    let expected_htu =
        crate::crypto::dpop::htu_from_base_and_path(&state.config.base_url, parts.uri.path())?;
    let proof_jkt =
        crate::crypto::dpop::validate_proof(proof, parts.method.as_str(), &expected_htu, &state.db)
            .await?;
    if proof_jkt != expected_jkt {
        return Err(AppError::Unauthorized("DPoP cnf mismatch".to_string()));
    }
    Ok(())
}

fn validate_mtls_bound_access(
    parts: &Parts,
    state: &AppState,
    expected_x5t: &str,
) -> Result<(), AppError> {
    let header_name = state
        .config
        .mtls_client_cert_header
        .as_deref()
        .filter(|header| !header.trim().is_empty())
        .ok_or_else(|| {
            AppError::Unauthorized(
                "mTLS binding required but server has no cert header configured".to_string(),
            )
        })?;
    let cert_header = parts
        .headers
        .get(header_name)
        .ok_or_else(|| {
            AppError::Unauthorized("mTLS binding required: missing cert header".to_string())
        })?
        .to_str()
        .map_err(|_| AppError::Unauthorized("invalid mTLS client certificate".to_string()))?;
    let presented = crate::crypto::mtls::cert_thumbprint_from_header(cert_header)?;
    if presented != expected_x5t {
        return Err(AppError::Unauthorized(
            "mTLS cert thumbprint mismatch".to_string(),
        ));
    }
    Ok(())
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    /// Extract the authenticated user from the request.
    ///
    /// Checks in order:
    /// 1. Authorization header (Bearer token)
    /// 2. Session cookie
    #[allow(clippy::manual_async_fn)]
    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        async move {
            let request_ip = extract_request_ip(parts);
            let request_ua = extract_request_user_agent(parts);
            // Try Bearer token first
            if let Some(auth_header) = parts.headers.get("authorization") {
                let auth_str = auth_header.to_str().map_err(|_| {
                    AppError::Unauthorized("Invalid authorization header".to_string())
                })?;

                let bearer_token = auth_str.strip_prefix("Bearer ");
                let dpop_token = auth_str.strip_prefix("DPoP ");
                if let Some(token) = bearer_token.or(dpop_token) {
                    let allow_api_key_fallback = bearer_token.is_some();
                    // Try JWT verification first. If it fails for a reason
                    // other than expiry, fall back to API-key validation so
                    // that OpenAI-compatible clients (which send API keys as
                    // `Authorization: Bearer <key>`) work against the LLM
                    // gateway and proxy routes.
                    let claims = match jwt::verify_token(&state.jwt_keys, &state.config, token) {
                        Ok(claims) => claims,
                        Err(AppError::TokenExpired) => return Err(AppError::TokenExpired),
                        Err(jwt_err) => {
                            if !allow_api_key_fallback {
                                return Err(jwt_err);
                            }
                            match crate::services::key_service::validate_api_key(&state.db, token)
                                .await
                            {
                                Ok((api_user_id_str, api_key)) => {
                                    ensure_api_key_purpose_route(&api_key, parts.uri.path())?;
                                    let user_id =
                                        Uuid::parse_str(&api_user_id_str).map_err(|_| {
                                            AppError::Internal(
                                                "Invalid user_id in API key".to_string(),
                                            )
                                        })?;

                                    let user_model = state
                                        .db
                                        .collection::<User>(USERS)
                                        .find_one(doc! { "_id": &api_user_id_str })
                                        .await
                                        .map_err(|e| {
                                            AppError::Internal(format!("User lookup failed: {e}"))
                                        })?;

                                    match user_model {
                                        Some(u) if u.is_active => {}
                                        _ => {
                                            return Err(AppError::Unauthorized(
                                                "User account is inactive".to_string(),
                                            ));
                                        }
                                    }

                                    let auth_user = AuthUser {
                                        user_id,
                                        session_id: None,
                                        scope: api_key.scopes.clone(),
                                        acting_client_id: None,
                                        oauth_client_id: None,
                                        token_jti: None,
                                        verified_catalog_grant: None,
                                        approval_owner_user_id: None,
                                        auth_method: AuthMethod::ApiKey,
                                        allow_all_services: api_key.allow_all_services,
                                        allow_all_nodes: api_key.allow_all_nodes,
                                        allowed_service_ids: api_key.allowed_service_ids.clone(),
                                        resource_uris: None,
                                        allowed_node_ids: api_key.allowed_node_ids.clone(),
                                        api_key_id: Some(api_key.id.clone()),
                                        api_key_name: Some(api_key.name.clone()),
                                        api_key_purpose: api_key.purpose,
                                        rate_limit_per_second: api_key.rate_limit_per_second,
                                        rate_limit_burst: api_key.rate_limit_burst,
                                        ip_address: request_ip.clone(),
                                        user_agent: request_ua.clone(),
                                    };
                                    auth_user.ensure_management_write_scope(
                                        &parts.method,
                                        parts.uri.path(),
                                    )?;
                                    return Ok(auth_user);
                                }
                                Err(_) => return Err(jwt_err),
                            }
                        }
                    };

                    // Every NyxID delegated-token mint sets both RFC 8693
                    // `act` and `delegated: true`. Reject either half by itself
                    // after signature verification so the middleware's
                    // unverified delegated peek cannot disagree with the
                    // authoritative classifier below.
                    ensure_delegated_claim_consistency(&claims)?;

                    if claims.token_type != "access" {
                        return Err(AppError::Unauthorized("Expected access token".to_string()));
                    }

                    if let Some(claims_jkt) = claims.cnf.as_ref().and_then(|c| c.jkt.as_deref()) {
                        validate_dpop_bound_access(parts, state, claims_jkt).await?;
                    }
                    if let Some(claims_x5t) =
                        claims.cnf.as_ref().and_then(|c| c.x5t_s256.as_deref())
                    {
                        validate_mtls_bound_access(parts, state, claims_x5t)?;
                    }

                    // Check if this is a service account token
                    if claims.sa == Some(true) {
                        let sa_id = claims.sub.clone();

                        // Verify the service account exists and is active
                        let sa = state
                            .db
                            .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
                            .find_one(doc! { "_id": &sa_id, "is_active": true })
                            .await
                            .map_err(|e| AppError::Internal(format!("SA lookup failed: {e}")))?
                            .ok_or_else(|| {
                                AppError::Unauthorized(
                                    "Service account is inactive or not found".to_string(),
                                )
                            })?;

                        // Check token revocation
                        let token_record = state
                            .db
                            .collection::<ServiceAccountToken>(SA_TOKENS)
                            .find_one(doc! { "jti": &claims.jti })
                            .await
                            .map_err(|e| {
                                AppError::Internal(format!("SA token lookup failed: {e}"))
                            })?;

                        if let Some(record) = token_record
                            && record.revoked
                        {
                            return Err(AppError::Unauthorized(
                                "Token has been revoked".to_string(),
                            ));
                        }

                        let sa_uuid = Uuid::parse_str(&sa_id).map_err(|_| {
                            AppError::Unauthorized("Invalid service account ID".to_string())
                        })?;

                        return Ok(AuthUser {
                            user_id: sa_uuid,
                            session_id: None,
                            scope: claims.scope.clone(),
                            acting_client_id: None,
                            oauth_client_id: None,
                            token_jti: None,
                            verified_catalog_grant: None,
                            approval_owner_user_id: Some(sa.effective_owner_user_id().to_string()),
                            auth_method: AuthMethod::ServiceAccount,
                            allow_all_services: true,
                            allow_all_nodes: true,
                            allowed_service_ids: vec![],
                            resource_uris: None,
                            allowed_node_ids: vec![],
                            api_key_id: None,
                            api_key_name: None,
                            api_key_purpose: ApiKeyPurpose::General,
                            rate_limit_per_second: None,
                            rate_limit_burst: None,
                            ip_address: request_ip.clone(),
                            user_agent: request_ua.clone(),
                        });
                    }

                    let user_id = Uuid::parse_str(&claims.sub)
                        .map_err(|_| AppError::Unauthorized("Invalid token subject".to_string()))?;

                    let user_id_str = user_id.to_string();

                    // Verify the user account is still active
                    let user_model = state
                        .db
                        .collection::<User>(USERS)
                        .find_one(doc! { "_id": &user_id_str })
                        .await
                        .map_err(|e| AppError::Internal(format!("User lookup failed: {e}")))?;

                    match user_model {
                        Some(u) if u.is_active => {}
                        _ => {
                            return Err(AppError::Unauthorized(
                                "User account is inactive".to_string(),
                            ));
                        }
                    }

                    let auth_method = if claims.act.is_some() {
                        AuthMethod::Delegated
                    } else if claims.relay == Some(true) {
                        AuthMethod::Relay
                    } else {
                        AuthMethod::AccessToken
                    };

                    // Online grant validation must complete before route admission uses the scope.
                    let verified_catalog_grant = if auth_method == AuthMethod::Delegated
                        && crate::services::catalog_delegation_service::scope_has_catalog_read(
                            &claims.scope,
                        )
                    {
                        Some(
                            crate::services::catalog_delegation_service::validate_live_grant(
                                &state.db,
                                &state.config,
                                &claims,
                            )
                            .await?,
                        )
                    } else {
                        None
                    };
                    if auth_method == AuthMethod::Delegated {
                        let request_path = parts
                            .extensions
                            .get::<OriginalUri>()
                            .map_or_else(|| parts.uri.path(), |uri| uri.path());
                        if !delegated_request_allowed(
                            &parts.method,
                            request_path,
                            &parts.headers,
                            &claims.scope,
                        ) {
                            return Err(AppError::Forbidden(
                                DELEGATED_ENDPOINT_FORBIDDEN.to_string(),
                            ));
                        }

                        if !is_delegated_native_path(request_path) {
                            let acting = claims
                                .act
                                .as_ref()
                                .expect("consistent delegated claims include act")
                                .sub
                                .as_str();
                            let path_class = delegated_path_class(request_path);
                            tracing::info!(
                                user_id = %user_id,
                                acting = %acting,
                                method = %parts.method,
                                path_class = %path_class,
                                outcome = "accepted",
                                "delegated_account_read"
                            );
                        }
                    }

                    // For relay tokens, verify the originating agent key is still
                    // active. Relay tokens are stateless JWTs that leave NyxID's
                    // trust boundary (shipped to a client callback URL); this DB
                    // check is the revocation lever the relay branch previously
                    // lacked, so deleting/deactivating the agent key immediately
                    // kills its relay tokens (matching the ApiKey path).
                    if auth_method == AuthMethod::Relay {
                        ensure_relay_agent_key_active(&state.db, &claims).await?;
                    }

                    // Relay tokens inherit the originating agent key's scope.
                    // OAuth access tokens, including delegated tokens, carry
                    // optional resource/service restrictions in JWT claims;
                    // absent claims are legacy allow-all.
                    let (
                        allow_all_services,
                        allow_all_nodes,
                        allowed_service_ids,
                        allowed_node_ids,
                        api_key_id,
                        api_key_name,
                    ) = if auth_method == AuthMethod::Relay {
                        relay_scope_from_claims(&claims)
                    } else if matches!(auth_method, AuthMethod::AccessToken | AuthMethod::Delegated)
                    {
                        (
                            claims.allow_all_services.unwrap_or(true),
                            claims.allow_all_nodes.unwrap_or(true),
                            claims.allowed_service_ids.clone().unwrap_or_default(),
                            claims.allowed_node_ids.clone().unwrap_or_default(),
                            None,
                            None,
                        )
                    } else {
                        (true, true, vec![], vec![], None, None)
                    };

                    return Ok(AuthUser {
                        user_id,
                        session_id: None,
                        scope: claims.scope.clone(),
                        acting_client_id: claims.act.map(|a| a.sub),
                        oauth_client_id: claims.client_id.clone(),
                        token_jti: Some(claims.jti.clone()),
                        verified_catalog_grant,
                        approval_owner_user_id: None,
                        auth_method,
                        allow_all_services,
                        allow_all_nodes,
                        allowed_service_ids,
                        resource_uris: claims.resources.clone(),
                        allowed_node_ids,
                        api_key_id,
                        api_key_name,
                        api_key_purpose: ApiKeyPurpose::General,
                        rate_limit_per_second: None,
                        rate_limit_burst: None,
                        ip_address: request_ip.clone(),
                        user_agent: request_ua.clone(),
                    });
                }
            }

            // Try session cookie
            let cookie_header = parts
                .headers
                .get("cookie")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");

            let session_token = parse_cookie(cookie_header, SESSION_COOKIE_NAME);

            if let Some(token) = session_token {
                let token_hash = hash_token(token);

                let session = state
                    .db
                    .collection::<Session>(SESSIONS)
                    .find_one(doc! { "token_hash": &token_hash, "revoked": false })
                    .await
                    .map_err(|e| AppError::Internal(format!("Session lookup failed: {e}")))?;

                if session.is_none() {
                    tracing::debug!("Session cookie present but no matching active session in DB");
                }

                match session {
                    Some(sess) if sess.expires_at > chrono::Utc::now() => {
                        let user_id = Uuid::parse_str(&sess.user_id).map_err(|_| {
                            AppError::Internal("Invalid user_id in session".to_string())
                        })?;
                        let session_id = Uuid::parse_str(&sess.id)
                            .map_err(|_| AppError::Internal("Invalid session id".to_string()))?;

                        // Verify the user account is still active
                        let user_model = state
                            .db
                            .collection::<User>(USERS)
                            .find_one(doc! { "_id": &sess.user_id })
                            .await
                            .map_err(|e| AppError::Internal(format!("User lookup failed: {e}")))?;

                        match user_model {
                            Some(u) if u.is_active => {
                                // Session-based auth uses an empty scope string.
                                // RBAC-scoped claims (roles, groups) are only
                                // included in OAuth tokens that explicitly request
                                // those scopes. Session users can retrieve RBAC
                                // data via the /oauth/userinfo endpoint instead.
                                return Ok(AuthUser {
                                    user_id,
                                    session_id: Some(session_id),
                                    scope: String::new(),
                                    acting_client_id: None,
                                    oauth_client_id: None,
                                    token_jti: None,
                                    verified_catalog_grant: None,
                                    approval_owner_user_id: None,
                                    auth_method: AuthMethod::Session,
                                    allow_all_services: true,
                                    allow_all_nodes: true,
                                    allowed_service_ids: vec![],
                                    resource_uris: None,
                                    allowed_node_ids: vec![],
                                    api_key_id: None,
                                    api_key_name: None,
                                    api_key_purpose: ApiKeyPurpose::General,
                                    rate_limit_per_second: None,
                                    rate_limit_burst: None,
                                    ip_address: request_ip.clone(),
                                    user_agent: request_ua.clone(),
                                });
                            }
                            _ => {
                                // User not found or inactive -- reject session
                                tracing::warn!(
                                    user_id = %sess.user_id,
                                    "Session auth rejected: user inactive or not found"
                                );
                            }
                        }
                    }
                    Some(sess) => {
                        tracing::debug!(
                            user_id = %sess.user_id,
                            session_id = %sess.id,
                            expires_at = %sess.expires_at,
                            "Session cookie present but session expired in DB"
                        );
                    }
                    None => {}
                }
            }

            // Legacy access-token cookies are no longer accepted for browser auth.
            // We still detect their presence for logging and CSRF hardening while
            // first-party web flows migrate to session-cookie-only auth.
            let access_token = parse_cookie(cookie_header, ACCESS_TOKEN_COOKIE_NAME);

            // Try API key (X-API-Key header)
            if let Some(api_key_header) = parts.headers.get("x-api-key") {
                let api_key = api_key_header
                    .to_str()
                    .map_err(|_| AppError::Unauthorized("Invalid API key header".to_string()))?;

                let (user_id_str, key) =
                    crate::services::key_service::validate_api_key(&state.db, api_key).await?;
                ensure_api_key_purpose_route(&key, parts.uri.path())?;

                let user_id = Uuid::parse_str(&user_id_str)
                    .map_err(|_| AppError::Internal("Invalid user_id in API key".to_string()))?;

                // Verify the user account is still active
                let user_model = state
                    .db
                    .collection::<User>(USERS)
                    .find_one(doc! { "_id": &user_id_str })
                    .await
                    .map_err(|e| AppError::Internal(format!("User lookup failed: {e}")))?;

                match user_model {
                    Some(u) if u.is_active => {}
                    _ => {
                        return Err(AppError::Unauthorized(
                            "User account is inactive".to_string(),
                        ));
                    }
                }

                let auth_user = AuthUser {
                    user_id,
                    session_id: None,
                    scope: key.scopes.clone(),
                    acting_client_id: None,
                    oauth_client_id: None,
                    token_jti: None,
                    verified_catalog_grant: None,
                    approval_owner_user_id: None,
                    auth_method: AuthMethod::ApiKey,
                    allow_all_services: key.allow_all_services,
                    allow_all_nodes: key.allow_all_nodes,
                    allowed_service_ids: key.allowed_service_ids.clone(),
                    resource_uris: None,
                    allowed_node_ids: key.allowed_node_ids.clone(),
                    api_key_id: Some(key.id.clone()),
                    api_key_name: Some(key.name.clone()),
                    api_key_purpose: key.purpose,
                    rate_limit_per_second: key.rate_limit_per_second,
                    rate_limit_burst: key.rate_limit_burst,
                    ip_address: request_ip,
                    user_agent: request_ua,
                };
                auth_user.ensure_management_write_scope(&parts.method, parts.uri.path())?;
                return Ok(auth_user);
            }

            tracing::debug!(
                has_session_cookie = session_token.is_some(),
                has_access_cookie = access_token.is_some(),
                has_api_key = parts.headers.get("x-api-key").is_some(),
                has_bearer = parts.headers.get("authorization").is_some(),
                "All auth methods exhausted"
            );

            Err(AppError::Unauthorized(
                "No valid authentication credentials provided".to_string(),
            ))
        }
    }
}

/// Resolve the service/node scope a relay token grants from its embedded
/// agent-key claims: `(allow_all_services, allow_all_nodes,
/// allowed_service_ids, allowed_node_ids, api_key_id, api_key_name)`.
///
/// Shared by the `AuthUser` extractor and the MCP transport so both enforce the
/// SAME agent-key allowlist. Previously the MCP path built its auth context
/// without reading these claims, silently treating relay tokens as unrestricted
/// (`allow_all_*` = true); routing both paths through this helper closes that
/// divergence.
pub fn relay_scope_from_claims(
    claims: &crate::crypto::jwt::Claims,
) -> (
    bool,
    bool,
    Vec<String>,
    Vec<String>,
    Option<String>,
    Option<String>,
) {
    (
        claims.relay_allow_all_services.unwrap_or(true),
        claims.relay_allow_all_nodes.unwrap_or(true),
        claims.relay_allowed_service_ids.clone().unwrap_or_default(),
        claims.relay_allowed_node_ids.clone().unwrap_or_default(),
        claims.relay_api_key_id.clone(),
        claims.relay_api_key_name.clone(),
    )
}

/// Verify the agent API key a relay token was minted from is still active.
///
/// Relay tokens (`X-NyxID-User-Token`) are stateless JWTs delivered to a
/// client-controlled callback URL and accepted as ordinary bearer auth on the
/// proxy/LLM surfaces. Without this check the token stayed valid for its full
/// TTL even after the agent key was deleted/deactivated (API keys are
/// soft-deleted via `is_active = false`). Requiring a live agent key gives
/// operators a real revocation lever and fails closed if the id is absent.
pub async fn ensure_relay_agent_key_active(
    db: &mongodb::Database,
    claims: &crate::crypto::jwt::Claims,
) -> Result<(), AppError> {
    let api_key_id = claims.relay_api_key_id.as_deref().ok_or_else(|| {
        AppError::Unauthorized("Relay token is missing its agent key binding".to_string())
    })?;

    let active = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": api_key_id, "is_active": true })
        .await
        .map_err(|e| AppError::Internal(format!("Relay agent key lookup failed: {e}")))?
        .is_some();

    if !active {
        return Err(AppError::Unauthorized(
            "Relay token's agent key is inactive or revoked".to_string(),
        ));
    }

    Ok(())
}

/// Middleware that rejects relay access tokens from non-proxy endpoints.
///
/// Relay tokens (`X-NyxID-User-Token`, JWT with `relay: true`) exist so a bot
/// callback recipient can proxy on behalf of the bot owner. They are a
/// first-party bearer credential that crosses NyxID's trust boundary, so they
/// must be usable ONLY on the delegated proxy/LLM surfaces. This middleware is
/// applied to every other `/api/v1` route group so a leaked/replayed relay
/// token cannot reach user, admin, key-management, or session endpoints.
pub async fn reject_relay_tokens(
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<impl IntoResponse, AppError> {
    if is_relay_request(&request) {
        return Err(AppError::Forbidden(
            "Relay tokens cannot access this endpoint".to_string(),
        ));
    }
    Ok(next.run(request).await)
}

/// Check if the request bears a relay token.
fn is_relay_request(request: &axum::http::Request<axum::body::Body>) -> bool {
    if let Some(auth_header) = request.headers().get("authorization")
        && let Ok(auth_str) = auth_header.to_str()
        && let Some(token) = auth_str.strip_prefix("Bearer ")
        && is_jwt_relay(token)
    {
        return true;
    }

    false
}

/// Peek at the JWT payload (without verifying signature) to check the `relay`
/// field. Mirrors `is_jwt_delegated`: a lightweight early check; a forged token
/// is still rejected later by signature verification in the `AuthUser`
/// extractor.
fn is_jwt_relay(token: &str) -> bool {
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() < 2 {
        return false;
    }

    let payload = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
        Ok(bytes) => bytes,
        Err(_) => match base64::engine::general_purpose::URL_SAFE.decode(parts[1]) {
            Ok(bytes) => bytes,
            Err(_) => return false,
        },
    };

    if let Ok(claims) = serde_json::from_slice::<serde_json::Value>(&payload) {
        return claims.get("relay") == Some(&serde_json::Value::Bool(true));
    }

    false
}

/// Fail-fast middleware for delegated tokens on management routers.
///
/// This only peeks at unverified claims. The `AuthUser` extractor repeats the
/// decision using verified claims and is the authoritative security boundary.
pub async fn reject_delegated_tokens(
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<impl IntoResponse, AppError> {
    if let Some(token) = delegated_bearer_token(&request)
        && is_jwt_delegated(token)
    {
        let scope = peek_jwt_scope(token).unwrap_or_default();
        let request_path = request
            .extensions()
            .get::<OriginalUri>()
            .map_or_else(|| request.uri().path(), |uri| uri.path());
        if !delegated_request_allowed(request.method(), request_path, request.headers(), &scope) {
            return Err(AppError::Forbidden(
                DELEGATED_ENDPOINT_FORBIDDEN.to_string(),
            ));
        }
    }
    Ok(next.run(request).await)
}

fn delegated_bearer_token(request: &axum::http::Request<axum::body::Body>) -> Option<&str> {
    request
        .headers()
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// Peek at the JWT payload (without verifying signature) to check the `delegated` field.
///
/// This is a lightweight check that avoids full JWT verification (which happens
/// later in the `AuthUser` extractor). We only inspect the unverified claims to
/// decide whether to reject early. If the token is forged, the extractor will
/// reject it during signature verification.
fn is_jwt_delegated(token: &str) -> bool {
    peek_jwt_claims(token)
        .is_some_and(|claims| claims.get("delegated") == Some(&serde_json::Value::Bool(true)))
}

fn peek_jwt_scope(token: &str) -> Option<String> {
    peek_jwt_claims(token)?
        .get("scope")?
        .as_str()
        .map(String::from)
}

fn peek_jwt_claims(token: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() < 2 {
        return None;
    }

    // Decode the payload (2nd part) from base64url (without padding)
    let payload = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
        Ok(bytes) => bytes,
        Err(_) => {
            // Retry with standard padding
            match base64::engine::general_purpose::URL_SAFE.decode(parts[1]) {
                Ok(bytes) => bytes,
                Err(_) => return None,
            }
        }
    };

    serde_json::from_slice(&payload).ok()
}

/// Middleware that rejects service account tokens from human-only endpoints.
pub async fn reject_service_account_tokens(
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<impl IntoResponse, AppError> {
    if is_service_account_request(&request) {
        return Err(AppError::Forbidden(
            "Service accounts cannot access this endpoint".to_string(),
        ));
    }
    Ok(next.run(request).await)
}

/// Middleware that rejects API-key credentials from human-only endpoints.
pub async fn reject_api_key_tokens(
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<impl IntoResponse, AppError> {
    if is_api_key_request(&request) {
        return Err(AppError::Forbidden(
            "API keys cannot access this endpoint".to_string(),
        ));
    }
    Ok(next.run(request).await)
}

fn is_api_key_request(request: &axum::http::Request<axum::body::Body>) -> bool {
    if request.headers().get("x-api-key").is_some() {
        return true;
    }

    request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|token| {
            token.starts_with("nyx_")
                || token.starts_with("nyxid_")
                || token.starts_with("nyxid_ag_")
                || token.starts_with("nyxid_sk_")
        })
        .unwrap_or(false)
}

/// Check if the request bears a service account token.
fn is_service_account_request(request: &axum::http::Request<axum::body::Body>) -> bool {
    // Check Authorization header
    if let Some(auth_header) = request.headers().get("authorization")
        && let Ok(auth_str) = auth_header.to_str()
        && let Some(token) = auth_str.strip_prefix("Bearer ")
        && is_jwt_service_account(token)
    {
        return true;
    }

    false
}

/// Peek at the JWT payload (without verifying signature) to check the `sa` field.
fn is_jwt_service_account(token: &str) -> bool {
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() < 2 {
        return false;
    }

    let payload = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
        Ok(bytes) => bytes,
        Err(_) => match base64::engine::general_purpose::URL_SAFE.decode(parts[1]) {
            Ok(bytes) => bytes,
            Err(_) => return false,
        },
    };

    if let Ok(claims) = serde_json::from_slice::<serde_json::Value>(&payload) {
        return claims.get("sa") == Some(&serde_json::Value::Bool(true));
    }

    false
}

/// Non-rejecting version of `AuthUser`.
///
/// Returns `None` instead of 401 when no valid credentials are found.
/// Used by the OAuth authorize endpoint to support unauthenticated browser
/// visits (MCP clients that haven't logged in yet).
pub struct OptionalAuthUser(pub Option<AuthUser>);

impl FromRequestParts<AppState> for OptionalAuthUser {
    type Rejection = std::convert::Infallible;

    #[allow(clippy::manual_async_fn)]
    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        async move {
            let result = AuthUser::from_request_parts(parts, state).await;
            match result {
                Ok(user) => Ok(OptionalAuthUser(Some(user))),
                Err(AppError::Unauthorized(_)) | Err(AppError::TokenExpired) => {
                    Ok(OptionalAuthUser(None))
                }
                Err(AppError::Forbidden(error)) => {
                    tracing::debug!(%error, "OptionalAuthUser rejected credentials");
                    Ok(OptionalAuthUser(None))
                }
                Err(other) => {
                    tracing::error!("OptionalAuthUser internal error: {other}");
                    Ok(OptionalAuthUser(None))
                }
            }
        }
    }
}

/// Parse a specific cookie value from a Cookie header string.
fn parse_cookie<'a>(cookie_header: &'a str, name: &str) -> Option<&'a str> {
    cookie_header.split(';').find_map(|pair| {
        let pair = pair.trim();
        let (key, value) = pair.split_once('=')?;
        if key.trim() == name {
            Some(value.trim())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};
    use axum::{Router, middleware, routing::get};
    use tower::ServiceExt;
    use uuid::Uuid;

    const DELEGATED_ALLOWED_MANAGEMENT_PATHS: &[&str] = &[
        "/api/v1/users/me",
        "/api/v1/users/me/consents",
        "/api/v1/keys",
        // Assistant-action authorization evidence. Strictly fewer properties
        // than the detail read directly above it, and no secret material, so
        // it belongs in the same allowed family.
        "/api/v1/keys/key-id/authorization",
        "/api/v1/api-keys",
        "/api/v1/api-keys/key-id/authorization",
        "/api/v1/api-keys/key-id/usage",
        "/api/v1/api-keys/key-id/bindings",
        "/api/v1/nodes",
        "/api/v1/nodes/node-id",
        "/api/v1/nodes/node-id/bindings",
        "/api/v1/catalog",
        "/api/v1/catalog/openai",
        "/api/v1/user-services",
        "/api/v1/endpoints",
        "/api/v1/orgs",
        "/api/v1/orgs/org-id",
        "/api/v1/orgs/org-id/members",
        "/api/v1/approvals/requests",
        "/api/v1/approvals/grants",
        "/api/v1/sessions",
        "/api/v1/notifications/settings",
        "/api/v1/developer/oauth-clients",
        "/api/v1/service-pools",
        "/api/v1/connections",
        "/api/v1/providers",
        "/api/v1/providers/my-tokens",
        "/api/v1/providers/provider-id",
        "/api/v1/providers/provider-id/credentials",
        "/api/v1/providers/provider-id/connect/telegram",
    ];

    fn test_auth_user(auth_method: AuthMethod, scope: &str) -> AuthUser {
        AuthUser {
            user_id: Uuid::new_v4(),
            session_id: None,
            scope: scope.to_string(),
            acting_client_id: None,
            oauth_client_id: None,
            token_jti: None,
            verified_catalog_grant: None,
            approval_owner_user_id: None,
            auth_method,
            allow_all_services: true,
            allow_all_nodes: true,
            allowed_service_ids: vec![],
            resource_uris: None,
            allowed_node_ids: vec![],
            api_key_id: None,
            api_key_name: None,
            api_key_purpose: ApiKeyPurpose::General,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            ip_address: None,
            user_agent: None,
        }
    }

    fn request_parts_with_headers(headers: &[(&str, &str)]) -> Parts {
        let mut builder = Request::builder().uri("/api/v1/keys");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let request = builder.body(Body::empty()).unwrap();
        request.into_parts().0
    }

    fn fake_jwt_from_value(payload: serde_json::Value) -> String {
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        format!("eyJhbGciOiJSUzI1NiJ9.{payload_b64}.fake_sig")
    }

    fn fake_delegated_jwt(scope: &str) -> String {
        fake_jwt_from_value(serde_json::json!({
            "sub": "user-123",
            "delegated": true,
            "act": { "sub": "aevatar" },
            "scope": scope,
        }))
    }

    async fn run_delegated_rejection_middleware(
        request: Request<Body>,
    ) -> axum::response::Response {
        Router::new()
            .fallback(|| async {
                (
                    StatusCode::IM_A_TEAPOT,
                    [("x-handler-result", "preserved")],
                    b"unchanged-handler-bytes".as_slice(),
                )
            })
            .layer(middleware::from_fn(reject_delegated_tokens))
            .oneshot(request)
            .await
            .expect("middleware response")
    }

    async fn run_human_only_rejection_layers(
        request: Request<Body>,
    ) -> axum::response::Response {
        Router::new()
            .fallback(|| async {
                (
                    StatusCode::IM_A_TEAPOT,
                    [("x-handler-result", "preserved")],
                    b"unchanged-handler-bytes".as_slice(),
                )
            })
            .layer(middleware::from_fn(reject_delegated_tokens))
            .layer(middleware::from_fn(reject_api_key_tokens))
            .layer(middleware::from_fn(reject_service_account_tokens))
            .layer(middleware::from_fn(reject_relay_tokens))
            .oneshot(request)
            .await
            .expect("human-only middleware response")
    }

    #[test]
    fn delegated_account_read_allows_expected_management_families() {
        let headers = HeaderMap::new();
        for path in DELEGATED_ALLOWED_MANAGEMENT_PATHS {
            assert!(
                delegated_request_allowed(&Method::GET, path, &headers, ACCOUNT_READ_SCOPE),
                "delegated account read should allow GET {path}"
            );
        }
    }

    #[test]
    fn delegated_account_read_rejects_every_management_write_method() {
        let headers = HeaderMap::new();
        for path in DELEGATED_ALLOWED_MANAGEMENT_PATHS {
            for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
                assert!(
                    !delegated_request_allowed(&method, path, &headers, ACCOUNT_READ_SCOPE),
                    "delegated account read should reject {method} {path}"
                );
            }
        }
    }

    #[test]
    fn delegated_proxy_scope_alone_cannot_read_any_management_family() {
        let headers = HeaderMap::new();
        for path in DELEGATED_ALLOWED_MANAGEMENT_PATHS {
            assert!(
                !delegated_request_allowed(&Method::GET, path, &headers, WIDE_PROXY_SCOPE),
                "plain proxy scope must not grant GET {path}"
            );
        }
    }

    #[test]
    fn catalog_read_scope_alone_grants_no_proxy_or_management_route() {
        let headers = HeaderMap::new();
        for path in [
            "/api/v1/keys",
            "/api/v1/orgs",
            "/api/v1/proxy/s/github/repos",
            "/api/v1/mcp/tools",
        ] {
            assert!(
                !delegated_request_allowed(&Method::GET, path, &headers, MCP_CATALOG_READ_SCOPE),
                "catalog read must not authorize GET {path}"
            );
        }
        assert!(!scope_contains(MCP_CATALOG_READ_SCOPE, PROXY_SCOPE));
        assert!(!scope_allows_llm_proxy(MCP_CATALOG_READ_SCOPE));
    }

    #[test]
    fn catalog_read_scope_allows_only_exact_catalog_route() {
        let headers = HeaderMap::new();
        assert!(delegated_request_allowed(
            &Method::GET,
            "/api/v1/mcp/config",
            &headers,
            MCP_CATALOG_READ_SCOPE
        ));
        for path in [
            "/api/v1/mcp/config/",
            "/api/v1/mcp//config",
            "/api/v1/mcp/config/extra",
            "/api/v1/mcp%2fconfig",
            "/api/v1/mcp/config%2fextra",
        ] {
            assert!(
                !delegated_request_allowed(&Method::GET, path, &headers, MCP_CATALOG_READ_SCOPE),
                "catalog read must reject GET {path}"
            );
        }
        assert!(!delegated_request_allowed(
            &Method::POST,
            "/api/v1/mcp/config",
            &headers,
            MCP_CATALOG_READ_SCOPE
        ));
        let websocket = HeaderMap::from_iter([
            (header::CONNECTION, "upgrade".parse().unwrap()),
            (header::UPGRADE, "websocket".parse().unwrap()),
        ]);
        assert!(!delegated_request_allowed(
            &Method::GET,
            "/api/v1/mcp/config",
            &websocket,
            MCP_CATALOG_READ_SCOPE
        ));
    }

    #[test]
    fn delegated_account_read_rejects_every_denied_class_and_protocol_get() {
        let headers = HeaderMap::new();
        let denied_paths = [
            "/api/v1/admin/users",
            "/api/v1/ssh/service-id",
            "/api/v1/ssh/service-id/terminal",
            "/api/v1/assistant/conversations/nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae/state",
            "/api/v1/assistant/wire-logs/7d6f176c-45c6-4efa-95b2-12dc58a7341f",
            "/api/v1/auth/social/github",
            "/api/v1/devices/code/poll",
            "/api/v1/cli-pairings/pairing-id/poll",
            "/api/v1/services",
            "/api/v1/mcp/config",
            "/api/v1/webhooks/telegram",
            "/api/v1/integrations/openclaw/mappings",
            "/api/v1/billing/wallet",
            "/api/v1/oracle/pools",
            "/api/v1/channel-bots",
            "/api/v1/channel-conversations/conversation-id",
            "/api/v1/platform-ops/x-search",
            "/api/v1/nodes/ws",
            "/api/v1/nodes/node-id/credentials",
            "/api/v1/nodes/node-id/credentials/pending",
            "/api/v1/nodes/node-id/credentials/pending/pending-id",
            "/api/v1/nodes/credentials/pending/fanout-id/fan-out",
            "/api/v1/providers/callback",
            "/api/v1/providers/provider-id/callback",
            "/api/v1/providers/provider-id/connect/oauth",
            "/api/v1/orgs/org-id/invites",
            "/api/v1/orgs/org-id/invites/invite-id",
        ];

        for path in denied_paths {
            assert!(
                delegated_read_denied_path(path),
                "missing deny classification for {path}"
            );
            assert!(
                !delegated_request_allowed(&Method::GET, path, &headers, ACCOUNT_READ_SCOPE),
                "delegated account read should reject GET {path}"
            );
        }
    }

    #[test]
    fn delegated_account_read_denies_org_invite_bearer_material() {
        let headers = HeaderMap::new();

        for path in [
            "/api/v1/orgs/org-id/invites",
            "/api/v1/orgs/org-id/invites/invite-id",
        ] {
            assert!(
                delegated_read_denied_path(path),
                "GET {path} must be denied"
            );
            assert!(!delegated_request_allowed(
                &Method::GET,
                path,
                &headers,
                ACCOUNT_READ_SCOPE,
            ));
        }

        for path in ["/api/v1/orgs", "/api/v1/orgs/org-id"] {
            assert!(
                !delegated_read_denied_path(path),
                "GET {path} stays readable"
            );
            assert!(delegated_request_allowed(
                &Method::GET,
                path,
                &headers,
                ACCOUNT_READ_SCOPE,
            ));
        }
    }

    #[test]
    fn delegated_account_read_rejects_websocket_upgrade_case_insensitively() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONNECTION, "keep-alive, UpGrAdE".parse().unwrap());
        headers.insert(header::UPGRADE, "WebSocket".parse().unwrap());

        assert!(is_websocket_upgrade(&headers));
        assert!(!delegated_request_allowed(
            &Method::GET,
            "/api/v1/keys",
            &headers,
            ACCOUNT_READ_SCOPE,
        ));
    }

    #[test]
    fn delegated_account_read_requires_exact_case_sensitive_scope_token() {
        let headers = HeaderMap::new();
        for scope in [
            "proxy:*",
            "account:reader",
            "fooaccount:read",
            "ACCOUNT:READ",
            "account:reader account:reader",
        ] {
            assert!(
                !delegated_request_allowed(&Method::GET, "/api/v1/keys", &headers, scope),
                "scope {scope:?} must not grant account read"
            );
        }

        for scope in [
            "account:read",
            "proxy:* account:read",
            "account:read account:read",
            "\taccount:read\nproxy:* ",
        ] {
            assert!(
                delegated_request_allowed(&Method::GET, "/api/v1/keys", &headers, scope),
                "scope {scope:?} contains the exact account-read token"
            );
        }
    }

    #[test]
    fn delegated_account_read_normalizes_paths_without_prefix_confusion() {
        let headers = HeaderMap::new();
        for path in [
            "//api/v1//keys",
            "/api/v1/keys/",
            "/api/v1/keys?include=all",
            "/api/v1/nodesXYZ",
            "/api/v1/administer",
        ] {
            assert!(
                delegated_request_allowed(&Method::GET, path, &headers, ACCOUNT_READ_SCOPE),
                "normalized path should remain allowed: {path}"
            );
        }

        for path in [
            "/api/v1/keys%2Fsecret",
            "/api/v1/keys%2fsecret",
            "/api/v1%2Fkeys",
        ] {
            assert!(
                !delegated_request_allowed(&Method::GET, path, &headers, ACCOUNT_READ_SCOPE),
                "percent-encoded slash must fail closed: {path}"
            );
        }
    }

    #[test]
    fn delegated_native_paths_keep_plain_proxy_scope_behavior() {
        let mut websocket_headers = HeaderMap::new();
        websocket_headers.insert(header::CONNECTION, "upgrade".parse().unwrap());
        websocket_headers.insert(header::UPGRADE, "websocket".parse().unwrap());
        let native_paths = [
            "/api/v1/proxy/s/openai/v1/models",
            "/api/v1/proxy/s/openai/a%2Fb",
            "/api/v1/llm/status",
            "/api/v1/delegation/refresh",
            "/api/v1/demo",
            "/api/v1/channel-relay/reply",
            "/api/v1/channel-events/conversation-id",
            "/api/v1/approvals/requests/request-id/status",
        ];

        for path in native_paths {
            assert!(
                is_delegated_native_path(path),
                "native path not recognized: {path}"
            );
            assert!(delegated_request_allowed(
                &Method::POST,
                path,
                &websocket_headers,
                WIDE_PROXY_SCOPE,
            ));
        }

        assert!(!is_delegated_native_path(
            "/api/v1/approvals/requests/request-id"
        ));
    }

    #[test]
    fn verified_delegated_claims_must_set_actor_and_flag_together() {
        let mut claims = jwt::Claims {
            sub: Uuid::new_v4().to_string(),
            iss: "nyxid".to_string(),
            aud: "http://localhost:3001".to_string(),
            exp: chrono::Utc::now().timestamp() + 60,
            iat: chrono::Utc::now().timestamp(),
            jti: Uuid::new_v4().to_string(),
            scope: ACCOUNT_READ_SCOPE.to_string(),
            token_type: "access".to_string(),
            client_id: None,
            roles: None,
            groups: None,
            permissions: None,
            sid: None,
            act: Some(jwt::ActorClaim {
                sub: "aevatar".to_string(),
            }),
            delegated: Some(true),
            sa: None,
            cnf: None,
            relay: None,
            relay_api_key_id: None,
            relay_api_key_name: None,
            relay_allowed_service_ids: None,
            relay_allowed_node_ids: None,
            relay_allow_all_services: None,
            relay_allow_all_nodes: None,
            resources: None,
            allowed_service_ids: None,
            allow_all_services: None,
            allowed_node_ids: None,
            allow_all_nodes: None,
            assistant_forward: None,
        };
        assert!(ensure_delegated_claim_consistency(&claims).is_ok());

        claims.act = None;
        assert!(matches!(
            ensure_delegated_claim_consistency(&claims),
            Err(AppError::Unauthorized(_))
        ));

        claims.act = Some(jwt::ActorClaim {
            sub: "aevatar".to_string(),
        });
        claims.delegated = None;
        assert!(matches!(
            ensure_delegated_claim_consistency(&claims),
            Err(AppError::Unauthorized(_))
        ));

        claims.act = None;
        claims.sa = Some(true);
        assert!(ensure_delegated_claim_consistency(&claims).is_ok());

        claims.sa = None;
        claims.relay = Some(true);
        assert!(ensure_delegated_claim_consistency(&claims).is_ok());

        claims.relay = None;
        assert!(ensure_delegated_claim_consistency(&claims).is_ok());
    }

    #[tokio::test]
    async fn delegated_middleware_preserves_allowed_handler_response_bytes() {
        let token = fake_delegated_jwt("proxy:* account:read");
        let request = Request::builder()
            .method(Method::GET)
            .uri("/api/v1/keys")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        let response = run_delegated_rejection_middleware(request).await;
        assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
        assert_eq!(response.headers()["x-handler-result"], "preserved");
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            "unchanged-handler-bytes"
        );
    }

    #[tokio::test]
    async fn delegated_middleware_keeps_existing_forbidden_error_for_denials() {
        for (method, path, scope) in [
            (Method::GET, "/api/v1/keys", WIDE_PROXY_SCOPE),
            (Method::POST, "/api/v1/keys", ACCOUNT_READ_SCOPE),
            (Method::GET, "/api/v1/admin/users", ACCOUNT_READ_SCOPE),
        ] {
            let token = fake_delegated_jwt(scope);
            let request = Request::builder()
                .method(method.clone())
                .uri(path)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap();
            let response = run_delegated_rejection_middleware(request).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {path}");
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert!(
                String::from_utf8_lossy(&body).contains(DELEGATED_ENDPOINT_FORBIDDEN),
                "forbidden response changed for {method} {path}"
            );
        }
    }

    #[tokio::test]
    async fn delegated_middleware_leaves_other_token_classes_byte_for_byte_unchanged() {
        let relay = fake_jwt_from_value(serde_json::json!({ "relay": true, "scope": "proxy:*" }));
        let service_account =
            fake_jwt_from_value(serde_json::json!({ "sa": true, "scope": "proxy:*" }));
        let access = fake_jwt_from_value(serde_json::json!({ "scope": "openid profile" }));
        let credentials = [
            (
                header::COOKIE.as_str(),
                "nyx_session=session-token".to_string(),
            ),
            ("x-api-key", "nyxid_ag_test-key".to_string()),
            (header::AUTHORIZATION.as_str(), format!("Bearer {relay}")),
            (
                header::AUTHORIZATION.as_str(),
                format!("Bearer {service_account}"),
            ),
            (header::AUTHORIZATION.as_str(), format!("Bearer {access}")),
        ];

        for (name, value) in credentials {
            let request = Request::builder()
                .method(Method::GET)
                .uri("/api/v1/keys")
                .header(name, value)
                .body(Body::empty())
                .unwrap();
            let response = run_delegated_rejection_middleware(request).await;
            assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
            assert_eq!(
                to_bytes(response.into_body(), usize::MAX).await.unwrap(),
                "unchanged-handler-bytes"
            );
        }
    }

    #[tokio::test]
    async fn human_only_layers_preserve_session_and_access_token_catalog_requests() {
        let access = fake_jwt_from_value(serde_json::json!({ "scope": "openid profile" }));
        for (name, value) in [
            (
                header::COOKIE.as_str(),
                "nyx_session=session-token".to_string(),
            ),
            (header::AUTHORIZATION.as_str(), format!("Bearer {access}")),
        ] {
            let request = Request::builder()
                .method(Method::GET)
                .uri("/api/v1/mcp/config")
                .header(name, value)
                .body(Body::empty())
                .unwrap();
            let response = run_human_only_rejection_layers(request).await;
            assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
            assert_eq!(
                to_bytes(response.into_body(), usize::MAX).await.unwrap(),
                "unchanged-handler-bytes"
            );
        }
    }

    #[tokio::test]
    async fn human_only_layers_keep_api_key_service_account_and_relay_catalog_denials() {
        let service_account = fake_jwt_from_value(serde_json::json!({ "sa": true }));
        let relay = fake_jwt_from_value(serde_json::json!({ "relay": true }));
        let requests = [
            (
                "api_key",
                Request::builder()
                    .uri("/api/v1/mcp/config")
                    .header("x-api-key", "nyxid_ag_test-key")
                    .body(Body::empty())
                    .unwrap(),
            ),
            (
                "service_account",
                Request::builder()
                    .uri("/api/v1/mcp/config")
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {service_account}"),
                    )
                    .body(Body::empty())
                    .unwrap(),
            ),
            (
                "relay",
                Request::builder()
                    .uri("/api/v1/mcp/config")
                    .header(header::AUTHORIZATION, format!("Bearer {relay}"))
                    .body(Body::empty())
                    .unwrap(),
            ),
        ];

        for (auth_method, request) in requests {
            let response = run_human_only_rejection_layers(request).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{auth_method}");
        }
    }

    fn sign_test_claims(state: &AppState, claims: &jwt::Claims) -> String {
        let mut jwt_header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        jwt_header.kid = Some(state.jwt_keys.kid.clone());
        jsonwebtoken::encode(&jwt_header, claims, &state.jwt_keys.encoding)
            .expect("sign test claims")
    }

    #[tokio::test]
    async fn authoritative_extractor_rejects_signed_delegated_claim_inconsistency() {
        let state = crate::test_utils::test_app_state_no_db().await;
        let user_id = Uuid::new_v4();
        let token = jwt::generate_delegated_access_token(
            &state.jwt_keys,
            &state.config,
            &user_id,
            ACCOUNT_READ_SCOPE,
            "aevatar",
            60,
            None,
        )
        .unwrap();
        let valid_claims = jwt::verify_token(&state.jwt_keys, &state.config, &token).unwrap();

        let mut delegated_without_actor = valid_claims.clone();
        delegated_without_actor.act = None;
        let mut actor_without_delegated = valid_claims;
        actor_without_delegated.delegated = None;

        for claims in [delegated_without_actor, actor_without_delegated] {
            let token = sign_test_claims(&state, &claims);
            let app = Router::new()
                .route(
                    "/api/v1/keys",
                    get(|_auth: AuthUser| async { StatusCode::NO_CONTENT }),
                )
                .with_state(state.clone());
            let response = app
                .oneshot(
                    Request::builder()
                        .uri("/api/v1/keys")
                        .header(header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn authoritative_extractor_rejects_management_read_without_account_scope_without_middleware()
     {
        use crate::models::user::{COLLECTION_NAME as USERS, UserType};

        let Some(db) = crate::test_utils::connect_test_database(
            "auth_delegated_extractor_without_reject_middleware",
        )
        .await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let user_id = Uuid::new_v4();
        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &user_id.to_string(),
                UserType::Person,
            ))
            .await
            .unwrap();
        let token = jwt::generate_delegated_access_token(
            &state.jwt_keys,
            &state.config,
            &user_id,
            WIDE_PROXY_SCOPE,
            "aevatar",
            60,
            None,
        )
        .unwrap();

        let response = Router::new()
            .route("/api/v1/keys", get(|_: AuthUser| async {}))
            .with_state(state)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/keys")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json["message"],
            format!("Forbidden: {DELEGATED_ENDPOINT_FORBIDDEN}")
        );
    }

    #[tokio::test]
    async fn authoritative_extractor_rejects_catalog_scope_without_live_grant_before_handler() {
        use crate::models::user::{COLLECTION_NAME as USERS, UserType};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let Some(db) = crate::test_utils::connect_test_database(
            "auth_delegated_catalog_without_live_grant",
        )
        .await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let user_id = Uuid::new_v4();
        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &user_id.to_string(),
                UserType::Person,
            ))
            .await
            .unwrap();
        let restrictions = jwt::TokenRestrictionClaims {
            resources: Some(Vec::new()),
            allowed_service_ids: Some(Vec::new()),
            allow_all_services: Some(false),
            allowed_node_ids: Some(Vec::new()),
            allow_all_nodes: Some(false),
        };
        let (token, _) = jwt::generate_delegated_access_token_for_client(
            &state.jwt_keys,
            &state.config,
            &user_id,
            MCP_CATALOG_READ_SCOPE,
            "catalog-actor",
            Some("catalog-receiver"),
            60,
            Some(&restrictions),
        )
        .unwrap();
        let handler_reached = Arc::new(AtomicBool::new(false));
        let handler_marker = Arc::clone(&handler_reached);
        let app = Router::new()
            .route(
                "/api/v1/mcp/config",
                get(move |_: AuthUser| {
                    handler_marker.store(true, Ordering::SeqCst);
                    async { StatusCode::NO_CONTENT }
                }),
            )
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/mcp/config")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(!handler_reached.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn forged_delegated_account_read_passes_peek_but_fails_real_router_verification() {
        let Some(db) = crate::test_utils::connect_test_database("auth_delegated_forged").await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db);
        let token = fake_delegated_jwt("proxy:* account:read");
        assert!(is_jwt_delegated(&token));
        assert_eq!(
            peek_jwt_scope(&token).as_deref(),
            Some("proxy:* account:read")
        );

        let (_, private) = crate::routes::build_router();
        let response = private
            .with_state(state)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/keys")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    fn delegated_fixture_api_key(
        id: &str,
        user_id: &str,
        key_hash: &str,
    ) -> crate::models::api_key::ApiKey {
        crate::models::api_key::ApiKey {
            id: id.to_string(),
            user_id: user_id.to_string(),
            name: "Delegated read fixture".to_string(),
            key_prefix: "nyxid_ag_fixture".to_string(),
            key_hash: key_hash.to_string(),
            scopes: "read proxy".to_string(),
            last_used_at: None,
            expires_at: None,
            is_active: true,
            created_at: chrono::Utc::now(),
            rotation_predecessor_id: None,
            state_version: 1,
            updated_at: Some(chrono::Utc::now()),
            description: None,
            allowed_service_ids: Vec::new(),
            allowed_node_ids: Vec::new(),
            allow_all_services: true,
            allow_all_nodes: true,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            platform: Some("aevatar".to_string()),
            callback_url: None,
            purpose: Default::default(),
            scheduled_write_enabled: false,
        }
    }

    fn delegated_fixture_node(
        id: &str,
        user_id: &str,
        auth_hash: &str,
        signing_hash: &str,
    ) -> crate::models::node::Node {
        crate::models::node::Node {
            id: id.to_string(),
            user_id: user_id.to_string(),
            name: "Delegated read fixture node".to_string(),
            status: crate::models::node::NodeStatus::Offline,
            auth_token_hash: auth_hash.to_string(),
            signing_secret_encrypted: Some(vec![9, 8, 7, 6]),
            signing_secret_hash: signing_hash.to_string(),
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None,
            metrics: Default::default(),
            connection_owner: None,
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    async fn delegated_router_response(
        app: &Router,
        method: Method,
        path: &str,
        token: &str,
    ) -> axum::response::Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("real router response")
    }

    fn assert_delegated_response_has_no_secret_fields(value: &serde_json::Value) {
        const SENSITIVE_FIELDS: &[&str] = &[
            "key",
            "key_hash",
            "credential",
            "credential_encrypted",
            "access_token",
            "access_token_encrypted",
            "refresh_token",
            "refresh_token_encrypted",
            "token",
            "token_hash",
            "auth_token",
            "auth_token_hash",
            "signing_secret",
            "signing_secret_hash",
            "signing_secret_encrypted",
            "client_secret",
            "password_hash",
            "password_reset_token",
            "email_verification_token",
            "worker_token",
            "worker_token_hash",
            "nonce",
            "secret",
            "verification_token",
            "encrypt_key",
            "user_code",
            "device_code",
        ];

        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    assert!(
                        !SENSITIVE_FIELDS.contains(&key.as_str()),
                        "delegated response exposed sensitive field {key}"
                    );
                    assert_delegated_response_has_no_secret_fields(value);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    assert_delegated_response_has_no_secret_fields(value);
                }
            }
            _ => {}
        }
    }

    #[tokio::test]
    async fn aevatar_account_read_real_router_enforces_read_acl_and_redacts_secrets() {
        use crate::models::agent_service_binding::{
            AgentServiceBinding, COLLECTION_NAME as AGENT_SERVICE_BINDINGS,
        };
        use crate::models::api_key::COLLECTION_NAME as API_KEYS;
        use crate::models::node::COLLECTION_NAME as NODES;
        use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
        use crate::models::org_invite::{COLLECTION_NAME as ORG_INVITES, OrgInvite};
        use crate::models::org_membership::{
            COLLECTION_NAME as ORG_MEMBERSHIPS, MemberScopeSource, OrgRole,
        };
        use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
        use crate::models::service_pool::{
            COLLECTION_NAME as SERVICE_POOLS, PoolStrategy, ServicePool, ServicePoolMember,
        };
        use crate::models::user::{COLLECTION_NAME as USERS, UserType};
        use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
        use crate::models::user_endpoint::COLLECTION_NAME as USER_ENDPOINTS;
        use crate::models::user_provider_credentials::{
            COLLECTION_NAME as USER_PROVIDER_CREDENTIALS, UserProviderCredentials,
        };
        use crate::models::user_provider_token::{
            COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
        };
        use crate::models::user_service::COLLECTION_NAME as USER_SERVICES;
        use crate::models::user_service_connection::{
            COLLECTION_NAME as USER_SERVICE_CONNECTIONS, UserServiceConnection,
        };

        let Some(db) =
            crate::test_utils::connect_test_database("auth_delegated_account_read_router").await
        else {
            return;
        };
        crate::services::role_service::seed_system_roles(&db)
            .await
            .expect("seed platform roles");
        let state = crate::test_utils::test_app_state(db.clone());
        let actor_id = Uuid::new_v4();
        let other_user_id = Uuid::new_v4();
        let visible_org_id = Uuid::new_v4();
        let hidden_org_id = Uuid::new_v4();

        db.collection::<User>(USERS)
            .insert_many([
                crate::test_utils::test_user(&actor_id.to_string(), UserType::Person),
                crate::test_utils::test_user(&other_user_id.to_string(), UserType::Person),
                crate::test_utils::test_user(&visible_org_id.to_string(), UserType::Org),
                crate::test_utils::test_user(&hidden_org_id.to_string(), UserType::Org),
            ])
            .await
            .unwrap();
        db.collection::<crate::models::org_membership::OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(crate::test_utils::test_membership(
                &visible_org_id.to_string(),
                &actor_id.to_string(),
                OrgRole::Admin,
                None,
            ))
            .await
            .unwrap();

        let invite_nonce = "redeemable-org-invite-nonce-fixture";
        db.collection::<OrgInvite>(ORG_INVITES)
            .insert_one(OrgInvite {
                id: Uuid::new_v4().to_string(),
                org_user_id: visible_org_id.to_string(),
                nonce: invite_nonce.to_string(),
                role: OrgRole::Admin,
                scope_source: MemberScopeSource::Override,
                allowed_service_ids: None,
                created_by: actor_id.to_string(),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                redeemed_by: None,
                redeemed_at: None,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let actor_api_key_id = Uuid::new_v4().to_string();
        let other_api_key_id = Uuid::new_v4().to_string();
        let actor_api_hash = "actor-api-key-hash-fixture";
        db.collection::<crate::models::api_key::ApiKey>(API_KEYS)
            .insert_many([
                delegated_fixture_api_key(&actor_api_key_id, &actor_id.to_string(), actor_api_hash),
                delegated_fixture_api_key(
                    &other_api_key_id,
                    &other_user_id.to_string(),
                    "other-api-key-hash-fixture",
                ),
            ])
            .await
            .unwrap();

        let actor_node_id = Uuid::new_v4().to_string();
        let other_node_id = Uuid::new_v4().to_string();
        let actor_node_auth_hash = "actor-node-auth-hash-fixture";
        let actor_node_signing_hash = "actor-node-signing-hash-fixture";
        db.collection::<crate::models::node::Node>(NODES)
            .insert_many([
                delegated_fixture_node(
                    &actor_node_id,
                    &actor_id.to_string(),
                    actor_node_auth_hash,
                    actor_node_signing_hash,
                ),
                delegated_fixture_node(
                    &other_node_id,
                    &other_user_id.to_string(),
                    "other-node-auth-hash-fixture",
                    "other-node-signing-hash-fixture",
                ),
            ])
            .await
            .unwrap();

        let raw_credential = "delegated-account-read-plaintext-secret-fixture";
        let actor_external_key_id = Uuid::new_v4().to_string();
        let encrypted_credential = state
            .encryption_keys
            .encrypt(raw_credential.as_bytes())
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(UserApiKey {
                id: actor_external_key_id.clone(),
                user_id: actor_id.to_string(),
                label: "Secret-bearing fixture".to_string(),
                credential_type: "api_key".to_string(),
                credential_encrypted: Some(encrypted_credential),
                access_token_encrypted: None,
                refresh_token_encrypted: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_attempt_nonce: None,
                user_oauth_client_id_encrypted: None,
                user_oauth_client_secret_encrypted: None,
                credential_source: None,
                status: "active".to_string(),
                last_used_at: None,
                last_authorized_at: None,
                error_message: None,
                source: Some("user_created".to_string()),
                source_id: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                credential_epoch: 1,
            })
            .await
            .unwrap();

        let actor_endpoint_id = Uuid::new_v4().to_string();
        let visible_org_endpoint_id = Uuid::new_v4().to_string();
        let hidden_org_endpoint_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user_endpoint::UserEndpoint>(USER_ENDPOINTS)
            .insert_many([
                crate::test_utils::test_user_endpoint(
                    &actor_endpoint_id,
                    &actor_id.to_string(),
                    "Actor endpoint",
                    "https://actor-api.example.test",
                    None,
                    None,
                ),
                crate::test_utils::test_user_endpoint(
                    &visible_org_endpoint_id,
                    &visible_org_id.to_string(),
                    "Visible org endpoint",
                    "https://visible-org.example.test",
                    None,
                    None,
                ),
                crate::test_utils::test_user_endpoint(
                    &hidden_org_endpoint_id,
                    &hidden_org_id.to_string(),
                    "Hidden org endpoint",
                    "https://hidden-org.example.test",
                    None,
                    None,
                ),
            ])
            .await
            .unwrap();

        let actor_service_id = Uuid::new_v4().to_string();
        let visible_org_service_id = Uuid::new_v4().to_string();
        let hidden_org_service_id = Uuid::new_v4().to_string();
        let mut actor_service = crate::test_utils::test_user_service(
            &actor_service_id,
            &actor_id.to_string(),
            "actor-service",
            &actor_endpoint_id,
            None,
            None,
        );
        actor_service.api_key_id = Some(actor_external_key_id.clone());
        actor_service.auth_method = "bearer".to_string();
        actor_service.auth_key_name = "Authorization".to_string();
        db.collection::<crate::models::user_service::UserService>(USER_SERVICES)
            .insert_many([
                actor_service,
                crate::test_utils::test_user_service(
                    &visible_org_service_id,
                    &visible_org_id.to_string(),
                    "visible-org-service",
                    &visible_org_endpoint_id,
                    None,
                    None,
                ),
                crate::test_utils::test_user_service(
                    &hidden_org_service_id,
                    &hidden_org_id.to_string(),
                    "hidden-org-service",
                    &hidden_org_endpoint_id,
                    None,
                    None,
                ),
            ])
            .await
            .unwrap();

        db.collection::<AgentServiceBinding>(AGENT_SERVICE_BINDINGS)
            .insert_one(AgentServiceBinding {
                id: Uuid::new_v4().to_string(),
                api_key_id: actor_api_key_id.clone(),
                user_service_id: actor_service_id.clone(),
                user_api_key_id: actor_external_key_id.clone(),
                user_id: actor_id.to_string(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let provider_id = Uuid::new_v4().to_string();
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(ProviderConfig {
                id: provider_id.clone(),
                slug: "delegated-provider".to_string(),
                name: "Delegated provider fixture".to_string(),
                description: None,
                provider_type: "oauth2".to_string(),
                authorization_url: Some("https://auth.example.test/authorize".to_string()),
                token_url: Some("https://auth.example.test/token".to_string()),
                revocation_url: None,
                revocation: None,
                default_scopes: Some(vec!["read:user".to_string()]),
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
                credential_mode: "both".to_string(),
                token_endpoint_auth_method: "client_secret_post".to_string(),
                extra_auth_params: None,
                device_code_format: "rfc8628".to_string(),
                client_id_param_name: None,
                requires_gateway_url: false,
                created_by: actor_id.to_string(),
                revocation_seed_version: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(UserProviderToken {
                id: Uuid::new_v4().to_string(),
                user_id: actor_id.to_string(),
                provider_config_id: provider_id.clone(),
                connection_id: Some(Uuid::new_v4().to_string()),
                credential_user_id: None,
                token_type: "oauth2".to_string(),
                access_token_encrypted: Some(vec![7, 8, 9]),
                refresh_token_encrypted: Some(vec![10, 11, 12]),
                token_scopes: Some("read:user".to_string()),
                expires_at: None,
                api_key_encrypted: None,
                status: "active".to_string(),
                state_version: 1,
                last_refreshed_at: None,
                last_used_at: None,
                error_message: None,
                label: Some("Connected account".to_string()),
                metadata: None,
                gateway_url: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        db.collection::<UserProviderCredentials>(USER_PROVIDER_CREDENTIALS)
            .insert_one(UserProviderCredentials {
                id: Uuid::new_v4().to_string(),
                user_id: actor_id.to_string(),
                provider_config_id: provider_id.clone(),
                client_id_encrypted: Some(vec![13, 14, 15]),
                client_secret_encrypted: Some(vec![16, 17, 18]),
                label: Some("BYO OAuth app".to_string()),
                state_version: 1,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let oauth_client_id = Uuid::new_v4().to_string();
        let oauth_client_hash = "delegated-oauth-client-secret-hash-fixture";
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(OauthClient {
                id: oauth_client_id.clone(),
                client_name: "Delegated OAuth client fixture".to_string(),
                client_secret_hash: oauth_client_hash.to_string(),
                redirect_uris: vec!["https://client.example.test/callback".to_string()],
                allowed_scopes: "openid profile".to_string(),
                scope_provenance: Default::default(),
                grant_types: "authorization_code".to_string(),
                client_type: "confidential".to_string(),
                is_active: true,
                delegation_scopes: "proxy:*".to_string(),
                default_service_catalog_slugs: Vec::new(),
                broker_capability_enabled: false,
                revocation_webhook_url: None,
                revocation_webhook_secret_encrypted: Some(vec![19, 20, 21]),
                connection_webhook_url: None,
                connection_webhook_secret_encrypted: None,
                connection_webhook_key_id: None,
                connection_webhook_enabled: false,
                created_by: Some(actor_id.to_string()),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let service_pool_id = Uuid::new_v4().to_string();
        db.collection::<ServicePool>(SERVICE_POOLS)
            .insert_one(ServicePool {
                id: service_pool_id.clone(),
                user_id: actor_id.to_string(),
                slug: "delegated-pool".to_string(),
                name: "Delegated pool fixture".to_string(),
                description: None,
                strategy: PoolStrategy::RoundRobin,
                members: vec![ServicePoolMember {
                    user_service_id: actor_service_id.clone(),
                    weight: 1,
                    enabled: true,
                }],
                rr_counter: 0,
                is_active: true,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        db.collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
            .insert_one(UserServiceConnection {
                id: Uuid::new_v4().to_string(),
                user_id: actor_id.to_string(),
                service_id: actor_service_id.clone(),
                credential_encrypted: Some(vec![22, 23, 24]),
                credential_type: Some("api_key".to_string()),
                credential_label: Some("Connection secret fixture".to_string()),
                metadata: None,
                is_active: true,
                state_version: 1,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let token = jwt::generate_delegated_access_token(
            &state.jwt_keys,
            &state.config,
            &actor_id,
            "proxy:* account:read",
            "aevatar",
            jwt::DELEGATED_TOKEN_TTL_SECS,
            None,
        )
        .unwrap();
        let rollback_token = jwt::generate_delegated_access_token(
            &state.jwt_keys,
            &state.config,
            &actor_id,
            WIDE_PROXY_SCOPE,
            "aevatar",
            jwt::DELEGATED_TOKEN_TTL_SECS,
            None,
        )
        .unwrap();
        let (_, private) = crate::routes::build_router();
        let app = private.with_state(state);

        let response_paths = vec![
            "/api/v1/users/me".to_string(),
            "/api/v1/keys".to_string(),
            format!("/api/v1/keys/{actor_service_id}"),
            "/api/v1/api-keys".to_string(),
            format!("/api/v1/api-keys/{actor_api_key_id}/bindings"),
            "/api/v1/nodes".to_string(),
            "/api/v1/catalog".to_string(),
            "/api/v1/user-services".to_string(),
            "/api/v1/endpoints".to_string(),
            "/api/v1/orgs".to_string(),
            format!("/api/v1/orgs/{visible_org_id}"),
            "/api/v1/sessions".to_string(),
            "/api/v1/providers".to_string(),
            "/api/v1/providers/my-tokens".to_string(),
            format!("/api/v1/providers/{provider_id}"),
            format!("/api/v1/providers/{provider_id}/credentials"),
            "/api/v1/notifications/devices".to_string(),
            format!("/api/v1/developer/oauth-clients/{oauth_client_id}"),
            "/api/v1/service-pools".to_string(),
            format!("/api/v1/service-pools/{service_pool_id}"),
            "/api/v1/connections".to_string(),
            "/api/v1/approvals/requests".to_string(),
            "/api/v1/approvals/grants".to_string(),
            "/api/v1/approvals/service-configs".to_string(),
        ];
        let mut response_json = Vec::new();
        for path in &response_paths {
            let response = delegated_router_response(&app, Method::GET, path, &token).await;
            assert_eq!(response.status(), StatusCode::OK, "GET {path}");
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_delegated_response_has_no_secret_fields(&json);
            let serialized = serde_json::to_string(&json).unwrap();
            for fixture in [
                raw_credential,
                actor_api_hash,
                actor_node_auth_hash,
                actor_node_signing_hash,
                oauth_client_hash,
            ] {
                assert!(
                    !serialized.contains(fixture),
                    "GET {path} exposed fixture {fixture}"
                );
            }
            response_json.push((path.as_str(), serialized));
        }

        let orgs = response_json
            .iter()
            .find(|(path, _)| *path == "/api/v1/orgs")
            .unwrap()
            .1
            .as_str();
        assert!(orgs.contains(&visible_org_id.to_string()));
        assert!(!orgs.contains(&hidden_org_id.to_string()));

        let services = response_json
            .iter()
            .find(|(path, _)| *path == "/api/v1/user-services")
            .unwrap()
            .1
            .as_str();
        assert!(services.contains(&actor_service_id));
        assert!(services.contains(&visible_org_service_id));
        assert!(!services.contains(&hidden_org_service_id));

        let invite_path = format!("/api/v1/orgs/{visible_org_id}/invites");
        let invite_response =
            delegated_router_response(&app, Method::GET, &invite_path, &token).await;
        assert_eq!(invite_response.status(), StatusCode::FORBIDDEN);
        let invite_body = to_bytes(invite_response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&invite_body).contains(invite_nonce));

        for path in [
            format!("/api/v1/api-keys/{other_api_key_id}"),
            format!("/api/v1/nodes/{other_node_id}"),
        ] {
            let response = delegated_router_response(&app, Method::GET, &path, &token).await;
            assert!(
                matches!(
                    response.status(),
                    StatusCode::NOT_FOUND | StatusCode::FORBIDDEN
                ),
                "cross-user GET {path} returned {}",
                response.status()
            );
        }

        for (method, path) in [
            (Method::POST, "/api/v1/keys".to_string()),
            (Method::PUT, "/api/v1/users/me".to_string()),
            (
                Method::POST,
                format!("/api/v1/api-keys/{actor_api_key_id}/rotate"),
            ),
            (
                Method::DELETE,
                format!("/api/v1/api-keys/{actor_api_key_id}"),
            ),
        ] {
            let response = delegated_router_response(&app, method.clone(), &path, &token).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {path}");
        }

        let response =
            delegated_router_response(&app, Method::GET, "/api/v1/keys", &rollback_token).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let encoded_proxy_path = "/api/v1/proxy/s/actor-service/a%2Fb";
        let response =
            delegated_router_response(&app, Method::GET, encoded_proxy_path, &rollback_token).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(!String::from_utf8_lossy(&body).contains(DELEGATED_ENDPOINT_FORBIDDEN));

        let actor_node = delegated_router_response(
            &app,
            Method::GET,
            &format!("/api/v1/nodes/{actor_node_id}"),
            &token,
        )
        .await;
        assert_eq!(actor_node.status(), StatusCode::OK);
    }

    #[test]
    fn extract_request_ip_prefers_first_forwarded_for_hop() {
        let mut parts = request_parts_with_headers(&[
            ("x-forwarded-for", " 203.0.113.7, 10.0.0.1 "),
            ("x-real-ip", "198.51.100.9"),
        ]);
        parts
            .extensions
            .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 55], 443))));

        assert_eq!(extract_request_ip(&parts), Some("203.0.113.7".to_string()));
    }

    #[test]
    fn extract_request_ip_falls_back_to_real_ip_when_forwarded_for_empty() {
        let parts = request_parts_with_headers(&[
            ("x-forwarded-for", " , 10.0.0.1"),
            ("x-real-ip", " 198.51.100.9 "),
        ]);

        assert_eq!(extract_request_ip(&parts), Some("198.51.100.9".to_string()));
    }

    #[test]
    fn extract_request_ip_falls_back_to_connect_info_peer() {
        let mut parts = request_parts_with_headers(&[]);
        parts
            .extensions
            .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 55], 443))));

        assert_eq!(extract_request_ip(&parts), Some("192.0.2.55".to_string()));
    }

    #[test]
    fn extract_request_ip_returns_none_without_headers_or_peer() {
        let parts = request_parts_with_headers(&[]);

        assert_eq!(extract_request_ip(&parts), None);
    }

    #[test]
    fn extract_request_user_agent_returns_header_value() {
        let parts = request_parts_with_headers(&[(header::USER_AGENT.as_str(), "NyxID CLI/0.6.0")]);

        assert_eq!(
            extract_request_user_agent(&parts),
            Some("NyxID CLI/0.6.0".to_string())
        );
    }

    #[test]
    fn extract_request_user_agent_returns_none_when_absent() {
        let parts = request_parts_with_headers(&[]);

        assert_eq!(extract_request_user_agent(&parts), None);
    }

    #[tokio::test]
    async fn validate_dpop_bound_access_requires_dpop_header() {
        let Some(db) = crate::test_utils::connect_test_database("auth_dpop_missing").await else {
            return;
        };
        let state = crate::test_utils::test_app_state(db);
        let parts = request_parts_with_headers(&[]);

        let err = validate_dpop_bound_access(&parts, &state, "expected-jkt")
            .await
            .expect_err("missing DPoP");

        assert!(matches!(err, AppError::Unauthorized(message) if message == "DPoP proof required"));
    }

    #[test]
    fn parse_cookie_single() {
        assert_eq!(
            parse_cookie("nyx_session=abc123", "nyx_session"),
            Some("abc123")
        );
    }

    #[test]
    fn parse_cookie_multiple() {
        let header = "theme=dark; nyx_session=token123; lang=en";
        assert_eq!(parse_cookie(header, "nyx_session"), Some("token123"));
        assert_eq!(parse_cookie(header, "theme"), Some("dark"));
        assert_eq!(parse_cookie(header, "lang"), Some("en"));
    }

    #[test]
    fn parse_cookie_missing() {
        assert_eq!(parse_cookie("other=value", "nyx_session"), None);
    }

    #[test]
    fn parse_cookie_empty_header() {
        assert_eq!(parse_cookie("", "nyx_session"), None);
    }

    #[test]
    fn parse_cookie_with_spaces() {
        let header = " nyx_session = abc123 ; theme = dark ";
        assert_eq!(parse_cookie(header, "nyx_session"), Some("abc123"));
        assert_eq!(parse_cookie(header, "theme"), Some("dark"));
    }

    #[test]
    fn parse_cookie_value_with_equals() {
        // Cookie values can contain '=' (e.g. base64 tokens)
        let header = "nyx_session=abc=def=";
        // split_once only splits on first '=', so value is "abc=def="
        assert_eq!(parse_cookie(header, "nyx_session"), Some("abc=def="));
    }

    #[test]
    fn session_cookie_name_constant() {
        assert_eq!(SESSION_COOKIE_NAME, "nyx_session");
    }

    #[test]
    fn access_token_cookie_name_constant() {
        assert_eq!(ACCESS_TOKEN_COOKIE_NAME, "nyx_access_token");
    }

    #[test]
    fn api_key_auth_includes_key_identity() {
        let user = AuthUser {
            user_id: Uuid::new_v4(),
            session_id: None,
            scope: "read proxy".to_string(),
            acting_client_id: None,
            oauth_client_id: None,
            token_jti: None,
            verified_catalog_grant: None,
            approval_owner_user_id: None,
            auth_method: AuthMethod::ApiKey,
            allow_all_services: false,
            allow_all_nodes: true,
            allowed_service_ids: vec!["svc-1".to_string()],
            resource_uris: None,
            allowed_node_ids: vec![],
            api_key_id: Some("key-uuid-123".to_string()),
            api_key_name: Some("coding-agent".to_string()),
            api_key_purpose: ApiKeyPurpose::General,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            ip_address: None,
            user_agent: None,
        };
        assert_eq!(user.api_key_id.as_deref(), Some("key-uuid-123"));
        assert_eq!(user.api_key_name.as_deref(), Some("coding-agent"));
    }

    #[test]
    fn non_api_key_auth_has_no_key_identity() {
        let user = test_auth_user(AuthMethod::Session, "");
        assert!(user.api_key_id.is_none());
        assert!(user.api_key_name.is_none());
    }

    #[test]
    fn proxy_resolution_user_id_uses_service_account_owner_when_present() {
        let mut user = test_auth_user(AuthMethod::ServiceAccount, "proxy");
        let service_account_id = user.user_id.to_string();
        let owner_id = Uuid::new_v4().to_string();
        user.approval_owner_user_id = Some(owner_id.clone());

        assert_eq!(user.proxy_resolution_user_id(), owner_id);
        assert_eq!(user.user_id.to_string(), service_account_id);
    }

    #[test]
    fn proxy_resolution_user_id_uses_subject_for_service_account_without_owner() {
        let user = test_auth_user(AuthMethod::ServiceAccount, "proxy");

        assert_eq!(user.proxy_resolution_user_id(), user.user_id.to_string());
    }

    #[test]
    fn proxy_resolution_user_id_uses_subject_for_non_service_account() {
        let mut user = test_auth_user(AuthMethod::ApiKey, "proxy");
        user.approval_owner_user_id = Some(Uuid::new_v4().to_string());

        assert_eq!(user.proxy_resolution_user_id(), user.user_id.to_string());
    }

    #[test]
    fn session_auth_can_use_proxy_without_scope() {
        let auth_user = test_auth_user(AuthMethod::Session, "");

        assert!(auth_user.can_use_rest_proxy());
        assert!(auth_user.can_use_llm_proxy());
    }

    #[test]
    fn access_tokens_require_proxy_scope_for_rest_proxy() {
        let auth_user = test_auth_user(AuthMethod::AccessToken, "openid profile email");

        assert!(!auth_user.can_use_rest_proxy());
        assert!(auth_user.ensure_rest_proxy_access().is_err());
    }

    #[test]
    fn delegated_llm_scope_does_not_grant_rest_proxy() {
        let auth_user = test_auth_user(AuthMethod::Delegated, "llm:proxy");

        assert!(!auth_user.can_use_rest_proxy());
        assert!(auth_user.can_use_llm_proxy());
    }

    #[test]
    fn api_key_proxy_scope_grants_proxy_and_llm_access() {
        let auth_user = test_auth_user(AuthMethod::ApiKey, "read proxy");

        assert!(auth_user.can_use_rest_proxy());
        assert!(auth_user.can_use_llm_proxy());
    }

    // L1: Tests for delegated token detection (C1 fix)

    #[test]
    fn is_jwt_delegated_detects_delegated_token() {
        // Build a fake JWT payload with delegated: true
        let payload = serde_json::json!({
            "sub": "user-123",
            "delegated": true,
            "act": { "sub": "client-1" }
        });
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJSUzI1NiJ9.{payload_b64}.fake_sig");
        assert!(is_jwt_delegated(&fake_jwt));
    }

    #[test]
    fn is_jwt_delegated_passes_normal_token() {
        let payload = serde_json::json!({
            "sub": "user-123",
            "scope": "openid profile"
        });
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJSUzI1NiJ9.{payload_b64}.fake_sig");
        assert!(!is_jwt_delegated(&fake_jwt));
    }

    #[test]
    fn is_jwt_delegated_handles_invalid_jwt() {
        assert!(!is_jwt_delegated("not-a-jwt"));
        assert!(!is_jwt_delegated(""));
        assert!(!is_jwt_delegated("a.b"));
        assert!(!is_jwt_delegated("a.!!!invalid_base64!!!.c"));
    }

    // Tests for service account token detection

    #[test]
    fn is_jwt_service_account_detects_sa_token() {
        let payload = serde_json::json!({
            "sub": "sa-id-123",
            "sa": true
        });
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJSUzI1NiJ9.{payload_b64}.fake_sig");
        assert!(is_jwt_service_account(&fake_jwt));
    }

    #[test]
    fn is_jwt_service_account_passes_normal_token() {
        let payload = serde_json::json!({
            "sub": "user-123",
            "scope": "openid profile"
        });
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJSUzI1NiJ9.{payload_b64}.fake_sig");
        assert!(!is_jwt_service_account(&fake_jwt));
    }

    #[test]
    fn is_jwt_service_account_false_when_sa_is_false() {
        let payload = serde_json::json!({
            "sub": "sa-id-123",
            "sa": false
        });
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJSUzI1NiJ9.{payload_b64}.fake_sig");
        assert!(!is_jwt_service_account(&fake_jwt));
    }

    #[test]
    fn is_jwt_service_account_handles_invalid_jwt() {
        assert!(!is_jwt_service_account("not-a-jwt"));
        assert!(!is_jwt_service_account(""));
        assert!(!is_jwt_service_account("a.b"));
    }

    #[test]
    fn is_jwt_delegated_false_when_delegated_is_false() {
        let payload = serde_json::json!({
            "sub": "user-123",
            "delegated": false
        });
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJSUzI1NiJ9.{payload_b64}.fake_sig");
        assert!(!is_jwt_delegated(&fake_jwt));
    }

    #[test]
    fn service_account_request_detection_uses_bearer_header() {
        let payload = serde_json::json!({
            "sub": "sa-id-123",
            "sa": true
        });
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJSUzI1NiJ9.{payload_b64}.fake_sig");
        let request = Request::builder()
            .header(header::AUTHORIZATION, format!("Bearer {fake_jwt}"))
            .body(Body::empty())
            .unwrap();

        assert!(is_service_account_request(&request));
    }

    #[test]
    fn service_account_request_detection_ignores_legacy_access_cookie() {
        let payload = serde_json::json!({
            "sub": "sa-id-123",
            "sa": true
        });
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJSUzI1NiJ9.{payload_b64}.fake_sig");
        let request = Request::builder()
            .header(
                header::COOKIE,
                format!("{ACCESS_TOKEN_COOKIE_NAME}={fake_jwt}"),
            )
            .body(Body::empty())
            .unwrap();

        assert!(!is_service_account_request(&request));
    }

    #[test]
    fn api_key_management_write_routes_require_write_scope() {
        let user = test_auth_user(AuthMethod::ApiKey, "read proxy");
        let write_routes = [
            (Method::POST, "/api/v1/api-keys"),
            (Method::POST, "/api/v1/api-keys/key-1/rotate"),
            (Method::POST, "/api/v1/keys"),
            (Method::PUT, "/api/v1/keys/key-1"),
            (Method::DELETE, "/api/v1/keys/key-1"),
            (Method::PUT, "/api/v1/endpoints/endpoint-1"),
            (Method::DELETE, "/api/v1/endpoints/endpoint-1"),
            (Method::PUT, "/api/v1/api-keys/external/key-1"),
            (Method::DELETE, "/api/v1/api-keys/external/key-1"),
            (Method::PUT, "/api/v1/user-services/service-1"),
            (Method::DELETE, "/api/v1/user-services/service-1"),
        ];

        for (method, path) in write_routes {
            assert!(
                api_key_management_write_requires_scope(&method, path),
                "{method:?} {path} should require write scope"
            );
            assert!(
                user.ensure_management_write_scope(&method, path).is_err(),
                "{method:?} {path} should reject read-only API key auth"
            );
        }
    }

    #[test]
    fn api_key_write_or_admin_scope_can_use_management_write_routes() {
        let write_user = test_auth_user(AuthMethod::ApiKey, "read write");
        let admin_user = test_auth_user(AuthMethod::ApiKey, "read admin");

        for user in [write_user, admin_user] {
            assert!(
                user.ensure_management_write_scope(&Method::POST, "/api/v1/keys")
                    .is_ok()
            );
            assert!(
                user.ensure_management_write_scope(&Method::PUT, "/api/v1/api-keys/external/key-1")
                    .is_ok()
            );
        }
    }

    #[test]
    fn api_key_read_and_operational_routes_do_not_require_management_write_scope() {
        let user = test_auth_user(AuthMethod::ApiKey, "read proxy");
        let allowed_routes = [
            (Method::GET, "/api/v1/keys"),
            (Method::GET, "/api/v1/api-keys/external"),
            (Method::POST, "/api/v1/proxy/s/openai/v1/chat/completions"),
            (Method::POST, "/api/v1/llm/gateway/v1/chat/completions"),
            (Method::POST, "/api/v1/channel-relay/reply"),
            (Method::POST, "/api/v1/channel-events/conversation-1"),
            (Method::POST, "/api/v1/platform-ops/x-search"),
            (Method::POST, "/api/v1/ssh/service-1/exec"),
            (Method::POST, "/oauth/token"),
        ];

        for (method, path) in allowed_routes {
            assert!(
                !api_key_management_write_requires_scope(&method, path),
                "{method:?} {path} should not use management write-scope gating"
            );
            assert!(
                user.ensure_management_write_scope(&method, path).is_ok(),
                "{method:?} {path} should not reject at the management scope layer"
            );
        }
    }

    #[test]
    fn api_key_read_only_cannot_write() {
        let user = test_auth_user(AuthMethod::ApiKey, "read");
        assert!(!user.can_write());
        assert!(user.ensure_write_scope().is_err());
    }

    #[test]
    fn api_key_read_proxy_cannot_write() {
        let user = test_auth_user(AuthMethod::ApiKey, "read proxy");
        assert!(!user.can_write());
        assert!(user.ensure_write_scope().is_err());
    }

    #[test]
    fn api_key_write_scope_can_write() {
        let user = test_auth_user(AuthMethod::ApiKey, "read write");
        assert!(user.can_write());
        assert!(user.ensure_write_scope().is_ok());
    }

    #[test]
    fn api_key_admin_scope_can_write() {
        let user = test_auth_user(AuthMethod::ApiKey, "read admin");
        assert!(user.can_write());
        assert!(user.ensure_write_scope().is_ok());
    }

    #[test]
    fn session_auth_can_write_without_scope() {
        let user = test_auth_user(AuthMethod::Session, "");
        assert!(user.can_write());
        assert!(user.ensure_write_scope().is_ok());
    }

    #[test]
    fn access_token_can_write_without_scope() {
        let user = test_auth_user(AuthMethod::AccessToken, "openid profile");
        assert!(user.can_write());
        assert!(user.ensure_write_scope().is_ok());
    }

    #[test]
    fn delegated_token_can_write_without_scope() {
        let user = test_auth_user(AuthMethod::Delegated, "openid");
        assert!(user.can_write());
        assert!(user.ensure_write_scope().is_ok());
    }

    #[test]
    fn service_account_can_write_without_scope() {
        let user = test_auth_user(AuthMethod::ServiceAccount, "");
        assert!(user.can_write());
        assert!(user.ensure_write_scope().is_ok());
    }

    // -- scope_contains / scope helpers --

    #[test]
    fn scope_contains_finds_single_scope() {
        assert!(scope_contains("proxy", "proxy"));
    }

    #[test]
    fn scope_contains_finds_scope_in_list() {
        assert!(scope_contains("read proxy write", "proxy"));
        assert!(scope_contains("read proxy write", "read"));
        assert!(scope_contains("read proxy write", "write"));
    }

    #[test]
    fn scope_contains_rejects_missing_scope() {
        assert!(!scope_contains("read proxy", "write"));
    }

    #[test]
    fn scope_contains_rejects_partial_match() {
        assert!(!scope_contains("proxy:*", "proxy"));
        assert!(!scope_contains("proxy", "proxy:*"));
    }

    #[test]
    fn scope_contains_handles_empty_scopes() {
        assert!(!scope_contains("", "proxy"));
    }

    #[test]
    fn scope_contains_handles_extra_whitespace() {
        assert!(scope_contains("  read   proxy   write  ", "proxy"));
    }

    #[test]
    fn scope_allows_rest_proxy_with_proxy_scope() {
        assert!(scope_allows_rest_proxy("proxy"));
        assert!(scope_allows_rest_proxy("read proxy"));
    }

    #[test]
    fn scope_allows_rest_proxy_with_wide_scope() {
        assert!(scope_allows_rest_proxy("proxy:*"));
        assert!(scope_allows_rest_proxy("read proxy:*"));
    }

    #[test]
    fn scope_allows_rest_proxy_rejects_llm_only() {
        assert!(!scope_allows_rest_proxy("llm:proxy"));
        assert!(!scope_allows_rest_proxy("read write"));
    }

    #[test]
    fn scope_allows_llm_proxy_with_any_proxy_scope() {
        assert!(scope_allows_llm_proxy("proxy"));
        assert!(scope_allows_llm_proxy("proxy:*"));
        assert!(scope_allows_llm_proxy("llm:proxy"));
    }

    #[test]
    fn scope_allows_llm_proxy_rejects_non_proxy() {
        assert!(!scope_allows_llm_proxy("read write"));
        assert!(!scope_allows_llm_proxy("admin"));
    }

    // -- path_matches_prefix --

    #[test]
    fn path_matches_prefix_exact() {
        assert!(path_matches_prefix("/api/v1", "/api/v1"));
    }

    #[test]
    fn path_matches_prefix_with_subpath() {
        assert!(path_matches_prefix("/api/v1/keys", "/api/v1"));
        assert!(path_matches_prefix("/api/v1/keys/abc", "/api/v1"));
    }

    #[test]
    fn path_matches_prefix_rejects_partial_segment() {
        // "/api/v1extra" should NOT match prefix "/api/v1"
        assert!(!path_matches_prefix("/api/v1extra", "/api/v1"));
    }

    #[test]
    fn path_matches_prefix_rejects_unrelated() {
        assert!(!path_matches_prefix("/other/path", "/api/v1"));
    }

    #[test]
    fn scheduled_keys_are_confined_to_concrete_proxy_execution_routes() {
        let mut key = delegated_fixture_api_key("key-1", "user-1", "hash-1");
        key.purpose = crate::models::api_key::ApiKeyPurpose::ScheduledInvocation;
        key.scheduled_write_enabled = true;

        assert!(ensure_api_key_purpose_route(&key, "/api/v1/proxy/s/service/items").is_ok());
        assert!(ensure_api_key_purpose_route(&key, "/api/v1/proxy/service-id/items").is_ok());
        assert!(ensure_api_key_purpose_route(&key, "/api/v1/proxy/services").is_err());
        assert!(ensure_api_key_purpose_route(&key, "/api/v1/llm/chat/completions").is_err());
        assert!(ensure_api_key_purpose_route(&key, "/api/v1/mcp").is_err());

        key.purpose = crate::models::api_key::ApiKeyPurpose::General;
        assert!(ensure_api_key_purpose_route(&key, "/api/v1/llm/chat/completions").is_ok());
    }

    // -- approval_requester_type --

    #[test]
    fn approval_requester_type_api_key() {
        let user = test_auth_user(AuthMethod::ApiKey, "proxy");
        assert_eq!(user.approval_requester_type(), Some("api_key"));
    }

    #[test]
    fn approval_requester_type_delegated() {
        let user = test_auth_user(AuthMethod::Delegated, "proxy");
        assert_eq!(user.approval_requester_type(), Some("delegated"));
    }

    #[test]
    fn approval_requester_type_service_account() {
        let user = test_auth_user(AuthMethod::ServiceAccount, "proxy");
        assert_eq!(user.approval_requester_type(), Some("service_account"));
    }

    #[test]
    fn approval_requester_type_access_token() {
        let user = test_auth_user(AuthMethod::AccessToken, "proxy");
        assert_eq!(user.approval_requester_type(), Some("access_token"));
    }

    #[test]
    fn approval_requester_type_relay() {
        let user = test_auth_user(AuthMethod::Relay, "proxy");
        assert_eq!(user.approval_requester_type(), Some("relay"));
    }

    #[test]
    fn approval_requester_type_session_is_none() {
        let user = test_auth_user(AuthMethod::Session, "");
        assert_eq!(user.approval_requester_type(), None);
    }

    // -- approval_requester_id --

    #[test]
    fn approval_requester_id_without_acting_client() {
        let user = test_auth_user(AuthMethod::ApiKey, "proxy");
        assert_eq!(user.approval_requester_id(), user.user_id.to_string());
    }

    #[test]
    fn approval_requester_id_with_acting_client() {
        let mut user = test_auth_user(AuthMethod::Delegated, "proxy");
        user.acting_client_id = Some("client-abc".to_string());
        assert_eq!(user.approval_requester_id(), "client-abc");
    }

    // -- effective_approval_owner_user_id --

    #[test]
    fn effective_approval_owner_defaults_to_user_id() {
        let user = test_auth_user(AuthMethod::Session, "");
        assert_eq!(
            user.effective_approval_owner_user_id(),
            user.user_id.to_string()
        );
    }

    #[test]
    fn effective_approval_owner_uses_override_when_set() {
        let mut user = test_auth_user(AuthMethod::ServiceAccount, "proxy");
        user.approval_owner_user_id = Some("owner-xyz".to_string());
        assert_eq!(user.effective_approval_owner_user_id(), "owner-xyz");
    }

    // -- has_scope --

    #[test]
    fn has_scope_finds_exact_match() {
        let user = test_auth_user(AuthMethod::ApiKey, "read proxy write");
        assert!(user.has_scope("proxy"));
        assert!(user.has_scope("read"));
        assert!(user.has_scope("write"));
    }

    #[test]
    fn has_scope_rejects_absent_scope() {
        let user = test_auth_user(AuthMethod::ApiKey, "read proxy");
        assert!(!user.has_scope("admin"));
        assert!(!user.has_scope("write"));
    }

    // -- ensure_rest_proxy_access / ensure_llm_proxy_access errors --

    #[test]
    fn ensure_rest_proxy_access_error_mentions_expected_scopes() {
        let user = test_auth_user(AuthMethod::ApiKey, "read");
        let err = user.ensure_rest_proxy_access().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("proxy"),
            "Error should mention proxy scope: {msg}"
        );
    }

    #[test]
    fn ensure_llm_proxy_access_error_mentions_expected_scopes() {
        let user = test_auth_user(AuthMethod::ApiKey, "read");
        let err = user.ensure_llm_proxy_access().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("llm:proxy") || msg.contains("proxy"),
            "Error should mention LLM scope: {msg}"
        );
    }

    // -- relay auth method bypasses --

    #[test]
    fn relay_auth_method_allows_proxy_with_proxy_scope() {
        let user = test_auth_user(AuthMethod::Relay, "proxy");
        assert!(user.can_use_rest_proxy());
        assert!(user.can_use_llm_proxy());
    }

    #[test]
    fn relay_auth_method_without_proxy_scope_cannot_rest_proxy() {
        let user = test_auth_user(AuthMethod::Relay, "read");
        assert!(!user.can_use_rest_proxy());
    }

    // -- reject_relay_tokens detection (deny-by-default off the proxy surfaces) --

    fn fake_jwt(payload_json: &str) -> String {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
        format!("header.{b64}.sig")
    }

    #[test]
    fn is_jwt_relay_detects_relay_claim() {
        assert!(is_jwt_relay(&fake_jwt(r#"{"relay":true}"#)));
        assert!(!is_jwt_relay(&fake_jwt(r#"{"relay":false}"#)));
        assert!(!is_jwt_relay(&fake_jwt(r#"{"sub":"u1"}"#)));
        assert!(!is_jwt_relay("not-a-jwt"));
        assert!(!is_jwt_relay(""));
    }

    #[test]
    fn is_relay_request_matches_bearer_relay_jwt() {
        let relay = fake_jwt(r#"{"relay":true}"#);
        let req = Request::builder()
            .uri("/api/v1/keys")
            .header("authorization", format!("Bearer {relay}"))
            .body(Body::empty())
            .unwrap();
        assert!(is_relay_request(&req));

        // A normal (non-relay) access token must NOT be treated as relay.
        let non_relay = fake_jwt(r#"{"sub":"u1"}"#);
        let req2 = Request::builder()
            .uri("/api/v1/keys")
            .header("authorization", format!("Bearer {non_relay}"))
            .body(Body::empty())
            .unwrap();
        assert!(!is_relay_request(&req2));

        // No Authorization header at all.
        let req3 = Request::builder()
            .uri("/api/v1/keys")
            .body(Body::empty())
            .unwrap();
        assert!(!is_relay_request(&req3));
    }

    // -- parse_cookie edge cases --

    #[test]
    fn parse_cookie_with_no_value() {
        // "key=" has empty value
        assert_eq!(parse_cookie("nyx_session=", "nyx_session"), Some(""));
    }

    #[test]
    fn parse_cookie_duplicate_keys_returns_first() {
        let header = "nyx_session=first; nyx_session=second";
        assert_eq!(parse_cookie(header, "nyx_session"), Some("first"));
    }

    #[test]
    fn parse_cookie_name_substring_not_matched() {
        let header = "nyx_session_extra=abc";
        assert_eq!(parse_cookie(header, "nyx_session"), None);
    }

    // -- is_jwt_delegated with padded base64 --

    #[test]
    fn is_jwt_delegated_handles_padded_base64() {
        let payload = serde_json::json!({
            "sub": "u",
            "delegated": true
        });
        // Use URL_SAFE (with padding) to produce a padded payload
        let payload_b64 =
            base64::engine::general_purpose::URL_SAFE.encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJSUzI1NiJ9.{payload_b64}.sig");
        assert!(is_jwt_delegated(&fake_jwt));
    }

    #[test]
    fn is_jwt_service_account_handles_padded_base64() {
        let payload = serde_json::json!({
            "sub": "s",
            "sa": true
        });
        let payload_b64 =
            base64::engine::general_purpose::URL_SAFE.encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJSUzI1NiJ9.{payload_b64}.sig");
        assert!(is_jwt_service_account(&fake_jwt));
    }

    // -- management write scope: delegation / relay routes are exempt --

    #[test]
    fn delegation_refresh_route_exempt_from_management_write_scope() {
        assert!(!api_key_management_write_requires_scope(
            &Method::POST,
            "/api/v1/delegation/refresh"
        ));
    }

    #[test]
    fn channel_relay_reply_route_exempt_from_management_write_scope() {
        assert!(!api_key_management_write_requires_scope(
            &Method::POST,
            "/api/v1/channel-relay/reply"
        ));
    }

    #[test]
    fn get_requests_never_require_management_write_scope() {
        assert!(!api_key_management_write_requires_scope(
            &Method::GET,
            "/api/v1/keys"
        ));
        assert!(!api_key_management_write_requires_scope(
            &Method::GET,
            "/api/v1/api-keys"
        ));
    }
}
