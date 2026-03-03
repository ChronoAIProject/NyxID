use axum::{body::Body, extract::Extension, http::Request, middleware::Next, response::Response};
use governor::{
    Quota, RateLimiter,
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
};
use std::collections::HashMap;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::errors::AppError;

/// A shared rate limiter instance for global fallback.
/// Uses a token-bucket algorithm via the `governor` crate.
pub type SharedRateLimiter = Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>;

/// Per-IP rate limiter state using a simple sliding window approach.
#[derive(Clone)]
pub struct PerIpRateLimiter {
    /// Map of IP address to (request count, window start time)
    state: Arc<Mutex<HashMap<IpAddr, (u32, Instant)>>>,
    /// Maximum requests allowed per window
    max_requests: u32,
    /// Window duration in seconds
    window_secs: u64,
}

impl PerIpRateLimiter {
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window_secs,
        }
    }

    /// Check if a request from the given IP should be allowed.
    /// Returns true if allowed, false if rate limited.
    pub fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        let entry = state.entry(ip).or_insert((0, now));

        // Reset window if expired
        if now.duration_since(entry.1).as_secs() >= self.window_secs {
            entry.0 = 0;
            entry.1 = now;
        }

        if entry.0 >= self.max_requests {
            return false;
        }

        entry.0 += 1;
        true
    }

    /// Periodically clean up expired entries to prevent memory growth.
    /// Call this from a background task.
    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.retain(|_, (_, start)| now.duration_since(*start).as_secs() < self.window_secs * 2);
    }
}

/// Shared per-IP rate limiter type for use as an Extension.
pub type SharedPerIpRateLimiter = Arc<PerIpRateLimiter>;

/// Create a new global rate limiter (kept as fallback).
///
/// The limiter allows `per_second` requests per second with a burst capacity
/// of `burst` requests.
pub fn create_rate_limiter(per_second: u64, burst: u32) -> SharedRateLimiter {
    let quota = Quota::per_second(NonZeroU32::new(per_second as u32).unwrap_or(NonZeroU32::MIN))
        .allow_burst(NonZeroU32::new(burst).unwrap_or(NonZeroU32::MIN));

    Arc::new(RateLimiter::direct(quota))
}

/// Create a per-IP rate limiter.
pub fn create_per_ip_rate_limiter(max_requests: u32, window_secs: u64) -> SharedPerIpRateLimiter {
    Arc::new(PerIpRateLimiter::new(max_requests, window_secs))
}

/// Extract the client IP address from the request.
/// Checks X-Forwarded-For, X-Real-IP headers, then falls back to a default.
///
/// TODO(SEC-2): X-Forwarded-For and X-Real-IP headers can be spoofed by
/// clients, allowing rate limit bypass. In production, either:
/// 1. Configure the reverse proxy to strip/override client-supplied headers
///    and only trust headers from known proxy IPs, or
/// 2. Use Axum's `ConnectInfo<SocketAddr>` to get the real peer address
///    and only fall back to forwarded headers when the peer is a trusted proxy.
///    Document the required reverse proxy configuration in DEPLOYMENT.md.
fn extract_client_ip(request: &Request<Body>) -> IpAddr {
    // Try X-Forwarded-For first
    if let Some(forwarded_for) = request.headers().get("x-forwarded-for")
        && let Ok(value) = forwarded_for.to_str()
        && let Some(first_ip) = value.split(',').next()
        && let Ok(ip) = first_ip.trim().parse::<IpAddr>()
    {
        return ip;
    }

    // Try X-Real-IP
    if let Some(real_ip) = request.headers().get("x-real-ip")
        && let Ok(value) = real_ip.to_str()
        && let Ok(ip) = value.trim().parse::<IpAddr>()
    {
        return ip;
    }

    // Fallback to loopback (in production, the reverse proxy should always set headers)
    IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
}

/// Axum middleware that enforces per-IP rate limiting with global fallback.
///
/// Expects both `SharedPerIpRateLimiter` and `SharedRateLimiter` as layer Extensions.
/// Returns 429 Too Many Requests when the limit is exceeded.
/// Paths exempt from rate limiting (authenticated via other means).
const RATE_LIMIT_EXEMPT_PATHS: &[&str] = &["/mcp", "/.well-known/", "/health"];

pub async fn rate_limit_middleware(
    Extension(per_ip_limiter): Extension<SharedPerIpRateLimiter>,
    Extension(global_limiter): Extension<SharedRateLimiter>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let path = request.uri().path();

    // Skip rate limiting for exempt paths (MCP has its own auth + session management)
    if RATE_LIMIT_EXEMPT_PATHS.iter().any(|p| path.starts_with(p)) {
        return Ok(next.run(request).await);
    }

    let client_ip = extract_client_ip(&request);

    // Check per-IP rate limit first
    if !per_ip_limiter.check(client_ip) {
        tracing::warn!(
            path = %path,
            ip = %client_ip,
            "Per-IP rate limit exceeded"
        );
        return Err(AppError::RateLimited);
    }

    // Also check global rate limit as a safety net
    if global_limiter.check().is_err() {
        tracing::warn!(
            path = %path,
            "Global rate limit exceeded"
        );
        return Err(AppError::RateLimited);
    }

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn per_ip_allows_under_limit() {
        let limiter = PerIpRateLimiter::new(3, 60);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
    }

    #[test]
    fn per_ip_blocks_over_limit() {
        let limiter = PerIpRateLimiter::new(2, 60);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
        assert!(!limiter.check(ip));
    }

    #[test]
    fn per_ip_different_ips_independent() {
        let limiter = PerIpRateLimiter::new(1, 60);
        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        assert!(limiter.check(ip1));
        assert!(!limiter.check(ip1));
        assert!(limiter.check(ip2));
    }

    #[test]
    fn per_ip_ipv6_works() {
        let limiter = PerIpRateLimiter::new(1, 60);
        let ip = IpAddr::V6(Ipv6Addr::LOCALHOST);
        assert!(limiter.check(ip));
        assert!(!limiter.check(ip));
    }

    #[test]
    fn cleanup_does_not_panic() {
        let limiter = PerIpRateLimiter::new(100, 0);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        limiter.check(ip);
        limiter.cleanup();
    }

    #[test]
    fn create_rate_limiter_does_not_panic() {
        let _limiter = create_rate_limiter(10, 30);
    }

    #[test]
    fn create_per_ip_rate_limiter_does_not_panic() {
        let _limiter = create_per_ip_rate_limiter(30, 1);
    }

    #[test]
    fn extract_client_ip_x_forwarded_for() {
        let req = Request::builder()
            .header("x-forwarded-for", "203.0.113.50, 70.41.3.18")
            .body(Body::empty())
            .unwrap();
        let ip = extract_client_ip(&req);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 50)));
    }

    #[test]
    fn extract_client_ip_x_real_ip() {
        let req = Request::builder()
            .header("x-real-ip", "198.51.100.22")
            .body(Body::empty())
            .unwrap();
        let ip = extract_client_ip(&req);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(198, 51, 100, 22)));
    }

    #[test]
    fn extract_client_ip_fallback_to_localhost() {
        let req = Request::builder().body(Body::empty()).unwrap();
        let ip = extract_client_ip(&req);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn extract_client_ip_invalid_header_falls_through() {
        let req = Request::builder()
            .header("x-forwarded-for", "not-an-ip")
            .body(Body::empty())
            .unwrap();
        let ip = extract_client_ip(&req);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn extract_client_ip_prefers_forwarded_for_over_real_ip() {
        let req = Request::builder()
            .header("x-forwarded-for", "1.2.3.4")
            .header("x-real-ip", "5.6.7.8")
            .body(Body::empty())
            .unwrap();
        let ip = extract_client_ip(&req);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)));
    }
}
