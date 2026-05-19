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
///
/// Drops CR, LF, and NUL bytes to prevent CRLF injection. Printable ASCII
/// bytes are passed through unchanged, except `%` is escaped as `%25` so
/// percent-decoding round-trips unambiguously. All other bytes are encoded as
/// uppercase `%XX`, including UTF-8 multibyte sequences byte by byte.
///
/// The output is pure printable ASCII and is therefore safe to feed into
/// `reqwest::header::HeaderValue::from_str` and any HTTP/2 HPACK or HTTP/3
/// QPACK encoder, and round-trips through `HeaderValue::to_str` -- the
/// stricter check that today fails the WS proxy path on non-ASCII bytes.
///
/// This is the contract for every value passed to it: any current or future
/// caller emitting an HTTP header should funnel through here so wire-safety
/// holds across all proxy paths (direct HTTP, direct WS, node HTTP, node WS).
/// Downstream services should percent-decode `X-NyxID-User-Id`,
/// `X-NyxID-User-Email`, and `X-NyxID-User-Name` to recover the original
/// UTF-8 values.
fn sanitize_header_value(val: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let mut out = String::with_capacity(val.len());
    for b in val.bytes() {
        match b {
            b'\r' | b'\n' | 0 => {}
            b'%' => out.push_str("%25"),
            0x20..=0x7E => out.push(b as char),
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0x0F) as usize] as char);
            }
        }
    }
    out
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
            slug: None,
            avatar_url: None,
            email_verified: true,
            email_verification_token: None,
            password_reset_token: None,
            password_reset_expires_at: None,
            is_active: true,
            is_admin: false,
            is_operator: false,
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
            profile_config: Default::default(),
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

    #[test]
    fn sanitizes_non_ascii_display_name_to_percent_encoded() {
        let mut user = make_user();
        user.display_name = Some("赵奕旗".to_string());

        let mut svc = dummy_service();
        svc.identity_propagation_mode = "headers".to_string();
        svc.identity_include_user_id = false;
        svc.identity_include_email = false;
        svc.identity_include_name = true;

        let headers = build_identity_headers(&user, &svc);
        let name_header = headers
            .iter()
            .find(|(n, _)| n == "X-NyxID-User-Name")
            .unwrap();
        assert_eq!(name_header.1, "%E8%B5%B5%E5%A5%95%E6%97%97");

        let header_value = reqwest::header::HeaderValue::from_str(&name_header.1).unwrap();
        assert_eq!(
            header_value.to_str().unwrap(),
            "%E8%B5%B5%E5%A5%95%E6%97%97"
        );
    }

    #[test]
    fn sanitizes_percent_self_escapes() {
        let mut user = make_user();
        user.display_name = Some("100% off".to_string());

        let mut svc = dummy_service();
        svc.identity_propagation_mode = "headers".to_string();
        svc.identity_include_user_id = false;
        svc.identity_include_email = false;
        svc.identity_include_name = true;

        let headers = build_identity_headers(&user, &svc);
        let name_header = headers
            .iter()
            .find(|(n, _)| n == "X-NyxID-User-Name")
            .unwrap();
        assert_eq!(name_header.1, "100%25 off");
    }

    #[test]
    fn sanitizes_drops_crlf_and_nul_keeps_other_controls_encoded() {
        let mut user = make_user();
        user.display_name = Some("alice\r\n\tbob\0\x01".to_string());

        let mut svc = dummy_service();
        svc.identity_propagation_mode = "headers".to_string();
        svc.identity_include_user_id = false;
        svc.identity_include_email = false;
        svc.identity_include_name = true;

        let headers = build_identity_headers(&user, &svc);
        let name_header = headers
            .iter()
            .find(|(n, _)| n == "X-NyxID-User-Name")
            .unwrap();
        assert_eq!(name_header.1, "alice%09bob%01");
        assert!(reqwest::header::HeaderValue::from_str(&name_header.1).is_ok());
    }

    #[test]
    fn sanitizes_ascii_values_byte_for_byte() {
        let user = make_user();

        let mut svc = dummy_service();
        svc.identity_propagation_mode = "headers".to_string();
        svc.identity_include_user_id = true;
        svc.identity_include_email = true;
        svc.identity_include_name = true;

        let headers = build_identity_headers(&user, &svc);
        let user_id_header = headers
            .iter()
            .find(|(n, _)| n == "X-NyxID-User-Id")
            .unwrap();
        let email_header = headers
            .iter()
            .find(|(n, _)| n == "X-NyxID-User-Email")
            .unwrap();
        let name_header = headers
            .iter()
            .find(|(n, _)| n == "X-NyxID-User-Name")
            .unwrap();

        assert_eq!(user_id_header.1, "user-123");
        assert_eq!(email_header.1, "alice@example.com");
        assert_eq!(name_header.1, "Alice");
    }
}
