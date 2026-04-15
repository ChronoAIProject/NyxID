use axum::{
    extract::FromRequestParts,
    http::{Method, request::Parts},
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
    /// Channel relay callback token (JWT with `relay: true`).
    /// Bypasses approval enforcement like Session.
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
    /// List of Node IDs this key can route through (only checked when allow_all_nodes is false).
    pub allowed_node_ids: Vec<String>,
    /// API key ID when auth_method == ApiKey (for agent identity tracking)
    pub api_key_id: Option<String>,
    /// Human-readable API key name (for audit logs)
    pub api_key_name: Option<String>,
    /// Per-agent rate limit (from ApiKey), None = use user-level defaults
    pub rate_limit_per_second: Option<u32>,
    pub rate_limit_burst: Option<u32>,
}

impl AuthUser {
    /// Resource owner whose approval settings should be consulted.
    pub fn effective_approval_owner_user_id(&self) -> String {
        self.approval_owner_user_id
            .clone()
            .unwrap_or_else(|| self.user_id.to_string())
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

fn scope_contains(scopes: &str, expected: &str) -> bool {
    scopes.split_whitespace().any(|scope| scope == expected)
}

pub fn scope_allows_rest_proxy(scopes: &str) -> bool {
    scope_contains(scopes, PROXY_SCOPE) || scope_contains(scopes, WIDE_PROXY_SCOPE)
}

pub fn scope_allows_llm_proxy(scopes: &str) -> bool {
    scope_allows_rest_proxy(scopes) || scope_contains(scopes, LLM_PROXY_SCOPE)
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
        "/api/v1/proxy",
        "/api/v1/ssh",
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
            // Try Bearer token first
            if let Some(auth_header) = parts.headers.get("authorization") {
                let auth_str = auth_header.to_str().map_err(|_| {
                    AppError::Unauthorized("Invalid authorization header".to_string())
                })?;

                if let Some(token) = auth_str.strip_prefix("Bearer ") {
                    // Try JWT verification first. If it fails for a reason
                    // other than expiry, fall back to API-key validation so
                    // that OpenAI-compatible clients (which send API keys as
                    // `Authorization: Bearer <key>`) work against the LLM
                    // gateway and proxy routes.
                    let claims = match jwt::verify_token(&state.jwt_keys, &state.config, token) {
                        Ok(claims) => claims,
                        Err(AppError::TokenExpired) => return Err(AppError::TokenExpired),
                        Err(jwt_err) => {
                            match crate::services::key_service::validate_api_key(&state.db, token)
                                .await
                            {
                                Ok((api_user_id_str, api_key)) => {
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
                                        approval_owner_user_id: None,
                                        auth_method: AuthMethod::ApiKey,
                                        allow_all_services: api_key.allow_all_services,
                                        allow_all_nodes: api_key.allow_all_nodes,
                                        allowed_service_ids: api_key.allowed_service_ids.clone(),
                                        allowed_node_ids: api_key.allowed_node_ids.clone(),
                                        api_key_id: Some(api_key.id.clone()),
                                        api_key_name: Some(api_key.name.clone()),
                                        rate_limit_per_second: api_key.rate_limit_per_second,
                                        rate_limit_burst: api_key.rate_limit_burst,
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

                    if claims.token_type != "access" {
                        return Err(AppError::Unauthorized("Expected access token".to_string()));
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
                            approval_owner_user_id: Some(sa.effective_owner_user_id().to_string()),
                            auth_method: AuthMethod::ServiceAccount,
                            allow_all_services: true,
                            allow_all_nodes: true,
                            allowed_service_ids: vec![],
                            allowed_node_ids: vec![],
                            api_key_id: None,
                            api_key_name: None,
                            rate_limit_per_second: None,
                            rate_limit_burst: None,
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

                    // For relay tokens, inherit the agent key's scope restrictions.
                    // For regular access tokens, allow all (scope enforced at JWT level).
                    let (
                        allow_all_services,
                        allow_all_nodes,
                        allowed_service_ids,
                        allowed_node_ids,
                        api_key_id,
                        api_key_name,
                    ) = if auth_method == AuthMethod::Relay {
                        (
                            claims.relay_allow_all_services.unwrap_or(true),
                            claims.relay_allow_all_nodes.unwrap_or(true),
                            claims.relay_allowed_service_ids.clone().unwrap_or_default(),
                            claims.relay_allowed_node_ids.clone().unwrap_or_default(),
                            claims.relay_api_key_id.clone(),
                            claims.relay_api_key_name.clone(),
                        )
                    } else {
                        (true, true, vec![], vec![], None, None)
                    };

                    return Ok(AuthUser {
                        user_id,
                        session_id: None,
                        scope: claims.scope.clone(),
                        acting_client_id: claims.act.map(|a| a.sub),
                        approval_owner_user_id: None,
                        auth_method,
                        allow_all_services,
                        allow_all_nodes,
                        allowed_service_ids,
                        allowed_node_ids,
                        api_key_id,
                        api_key_name,
                        rate_limit_per_second: None,
                        rate_limit_burst: None,
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
                                    approval_owner_user_id: None,
                                    auth_method: AuthMethod::Session,
                                    allow_all_services: true,
                                    allow_all_nodes: true,
                                    allowed_service_ids: vec![],
                                    allowed_node_ids: vec![],
                                    api_key_id: None,
                                    api_key_name: None,
                                    rate_limit_per_second: None,
                                    rate_limit_burst: None,
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
                    approval_owner_user_id: None,
                    auth_method: AuthMethod::ApiKey,
                    allow_all_services: key.allow_all_services,
                    allow_all_nodes: key.allow_all_nodes,
                    allowed_service_ids: key.allowed_service_ids.clone(),
                    allowed_node_ids: key.allowed_node_ids.clone(),
                    api_key_id: Some(key.id.clone()),
                    api_key_name: Some(key.name.clone()),
                    rate_limit_per_second: key.rate_limit_per_second,
                    rate_limit_burst: key.rate_limit_burst,
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

/// Middleware that rejects delegated tokens from accessing protected endpoints.
///
/// Delegated tokens (with `delegated: true` in JWT claims) are constrained to
/// proxy and LLM gateway routes only. This middleware should be applied to all
/// other route groups under `/api/v1`.
pub async fn reject_delegated_tokens(
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<impl IntoResponse, AppError> {
    if is_delegated_request(&request) {
        return Err(AppError::Forbidden(
            "Delegated tokens cannot access this endpoint".to_string(),
        ));
    }
    Ok(next.run(request).await)
}

/// Check if the request bears a delegated token.
fn is_delegated_request(request: &axum::http::Request<axum::body::Body>) -> bool {
    // Check Authorization header
    if let Some(auth_header) = request.headers().get("authorization")
        && let Ok(auth_str) = auth_header.to_str()
        && let Some(token) = auth_str.strip_prefix("Bearer ")
        && is_jwt_delegated(token)
    {
        return true;
    }

    false
}

/// Peek at the JWT payload (without verifying signature) to check the `delegated` field.
///
/// This is a lightweight check that avoids full JWT verification (which happens
/// later in the `AuthUser` extractor). We only inspect the unverified claims to
/// decide whether to reject early. If the token is forged, the extractor will
/// reject it during signature verification.
fn is_jwt_delegated(token: &str) -> bool {
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() < 2 {
        return false;
    }

    // Decode the payload (2nd part) from base64url (without padding)
    let payload = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
        Ok(bytes) => bytes,
        Err(_) => {
            // Retry with standard padding
            match base64::engine::general_purpose::URL_SAFE.decode(parts[1]) {
                Ok(bytes) => bytes,
                Err(_) => return false,
            }
        }
    };

    // Parse as JSON and check for delegated field
    if let Ok(claims) = serde_json::from_slice::<serde_json::Value>(&payload) {
        return claims.get("delegated") == Some(&serde_json::Value::Bool(true));
    }

    false
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
    use axum::body::Body;
    use axum::http::{Request, header};
    use uuid::Uuid;

    fn test_auth_user(auth_method: AuthMethod, scope: &str) -> AuthUser {
        AuthUser {
            user_id: Uuid::new_v4(),
            session_id: None,
            scope: scope.to_string(),
            acting_client_id: None,
            approval_owner_user_id: None,
            auth_method,
            allow_all_services: true,
            allow_all_nodes: true,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            api_key_id: None,
            api_key_name: None,
            rate_limit_per_second: None,
            rate_limit_burst: None,
        }
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
            approval_owner_user_id: None,
            auth_method: AuthMethod::ApiKey,
            allow_all_services: false,
            allow_all_nodes: true,
            allowed_service_ids: vec!["svc-1".to_string()],
            allowed_node_ids: vec![],
            api_key_id: Some("key-uuid-123".to_string()),
            api_key_name: Some("coding-agent".to_string()),
            rate_limit_per_second: None,
            rate_limit_burst: None,
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
    fn delegated_request_detection_uses_bearer_header() {
        let payload = serde_json::json!({
            "sub": "user-123",
            "delegated": true,
            "act": { "sub": "client-1" }
        });
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let fake_jwt = format!("eyJhbGciOiJSUzI1NiJ9.{payload_b64}.fake_sig");
        let request = Request::builder()
            .header(header::AUTHORIZATION, format!("Bearer {fake_jwt}"))
            .body(Body::empty())
            .unwrap();

        assert!(is_delegated_request(&request));
    }

    #[test]
    fn delegated_request_detection_ignores_legacy_access_cookie() {
        let payload = serde_json::json!({
            "sub": "user-123",
            "delegated": true,
            "act": { "sub": "client-1" }
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

        assert!(!is_delegated_request(&request));
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
}
