use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::IntoResponse;

use super::server::WizardState;

/// Allowed HTTP methods for the proxy.
const ALLOWED_METHODS: &[Method] = &[
    Method::GET,
    Method::POST,
    Method::PUT,
    Method::DELETE,
    Method::PATCH,
];

/// Maximum proxy request body size (1 MB).
const MAX_PROXY_BODY_SIZE: usize = 1024 * 1024;

// ---------------------------------------------------------------------------
// Validation (extracted for testability)
// ---------------------------------------------------------------------------

/// Reject requests with a cross-origin `Origin` header.
/// Same-origin fetch from our page sends `Origin: http://127.0.0.1:{port}`.
/// A malicious page in another tab (even on a different localhost port) gets rejected.
/// Requests without an Origin header (same-origin navigations, curl) pass
/// since the server is 127.0.0.1-only.
fn validate_origin(headers: &HeaderMap, wizard_port: u16) -> Result<(), StatusCode> {
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        let expected = format!("http://127.0.0.1:{wizard_port}");
        if origin != expected {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    Ok(())
}

fn validate_method(method: &Method) -> Result<(), StatusCode> {
    if !ALLOWED_METHODS.contains(method) {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }
    Ok(())
}

fn validate_body_size(body: &Bytes) -> Result<(), StatusCode> {
    if body.len() > MAX_PROXY_BODY_SIZE {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    Ok(())
}

/// Extract the upstream path from `/api/proxy/…`, rejecting traversal.
fn extract_upstream_path(uri: &Uri) -> Result<String, StatusCode> {
    let raw_path = uri.path().strip_prefix("/api/proxy").unwrap_or("/");
    if raw_path.contains("..") {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(match uri.query() {
        Some(q) => format!("{raw_path}?{q}"),
        None => raw_path.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// API proxy handler.
///
/// Forwards browser requests from `/api/proxy/*` to the NyxID backend,
/// injecting the saved Bearer token. The browser page never sees the token.
pub async fn proxy_api(
    State(state): State<Arc<WizardState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Err(status) = validate_origin(&headers, state.port) {
        return (status, "Cross-origin request rejected").into_response();
    }
    if let Err(status) = validate_method(&method) {
        return (status, "Method not allowed").into_response();
    }
    if let Err(status) = validate_body_size(&body) {
        return (status, "Request body too large").into_response();
    }
    let path_with_query = match extract_upstream_path(&uri) {
        Ok(p) => p,
        Err(status) => return (status, "Invalid path").into_response(),
    };

    // --- Collect headers to forward (skip hop-by-hop and auth) ---
    let forwarded_headers: Vec<(String, String)> = headers
        .iter()
        .filter(|(k, _)| {
            let name = k.as_str();
            !matches!(
                name,
                "host" | "connection" | "transfer-encoding" | "authorization" | "origin"
            )
        })
        .filter_map(|(k, v)| v.to_str().ok().map(|val| (k.to_string(), val.to_string())))
        .collect();

    let body_slice = if body.is_empty() {
        None
    } else {
        Some(body.as_ref())
    };

    // --- Forward via ApiClient (handles Bearer injection + 401 refresh) ---
    let resp = {
        let mut api = state.api_client.lock().await;
        api.proxy_request(
            method.as_str(),
            &path_with_query,
            &forwarded_headers,
            body_slice,
        )
        .await
    };

    match resp {
        Ok(upstream) => {
            let status =
                StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let content_type = upstream
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            let body_bytes = upstream.bytes().await.unwrap_or_default();

            let mut response = (status, body_bytes).into_response();
            if let Some(Ok(val)) = content_type.map(|ct| ct.parse()) {
                response.headers_mut().insert("content-type", val);
            }
            response
        }
        Err(e) => (StatusCode::BAD_GATEWAY, format!("Proxy error: {e:#}")).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Uri;

    // -- Origin validation --

    const TEST_PORT: u16 = 52341;

    #[test]
    fn origin_exact_port_allowed() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://127.0.0.1:52341".parse().unwrap());
        assert!(validate_origin(&headers, TEST_PORT).is_ok());
    }

    #[test]
    fn origin_absent_allowed() {
        let headers = HeaderMap::new();
        assert!(validate_origin(&headers, TEST_PORT).is_ok());
    }

    #[test]
    fn origin_cross_origin_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "https://evil.example.com".parse().unwrap());
        assert_eq!(
            validate_origin(&headers, TEST_PORT).unwrap_err(),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn origin_different_localhost_port_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "http://127.0.0.1:9999".parse().unwrap());
        assert_eq!(
            validate_origin(&headers, TEST_PORT).unwrap_err(),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn origin_localhost_wrong_scheme_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", "https://127.0.0.1:52341".parse().unwrap());
        assert_eq!(
            validate_origin(&headers, TEST_PORT).unwrap_err(),
            StatusCode::FORBIDDEN
        );
    }

    // -- Method allowlist --

    #[test]
    fn method_get_allowed() {
        assert!(validate_method(&Method::GET).is_ok());
    }

    #[test]
    fn method_post_allowed() {
        assert!(validate_method(&Method::POST).is_ok());
    }

    #[test]
    fn method_connect_rejected() {
        assert_eq!(
            validate_method(&Method::CONNECT).unwrap_err(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }

    #[test]
    fn method_trace_rejected() {
        assert_eq!(
            validate_method(&Method::TRACE).unwrap_err(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }

    #[test]
    fn method_options_rejected() {
        assert_eq!(
            validate_method(&Method::OPTIONS).unwrap_err(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }

    // -- Body size --

    #[test]
    fn body_within_limit() {
        let body = Bytes::from(vec![0u8; 1024]);
        assert!(validate_body_size(&body).is_ok());
    }

    #[test]
    fn body_over_limit() {
        let body = Bytes::from(vec![0u8; MAX_PROXY_BODY_SIZE + 1]);
        assert_eq!(
            validate_body_size(&body).unwrap_err(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }

    // -- Path extraction --

    #[test]
    fn path_simple() {
        let uri: Uri = "/api/proxy/api-keys".parse().unwrap();
        assert_eq!(extract_upstream_path(&uri).unwrap(), "/api-keys");
    }

    #[test]
    fn path_with_query() {
        let uri: Uri = "/api/proxy/api-keys?page=1&limit=10".parse().unwrap();
        assert_eq!(
            extract_upstream_path(&uri).unwrap(),
            "/api-keys?page=1&limit=10"
        );
    }

    #[test]
    fn path_traversal_rejected() {
        let uri: Uri = "/api/proxy/../../../etc/passwd".parse().unwrap();
        assert_eq!(
            extract_upstream_path(&uri).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn path_traversal_mid_path_rejected() {
        let uri: Uri = "/api/proxy/keys/../admin/users".parse().unwrap();
        assert_eq!(
            extract_upstream_path(&uri).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn path_root_fallback() {
        let uri: Uri = "/unexpected".parse().unwrap();
        assert_eq!(extract_upstream_path(&uri).unwrap(), "/");
    }
}
