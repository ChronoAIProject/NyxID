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

fn allowed_origins(state: &AppState) -> BTreeSet<String> {
    [
        state.config.frontend_url.as_str(),
        state.config.base_url.as_str(),
    ]
    .into_iter()
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

    let browser_request =
        looks_like_browser_request(request.headers()) || has_browser_auth_cookie(request.headers());

    if !browser_request {
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

    let allowed = allowed_origins(&state);
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
    fn parse_origin_extracts_origin_only() {
        assert_eq!(
            parse_origin("https://app.example.com/path?x=1"),
            Some("https://app.example.com".to_string())
        );
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
    fn bearer_only_requests_do_not_look_like_browser_requests() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer test-token".parse().unwrap());

        assert!(!looks_like_browser_request(&headers));
        assert!(!has_browser_auth_cookie(&headers));
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
}
