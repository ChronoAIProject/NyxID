use std::collections::BTreeSet;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Method, Request, header},
    middleware::Next,
    response::Response,
};
use url::Url;

use crate::AppState;
use crate::errors::AppError;
use crate::mw::auth::{ACCESS_TOKEN_COOKIE_NAME, SESSION_COOKIE_NAME};

fn is_unsafe_method(method: &Method) -> bool {
    !matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

fn is_social_callback_path(path: &str) -> bool {
    path.starts_with("/api/v1/auth/social/") && path.ends_with("/callback")
}

fn looks_like_browser_request(headers: &HeaderMap) -> bool {
    headers.contains_key(header::ORIGIN)
        || headers.contains_key(header::REFERER)
        || headers.contains_key("sec-fetch-site")
        || headers.contains_key("sec-fetch-mode")
        || headers.contains_key("sec-fetch-dest")
}

fn has_browser_auth_cookie(headers: &HeaderMap) -> bool {
    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");

    cookie_header.split(';').any(|pair| {
        let Some((key, _value)) = pair.trim().split_once('=') else {
            return false;
        };
        matches!(key.trim(), SESSION_COOKIE_NAME | ACCESS_TOKEN_COOKIE_NAME)
    })
}

fn extract_request_origin(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_origin)
        .or_else(|| {
            headers
                .get(header::REFERER)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_origin)
        })
}

fn parse_origin(value: &str) -> Option<String> {
    Url::parse(value)
        .ok()
        .map(|url| url.origin().ascii_serialization())
}

fn allowed_origins(
    frontend_url: &str,
    base_url: &str,
    extra_origins: &[String],
) -> BTreeSet<String> {
    [frontend_url, base_url]
        .into_iter()
        .chain(extra_origins.iter().map(String::as_str))
        .filter_map(parse_origin)
        .collect()
}

pub async fn browser_csrf_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    if !is_unsafe_method(request.method()) || is_social_callback_path(request.uri().path()) {
        return Ok(next.run(request).await);
    }

    // CSRF only protects cookie-based authentication. Bearer tokens and API
    // keys are not ambient credentials — the browser never auto-attaches them,
    // so cross-origin requests that rely solely on explicit Authorization
    // headers carry no CSRF risk. This allows third-party SPAs (e.g. OAuth
    // clients on different domains) to call the proxy with Bearer tokens.
    if !has_browser_auth_cookie(request.headers()) {
        return Ok(next.run(request).await);
    }

    if !looks_like_browser_request(request.headers()) {
        return Ok(next.run(request).await);
    }

    let Some(request_origin) = extract_request_origin(request.headers()) else {
        tracing::warn!(
            path = %request.uri().path(),
            "Blocked unsafe browser request without Origin or Referer"
        );
        return Err(AppError::Forbidden(
            "Cross-site request blocked".to_string(),
        ));
    };

    let allowed = allowed_origins(
        &state.config.frontend_url,
        &state.config.base_url,
        &state.config.csrf_trusted_origins,
    );
    if allowed.contains(&request_origin) {
        return Ok(next.run(request).await);
    }

    tracing::warn!(
        path = %request.uri().path(),
        origin = %request_origin,
        allowed_origins = ?allowed,
        "Blocked unsafe browser request with disallowed origin"
    );

    Err(AppError::Forbidden(
        "Cross-site request blocked".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_method_matrix_matches_browser_csrf_rules() {
        for method in [Method::GET, Method::HEAD, Method::OPTIONS, Method::TRACE] {
            assert!(!is_unsafe_method(&method), "{method} should be safe");
        }

        for method in [
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::CONNECT,
        ] {
            assert!(is_unsafe_method(&method), "{method} should be unsafe");
        }
    }

    #[test]
    fn parse_origin_extracts_origin_only() {
        assert_eq!(
            parse_origin("https://app.example.com/path?x=1"),
            Some("https://app.example.com".to_string())
        );
    }

    #[test]
    fn parse_origin_rejects_non_url_values() {
        assert_eq!(parse_origin("not a url"), None);
        assert_eq!(parse_origin("/relative/path"), None);
    }

    #[test]
    fn trusted_browser_origins_only_include_frontend_and_backend() {
        let allowed = allowed_origins("https://app.example.com", "https://auth.example.com", &[]);

        assert_eq!(allowed.len(), 2);
        assert!(allowed.contains("https://app.example.com"));
        assert!(allowed.contains("https://auth.example.com"));
    }

    #[test]
    fn trusted_browser_origins_include_csrf_trusted_origins() {
        let extras = vec![
            "https://other.example.com".to_string(),
            "https://third.example.com/".to_string(),
            "not a url".to_string(),
        ];
        let allowed = allowed_origins(
            "https://app.example.com",
            "https://auth.example.com",
            &extras,
        );

        assert!(allowed.contains("https://app.example.com"));
        assert!(allowed.contains("https://auth.example.com"));
        assert!(allowed.contains("https://other.example.com"));
        assert!(allowed.contains("https://third.example.com"));
        assert_eq!(allowed.len(), 4);
    }

    #[test]
    fn trusted_browser_origins_deduplicate_equivalent_origins() {
        let extras = vec![
            "https://app.example.com/extra-path".to_string(),
            "https://auth.example.com".to_string(),
        ];
        let allowed = allowed_origins(
            "https://app.example.com",
            "https://auth.example.com/",
            &extras,
        );

        assert_eq!(allowed.len(), 2);
        assert!(allowed.contains("https://app.example.com"));
        assert!(allowed.contains("https://auth.example.com"));
    }

    #[test]
    fn social_callback_path_is_exempt() {
        assert!(is_social_callback_path(
            "/api/v1/auth/social/apple/callback"
        ));
        assert!(is_social_callback_path(
            "/api/v1/auth/social/google/callback"
        ));
        assert!(!is_social_callback_path("/api/v1/auth/social/google"));
    }

    #[test]
    fn social_callback_exemption_requires_exact_prefix_and_suffix_shape() {
        assert!(!is_social_callback_path(
            "/api/v1/auth/social/google/callback/extra"
        ));
        assert!(!is_social_callback_path("/auth/social/google/callback"));
    }

    #[test]
    fn browser_auth_cookie_detection_checks_session_and_legacy_access_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "theme=dark; nyx_session=abc123".parse().unwrap(),
        );
        assert!(has_browser_auth_cookie(&headers));

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "nyx_access_token=jwt; other=value".parse().unwrap(),
        );
        assert!(has_browser_auth_cookie(&headers));
    }

    #[test]
    fn browser_auth_cookie_detection_ignores_malformed_and_similar_cookie_names() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "nyx_session_extra=abc; flag; other=value".parse().unwrap(),
        );
        assert!(!has_browser_auth_cookie(&headers));
    }

    #[test]
    fn bearer_only_requests_do_not_look_like_browser_requests() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer test-token".parse().unwrap());

        assert!(!looks_like_browser_request(&headers));
        assert!(!has_browser_auth_cookie(&headers));
    }

    #[test]
    fn cross_origin_bearer_without_cookies_bypasses_csrf() {
        // A third-party SPA sends Origin + Authorization: Bearer but no cookies.
        // This is safe because Bearer tokens are not ambient credentials.
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            "https://other-app.example.com".parse().unwrap(),
        );
        headers.insert(header::AUTHORIZATION, "Bearer oauth-token".parse().unwrap());

        assert!(looks_like_browser_request(&headers));
        assert!(!has_browser_auth_cookie(&headers));
        // No auth cookies → CSRF middleware skips → request proceeds
    }

    #[test]
    fn api_key_only_requests_do_not_look_like_browser_requests() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "nyx_k_test".parse().unwrap());

        assert!(!looks_like_browser_request(&headers));
        assert!(!has_browser_auth_cookie(&headers));
    }

    #[test]
    fn origin_header_marks_request_as_browser_originated() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://app.example.com".parse().unwrap());

        assert!(looks_like_browser_request(&headers));
        assert_eq!(
            extract_request_origin(&headers),
            Some("https://app.example.com".to_string())
        );
    }

    #[test]
    fn fetch_metadata_headers_mark_request_as_browser_originated() {
        for name in ["sec-fetch-site", "sec-fetch-mode", "sec-fetch-dest"] {
            let mut headers = HeaderMap::new();
            headers.insert(name, "same-origin".parse().unwrap());
            assert!(looks_like_browser_request(&headers), "{name} should match");
        }
    }

    #[test]
    fn extract_request_origin_prefers_origin_over_referer() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://app.example.com".parse().unwrap());
        headers.insert(
            header::REFERER,
            "https://other.example.com/path".parse().unwrap(),
        );

        assert_eq!(
            extract_request_origin(&headers),
            Some("https://app.example.com".to_string())
        );
    }

    #[test]
    fn extract_request_origin_falls_back_to_referer_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::REFERER,
            "https://app.example.com/path?token=secret".parse().unwrap(),
        );

        assert_eq!(
            extract_request_origin(&headers),
            Some("https://app.example.com".to_string())
        );
    }
}
