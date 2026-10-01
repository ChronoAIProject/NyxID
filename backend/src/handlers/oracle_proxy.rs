//! Reverse proxy for the standalone oracle service.
//!
//! When `ORACLE_UPSTREAM_URL` is set, every `/api/v1/oracle/*` request
//! (consumer and worker plane) is forwarded here instead of the in-process
//! relay handlers. The proxy keeps the caller's credential headers, adds the
//! usual `X-Forwarded-*` headers, and streams both bodies so long polls, SSE
//! and multi-MiB results are never buffered in NyxID.
//!
//! The literal value `hold` answers every oracle request with
//! `503` + `Retry-After: 10`. Operators use it during the cutover window
//! while the standalone service takes over the database.

use std::net::SocketAddr;

use axum::{
    Extension,
    body::Body,
    extract::{ConnectInfo, OriginalUri, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, header},
    response::{IntoResponse, Response},
};

use crate::AppState;
use crate::config::is_oracle_hold;
use crate::errors::AppError;

/// Request headers copied to the upstream verbatim (every value).
const FORWARDED_REQUEST_HEADERS: [HeaderName; 8] = [
    header::AUTHORIZATION,
    HeaderName::from_static("x-api-key"),
    header::CONTENT_TYPE,
    header::CONTENT_LENGTH,
    header::ACCEPT,
    header::USER_AGENT,
    header::CACHE_CONTROL,
    HeaderName::from_static("x-request-id"),
];

/// Response headers copied back to the caller verbatim.
const FORWARDED_RESPONSE_HEADERS: [HeaderName; 6] = [
    header::CONTENT_TYPE,
    header::CACHE_CONTROL,
    header::CONTENT_LENGTH,
    header::RETRY_AFTER,
    HeaderName::from_static("x-accel-buffering"),
    HeaderName::from_static("x-request-id"),
];

/// Forward one oracle request to `ORACLE_UPSTREAM_URL`.
///
/// Mounted as the fallback of the oracle consumer and worker routers, so the
/// original path and query are rebuilt from `OriginalUri`.
pub async fn forward(
    State(state): State<AppState>,
    OriginalUri(original): OriginalUri,
    method: Method,
    headers: HeaderMap,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    body: Body,
) -> Response {
    let Some(upstream) = state.config.oracle_upstream_url.as_deref() else {
        return AppError::OracleUpstreamUnavailable("upstream not configured".to_string())
            .into_response();
    };
    if is_oracle_hold(upstream) {
        return AppError::OracleUpstreamHold.into_response();
    }

    let path_and_query = original
        .path_and_query()
        .map_or_else(|| original.path(), |pq| pq.as_str());
    let url = format!("{}{}", upstream.trim_end_matches('/'), path_and_query);

    let peer_ip = peer.map(|Extension(ConnectInfo(addr))| addr.ip().to_string());
    let upstream_headers = build_upstream_headers(&headers, peer_ip, &state.config.base_url);

    let mut request = state
        .http_client
        .request(method.clone(), &url)
        .headers(upstream_headers);
    if request_has_body(&method, &headers) {
        request = request.body(reqwest::Body::wrap_stream(body.into_data_stream()));
    }

    let upstream_response = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            tracing::warn!(error = %err, path = %path_and_query, "oracle upstream request failed");
            return AppError::OracleUpstreamUnavailable(describe_upstream_error(&err))
                .into_response();
        }
    };

    let status = upstream_response.status();
    let mut response_headers = HeaderMap::new();
    for name in FORWARDED_RESPONSE_HEADERS {
        for value in upstream_response.headers().get_all(&name) {
            response_headers.append(name.clone(), value.clone());
        }
    }

    let mut response = Response::new(Body::from_stream(upstream_response.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    response
}

/// Copy the allow-listed request headers and add the `X-Forwarded-*` trio.
fn build_upstream_headers(
    headers: &HeaderMap,
    peer_ip: Option<String>,
    base_url: &str,
) -> HeaderMap {
    let mut out = HeaderMap::new();
    for name in FORWARDED_REQUEST_HEADERS {
        for value in headers.get_all(&name) {
            out.append(name.clone(), value.clone());
        }
    }

    if let Some(ip) = client_ip(headers).or(peer_ip)
        && let Ok(value) = HeaderValue::from_str(&ip)
    {
        out.insert(HeaderName::from_static("x-forwarded-for"), value);
    }

    let proto = header_str(headers, "x-forwarded-proto")
        .map(str::to_string)
        .unwrap_or_else(|| {
            if base_url.starts_with("https://") {
                "https".to_string()
            } else {
                "http".to_string()
            }
        });
    if let Ok(value) = HeaderValue::from_str(&proto) {
        out.insert(HeaderName::from_static("x-forwarded-proto"), value);
    }

    let host = header_str(headers, "x-forwarded-host")
        .or_else(|| header_str(headers, "host"))
        .map(str::to_string)
        .or_else(|| {
            url::Url::parse(base_url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
        });
    if let Some(host) = host
        && let Ok(value) = HeaderValue::from_str(&host)
    {
        out.insert(HeaderName::from_static("x-forwarded-host"), value);
    }

    out
}

/// First hop of `X-Forwarded-For`, then `X-Real-IP`. Mirrors the lookup the
/// `AuthUser` extractor uses for audit attribution.
fn client_ip(headers: &HeaderMap) -> Option<String> {
    header_str(headers, "x-forwarded-for")
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| {
            header_str(headers, "x-real-ip")
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        })
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Only attach a streamed body when the caller sent one. Streaming an empty
/// body would make reqwest emit `Transfer-Encoding: chunked` on GETs.
fn request_has_body(method: &Method, headers: &HeaderMap) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return false;
    }
    let declared_length = header_str(headers, "content-length")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .is_some_and(|len| len > 0);
    declared_length || headers.contains_key(header::TRANSFER_ENCODING)
}

/// Short, URL-free reason for the 502 body. The upstream address is an
/// operator secret and never reaches clients.
fn describe_upstream_error(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "connect timeout".to_string()
    } else if err.is_connect() {
        "connection failed".to_string()
    } else if err.is_request() {
        "request failed".to_string()
    } else {
        "upstream error".to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{
        Router,
        body::to_bytes,
        extract::{DefaultBodyLimit, Request},
        http::StatusCode,
        routing::any,
    };
    use tower::ServiceExt;

    use super::*;
    use crate::test_utils::test_app_state_no_db;

    #[derive(Clone, Debug, Default)]
    struct Captured {
        method: String,
        path_and_query: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl Captured {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
        }
    }

    /// Spawn a local upstream that records the request and answers with a
    /// fixed status and header set.
    async fn spawn_upstream(captured: Arc<Mutex<Option<Captured>>>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().fallback(any(move |request: Request| {
            let captured = captured.clone();
            async move {
                let (parts, body) = request.into_parts();
                let body = to_bytes(body, usize::MAX).await.unwrap().to_vec();
                let request_id = parts
                    .headers
                    .get("x-request-id")
                    .cloned()
                    .unwrap_or_else(|| HeaderValue::from_static("none"));
                *captured.lock().unwrap() = Some(Captured {
                    method: parts.method.to_string(),
                    path_and_query: parts
                        .uri
                        .path_and_query()
                        .map(|pq| pq.to_string())
                        .unwrap_or_default(),
                    headers: parts
                        .headers
                        .iter()
                        .map(|(n, v)| (n.to_string(), v.to_str().unwrap_or("").to_string()))
                        .collect(),
                    body,
                });
                let mut response = Response::new(Body::from(r#"{"ok":true}"#));
                *response.status_mut() = StatusCode::CREATED;
                let headers = response.headers_mut();
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                );
                headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                headers.insert(header::RETRY_AFTER, HeaderValue::from_static("3"));
                headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
                headers.insert("x-request-id", request_id);
                headers.insert("x-upstream-secret", HeaderValue::from_static("do-not-copy"));
                response
            }
        }));
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    async fn proxy_app(upstream: &str) -> Router {
        let mut state = test_app_state_no_db().await;
        state.config.oracle_upstream_url = Some(upstream.to_string());
        state.config.base_url = "https://nyxid.example".to_string();
        Router::new()
            .nest(
                "/api/v1/oracle",
                Router::new()
                    .fallback(forward)
                    .layer(DefaultBodyLimit::disable()),
            )
            .with_state(state)
    }

    #[tokio::test]
    async fn forwards_allow_listed_headers_body_and_response() {
        let captured = Arc::new(Mutex::new(None));
        let upstream = spawn_upstream(captured.clone()).await;
        let app = proxy_app(&upstream).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/oracle/pools/demo/tasks?wait=1")
                    .header(header::AUTHORIZATION, "Bearer nyxid_ag_test")
                    .header("x-api-key", "nyxid_ag_other")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::CONTENT_LENGTH, "12")
                    .header(header::ACCEPT, "text/event-stream")
                    .header(header::USER_AGENT, "nyxid-cli/1.0")
                    .header(header::CACHE_CONTROL, "no-cache")
                    .header("x-request-id", "req-123")
                    .header("x-forwarded-for", "203.0.113.5, 10.0.0.1")
                    .header("x-forwarded-proto", "https")
                    .header("host", "nyxid.example")
                    .header(header::COOKIE, "nyx_session=secret")
                    .body(Body::from(r#"{"prompt":1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let headers = response.headers();
        assert_eq!(
            headers.get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
        assert_eq!(headers.get(header::RETRY_AFTER).unwrap(), "3");
        assert_eq!(headers.get("x-accel-buffering").unwrap(), "no");
        assert_eq!(headers.get("x-request-id").unwrap(), "req-123");
        assert!(headers.get("x-upstream-secret").is_none());
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body[..], br#"{"ok":true}"#);

        let seen = captured
            .lock()
            .unwrap()
            .clone()
            .expect("upstream saw the request");
        assert_eq!(seen.method, "POST");
        assert_eq!(
            seen.path_and_query,
            "/api/v1/oracle/pools/demo/tasks?wait=1"
        );
        assert_eq!(seen.body, br#"{"prompt":1}"#);
        assert_eq!(seen.header("authorization"), Some("Bearer nyxid_ag_test"));
        assert_eq!(seen.header("x-api-key"), Some("nyxid_ag_other"));
        assert_eq!(seen.header("content-type"), Some("application/json"));
        assert_eq!(seen.header("content-length"), Some("12"));
        assert_eq!(seen.header("accept"), Some("text/event-stream"));
        assert_eq!(seen.header("user-agent"), Some("nyxid-cli/1.0"));
        assert_eq!(seen.header("cache-control"), Some("no-cache"));
        assert_eq!(seen.header("x-request-id"), Some("req-123"));
        assert_eq!(seen.header("x-forwarded-for"), Some("203.0.113.5"));
        assert_eq!(seen.header("x-forwarded-proto"), Some("https"));
        assert_eq!(seen.header("x-forwarded-host"), Some("nyxid.example"));
        assert_eq!(seen.header("cookie"), None);
    }

    #[tokio::test]
    async fn get_without_body_is_not_chunked() {
        let captured = Arc::new(Mutex::new(None));
        let upstream = spawn_upstream(captured.clone()).await;
        let app = proxy_app(&upstream).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/oracle/worker/task")
                    .header(header::AUTHORIZATION, "Bearer nyx_owk_test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let seen = captured
            .lock()
            .unwrap()
            .clone()
            .expect("upstream saw the request");
        assert_eq!(seen.method, "GET");
        assert_eq!(seen.path_and_query, "/api/v1/oracle/worker/task");
        assert!(seen.body.is_empty());
        assert_eq!(seen.header("transfer-encoding"), None);
        // No proxy headers on the way in: proto falls back to the base URL
        // scheme and host to the base URL host.
        assert_eq!(seen.header("x-forwarded-proto"), Some("https"));
        assert_eq!(seen.header("x-forwarded-host"), Some("nyxid.example"));
    }

    #[tokio::test]
    async fn hold_answers_503_with_retry_after() {
        let app = proxy_app("hold").await;
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/oracle/pools/demo/tasks")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers().get(header::RETRY_AFTER).unwrap(), "10");
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "oracle_upstream_unavailable");
        assert_eq!(json["error_code"], 11017);
    }

    #[tokio::test]
    async fn unreachable_upstream_answers_502() {
        // Port 1 is never listening on loopback; the connection is refused.
        let app = proxy_app("http://127.0.0.1:1").await;
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/oracle/pools")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "oracle_upstream_unavailable");
        assert_eq!(json["error_code"], 11017);
        let message = json["message"].as_str().unwrap();
        assert!(
            message.starts_with("Oracle service unavailable: "),
            "{message}"
        );
        assert!(!message.contains("127.0.0.1"), "{message}");
    }

    #[test]
    fn request_has_body_rules() {
        let mut headers = HeaderMap::new();
        assert!(!request_has_body(&Method::POST, &headers));
        assert!(!request_has_body(&Method::GET, &headers));
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("0"));
        assert!(!request_has_body(&Method::POST, &headers));
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("12"));
        assert!(request_has_body(&Method::POST, &headers));
        assert!(!request_has_body(&Method::GET, &headers));
        let mut chunked = HeaderMap::new();
        chunked.insert(
            header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        assert!(request_has_body(&Method::PUT, &chunked));
    }
}
