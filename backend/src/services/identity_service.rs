use chrono::Utc;
use jsonwebtoken::{Algorithm, Header, encode};
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<String>>,
}

/// SEC-M1: Sanitize a string for use as an HTTP header value.
/// Removes CR, LF, and NUL characters that could enable CRLF injection.
fn sanitize_header_value(val: &str) -> String {
    val.chars()
        .filter(|c| !matches!(c, '\r' | '\n' | '\0'))
        .collect()
}

/// Build identity headers for a proxied request based on service configuration.
pub fn build_identity_headers(user: &User, service: &DownstreamService) -> Vec<(String, String)> {
    let mode = service.identity_propagation_mode.as_str();

    if mode == "none" {
        return vec![];
    }

    if !service.identity_include_user_id
        && !service.identity_include_email
        && !service.identity_include_name
    {
        tracing::warn!(
            service_id = %service.id,
            mode = %mode,
            "identity_propagation_mode is '{}' but all identity_include_* flags are false; \
             no user identity headers will be sent. Enable at least one include flag.",
            mode,
        );
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
        && let Some(ref name) = user.display_name
    {
        headers.push(("X-NyxID-User-Name".to_string(), sanitize_header_value(name)));
    }

    headers
}

/// Generate a short-lived signed JWT identity assertion.
/// Used when service.identity_propagation_mode is "jwt" or "both".
///
/// Resolves the user's RBAC data (roles, groups, permissions) from the
/// database and includes them in the assertion. This allows downstream
/// services to make authorization decisions without calling back to NyxID.
pub async fn generate_identity_assertion(
    jwt_keys: &JwtKeys,
    config: &AppConfig,
    user: &User,
    service: &DownstreamService,
    db: &mongodb::Database,
) -> AppResult<String> {
    let now = Utc::now().timestamp();

    let audience = service
        .identity_jwt_audience
        .as_deref()
        .unwrap_or(&service.base_url);

    let rbac = super::rbac_helpers::resolve_user_rbac(db, &user.id).await?;

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
        roles: Some(rbac.role_slugs),
        groups: Some(rbac.group_slugs),
        permissions: Some(rbac.permissions),
    };

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(jwt_keys.kid.clone());

    encode(&header, &claims, &jwt_keys.encoding)
        .map_err(|e| AppError::Internal(format!("Failed to encode identity assertion: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::test_helpers::dummy_service;
    use chrono::Utc;

    fn make_user() -> User {
        User {
            id: "user-123".to_string(),
            email: "alice@example.com".to_string(),
            password_hash: None,
            display_name: Some("Alice".to_string()),
            avatar_url: None,
            email_verified: true,
            email_verification_token: None,
            password_reset_token: None,
            password_reset_expires_at: None,
            is_active: true,
            is_admin: false,
            role_ids: vec![],
            group_ids: vec![],
            invite_code_id: None,
            mfa_enabled: false,
            social_provider: None,
            social_provider_id: None,
            user_type: crate::models::user::UserType::Person,
            primary_org_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_login_at: None,
        }
    }

    #[test]
    fn mode_none_returns_empty() {
        let user = make_user();
        let mut svc = dummy_service();
        svc.identity_propagation_mode = "none".to_string();
        let headers = build_identity_headers(&user, &svc);
        assert!(headers.is_empty());
    }

    #[test]
    fn mode_headers_with_explicit_flags() {
        let user = make_user();
        let mut svc = dummy_service();
        svc.identity_propagation_mode = "headers".to_string();
        svc.identity_include_email = true;
        svc.identity_include_name = false;
        svc.identity_include_user_id = false;

        let headers = build_identity_headers(&user, &svc);
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "X-NyxID-User-Email");
        assert_eq!(headers[0].1, "alice@example.com");
    }

    #[test]
    fn mode_headers_all_flags_off_returns_empty() {
        let user = make_user();
        let mut svc = dummy_service();
        svc.identity_propagation_mode = "headers".to_string();
        svc.identity_include_email = false;
        svc.identity_include_name = false;
        svc.identity_include_user_id = false;

        // All flags off means no identity headers at runtime (a misconfiguration
        // that is caught by a warning log). The provisioning layer is responsible
        // for setting sensible defaults when copying from catalog.
        let headers = build_identity_headers(&user, &svc);
        assert!(headers.is_empty());
    }

    #[test]
    fn mode_headers_selective_flags() {
        let user = make_user();
        let mut svc = dummy_service();
        svc.identity_propagation_mode = "headers".to_string();
        svc.identity_include_user_id = true;
        svc.identity_include_email = true;
        svc.identity_include_name = true;

        let headers = build_identity_headers(&user, &svc);
        let names: Vec<&str> = headers.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"X-NyxID-User-Id"));
        assert!(names.contains(&"X-NyxID-User-Email"));
        assert!(names.contains(&"X-NyxID-User-Name"));
    }

    #[test]
    fn sanitizes_crlf_injection() {
        let mut user = make_user();
        user.email = "alice@example.com\r\nX-Injected: yes".to_string();

        let mut svc = dummy_service();
        svc.identity_propagation_mode = "headers".to_string();
        svc.identity_include_email = true;

        let headers = build_identity_headers(&user, &svc);
        let email_header = headers
            .iter()
            .find(|(n, _)| n == "X-NyxID-User-Email")
            .unwrap();
        assert!(!email_header.1.contains('\r'));
        assert!(!email_header.1.contains('\n'));
    }
}
