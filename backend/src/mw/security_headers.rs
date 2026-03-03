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
    headers.insert(
        header::CACHE_CONTROL,
        "no-store, no-cache, must-revalidate".parse().unwrap(),
    );
    headers.insert(header::PRAGMA, "no-cache".parse().unwrap());

    response
}
