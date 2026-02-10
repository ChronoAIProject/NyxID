use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, Header};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::crypto::jwt::JwtKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::DownstreamService;
use crate::models::user::User;

/// Short-lived identity assertion JWT claims.
#[derive(Debug, Serialize, Deserialize)]
pub struct IdentityAssertionClaims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    pub jti: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub nyx_service_id: String,
}

/// SEC-M1: Sanitize a string for use as an HTTP header value.
/// Removes CR, LF, and NUL characters that could enable CRLF injection.
fn sanitize_header_value(val: &str) -> String {
    val.chars()
        .filter(|c| !matches!(c, '\r' | '\n' | '\0'))
        .collect()
}

/// Build identity headers for a proxied request based on service configuration.
pub fn build_identity_headers(
    user: &User,
    service: &DownstreamService,
) -> Vec<(String, String)> {
    let mode = service.identity_propagation_mode.as_str();

    if mode == "none" {
        return vec![];
    }

    let mut headers = Vec::new();

    if service.identity_include_user_id {
        headers.push((
            "X-NyxID-User-Id".to_string(),
            sanitize_header_value(&user.id),
        ));
    }

    if service.identity_include_email {
        headers.push((
            "X-NyxID-User-Email".to_string(),
            sanitize_header_value(&user.email),
        ));
    }

    if service.identity_include_name
        && let Some(ref name) = user.display_name {
            headers.push((
                "X-NyxID-User-Name".to_string(),
                sanitize_header_value(name),
            ));
        }

    headers
}

/// Generate a short-lived signed JWT identity assertion.
/// Used when service.identity_propagation_mode is "jwt" or "both".
pub fn generate_identity_assertion(
    jwt_keys: &JwtKeys,
    config: &AppConfig,
    user: &User,
    service: &DownstreamService,
) -> AppResult<String> {
    let now = Utc::now().timestamp();

    let audience = service
        .identity_jwt_audience
        .as_deref()
        .unwrap_or(&service.base_url);

    let claims = IdentityAssertionClaims {
        sub: user.id.clone(),
        iss: config.jwt_issuer.clone(),
        aud: audience.to_string(),
        exp: now + 60, // 60-second lifetime
        iat: now,
        jti: Uuid::new_v4().to_string(),
        email: if service.identity_include_email {
            Some(user.email.clone())
        } else {
            None
        },
        name: if service.identity_include_name {
            user.display_name.clone()
        } else {
            None
        },
        nyx_service_id: service.id.clone(),
    };

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(jwt_keys.kid.clone());

    encode(&header, &claims, &jwt_keys.encoding)
        .map_err(|e| AppError::Internal(format!("Failed to encode identity assertion: {e}")))
}
