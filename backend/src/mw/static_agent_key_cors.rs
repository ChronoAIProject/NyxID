use std::collections::BTreeSet;

use axum::{
    body::Body,
    extract::OriginalUri,
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header},
    middleware::Next,
    response::Response,
};
use url::Url;

use crate::{config::AppConfig, errors::AppError};

const REQUEST_PATH: &str = "/api/v1/auth/agent-key/request";
const POLL_PATH: &str = "/api/v1/auth/agent-key/poll";
const SELF_PATH: &str = "/api/v1/auth/agent-key/self";
const RESPONSES_PATH: &str = "/api/v1/proxy/s/llm-nyx/v1/responses";
const RESPONSES_PREFIX: &str = "/api/v1/proxy/s/llm-nyx/v1/responses/";

const ALLOWED_HEADERS: &[&str] = &["accept", "authorization", "content-type", "idempotency-key"];

/// Route-aware, credential-free CORS policy for static Agent Key SDK hosts.
///
/// This policy is deliberately separate from the first-party private CORS
/// policy. A configured static origin can read only the approved Agent Key
/// lifecycle and Nyx Responses routes, and browsers are never granted
/// credentialed cross-origin access.
#[derive(Clone, Debug, Default)]
pub(crate) struct StaticAgentKeyCors {
    origins: BTreeSet<String>,
}

impl StaticAgentKeyCors {
    pub(crate) fn from_config(config: &AppConfig) -> Self {
        let first_party = first_party_origins(config);
        let mut origins = BTreeSet::new();
        for raw in &config.static_agent_key_origins {
            let Some(origin) = normalize_origin(raw) else {
                tracing::warn!(origin = %raw, "Ignoring invalid STATIC_AGENT_KEY_ORIGINS entry");
                continue;
            };
            if first_party.contains(&origin) {
                tracing::warn!(origin = %origin, "Ignoring static Agent Key origin that overlaps a first-party origin");
                continue;
            }
            origins.insert(origin);
        }
        Self { origins }
    }

    #[cfg(test)]
    fn contains(&self, origin: &str) -> bool {
        self.origins.contains(origin)
    }
}

fn first_party_origins(config: &AppConfig) -> BTreeSet<String> {
    config
        .accepted_frontend_urls()
        .chain(config.accepted_base_urls())
        .chain(config.cors_allowed_origins.iter().map(String::as_str))
        .chain(config.csrf_trusted_origins.iter().map(String::as_str))
        .filter_map(normalize_origin)
        .collect()
}

fn normalize_origin(raw: &str) -> Option<String> {
    let url = Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || (url.path() != "" && url.path() != "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(url.origin().ascii_serialization())
}

fn request_path(request: &Request<Body>) -> &str {
    request
        .extensions()
        .get::<OriginalUri>()
        .map_or_else(|| request.uri().path(), |uri| uri.path())
}

fn allowed_method(path: &str) -> Option<&'static [Method]> {
    match path {
        REQUEST_PATH | POLL_PATH => Some(&[Method::POST]),
        SELF_PATH => Some(&[Method::GET, Method::DELETE]),
        RESPONSES_PATH => Some(&[Method::POST]),
        path if path.starts_with(RESPONSES_PREFIX) => {
            let response_id = &path[RESPONSES_PREFIX.len()..];
            (!response_id.is_empty() && !response_id.contains('/')).then_some(&[Method::GET])
        }
        _ => None,
    }
}

fn requested_headers_are_allowed(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .all(|name| {
                    let name = name.to_ascii_lowercase();
                    ALLOWED_HEADERS.contains(&name.as_str())
                })
        })
        .unwrap_or(true)
}

fn append_vary_origin(headers: &mut HeaderMap) {
    let value = headers
        .get(header::VARY)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !value
        .split(',')
        .any(|entry| entry.trim().eq_ignore_ascii_case("origin"))
    {
        let next = if value.is_empty() {
            "Origin".to_string()
        } else {
            format!("{value}, Origin")
        };
        if let Ok(value) = HeaderValue::from_str(&next) {
            headers.insert(header::VARY, value);
        }
    }
}

fn strip_credentials(response: &mut Response) {
    response
        .headers_mut()
        .remove(header::ACCESS_CONTROL_ALLOW_CREDENTIALS);
}

fn add_cors_headers(
    response: &mut Response,
    origin: &str,
    preflight: bool,
    method: Option<&Method>,
) {
    strip_credentials(response);
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(origin) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
    }
    append_vary_origin(headers);
    if preflight {
        if let Some(method) = method {
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_str(method.as_str()).expect("HTTP method is valid"),
            );
        }
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("Accept, Authorization, Content-Type, Idempotency-Key"),
        );
        headers.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("300"),
        );
    } else {
        headers.insert(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static("Retry-After, X-Request-Id"),
        );
    }
}

pub(crate) async fn middleware(
    axum::extract::State(policy): axum::extract::State<StaticAgentKeyCors>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let Some(origin) = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .and_then(normalize_origin)
        .filter(|origin| policy.origins.contains(origin))
    else {
        return Ok(next.run(request).await);
    };

    let path = request_path(&request);
    let Some(methods) = allowed_method(path) else {
        let mut response = next.run(request).await;
        strip_credentials(&mut response);
        return Ok(response);
    };

    if request.headers().contains_key(header::COOKIE) {
        return Err(AppError::Forbidden(
            "Static Agent Key origins cannot use browser cookies".into(),
        ));
    }

    if request.method() == Method::OPTIONS {
        let requested_method = request
            .headers()
            .get(header::ACCESS_CONTROL_REQUEST_METHOD)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<Method>().ok());
        if !requested_method
            .as_ref()
            .is_some_and(|method| methods.contains(method))
            || !requested_headers_are_allowed(request.headers())
        {
            let mut response = next.run(request).await;
            strip_credentials(&mut response);
            return Ok(response);
        }

        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NO_CONTENT;
        add_cors_headers(&mut response, &origin, true, requested_method.as_ref());
        return Ok(response);
    }

    if !methods.contains(request.method()) {
        let mut response = next.run(request).await;
        strip_credentials(&mut response);
        return Ok(response);
    }

    let mut response = next.run(request).await;
    add_cors_headers(&mut response, &origin, false, None);
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        routing::{any, get},
    };
    use tower::ServiceExt;

    fn config() -> AppConfig {
        let mut config = crate::test_utils::test_app_config();
        config.frontend_url = "https://app.example".into();
        config.base_url = "https://api.example".into();
        config.static_agent_key_origins = vec![
            "https://static.example/".into(),
            "https://app.example".into(),
            "https://static.example/path".into(),
        ];
        config
    }

    #[test]
    fn policy_normalizes_exact_origins_and_excludes_first_party() {
        let policy = StaticAgentKeyCors::from_config(&config());
        assert!(policy.contains("https://static.example"));
        assert!(!policy.contains("https://app.example"));
        assert_eq!(policy.origins.len(), 1);
    }

    #[tokio::test]
    async fn allowed_preflight_is_limited_to_sdk_headers_and_methods() {
        let policy = StaticAgentKeyCors::from_config(&config());
        let app = Router::new()
            .route(REQUEST_PATH, any(|| async { "ok" }))
            .with_state(policy.clone())
            .layer(axum::middleware::from_fn_with_state(policy, middleware));
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri(REQUEST_PATH)
                    .header("Origin", "https://static.example")
                    .header("Access-Control-Request-Method", "POST")
                    .header(
                        "Access-Control-Request-Headers",
                        "authorization,content-type",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "https://static.example"
        );
        assert!(
            !response
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
        );
    }

    #[tokio::test]
    async fn disallowed_route_and_cookie_request_receive_no_static_cors() {
        let policy = StaticAgentKeyCors::from_config(&config());
        let app = Router::new()
            .route("/api/v1/users/me", get(|| async { "private" }))
            .route(SELF_PATH, get(|| async { "self" }))
            .with_state(policy.clone())
            .layer(axum::middleware::from_fn_with_state(policy, middleware));
        let disallowed = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/users/me")
                    .header("Origin", "https://static.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            !disallowed
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        );

        let cookie = app
            .oneshot(
                Request::builder()
                    .uri(SELF_PATH)
                    .header("Origin", "https://static.example")
                    .header("Cookie", "nyx_session=ambient")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cookie.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn allowed_actual_response_gets_origin_without_credentials() {
        let policy = StaticAgentKeyCors::from_config(&config());
        let app = Router::new()
            .route(SELF_PATH, get(|| async { "self" }))
            .with_state(policy.clone())
            .layer(axum::middleware::from_fn_with_state(policy, middleware));
        let response = app
            .oneshot(
                Request::builder()
                    .uri(SELF_PATH)
                    .header("Origin", "https://static.example")
                    .header("Authorization", "Bearer child")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "https://static.example"
        );
        assert!(
            !response
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
        );
    }

    #[test]
    fn response_route_requires_one_id_segment() {
        assert!(allowed_method(RESPONSES_PATH).is_some());
        assert!(allowed_method("/api/v1/proxy/s/llm-nyx/v1/responses/r_1").is_some());
        assert!(allowed_method("/api/v1/proxy/s/llm-nyx/v1/responses/r_1/events").is_none());
        assert!(allowed_method("/api/v1/proxy/s/llm-openai/v1/responses/r_1").is_none());
    }
}
