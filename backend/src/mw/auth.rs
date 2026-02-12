use axum::{
    extract::FromRequestParts,
    http::request::Parts,
    middleware::Next,
    response::IntoResponse,
};
use base64::Engine as _;
use mongodb::bson::doc;
use uuid::Uuid;

use crate::crypto::jwt;
use crate::crypto::token::hash_token;
use crate::errors::AppError;
use crate::models::service_account::{ServiceAccount, COLLECTION_NAME as SERVICE_ACCOUNTS};
use crate::models::service_account_token::{
    ServiceAccountToken, COLLECTION_NAME as SA_TOKENS,
};
use crate::models::session::{Session, COLLECTION_NAME as SESSIONS};
use crate::models::user::{User, COLLECTION_NAME as USERS};
use crate::AppState;

/// Authenticated user extracted from session cookie or Bearer token.
///
/// This acts as an Axum extractor: handlers that include `AuthUser` in their
/// parameters will automatically reject unauthenticated requests.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub session_id: Option<Uuid>,
    /// Space-separated scopes from the access token (empty for session/API key auth).
    pub scope: String,
    /// If this is a delegated request, the OAuth client_id of the acting service.
    pub acting_client_id: Option<String>,
}

/// Name of the session cookie.
pub const SESSION_COOKIE_NAME: &str = "nyx_session";

/// Name of the access token cookie.
pub const ACCESS_TOKEN_COOKIE_NAME: &str = "nyx_access_token";

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
                let auth_str = auth_header
                    .to_str()
                    .map_err(|_| AppError::Unauthorized("Invalid authorization header".to_string()))?;

                if let Some(token) = auth_str.strip_prefix("Bearer ") {
                    let claims = jwt::verify_token(&state.jwt_keys, &state.config, token)?;

                    if claims.token_type != "access" {
                        return Err(AppError::Unauthorized(
                            "Expected access token".to_string(),
                        ));
                    }

                    // Check if this is a service account token
                    if claims.sa == Some(true) {
                        let sa_id = claims.sub.clone();

                        // Verify the service account exists and is active
                        let _sa = state
                            .db
                            .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
                            .find_one(doc! { "_id": &sa_id, "is_active": true })
                            .await
                            .map_err(|e| {
                                AppError::Internal(format!("SA lookup failed: {e}"))
                            })?
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

                        if let Some(record) = token_record {
                            if record.revoked {
                                return Err(AppError::Unauthorized(
                                    "Token has been revoked".to_string(),
                                ));
                            }
                        }

                        let sa_uuid = Uuid::parse_str(&sa_id).map_err(|_| {
                            AppError::Unauthorized(
                                "Invalid service account ID".to_string(),
                            )
                        })?;

                        return Ok(AuthUser {
                            user_id: sa_uuid,
                            session_id: None,
                            scope: claims.scope.clone(),
                            acting_client_id: None,
                        });
                    }

                    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| {
                        AppError::Unauthorized("Invalid token subject".to_string())
                    })?;

                    let user_id_str = user_id.to_string();

                    // Verify the user account is still active
                    let user_model = state
                        .db
                        .collection::<User>(USERS)
                        .find_one(doc! { "_id": &user_id_str })
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

                    return Ok(AuthUser {
                        user_id,
                        session_id: None,
                        scope: claims.scope.clone(),
                        acting_client_id: claims.act.map(|a| a.sub),
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

                if let Some(sess) = session
                    && sess.expires_at > chrono::Utc::now() {
                        let user_id = Uuid::parse_str(&sess.user_id).map_err(|_| {
                            AppError::Internal("Invalid user_id in session".to_string())
                        })?;
                        let session_id = Uuid::parse_str(&sess.id).map_err(|_| {
                            AppError::Internal("Invalid session id".to_string())
                        })?;

                        // Verify the user account is still active
                        let user_model = state
                            .db
                            .collection::<User>(USERS)
                            .find_one(doc! { "_id": &sess.user_id })
                            .await
                            .map_err(|e| {
                                AppError::Internal(format!("User lookup failed: {e}"))
                            })?;

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
            }

            // Also try access token cookie
            let access_token = parse_cookie(cookie_header, ACCESS_TOKEN_COOKIE_NAME);

            if let Some(token) = access_token {
                let claims = jwt::verify_token(&state.jwt_keys, &state.config, token)?;

                if claims.token_type != "access" {
                    return Err(AppError::Unauthorized(
                        "Expected access token".to_string(),
                    ));
                }

                let user_id = Uuid::parse_str(&claims.sub).map_err(|_| {
                    AppError::Unauthorized("Invalid token subject".to_string())
                })?;

                let user_id_str = user_id.to_string();

                // Verify the user account is still active
                let user_model = state
                    .db
                    .collection::<User>(USERS)
                    .find_one(doc! { "_id": &user_id_str })
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

                return Ok(AuthUser {
                    user_id,
                    session_id: None,
                    scope: claims.scope.clone(),
                    acting_client_id: claims.act.map(|a| a.sub),
                });
            }

            // Try API key (X-API-Key header)
            if let Some(api_key_header) = parts.headers.get("x-api-key") {
                let api_key = api_key_header
                    .to_str()
                    .map_err(|_| AppError::Unauthorized("Invalid API key header".to_string()))?;

                let (user_id_str, _key) =
                    crate::services::key_service::validate_api_key(&state.db, api_key).await?;

                let user_id = Uuid::parse_str(&user_id_str).map_err(|_| {
                    AppError::Internal("Invalid user_id in API key".to_string())
                })?;

                // Verify the user account is still active
                let user_model = state
                    .db
                    .collection::<User>(USERS)
                    .find_one(doc! { "_id": &user_id_str })
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

                return Ok(AuthUser {
                    user_id,
                    session_id: None,
                    scope: String::new(),
                    acting_client_id: None,
                });
            }

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

/// Check if the request bears a delegated token (Bearer header or access token cookie).
fn is_delegated_request(request: &axum::http::Request<axum::body::Body>) -> bool {
    // Check Authorization header
    if let Some(auth_header) = request.headers().get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ") {
                if is_jwt_delegated(token) {
                    return true;
                }
            }
        }
    }

    // Check access token cookie
    if let Some(cookie_header) = request.headers().get("cookie") {
        if let Ok(cookie_str) = cookie_header.to_str() {
            if let Some(token) = parse_cookie(cookie_str, ACCESS_TOKEN_COOKIE_NAME) {
                if is_jwt_delegated(token) {
                    return true;
                }
            }
        }
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

/// Check if the request bears a service account token (Bearer header or access token cookie).
fn is_service_account_request(request: &axum::http::Request<axum::body::Body>) -> bool {
    // Check Authorization header
    if let Some(auth_header) = request.headers().get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ") {
                if is_jwt_service_account(token) {
                    return true;
                }
            }
        }
    }

    // Check access token cookie
    if let Some(cookie_header) = request.headers().get("cookie") {
        if let Ok(cookie_str) = cookie_header.to_str() {
            if let Some(token) = parse_cookie(cookie_str, ACCESS_TOKEN_COOKIE_NAME) {
                if is_jwt_service_account(token) {
                    return true;
                }
            }
        }
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
        Err(_) => {
            match base64::engine::general_purpose::URL_SAFE.decode(parts[1]) {
                Ok(bytes) => bytes,
                Err(_) => return false,
            }
        }
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

    #[test]
    fn parse_cookie_single() {
        assert_eq!(parse_cookie("nyx_session=abc123", "nyx_session"), Some("abc123"));
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

}
