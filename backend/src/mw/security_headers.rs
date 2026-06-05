use axum::{
    body::Body,
    http::{Request, header},
    middleware::Next,
    response::Response,
};

/// Middleware that adds security-related HTTP headers to every response.
///
/// Headers added:
/// - Strict-Transport-Security (HSTS)
/// - X-Content-Type-Options
/// - X-Frame-Options
/// - Content-Security-Policy
/// - Referrer-Policy
/// - Permissions-Policy
/// - X-XSS-Protection
pub async fn security_headers_middleware(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    // HSTS: enforce HTTPS for 1 year, including subdomains
    headers.insert(
        header::STRICT_TRANSPORT_SECURITY,
        "max-age=31536000; includeSubDomains; preload"
            .parse()
            .unwrap(),
    );

    // Prevent MIME-type sniffing
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());

    // Prevent framing (clickjacking protection)
    headers.insert(header::X_FRAME_OPTIONS, "DENY".parse().unwrap());

    // Content Security Policy — only set if the handler hasn't already provided one
    // (e.g. oauth_success_page sets a custom CSP allowing inline style/script).
    if !headers.contains_key(header::CONTENT_SECURITY_POLICY) {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; frame-ancestors 'none'"
                .parse()
                .unwrap(),
        );
    }

    // Control referrer information
    headers.insert(
        header::REFERRER_POLICY,
        "strict-origin-when-cross-origin".parse().unwrap(),
    );

    // Restrict browser features
    headers.insert(
        "permissions-policy".parse::<header::HeaderName>().unwrap(),
        "camera=(), microphone=(), geolocation=(), interest-cohort=()"
            .parse()
            .unwrap(),
    );

    // Legacy XSS protection (for older browsers)
    headers.insert(
        "x-xss-protection".parse::<header::HeaderName>().unwrap(),
        "1; mode=block".parse().unwrap(),
    );

    // Prevent caching of API responses (SEC-6: protects credential endpoints)
    if !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(
            header::CACHE_CONTROL,
            "no-store, no-cache, must-revalidate".parse().unwrap(),
        );
    }
    headers.insert(header::PRAGMA, "no-cache".parse().unwrap());

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::{Router, routing::get};
    use tower::ServiceExt;

    async fn ok_handler() -> StatusCode {
        StatusCode::OK
    }

    async fn custom_csp_handler() -> Response {
        let mut resp = Response::new(Body::empty());
        resp.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'self'".parse().unwrap(),
        );
        resp
    }

    #[tokio::test]
    async fn injects_all_security_headers() {
        let app = Router::new()
            .route("/test", get(ok_handler))
            .layer(axum::middleware::from_fn(security_headers_middleware));
        let resp = app
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(
            resp.headers()
                .contains_key(header::STRICT_TRANSPORT_SECURITY)
        );
        assert!(resp.headers().contains_key(header::X_CONTENT_TYPE_OPTIONS));
        assert!(resp.headers().contains_key(header::X_FRAME_OPTIONS));
        assert!(resp.headers().contains_key(header::CONTENT_SECURITY_POLICY));
        assert!(resp.headers().contains_key(header::REFERRER_POLICY));
        assert!(resp.headers().contains_key(header::CACHE_CONTROL));
        assert!(resp.headers().contains_key(header::PRAGMA));
        assert_eq!(
            resp.headers().get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
            "nosniff"
        );
        assert_eq!(resp.headers().get(header::X_FRAME_OPTIONS).unwrap(), "DENY");
    }

    #[tokio::test]
    async fn preserves_handler_csp() {
        let app = Router::new()
            .route("/csp", get(custom_csp_handler))
            .layer(axum::middleware::from_fn(security_headers_middleware));
        let resp = app
            .oneshot(Request::builder().uri("/csp").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_SECURITY_POLICY).unwrap(),
            "default-src 'self'"
        );
    }

    /// Helper to get a middleware-wrapped response for header value assertions.
    async fn get_security_response() -> Response {
        let app = Router::new()
            .route("/test", get(ok_handler))
            .layer(axum::middleware::from_fn(security_headers_middleware));
        app.oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn hsts_includes_preload_and_subdomains() {
        let resp = get_security_response().await;
        let hsts = resp
            .headers()
            .get(header::STRICT_TRANSPORT_SECURITY)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            hsts.contains("max-age=31536000"),
            "HSTS missing 1-year max-age"
        );
        assert!(
            hsts.contains("includeSubDomains"),
            "HSTS missing includeSubDomains"
        );
        assert!(hsts.contains("preload"), "HSTS missing preload");
    }

    #[tokio::test]
    async fn default_csp_denies_all() {
        let resp = get_security_response().await;
        let csp = resp
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            csp.contains("default-src 'none'"),
            "CSP missing default-src 'none': {csp}"
        );
        assert!(
            csp.contains("frame-ancestors 'none'"),
            "CSP missing frame-ancestors: {csp}"
        );
    }

    #[tokio::test]
    async fn x_frame_options_is_deny() {
        let resp = get_security_response().await;
        assert_eq!(resp.headers().get(header::X_FRAME_OPTIONS).unwrap(), "DENY");
    }

    #[tokio::test]
    async fn referrer_policy_is_strict_origin() {
        let resp = get_security_response().await;
        assert_eq!(
            resp.headers().get(header::REFERRER_POLICY).unwrap(),
            "strict-origin-when-cross-origin"
        );
    }

    #[tokio::test]
    async fn permissions_policy_restricts_sensitive_apis() {
        let resp = get_security_response().await;
        let pp = resp
            .headers()
            .get("permissions-policy")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            pp.contains("camera=()"),
            "permissions-policy missing camera=(): {pp}"
        );
        assert!(
            pp.contains("microphone=()"),
            "permissions-policy missing microphone=(): {pp}"
        );
        assert!(
            pp.contains("geolocation=()"),
            "permissions-policy missing geolocation=(): {pp}"
        );
        assert!(
            pp.contains("interest-cohort=()"),
            "permissions-policy missing interest-cohort=(): {pp}"
        );
    }

    #[tokio::test]
    async fn xss_protection_header_set() {
        let resp = get_security_response().await;
        let xss = resp
            .headers()
            .get("x-xss-protection")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(xss, "1; mode=block");
    }

    #[tokio::test]
    async fn cache_control_prevents_caching() {
        let resp = get_security_response().await;
        let cc = resp
            .headers()
            .get(header::CACHE_CONTROL)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            cc.contains("no-store"),
            "Cache-Control missing no-store: {cc}"
        );
        assert!(
            cc.contains("no-cache"),
            "Cache-Control missing no-cache: {cc}"
        );
        assert!(
            cc.contains("must-revalidate"),
            "Cache-Control missing must-revalidate: {cc}"
        );
    }

    #[tokio::test]
    async fn pragma_no_cache_set() {
        let resp = get_security_response().await;
        assert_eq!(resp.headers().get(header::PRAGMA).unwrap(), "no-cache");
    }

    #[tokio::test]
    async fn nosniff_header_value() {
        let resp = get_security_response().await;
        assert_eq!(
            resp.headers().get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
            "nosniff"
        );
    }
}
