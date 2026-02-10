use axum::{
    body::Body,
    extract::Extension,
    http::Request,
    middleware::Next,
    response::Response,
};
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
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
/// Document the required reverse proxy configuration in DEPLOYMENT.md.
fn extract_client_ip(request: &Request<Body>) -> IpAddr {
    // Try X-Forwarded-For first
    if let Some(forwarded_for) = request.headers().get("x-forwarded-for") {
        if let Ok(value) = forwarded_for.to_str() {
            if let Some(first_ip) = value.split(',').next() {
                if let Ok(ip) = first_ip.trim().parse::<IpAddr>() {
                    return ip;
                }
            }
        }
    }

    // Try X-Real-IP
    if let Some(real_ip) = request.headers().get("x-real-ip") {
        if let Ok(value) = real_ip.to_str() {
            if let Ok(ip) = value.trim().parse::<IpAddr>() {
                return ip;
            }
        }
    }

    // Fallback to loopback (in production, the reverse proxy should always set headers)
    IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
}

/// Axum middleware that enforces per-IP rate limiting with global fallback.
///
/// Expects both `SharedPerIpRateLimiter` and `SharedRateLimiter` as layer Extensions.
/// Returns 429 Too Many Requests when the limit is exceeded.
pub async fn rate_limit_middleware(
    Extension(per_ip_limiter): Extension<SharedPerIpRateLimiter>,
    Extension(global_limiter): Extension<SharedRateLimiter>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let client_ip = extract_client_ip(&request);

    // Check per-IP rate limit first
    if !per_ip_limiter.check(client_ip) {
        tracing::warn!(
            path = %request.uri().path(),
            ip = %client_ip,
            "Per-IP rate limit exceeded"
        );
        return Err(AppError::RateLimited);
    }

    // Also check global rate limit as a safety net
    if global_limiter.check().is_err() {
        tracing::warn!(
            path = %request.uri().path(),
            "Global rate limit exceeded"
        );
        return Err(AppError::RateLimited);
    }

    Ok(next.run(request).await)
}
