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

    // Keep SSE unbuffered end to end.
    //
    // Token-by-token streaming dies behind a buffering reverse proxy: nginx
    // and friends default to `proxy_buffering on`, hold the whole response,
    // and release it as one blob at the end — the stream still "works", it
    // just stops being a stream. `X-Accel-Buffering: no` is the documented
    // opt-out and is inert where no such proxy exists.
    //
    // Applied here rather than per-handler because NyxID emits SSE from
    // several independent places (the proxy's direct and node-routed paths,
    // the Codex translator, the LLM gateway's metered and translated
    // builders, MCP's notification channel). Per-handler opt-in left most
    // of them buffered and silently re-broke every time a new streaming
    // surface appeared. `insert` (not append) guarantees exactly one value
    // even if a handler or upstream already set one.
    if response_is_sse(response.headers()) {
        response.headers_mut().insert(
            "x-accel-buffering".parse::<header::HeaderName>().unwrap(),
            "no".parse().unwrap(),
        );
    }

    response
}

/// Whether a `Content-Type` value names the SSE media type.
///
/// Compares only the media type, case-insensitively: media types are
/// case-insensitive per RFC 9110, and SSE responses normally carry
/// parameters (`text/event-stream; charset=utf-8`). Substring matching
/// would both miss `Text/Event-Stream` and false-positive on an unrelated
/// type whose parameters happen to contain the string.
///
/// Shared with the proxy so "is this SSE?" has one answer everywhere —
/// the proxy also keys `content-length` stripping and SSE usage
/// observation off it.
pub fn is_sse_media_type(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case("text/event-stream")
}

/// Whether a response is Server-Sent Events.
///
/// Duplicate `Content-Type` fields are malformed HTTP but representable,
/// and an upstream can produce them. Any SSE value marks the response:
/// for an anti-buffering safeguard, wrongly disabling buffering is
/// harmless while wrongly leaving it on silently un-streams the response.
fn response_is_sse(headers: &header::HeaderMap) -> bool {
    headers
        .get_all(header::CONTENT_TYPE)
        .iter()
        .any(|value| value.to_str().is_ok_and(is_sse_media_type))
}

/// Apply NyxID's global response-header policy to a router.
///
/// Production assembly (`main.rs`) wraps the fully merged router with
/// this, so every route — public OAuth, `/api/v1`, proxy, LLM gateway,
/// `/mcp` — inherits the security headers and the SSE anti-buffering
/// mark. Keeping it a named function means the guarantee is applied in
/// exactly one reviewable place instead of an inline `.layer()` that is
/// easy to drop or scope to one branch by accident.
pub fn with_response_headers<S>(router: axum::Router<S>) -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router.layer(axum::middleware::from_fn(security_headers_middleware))
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

    // ---- SSE anti-buffering (X-Accel-Buffering) ----
    //
    // This middleware wraps the entire merged router, so these cover every
    // SSE surface at once: the proxy's direct and node-routed paths, the
    // Codex translator, the LLM gateway's metered and translated builders,
    // and MCP's notification channel.

    #[tokio::test]
    async fn duplicate_content_type_with_one_sse_value_is_marked() {
        // Malformed but representable: an upstream can emit two
        // Content-Type fields. Missing the SSE one would silently leave the
        // stream buffered, so any SSE value marks the response.
        async fn handler() -> Response {
            let mut resp = Response::new(Body::empty());
            resp.headers_mut()
                .append(header::CONTENT_TYPE, "application/json".parse().unwrap());
            resp.headers_mut()
                .append(header::CONTENT_TYPE, "text/event-stream".parse().unwrap());
            resp
        }

        let app = Router::new()
            .route("/stream", get(handler))
            .layer(axum::middleware::from_fn(security_headers_middleware));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.headers().get("x-accel-buffering").unwrap(), "no");
    }

    /// The wrapper production assembly uses. Exercising it (rather than a
    /// hand-attached layer) keeps the test honest about what `main.rs`
    /// actually calls.
    #[tokio::test]
    async fn with_response_headers_marks_sse() {
        async fn handler() -> Response {
            let mut resp = Response::new(Body::empty());
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                "text/event-stream; charset=utf-8".parse().unwrap(),
            );
            resp
        }

        let app = with_response_headers(Router::new().route("/stream", get(handler)));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.headers().get("x-accel-buffering").unwrap(), "no");
        assert_eq!(
            resp.headers().get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
            "nosniff",
            "security headers travel with the same wrapper"
        );
    }

    #[test]
    fn is_sse_media_type_ignores_case_and_parameters() {
        assert!(is_sse_media_type("text/event-stream"));
        assert!(is_sse_media_type("Text/Event-Stream; charset=utf-8"));
        assert!(is_sse_media_type("  text/event-stream  "));
        assert!(!is_sse_media_type("application/json"));
        assert!(!is_sse_media_type(
            "application/json; note=text/event-stream"
        ));
        assert!(!is_sse_media_type(""));
    }

    async fn response_through_middleware(content_type: Option<&'static str>) -> Response {
        async fn handler(
            axum::extract::State(content_type): axum::extract::State<Option<&'static str>>,
        ) -> Response {
            let mut resp = Response::new(Body::empty());
            if let Some(ct) = content_type {
                resp.headers_mut()
                    .insert(header::CONTENT_TYPE, ct.parse().unwrap());
            }
            resp
        }

        let app = Router::new()
            .route("/stream", get(handler))
            .with_state(content_type)
            .layer(axum::middleware::from_fn(security_headers_middleware));
        app.oneshot(
            Request::builder()
                .uri("/stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn sse_response_is_marked_unbufferable() {
        let resp = response_through_middleware(Some("text/event-stream")).await;
        let values: Vec<_> = resp.headers().get_all("x-accel-buffering").iter().collect();
        assert_eq!(values.len(), 1, "exactly one value, never a duplicate");
        assert_eq!(values[0], "no");
    }

    #[tokio::test]
    async fn sse_with_charset_parameter_is_marked() {
        // The shape real SSE handlers emit.
        let resp = response_through_middleware(Some("text/event-stream; charset=utf-8")).await;
        assert_eq!(resp.headers().get("x-accel-buffering").unwrap(), "no");
    }

    #[tokio::test]
    async fn sse_media_type_match_is_case_insensitive() {
        // Media types are case-insensitive per RFC 9110; a substring match
        // on the lowercase spelling would silently leave this one buffered.
        let resp = response_through_middleware(Some("Text/Event-Stream; charset=utf-8")).await;
        assert_eq!(resp.headers().get("x-accel-buffering").unwrap(), "no");
    }

    #[tokio::test]
    async fn non_sse_response_is_not_marked() {
        let resp = response_through_middleware(Some("application/json")).await;
        assert!(resp.headers().get("x-accel-buffering").is_none());
    }

    #[tokio::test]
    async fn non_sse_type_mentioning_sse_in_parameters_is_not_marked() {
        // Only the media type counts — parameters must not trigger it.
        let resp =
            response_through_middleware(Some("application/json; note=text/event-stream")).await;
        assert!(resp.headers().get("x-accel-buffering").is_none());
    }

    #[tokio::test]
    async fn response_without_content_type_is_not_marked() {
        let resp = response_through_middleware(None).await;
        assert!(resp.headers().get("x-accel-buffering").is_none());
    }

    /// A handler (or a forwarded upstream copy) that already set the header
    /// must not produce two values — NyxID's `no` is authoritative.
    #[tokio::test]
    async fn preexisting_buffering_header_is_replaced_not_duplicated() {
        async fn handler() -> Response {
            let mut resp = Response::new(Body::empty());
            resp.headers_mut()
                .insert(header::CONTENT_TYPE, "text/event-stream".parse().unwrap());
            resp.headers_mut()
                .insert("x-accel-buffering", "yes".parse().unwrap());
            resp
        }

        let app = Router::new()
            .route("/stream", get(handler))
            .layer(axum::middleware::from_fn(security_headers_middleware));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let values: Vec<_> = resp.headers().get_all("x-accel-buffering").iter().collect();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0], "no");
    }
}
