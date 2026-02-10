use axum::{
    extract::FromRequestParts,
    http::request::Parts,
};
use mongodb::bson::doc;
use uuid::Uuid;

use crate::crypto::jwt;
use crate::crypto::token::hash_token;
use crate::errors::AppError;
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
                                return Ok(AuthUser {
                                    user_id,
                                    session_id: Some(session_id),
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
                });
            }

            Err(AppError::Unauthorized(
                "No valid authentication credentials provided".to_string(),
            ))
        }
    }
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
}
