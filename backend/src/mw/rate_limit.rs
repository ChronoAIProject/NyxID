use axum::{
    body::Body,
    body::to_bytes,
    extract::{ConnectInfo, Extension, State},
    http::{HeaderMap, Request},
    middleware::Next,
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use governor::{
    Quota, RateLimiter,
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
};
use mongodb::{Database, bson::doc};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::errors::AppError;
use crate::models::device_code::{COLLECTION_NAME as DEVICE_CODES, DeviceCode};

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

/// Per-agent rate limiter keyed by API key ID.
/// Each agent gets its own token bucket keyed by API key ID.
/// `rate_limit_per_second` controls refill rate and `burst` controls capacity.
#[derive(Clone)]
pub struct PerAgentRateLimiter {
    state: Arc<Mutex<HashMap<String, AgentBucket>>>,
}

#[derive(Clone, Debug)]
struct AgentBucket {
    tokens: f64,
    last_refill: Instant,
}

impl PerAgentRateLimiter {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Check if a request from the given agent should be allowed.
    /// Returns true if allowed, false if rate limited.
    pub fn check(&self, agent_id: &str, rate_per_second: u32, burst_capacity: u32) -> bool {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let entry = state.entry(agent_id.to_string()).or_insert(AgentBucket {
            tokens: burst_capacity as f64,
            last_refill: now,
        });

        let elapsed_secs = now.duration_since(entry.last_refill).as_secs_f64();
        entry.tokens =
            (entry.tokens + elapsed_secs * rate_per_second as f64).min(burst_capacity as f64);
        entry.last_refill = now;

        if entry.tokens < 1.0 {
            return false;
        }
        entry.tokens -= 1.0;
        true
    }

    /// Remove stale entries to prevent unbounded memory growth.
    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.retain(|_, bucket| now.duration_since(bucket.last_refill).as_secs() < 120);
    }
}

pub type SharedPerAgentRateLimiter = Arc<PerAgentRateLimiter>;

/// Per-device-code rate limiter keyed by Ed25519 public key bytes.
///
/// The device authorization endpoints need a much stricter bucket than the
/// general API limiter because both user-code approval and poll verification
/// are security-sensitive. This bucket is intentionally keyed by the factory
/// public key rather than `device_code`, so leaking an opaque device code does
/// not give an attacker a fresh rate-limit identity.
#[derive(Clone)]
pub struct PerPubkeyRateLimiter {
    state: Arc<Mutex<HashMap<[u8; 32], AgentBucket>>>,
    tokens_per_second: f64,
    burst: u32,
}

impl PerPubkeyRateLimiter {
    pub fn new() -> Self {
        Self::new_with_rate(5.0 / 60.0, 5)
    }

    fn new_with_rate(tokens_per_second: f64, burst: u32) -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
            tokens_per_second,
            burst,
        }
    }

    /// Check if a request from the given device public key should be allowed.
    /// Returns true if allowed, false if rate limited.
    pub fn check(&self, pubkey: &[u8; 32]) -> bool {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let entry = state.entry(*pubkey).or_insert(AgentBucket {
            tokens: self.burst as f64,
            last_refill: now,
        });

        let elapsed_secs = now.duration_since(entry.last_refill).as_secs_f64();
        entry.tokens =
            (entry.tokens + elapsed_secs * self.tokens_per_second).min(self.burst as f64);
        entry.last_refill = now;

        if entry.tokens < 1.0 {
            return false;
        }
        entry.tokens -= 1.0;
        true
    }

    /// Remove stale entries to prevent unbounded memory growth.
    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.retain(|_, bucket| now.duration_since(bucket.last_refill).as_secs() < 120);
    }
}

pub type SharedPerPubkeyRateLimiter = Arc<PerPubkeyRateLimiter>;

#[derive(Clone)]
pub struct DeviceCodeRateLimiters {
    pub per_ip: SharedPerIpRateLimiter,
    pub per_pubkey: SharedPerPubkeyRateLimiter,
    pub db: Option<Database>,
    pub trusted_proxies: Arc<Vec<IpAddr>>,
}

/// Per-message edit limiter keyed by upstream platform message ID.
/// Used by the channel relay edit endpoint so progressive updates on one
/// message cannot starve the rest of the relay.
#[derive(Clone)]
pub struct PerMessageEditRateLimiter {
    state: Arc<Mutex<HashMap<String, AgentBucket>>>,
    rate_per_second: u32,
    burst: u32,
}

impl PerMessageEditRateLimiter {
    pub fn new(rate_per_second: u32, burst: u32) -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
            rate_per_second,
            burst,
        }
    }

    /// Check if an edit for the given upstream message should be allowed.
    pub fn check(&self, platform_message_id: &str) -> bool {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let entry = state
            .entry(platform_message_id.to_string())
            .or_insert(AgentBucket {
                tokens: self.burst as f64,
                last_refill: now,
            });

        let elapsed_secs = now.duration_since(entry.last_refill).as_secs_f64();
        entry.tokens =
            (entry.tokens + elapsed_secs * self.rate_per_second as f64).min(self.burst as f64);
        entry.last_refill = now;

        if entry.tokens < 1.0 {
            return false;
        }
        entry.tokens -= 1.0;
        true
    }

    /// Remove stale entries to prevent unbounded memory growth.
    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.retain(|_, bucket| now.duration_since(bucket.last_refill).as_secs() < 120);
    }
}

pub type SharedPerMessageEditRateLimiter = Arc<PerMessageEditRateLimiter>;

/// Per-channel rate limiter keyed by conversation_id for the HTTP Event
/// Gateway. Distinct from `PerAgentRateLimiter` because event-channel
/// throttling is per-conversation, not per-API-key.
///
/// Token bucket with a fixed rate_per_second and burst capacity shared by
/// every conversation. Rate parameters are set at construction time from
/// env-driven config; per-conversation overrides are not supported in the
/// initial implementation.
#[derive(Clone)]
pub struct PerChannelEventLimiter {
    state: Arc<Mutex<HashMap<String, AgentBucket>>>,
    rate_per_second: u32,
    burst: u32,
}

impl PerChannelEventLimiter {
    pub fn new(rate_per_second: u32, burst: u32) -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
            rate_per_second,
            burst,
        }
    }

    /// Check if an event for the given conversation should be allowed.
    /// Returns `true` if allowed, `false` if rate-limited.
    pub fn check(&self, conversation_id: &str) -> bool {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let entry = state
            .entry(conversation_id.to_string())
            .or_insert(AgentBucket {
                tokens: self.burst as f64,
                last_refill: now,
            });

        let elapsed_secs = now.duration_since(entry.last_refill).as_secs_f64();
        entry.tokens =
            (entry.tokens + elapsed_secs * self.rate_per_second as f64).min(self.burst as f64);
        entry.last_refill = now;

        if entry.tokens < 1.0 {
            return false;
        }
        entry.tokens -= 1.0;
        true
    }

    /// Remove stale entries to prevent unbounded memory growth.
    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.retain(|_, bucket| now.duration_since(bucket.last_refill).as_secs() < 120);
    }

    #[cfg(test)]
    fn active_conversations(&self) -> usize {
        self.state.lock().unwrap().len()
    }
}

pub type SharedPerChannelEventLimiter = Arc<PerChannelEventLimiter>;

/// Check per-agent rate limit. Call from proxy handlers after auth extraction.
pub fn check_agent_rate_limit(
    limiter: &PerAgentRateLimiter,
    auth_user: &crate::mw::auth::AuthUser,
) -> Result<(), crate::errors::AppError> {
    check_agent_rate_limit_raw(
        limiter,
        auth_user.api_key_id.as_deref(),
        auth_user.rate_limit_per_second,
        auth_user.rate_limit_burst,
    )
}

/// Check per-agent rate limit using raw API-key identity and limit fields.
/// Used by callers that don't hold an `AuthUser` (e.g. the MCP transport).
pub fn check_agent_rate_limit_raw(
    limiter: &PerAgentRateLimiter,
    api_key_id: Option<&str>,
    rate_limit_per_second: Option<u32>,
    rate_limit_burst: Option<u32>,
) -> Result<(), crate::errors::AppError> {
    if let (Some(agent_id), Some(rps)) = (api_key_id, rate_limit_per_second) {
        // When no explicit burst is set, use the sustained rate as the ceiling.
        // Users who want a higher burst can set rate_limit_burst explicitly.
        let burst = rate_limit_burst.unwrap_or(rps);
        if !limiter.check(agent_id, rps, burst) {
            tracing::warn!(
                agent_id = %agent_id,
                rate_limit = rps,
                "Per-agent rate limit exceeded"
            );
            return Err(crate::errors::AppError::RateLimited);
        }
    }
    Ok(())
}

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

/// Create a per-pubkey limiter for device authorization endpoints.
pub fn create_per_pubkey_rate_limiter() -> SharedPerPubkeyRateLimiter {
    Arc::new(PerPubkeyRateLimiter::new())
}

#[allow(dead_code)]
pub fn device_code_rate_limit_layer(limiters: DeviceCodeRateLimiters) -> impl Clone + 'static {
    axum::middleware::from_fn_with_state::<
        _,
        DeviceCodeRateLimiters,
        (State<DeviceCodeRateLimiters>, Request<Body>),
    >(limiters, device_code_rate_limit_middleware)
}

/// Resolve the client IP for per-IP rate-limit keying behind a
/// configurable trusted-proxy allowlist.
///
/// Most deployments put NyxID behind a reverse proxy (nginx, AWS ALB,
/// Fly.io, etc.); every request's TCP peer is then the proxy itself,
/// so a per-peer bucket collapses into a single site-wide bucket. The
/// `X-Forwarded-For` / `X-Real-IP` headers carry the real client IP,
/// but are client-spoofable when accepted unconditionally — which
/// would let an attacker bypass the very rate limit this helper
/// guards.
///
/// The trade-off is resolved with an allowlist: the forwarded headers
/// are honored only when the TCP peer is one of `trusted_proxies`.
/// Otherwise the peer IP wins, so:
///
///   - Direct-exposure deployments (no `TRUSTED_PROXY_IPS` configured)
///     get the pre-change behavior: per-peer bucket, unspoofable.
///   - Proxy deployments that list their proxy IPs in
///     `TRUSTED_PROXY_IPS` get per-real-client buckets.
///   - A request whose peer isn't trusted can still set
///     `X-Forwarded-For` — the header is ignored so bypass is
///     impossible.
///
/// `X-Forwarded-For` is read left-to-right per the de-facto standard:
/// the leftmost entry is the originating client, each subsequent
/// entry is a proxy closer to the server. We take the leftmost valid
/// IP, matching the behavior of `extract_ip` for audit logging. Only
/// entries that parse as `IpAddr` are accepted; malformed values fall
/// through to the peer.
pub fn resolve_client_ip_for_rate_limit(
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    trusted_proxies: &[IpAddr],
) -> Option<IpAddr> {
    let peer_ip = peer.map(|p| p.ip());

    // Peer must be a trusted proxy before we honor any forwarded
    // header. Empty allowlist => never trusted, preserving the
    // pre-change "peer IP wins" behavior.
    let peer_is_trusted = peer_ip
        .as_ref()
        .map(|ip| trusted_proxies.contains(ip))
        .unwrap_or(false);

    if peer_is_trusted {
        if let Some(ip) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse::<IpAddr>().ok())
        {
            return Some(ip);
        }

        if let Some(ip) = headers
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse::<IpAddr>().ok())
        {
            return Some(ip);
        }
    }

    peer_ip
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

pub async fn device_code_rate_limit_middleware(
    State(limiters): State<DeviceCodeRateLimiters>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    if !request.uri().path().starts_with("/api/v1/devices/code/") {
        return Ok(next.run(request).await);
    }

    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(peer)| *peer);
    enforce_device_code_ip_rate_limit(&limiters, request.headers(), peer, request.uri().path())?;

    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, 64 * 1024)
        .await
        .map_err(|_| AppError::BadRequest("Unable to read request body".to_string()))?;

    if let Some(pubkey) = extract_device_pubkey_for_rate_limit(limiters.db.as_ref(), &bytes).await
        && !limiters.per_pubkey.check(&pubkey)
    {
        tracing::warn!(
            path = %parts.uri.path(),
            device_pubkey = %hex::encode(pubkey),
            "Device-code per-pubkey rate limit exceeded"
        );
        return Err(AppError::DeviceCodeRateLimited);
    }

    let request = Request::from_parts(parts, Body::from(bytes));
    Ok(next.run(request).await)
}

fn enforce_device_code_ip_rate_limit(
    limiters: &DeviceCodeRateLimiters,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    path: &str,
) -> Result<Option<IpAddr>, AppError> {
    let Some(client_ip) =
        resolve_client_ip_for_rate_limit(headers, peer, limiters.trusted_proxies.as_slice())
    else {
        tracing::debug!(
            path = %path,
            "Skipping device-code per-IP rate limit because no trusted peer IP is available"
        );
        return Ok(None);
    };

    if !limiters.per_ip.check(client_ip) {
        tracing::warn!(
            path = %path,
            ip = %client_ip,
            "Device-code per-IP rate limit exceeded"
        );
        return Err(AppError::DeviceCodeRateLimited);
    }

    Ok(Some(client_ip))
}

async fn extract_device_pubkey_for_rate_limit(
    db: Option<&Database>,
    bytes: &[u8],
) -> Option<[u8; 32]> {
    if let Some(pubkey) = extract_device_pubkey_from_json(bytes) {
        return Some(pubkey);
    }

    let db = db?;
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let raw_device_code = value.get("device_code")?.as_str()?;
    let row = db
        .collection::<DeviceCode>(DEVICE_CODES)
        .find_one(doc! {
            "device_code_hash": crate::crypto::token::hash_token(raw_device_code),
        })
        .await
        .ok()??;
    row.device_pubkey.try_into().ok()
}

fn extract_device_pubkey_from_json(bytes: &[u8]) -> Option<[u8; 32]> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let raw = value.get("device_pubkey")?.as_str()?;
    let decoded = BASE64_STANDARD.decode(raw).ok()?;
    decoded.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::sync::Arc;

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

    #[test]
    fn per_agent_allows_under_limit() {
        let limiter = PerAgentRateLimiter::new();
        assert!(limiter.check("agent-1", 3, 3));
        assert!(limiter.check("agent-1", 3, 3));
        assert!(limiter.check("agent-1", 3, 3));
    }

    #[test]
    fn per_agent_blocks_over_limit() {
        let limiter = PerAgentRateLimiter::new();
        assert!(limiter.check("agent-2", 2, 2));
        assert!(limiter.check("agent-2", 2, 2));
        assert!(!limiter.check("agent-2", 2, 2));
    }

    #[test]
    fn per_agent_different_agents_independent() {
        let limiter = PerAgentRateLimiter::new();
        assert!(limiter.check("agent-a", 1, 1));
        assert!(!limiter.check("agent-a", 1, 1));
        assert!(limiter.check("agent-b", 1, 1));
    }

    #[test]
    fn per_agent_uses_burst_without_turning_it_into_sustained_limit() {
        let limiter = PerAgentRateLimiter::new();
        assert!(limiter.check("agent-burst", 1, 2));
        assert!(limiter.check("agent-burst", 1, 2));
        assert!(!limiter.check("agent-burst", 1, 2));
    }

    #[test]
    fn per_agent_cleanup_does_not_panic() {
        let limiter = PerAgentRateLimiter::new();
        limiter.check("agent-x", 10, 10);
        limiter.cleanup();
    }

    #[test]
    fn per_pubkey_allows_five_requests_per_window() {
        let limiter = PerPubkeyRateLimiter::new();
        let pubkey = [7u8; 32];

        for _ in 0..5 {
            assert!(limiter.check(&pubkey));
        }
        assert!(!limiter.check(&pubkey));
    }

    #[test]
    fn per_pubkey_isolates_distinct_public_keys() {
        let limiter = PerPubkeyRateLimiter::new();
        let pubkey_a = [1u8; 32];
        let pubkey_b = [2u8; 32];

        for _ in 0..5 {
            assert!(limiter.check(&pubkey_a));
        }
        assert!(!limiter.check(&pubkey_a));
        assert!(limiter.check(&pubkey_b));
    }

    #[test]
    fn per_pubkey_refills_over_time() {
        let limiter = PerPubkeyRateLimiter::new_with_rate(100.0, 1);
        let pubkey = [3u8; 32];

        assert!(limiter.check(&pubkey));
        assert!(!limiter.check(&pubkey));
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(limiter.check(&pubkey));
    }

    #[test]
    fn extracts_device_pubkey_from_base64_json() {
        let pubkey = [9u8; 32];
        let body = serde_json::json!({
            "device_pubkey": BASE64_STANDARD.encode(pubkey),
        });

        assert_eq!(
            extract_device_pubkey_from_json(&serde_json::to_vec(&body).unwrap()),
            Some(pubkey)
        );
    }

    #[test]
    fn rejects_missing_or_wrong_length_device_pubkey_for_rate_limit_keying() {
        let missing = serde_json::json!({ "hw_id": "esp32" });
        let short = serde_json::json!({
            "device_pubkey": BASE64_STANDARD.encode([1u8; 31]),
        });

        assert_eq!(
            extract_device_pubkey_from_json(&serde_json::to_vec(&missing).unwrap()),
            None
        );
        assert_eq!(
            extract_device_pubkey_from_json(&serde_json::to_vec(&short).unwrap()),
            None
        );
    }

    #[test]
    fn per_channel_limiter_allows_up_to_burst() {
        let limiter = PerChannelEventLimiter::new(100, 3);
        assert!(limiter.check("conv-a"));
        assert!(limiter.check("conv-a"));
        assert!(limiter.check("conv-a"));
        assert!(!limiter.check("conv-a"));
    }

    #[test]
    fn per_channel_limiter_isolates_conversations() {
        let limiter = PerChannelEventLimiter::new(100, 1);
        assert!(limiter.check("conv-a"));
        assert!(!limiter.check("conv-a"));
        // Second conversation still has its own bucket.
        assert!(limiter.check("conv-b"));
    }

    #[test]
    fn per_channel_limiter_refills_over_time() {
        // 100 req/s with burst 1 → roughly 10ms per token
        let limiter = PerChannelEventLimiter::new(100, 1);
        assert!(limiter.check("conv"));
        assert!(!limiter.check("conv"));
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(limiter.check("conv"));
    }

    #[test]
    fn per_channel_limiter_cleanup_does_not_panic() {
        let limiter = PerChannelEventLimiter::new(100, 100);
        limiter.check("conv-clean");
        limiter.cleanup();
        // Still usable after cleanup.
        assert!(limiter.check("conv-clean"));
    }

    fn socket(ip: IpAddr) -> SocketAddr {
        SocketAddr::new(ip, 4242)
    }

    #[test]
    fn resolve_client_ip_falls_back_to_peer_when_no_trusted_proxies() {
        let peer_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10));
        let mut headers = HeaderMap::new();
        // XFF set but we don't trust the peer: header must be
        // ignored so a direct-exposure deployment can't be spoofed.
        headers.insert("x-forwarded-for", "198.51.100.4".parse().unwrap());
        let resolved = resolve_client_ip_for_rate_limit(&headers, Some(socket(peer_ip)), &[]);
        assert_eq!(resolved, Some(peer_ip));
    }

    #[test]
    fn resolve_client_ip_honors_xff_when_peer_is_trusted_proxy() {
        let proxy_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let client_ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7));
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            format!("{client_ip}, 10.0.0.1").parse().unwrap(),
        );
        let resolved =
            resolve_client_ip_for_rate_limit(&headers, Some(socket(proxy_ip)), &[proxy_ip]);
        assert_eq!(resolved, Some(client_ip));
    }

    #[test]
    fn resolve_client_ip_honors_x_real_ip_fallback_when_trusted() {
        let proxy_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        let client_ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9));
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", client_ip.to_string().parse().unwrap());
        let resolved =
            resolve_client_ip_for_rate_limit(&headers, Some(socket(proxy_ip)), &[proxy_ip]);
        assert_eq!(resolved, Some(client_ip));
    }

    #[test]
    fn resolve_client_ip_ignores_xff_when_peer_not_in_allowlist() {
        let peer_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 99));
        let proxy_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 3));
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.55".parse().unwrap());
        let resolved =
            resolve_client_ip_for_rate_limit(&headers, Some(socket(peer_ip)), &[proxy_ip]);
        assert_eq!(resolved, Some(peer_ip));
    }

    #[test]
    fn resolve_client_ip_drops_malformed_xff_entry_and_uses_peer() {
        let proxy_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 4));
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "not-an-ip".parse().unwrap());
        let resolved =
            resolve_client_ip_for_rate_limit(&headers, Some(socket(proxy_ip)), &[proxy_ip]);
        assert_eq!(resolved, Some(proxy_ip));
    }

    #[test]
    fn resolve_client_ip_takes_leftmost_xff_entry() {
        let proxy_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "198.51.100.11, 192.0.2.1, 10.0.0.5".parse().unwrap(),
        );
        let resolved =
            resolve_client_ip_for_rate_limit(&headers, Some(socket(proxy_ip)), &[proxy_ip]);
        assert_eq!(resolved, Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 11))));
    }

    #[test]
    fn resolve_client_ip_handles_missing_peer() {
        // No peer means we can't make a trust decision. XFF must
        // still be ignored — returning `None` lets the caller decide
        // how to handle the ambiguity (typically: skip the per-IP
        // bucket entirely).
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.4".parse().unwrap());
        let resolved = resolve_client_ip_for_rate_limit(
            &headers,
            None,
            &[IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))],
        );
        assert!(resolved.is_none());
    }

    #[test]
    fn device_code_ip_limiter_skips_ip_bucket_without_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.4".parse().unwrap());
        let limiters = DeviceCodeRateLimiters {
            per_ip: Arc::new(PerIpRateLimiter::new(0, 60)),
            per_pubkey: Arc::new(PerPubkeyRateLimiter::new()),
            db: None,
            trusted_proxies: Arc::new(vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))]),
        };

        let result = enforce_device_code_ip_rate_limit(
            &limiters,
            &headers,
            None,
            "/api/v1/devices/code/poll",
        )
        .expect("missing peer should skip IP bucket");

        assert!(result.is_none());
    }

    #[test]
    fn device_code_ip_limiter_honors_xff_only_from_trusted_proxy() {
        let proxy_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let client_ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 4));
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", client_ip.to_string().parse().unwrap());
        let limiters = DeviceCodeRateLimiters {
            per_ip: Arc::new(PerIpRateLimiter::new(1, 60)),
            per_pubkey: Arc::new(PerPubkeyRateLimiter::new()),
            db: None,
            trusted_proxies: Arc::new(vec![proxy_ip]),
        };

        let first = enforce_device_code_ip_rate_limit(
            &limiters,
            &headers,
            Some(socket(proxy_ip)),
            "/api/v1/devices/code/request",
        )
        .expect("first request allowed");
        let second = enforce_device_code_ip_rate_limit(
            &limiters,
            &headers,
            Some(socket(proxy_ip)),
            "/api/v1/devices/code/request",
        )
        .expect_err("second request should be rate-limited by forwarded client IP");

        assert_eq!(first, Some(client_ip));
        assert!(matches!(second, AppError::DeviceCodeRateLimited));
    }

    #[test]
    fn per_channel_limiter_tracks_active_conversations() {
        let limiter = PerChannelEventLimiter::new(100, 10);
        limiter.check("conv-1");
        limiter.check("conv-2");
        limiter.check("conv-3");
        assert_eq!(limiter.active_conversations(), 3);
    }
}
