use axum::{
    body::Body,
    body::to_bytes,
    extract::{ConnectInfo, Extension, State},
    http::{HeaderMap, Request},
    middleware::Next,
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use mongodb::{Database, bson::doc};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, OnceLock};
#[cfg(test)]
use std::{collections::HashMap, sync::Mutex, time::Instant};

use crate::config::{TrustedProxyRange, normalize_ip_address};
use crate::errors::AppError;
use crate::models::device_code::{COLLECTION_NAME as DEVICE_CODES, DeviceCode};
use crate::services::cluster_slot_service::{RenewableSlotGuard, RenewableSlotManager};
use crate::services::coordination_service::{RateWindowStore, TokenBucketStore};

const GLOBAL_NAMESPACE: &str = "global";

#[derive(Clone)]
pub struct GlobalRateLimiter {
    db: Option<Database>,
    per_second: u32,
    burst: u32,
    #[cfg(test)]
    local: Arc<Mutex<AgentBucket>>,
}

impl GlobalRateLimiter {
    pub async fn check_shared(&self) -> Result<bool, AppError> {
        if let Some(db) = self.db.as_ref() {
            return Ok(TokenBucketStore::admit(
                db,
                GLOBAL_NAMESPACE,
                "all",
                self.per_second,
                self.burst,
            )
            .await?
            .allowed);
        }
        #[cfg(test)]
        {
            let now = Instant::now();
            let mut bucket = self.local.lock().unwrap_or_else(|error| error.into_inner());
            let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
            bucket.tokens =
                (bucket.tokens + elapsed * f64::from(self.per_second)).min(f64::from(self.burst));
            bucket.last_refill = now;
            if bucket.tokens < 1.0 {
                return Ok(false);
            }
            bucket.tokens -= 1.0;
            Ok(true)
        }
        #[cfg(not(test))]
        unreachable!("production rate limiters always have a MongoDB backend")
    }

    #[cfg(test)]
    pub fn new_local(per_second: u32, burst: u32) -> Self {
        Self {
            db: None,
            per_second,
            burst,
            local: Arc::new(Mutex::new(AgentBucket {
                tokens: f64::from(burst),
                last_refill: Instant::now(),
            })),
        }
    }
}

pub type SharedRateLimiter = Arc<GlobalRateLimiter>;

/// Per-IP rate limiter state using a simple sliding window approach.
#[derive(Clone)]
pub struct PerIpRateLimiter {
    db: Option<Database>,
    namespace: String,
    /// Map of IP address to (request count, window start time)
    #[cfg(test)]
    state: Arc<Mutex<HashMap<IpAddr, (u32, Instant)>>>,
    /// Maximum requests allowed per window
    max_requests: u32,
    /// Window duration in seconds
    window_secs: u64,
}

impl PerIpRateLimiter {
    #[cfg(test)]
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            db: None,
            namespace: "test_ip".to_string(),
            state: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window_secs,
        }
    }

    pub fn with_db(
        db: Database,
        namespace: impl Into<String>,
        max_requests: u32,
        window_secs: u64,
    ) -> Self {
        Self {
            db: Some(db),
            namespace: namespace.into(),
            #[cfg(test)]
            state: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window_secs,
        }
    }

    pub async fn check_shared(&self, ip: IpAddr) -> Result<bool, AppError> {
        if let Some(db) = self.db.as_ref() {
            return Ok(RateWindowStore::admit(
                db,
                &self.namespace,
                &ip.to_string(),
                u64::from(self.max_requests),
                std::time::Duration::from_secs(self.window_secs),
            )
            .await?
            .allowed);
        }
        #[cfg(test)]
        return Ok(self.check(ip));
        #[cfg(not(test))]
        unreachable!("production rate limiters always have a MongoDB backend")
    }

    /// Check if a request from the given IP should be allowed.
    /// Returns true if allowed, false if rate limited.
    #[cfg(test)]
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
    #[cfg(test)]
    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.retain(|_, (_, start)| now.duration_since(*start).as_secs() < self.window_secs * 2);
    }
}

/// Shared per-IP rate limiter type for use as an Extension.
pub type SharedPerIpRateLimiter = Arc<PerIpRateLimiter>;

/// Per-string-key limiter for cases where the security principal is not an IP
/// address, e.g. human user IDs. Uses the same fixed-window semantics as
/// `PerIpRateLimiter`.
#[derive(Clone)]
pub struct PerKeyRateLimiter {
    db: Option<Database>,
    namespace: String,
    #[cfg(test)]
    state: Arc<Mutex<HashMap<String, (u32, Instant)>>>,
    max_requests: u32,
    window_secs: u64,
}

impl PerKeyRateLimiter {
    #[cfg(test)]
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            db: None,
            namespace: "test_key".to_string(),
            #[cfg(test)]
            state: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window_secs,
        }
    }

    pub fn with_db(
        db: Database,
        namespace: impl Into<String>,
        max_requests: u32,
        window_secs: u64,
    ) -> Self {
        Self {
            db: Some(db),
            namespace: namespace.into(),
            #[cfg(test)]
            state: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window_secs,
        }
    }

    pub async fn check_shared(&self, key: &str) -> Result<bool, AppError> {
        if let Some(db) = self.db.as_ref() {
            return Ok(RateWindowStore::admit(
                db,
                &self.namespace,
                key,
                u64::from(self.max_requests),
                std::time::Duration::from_secs(self.window_secs),
            )
            .await?
            .allowed);
        }
        #[cfg(test)]
        return Ok(self.check(key));
        #[cfg(not(test))]
        unreachable!("production rate limiters always have a MongoDB backend")
    }

    #[cfg(test)]
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let entry = state.entry(key.to_string()).or_insert((0, now));

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

    #[cfg(test)]
    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.retain(|_, (_, start)| now.duration_since(*start).as_secs() < self.window_secs * 2);
    }
}

pub type SharedPerKeyRateLimiter = Arc<PerKeyRateLimiter>;

/// Cost control for the human-only direct assistant endpoint. A successful
/// acquisition consumes one request from the fixed window and one in-flight
/// slot until the returned permit is dropped.
#[derive(Clone)]
pub struct DirectChatRateLimiter {
    db: Option<Database>,
    slot_manager: Option<RenewableSlotManager>,
    #[cfg(test)]
    state: Arc<Mutex<HashMap<String, DirectChatUserState>>>,
    max_requests: u32,
    window_secs: u64,
    max_in_flight: u32,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct DirectChatUserState {
    request_count: u32,
    window_start: Instant,
    in_flight: u32,
}

impl DirectChatRateLimiter {
    #[cfg(test)]
    pub fn new(max_requests: u32, window_secs: u64, max_in_flight: u32) -> Self {
        Self {
            db: None,
            slot_manager: None,
            #[cfg(test)]
            state: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window_secs,
            max_in_flight,
        }
    }

    pub fn with_db(db: Database, slot_manager: RenewableSlotManager) -> Self {
        Self {
            db: Some(db),
            slot_manager: Some(slot_manager),
            #[cfg(test)]
            state: Arc::new(Mutex::new(HashMap::new())),
            max_requests: 10,
            window_secs: 60,
            max_in_flight: 2,
        }
    }

    pub async fn try_acquire(
        self: &Arc<Self>,
        user_id: &str,
    ) -> Result<DirectChatPermit, AppError> {
        if let (Some(db), Some(slot_manager)) = (&self.db, &self.slot_manager) {
            let slot = slot_manager
                .acquire("direct_chat", user_id, self.max_in_flight)
                .await?
                .ok_or(AppError::RateLimited)?;
            let admitted = RateWindowStore::admit(
                db,
                "direct_chat_request",
                user_id,
                u64::from(self.max_requests),
                std::time::Duration::from_secs(self.window_secs),
            )
            .await?
            .allowed;
            if !admitted {
                return Err(AppError::RateLimited);
            }
            return Ok(DirectChatPermit {
                slot: Some(slot),
                #[cfg(test)]
                local_limiter: None,
                #[cfg(test)]
                user_id: user_id.to_string(),
            });
        }

        #[cfg(not(test))]
        unreachable!("production direct-chat limiters always have a MongoDB backend");

        #[cfg(test)]
        {
            let now = Instant::now();
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let entry = state
                .entry(user_id.to_string())
                .or_insert(DirectChatUserState {
                    request_count: 0,
                    window_start: now,
                    in_flight: 0,
                });

            if now.duration_since(entry.window_start).as_secs() >= self.window_secs {
                entry.request_count = 0;
                entry.window_start = now;
            }

            if entry.request_count >= self.max_requests || entry.in_flight >= self.max_in_flight {
                return Err(AppError::RateLimited);
            }

            entry.request_count += 1;
            entry.in_flight += 1;
            drop(state);

            Ok(DirectChatPermit {
                slot: None,
                local_limiter: Some(self.clone()),
                user_id: user_id.to_string(),
            })
        }
    }

    #[cfg(test)]
    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.retain(|_, entry| {
            entry.in_flight > 0
                || now.duration_since(entry.window_start).as_secs() < self.window_secs * 2
        });
    }

    #[cfg(test)]
    fn release(&self, user_id: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = state.get_mut(user_id) {
            entry.in_flight = entry.in_flight.saturating_sub(1);
        }
    }
}

pub struct DirectChatPermit {
    slot: Option<RenewableSlotGuard>,
    #[cfg(test)]
    local_limiter: Option<Arc<DirectChatRateLimiter>>,
    #[cfg(test)]
    user_id: String,
}

impl DirectChatPermit {
    pub async fn cancelled(&self) {
        if let Some(slot) = self.slot.as_ref() {
            slot.cancelled().await;
        } else {
            std::future::pending::<()>().await;
        }
    }
}

impl Drop for DirectChatPermit {
    fn drop(&mut self) {
        #[cfg(test)]
        if let Some(limiter) = self.local_limiter.as_ref() {
            limiter.release(&self.user_id);
        }
    }
}

pub type SharedDirectChatRateLimiter = Arc<DirectChatRateLimiter>;

pub fn create_direct_chat_rate_limiter(
    db: Database,
    slot_manager: RenewableSlotManager,
) -> SharedDirectChatRateLimiter {
    Arc::new(DirectChatRateLimiter::with_db(db, slot_manager))
}

/// Per-agent rate limiter keyed by API key ID.
/// Each agent gets its own token bucket keyed by API key ID.
/// `rate_limit_per_second` controls refill rate and `burst` controls capacity.
#[derive(Clone)]
pub struct PerAgentRateLimiter {
    db: Option<Database>,
    namespace: String,
    #[cfg(test)]
    state: Arc<Mutex<HashMap<String, AgentBucket>>>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct AgentBucket {
    tokens: f64,
    last_refill: Instant,
}

impl PerAgentRateLimiter {
    #[cfg(test)]
    pub fn new() -> Self {
        Self {
            db: None,
            namespace: "test_agent".to_string(),
            #[cfg(test)]
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_db(db: Database, namespace: impl Into<String>) -> Self {
        Self {
            db: Some(db),
            namespace: namespace.into(),
            #[cfg(test)]
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn check_shared(
        &self,
        agent_id: &str,
        rate_per_second: u32,
        burst_capacity: u32,
    ) -> Result<bool, AppError> {
        if let Some(db) = self.db.as_ref() {
            return Ok(TokenBucketStore::admit(
                db,
                &self.namespace,
                agent_id,
                rate_per_second,
                burst_capacity,
            )
            .await?
            .allowed);
        }
        #[cfg(test)]
        return Ok(self.check(agent_id, rate_per_second, burst_capacity));
        #[cfg(not(test))]
        unreachable!("production rate limiters always have a MongoDB backend")
    }

    /// Check if a request from the given agent should be allowed.
    /// Returns true if allowed, false if rate limited.
    #[cfg(test)]
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
    #[cfg(test)]
    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.retain(|_, bucket| now.duration_since(bucket.last_refill).as_secs() < 120);
    }
}

pub type SharedPerAgentRateLimiter = Arc<PerAgentRateLimiter>;

#[derive(Clone, Copy, Debug)]
pub struct PlatformUserRateLimitPolicy {
    pub per_second: u32,
    pub burst: u32,
}

impl PlatformUserRateLimitPolicy {
    pub const fn new(per_second: u32, burst: u32) -> Self {
        Self { per_second, burst }
    }

    pub const fn disabled() -> Self {
        Self::new(0, 0)
    }
}

/// Per-(platform service, user) token bucket guarding shared master
/// credentials. The keyed bucket is deliberately separate from API-key
/// limiting so browser-session callers receive the same protection.
#[cfg(test)]
pub struct PlatformUserRateLimiter {
    inner: PerAgentRateLimiter,
    per_second: u32,
    burst: u32,
}

#[cfg(test)]
impl PlatformUserRateLimiter {
    pub fn new(per_second: u32, burst: u32) -> Self {
        Self {
            inner: PerAgentRateLimiter::new(),
            per_second,
            burst,
        }
    }

    /// Check a request for the given platform service and user.
    /// Returns true if allowed, false if rate limited.
    pub async fn check_shared(&self, service_id: &str, user_id: &str) -> Result<bool, AppError> {
        self.inner
            .check_shared(
                &format!("{service_id}:{user_id}"),
                self.per_second,
                self.burst,
            )
            .await
    }

    pub fn check(&self, service_id: &str, user_id: &str) -> bool {
        self.inner.check(
            &format!("{service_id}:{user_id}"),
            self.per_second,
            self.burst,
        )
    }
}

/// Enforce a platform per-user limit through an explicit limiter seam.
#[cfg(test)]
pub async fn enforce_platform_user_limit_with_limiter(
    limiter: Option<&PlatformUserRateLimiter>,
    service_id: &str,
    user_id: &str,
) -> Result<(), crate::errors::AppError> {
    if let Some(limiter) = limiter
        && !limiter.check_shared(service_id, user_id).await?
    {
        tracing::warn!(service_id, "Platform per-user rate limit exceeded");
        return Err(crate::errors::AppError::RateLimited);
    }
    Ok(())
}

pub async fn enforce_platform_user_limit(
    db: &Database,
    policy: PlatformUserRateLimitPolicy,
    service_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    if policy.per_second == 0 {
        return Ok(());
    }
    let key = format!("{service_id}:{user_id}");
    if !TokenBucketStore::admit(db, "platform_user", &key, policy.per_second, policy.burst)
        .await?
        .allowed
    {
        tracing::warn!(service_id, "Platform per-user rate limit exceeded");
        return Err(AppError::RateLimited);
    }
    Ok(())
}

/// Per-device-code rate limiter keyed by Ed25519 public key bytes.
///
/// The device authorization endpoints need a much stricter bucket than the
/// general API limiter because both user-code approval and poll verification
/// are security-sensitive. This bucket is intentionally keyed by the factory
/// public key rather than `device_code`, so leaking an opaque device code does
/// not give an attacker a fresh rate-limit identity.
#[derive(Clone)]
pub struct PerPubkeyRateLimiter {
    db: Option<Database>,
    namespace: String,
    #[cfg(test)]
    state: Arc<Mutex<HashMap<[u8; 32], AgentBucket>>>,
    #[cfg(test)]
    tokens_per_second: f64,
    burst: u32,
}

impl PerPubkeyRateLimiter {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::new_with_rate(5.0 / 60.0, 5)
    }

    #[cfg(test)]
    fn new_with_rate(tokens_per_second: f64, burst: u32) -> Self {
        Self {
            db: None,
            namespace: "test_pubkey".to_string(),
            state: Arc::new(Mutex::new(HashMap::new())),
            tokens_per_second,
            burst,
        }
    }

    pub fn with_db(db: Database, namespace: impl Into<String>) -> Self {
        Self {
            db: Some(db),
            namespace: namespace.into(),
            #[cfg(test)]
            state: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(test)]
            tokens_per_second: 5.0 / 60.0,
            burst: 5,
        }
    }

    pub async fn check_shared(&self, pubkey: &[u8; 32]) -> Result<bool, AppError> {
        if let Some(db) = self.db.as_ref() {
            return Ok(TokenBucketStore::admit_over_period(
                db,
                &self.namespace,
                &hex::encode(pubkey),
                5,
                std::time::Duration::from_secs(60),
                self.burst,
            )
            .await?
            .allowed);
        }
        #[cfg(test)]
        return Ok(self.check(pubkey));
        #[cfg(not(test))]
        unreachable!("production rate limiters always have a MongoDB backend")
    }

    #[cfg(test)]
    pub fn check(&self, pubkey: &[u8; 32]) -> bool {
        self.check_at(pubkey, Instant::now())
    }

    /// Check if a request from the given device public key should be allowed.
    /// Returns true if allowed, false if rate limited.
    #[cfg(test)]
    fn check_at(&self, pubkey: &[u8; 32], now: Instant) -> bool {
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
    #[cfg(test)]
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
    pub trusted_proxies: Arc<Vec<TrustedProxyRange>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientIpAttribution {
    Verified,
    Unverified,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedClientIp {
    pub ip: IpAddr,
    pub attribution: ClientIpAttribution,
}

/// Per-message edit limiter keyed by upstream platform message ID.
/// Used by the channel relay edit endpoint so progressive updates on one
/// message cannot starve the rest of the relay.
#[derive(Clone)]
pub struct PerMessageEditRateLimiter {
    db: Option<Database>,
    namespace: String,
    #[cfg(test)]
    state: Arc<Mutex<HashMap<String, AgentBucket>>>,
    rate_per_second: u32,
    burst: u32,
}

impl PerMessageEditRateLimiter {
    #[cfg(test)]
    pub fn new(rate_per_second: u32, burst: u32) -> Self {
        Self {
            db: None,
            namespace: "test_message_edit".to_string(),
            #[cfg(test)]
            state: Arc::new(Mutex::new(HashMap::new())),
            rate_per_second,
            burst,
        }
    }

    pub fn with_db(
        db: Database,
        namespace: impl Into<String>,
        rate_per_second: u32,
        burst: u32,
    ) -> Self {
        Self {
            db: Some(db),
            namespace: namespace.into(),
            #[cfg(test)]
            state: Arc::new(Mutex::new(HashMap::new())),
            rate_per_second,
            burst,
        }
    }

    pub async fn check_shared(&self, platform_message_id: &str) -> Result<bool, AppError> {
        if let Some(db) = self.db.as_ref() {
            return Ok(TokenBucketStore::admit(
                db,
                &self.namespace,
                platform_message_id,
                self.rate_per_second,
                self.burst,
            )
            .await?
            .allowed);
        }
        #[cfg(test)]
        return Ok(self.check(platform_message_id));
        #[cfg(not(test))]
        unreachable!("production rate limiters always have a MongoDB backend")
    }

    /// Check if an edit for the given upstream message should be allowed.
    #[cfg(test)]
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
    #[cfg(test)]
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
    db: Option<Database>,
    namespace: String,
    #[cfg(test)]
    state: Arc<Mutex<HashMap<String, AgentBucket>>>,
    rate_per_second: u32,
    burst: u32,
}

impl PerChannelEventLimiter {
    #[cfg(test)]
    pub fn new(rate_per_second: u32, burst: u32) -> Self {
        Self {
            db: None,
            namespace: "test_channel".to_string(),
            #[cfg(test)]
            state: Arc::new(Mutex::new(HashMap::new())),
            rate_per_second,
            burst,
        }
    }

    pub fn with_db(
        db: Database,
        namespace: impl Into<String>,
        rate_per_second: u32,
        burst: u32,
    ) -> Self {
        Self {
            db: Some(db),
            namespace: namespace.into(),
            #[cfg(test)]
            state: Arc::new(Mutex::new(HashMap::new())),
            rate_per_second,
            burst,
        }
    }

    pub async fn check_shared(&self, conversation_id: &str) -> Result<bool, AppError> {
        if let Some(db) = self.db.as_ref() {
            return Ok(TokenBucketStore::admit(
                db,
                &self.namespace,
                conversation_id,
                self.rate_per_second,
                self.burst,
            )
            .await?
            .allowed);
        }
        #[cfg(test)]
        return Ok(self.check(conversation_id));
        #[cfg(not(test))]
        unreachable!("production rate limiters always have a MongoDB backend")
    }

    /// Check if an event for the given conversation should be allowed.
    /// Returns `true` if allowed, `false` if rate-limited.
    #[cfg(test)]
    pub fn check(&self, conversation_id: &str) -> bool {
        self.check_at(conversation_id, Instant::now())
    }

    #[cfg(test)]
    fn check_at(&self, conversation_id: &str, now: Instant) -> bool {
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
    #[cfg(test)]
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
pub async fn check_agent_rate_limit(
    limiter: &PerAgentRateLimiter,
    auth_user: &crate::mw::auth::AuthUser,
) -> Result<(), crate::errors::AppError> {
    check_agent_rate_limit_raw(
        limiter,
        auth_user.api_key_id.as_deref(),
        auth_user.rate_limit_per_second,
        auth_user.rate_limit_burst,
    )
    .await
}

/// Check per-agent rate limit using raw API-key identity and limit fields.
/// Used by callers that don't hold an `AuthUser` (e.g. the MCP transport).
pub async fn check_agent_rate_limit_raw(
    limiter: &PerAgentRateLimiter,
    api_key_id: Option<&str>,
    rate_limit_per_second: Option<u32>,
    rate_limit_burst: Option<u32>,
) -> Result<(), crate::errors::AppError> {
    if let (Some(agent_id), Some(rps)) = (api_key_id, rate_limit_per_second) {
        // When no explicit burst is set, use the sustained rate as the ceiling.
        // Users who want a higher burst can set rate_limit_burst explicitly.
        let burst = rate_limit_burst.unwrap_or(rps);
        if !limiter.check_shared(agent_id, rps, burst).await? {
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
pub fn create_rate_limiter(db: Database, per_second: u64, burst: u32) -> SharedRateLimiter {
    Arc::new(GlobalRateLimiter {
        db: Some(db),
        per_second: u32::try_from(per_second).unwrap_or(u32::MAX),
        burst,
        #[cfg(test)]
        local: Arc::new(Mutex::new(AgentBucket {
            tokens: f64::from(burst),
            last_refill: Instant::now(),
        })),
    })
}

/// Create a per-IP rate limiter.
pub fn create_per_ip_rate_limiter(
    db: Database,
    namespace: impl Into<String>,
    max_requests: u32,
    window_secs: u64,
) -> SharedPerIpRateLimiter {
    Arc::new(PerIpRateLimiter::with_db(
        db,
        namespace,
        max_requests,
        window_secs,
    ))
}

/// Create a per-string-key rate limiter.
pub fn create_per_key_rate_limiter(
    db: Database,
    namespace: impl Into<String>,
    max_requests: u32,
    window_secs: u64,
) -> SharedPerKeyRateLimiter {
    Arc::new(PerKeyRateLimiter::with_db(
        db,
        namespace,
        max_requests,
        window_secs,
    ))
}

/// Create a per-pubkey limiter for device authorization endpoints.
pub fn create_per_pubkey_rate_limiter(
    db: Database,
    namespace: impl Into<String>,
) -> SharedPerPubkeyRateLimiter {
    Arc::new(PerPubkeyRateLimiter::with_db(db, namespace))
}

pub async fn enforce_public_ip_rate_limit(
    limiter: &SharedPerIpRateLimiter,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    trusted_proxies: &[TrustedProxyRange],
    path: &str,
) -> Result<Option<IpAddr>, AppError> {
    let Some(client_ip) = resolve_client_ip_for_rate_limit(headers, peer, trusted_proxies) else {
        tracing::warn!(
            path = %path,
            "Skipping public per-IP throttle: client IP unresolved; per-IP rate limiting is disabled for this request (daily quota remains the backstop)"
        );
        return Ok(None);
    };

    if !limiter.check_shared(client_ip).await? {
        tracing::warn!(
            path = %path,
            ip = %client_ip,
            "Public per-IP rate limit exceeded"
        );
        return Err(AppError::RateLimited);
    }

    Ok(Some(client_ip))
}

/// Resolve a client IP using only forwarded headers authenticated by a
/// configured trusted-proxy allowlist.
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
pub fn resolve_client_ip_for_rate_limit(
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    trusted_proxies: &[TrustedProxyRange],
) -> Option<IpAddr> {
    resolve_client_ip(headers, peer, trusted_proxies).map(|resolved| resolved.ip)
}

pub fn resolve_client_ip(
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    trusted_proxies: &[TrustedProxyRange],
) -> Option<ResolvedClientIp> {
    let peer_ip = peer.map(|peer| normalize_ip_address(peer.ip()));

    warn_if_proxy_attribution_is_collapsed(peer_ip, trusted_proxies);

    let peer_is_trusted = peer_ip.is_some_and(|ip| is_trusted_proxy(ip, trusted_proxies));

    if peer_is_trusted {
        if let Some(ip) = header_ip(headers, "cf-connecting-ip") {
            return Some(attributed_forwarded_ip(ip));
        }

        if let Some(ip) = rightmost_untrusted_xff(headers, trusted_proxies) {
            return Some(attributed_forwarded_ip(ip));
        }

        if let Some(ip) = header_ip(headers, "x-real-ip") {
            return Some(attributed_forwarded_ip(ip));
        }
    }

    peer_ip.map(|ip| ResolvedClientIp {
        ip,
        attribution: classify_unverified_ip(ip),
    })
}

/// Resolve rate-limit and audit IPs with a compatibility fallback for the
/// two legacy surfaces that historically trusted forwarded headers.
///
/// When no proxy ranges are configured this preserves the old XFF-first
/// behavior. As soon as the allowlist is non-empty it uses the strict trusted
/// resolver above, so forwarded headers from an untrusted peer are ignored.
pub fn resolve_client_ip_with_legacy_fallback(
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    trusted_proxies: &[TrustedProxyRange],
) -> Option<ResolvedClientIp> {
    if trusted_proxies.is_empty() {
        let peer_ip = peer.map(|address| normalize_ip_address(address.ip()));
        warn_if_proxy_attribution_is_collapsed(peer_ip, trusted_proxies);
        let ip = leftmost_xff(headers)
            .or_else(|| header_ip(headers, "x-real-ip"))
            .or(peer_ip)?;
        return Some(ResolvedClientIp {
            ip,
            attribution: classify_unverified_ip(ip),
        });
    }

    resolve_client_ip(headers, peer, trusted_proxies)
}

pub fn is_trusted_proxy(ip: IpAddr, trusted_proxies: &[TrustedProxyRange]) -> bool {
    let ip = normalize_ip_address(ip);
    trusted_proxies.iter().any(|range| range.contains(ip))
}

pub fn is_global_unicast(ip: IpAddr) -> bool {
    let ip = normalize_ip_address(ip);
    match ip {
        IpAddr::V4(address) => {
            let [a, b, c, _] = address.octets();
            !(a == 0
                || a == 10
                || a == 127
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 0 && c == 2)
                || (a == 192 && b == 88 && c == 99)
                || (a == 192 && b == 168)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113)
                || a >= 224)
        }
        IpAddr::V6(address) => {
            let segments = address.segments();
            // Current global-unicast allocation is 2000::/3. Documentation
            // addresses remain non-evidence even though they sit in that range.
            (segments[0] & 0xe000) == 0x2000 && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        }
    }
}

fn attributed_forwarded_ip(ip: IpAddr) -> ResolvedClientIp {
    let ip = normalize_ip_address(ip);
    ResolvedClientIp {
        ip,
        attribution: if is_global_unicast(ip) {
            ClientIpAttribution::Verified
        } else {
            ClientIpAttribution::Unavailable
        },
    }
}

fn classify_unverified_ip(ip: IpAddr) -> ClientIpAttribution {
    if is_global_unicast(ip) {
        ClientIpAttribution::Unverified
    } else {
        ClientIpAttribution::Unavailable
    }
}

fn header_ip(headers: &HeaderMap, name: &'static str) -> Option<IpAddr> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.contains(','))
        .and_then(|value| value.parse::<IpAddr>().ok())
        .map(normalize_ip_address)
}

fn leftmost_xff(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<IpAddr>().ok())
        .map(normalize_ip_address)
}

fn rightmost_untrusted_xff(
    headers: &HeaderMap,
    trusted_proxies: &[TrustedProxyRange],
) -> Option<IpAddr> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())?
        .split(',')
        .rev()
        .filter_map(|value| {
            value
                .trim()
                .parse::<IpAddr>()
                .ok()
                .map(normalize_ip_address)
        })
        .find(|ip| !is_trusted_proxy(*ip, trusted_proxies))
}

pub(crate) fn warn_if_proxy_attribution_is_collapsed(
    peer_ip: Option<IpAddr>,
    trusted_proxies: &[TrustedProxyRange],
) {
    static WARNED: OnceLock<()> = OnceLock::new();
    if trusted_proxies.is_empty() && peer_ip.is_some_and(|ip| !is_global_unicast(ip)) {
        WARNED.get_or_init(|| {
            tracing::warn!(
                "Request peer is private or loopback while TRUSTED_PROXY_IPS is empty; client IP attribution and per-IP rate limiting may be collapsing to the proxy address. Configure TRUSTED_PROXY_IPS with only trusted proxy addresses or CIDR ranges."
            );
        });
    }
}

/// Extract the global rate-limit key. An empty proxy allowlist deliberately
/// preserves the historical XFF/X-Real-IP/loopback order for deployment
/// compatibility. Configuring any trusted range switches this path to the
/// spoof-resistant resolver and its TCP-peer trust gate.
fn extract_client_ip(request: &Request<Body>, trusted_proxies: &[TrustedProxyRange]) -> IpAddr {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(address)| *address);
    if trusted_proxies.is_empty() {
        warn_if_proxy_attribution_is_collapsed(peer.map(|address| address.ip()), trusted_proxies);
        return leftmost_xff(request.headers())
            .or_else(|| header_ip(request.headers(), "x-real-ip"))
            .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    }

    resolve_client_ip(request.headers(), peer, trusted_proxies)
        .map(|resolved| resolved.ip)
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
}

/// Axum middleware that enforces per-IP rate limiting with global fallback.
///
/// Expects both `SharedPerIpRateLimiter` and `SharedRateLimiter` as layer Extensions.
/// Returns 429 Too Many Requests when the limit is exceeded.
/// Paths exempt from rate limiting (authenticated via other means).
const RATE_LIMIT_EXEMPT_PATHS: &[&str] = &["/mcp", "/.well-known/", "/health"];
const ASSISTANT_ACTIONS_EXEMPT_PATH: &str = "/api/v1/assistant/actions";

fn is_rate_limit_exempt(path: &str) -> bool {
    path == ASSISTANT_ACTIONS_EXEMPT_PATH
        || RATE_LIMIT_EXEMPT_PATHS
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

pub async fn rate_limit_middleware(
    Extension(per_ip_limiter): Extension<SharedPerIpRateLimiter>,
    Extension(global_limiter): Extension<SharedRateLimiter>,
    Extension(trusted_proxies): Extension<Arc<Vec<TrustedProxyRange>>>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let path = request.uri().path();

    // Skip rate limiting for exempt paths (MCP has its own auth + session management)
    if is_rate_limit_exempt(path) {
        return Ok(next.run(request).await);
    }

    let client_ip = extract_client_ip(&request, trusted_proxies.as_slice());

    // Check per-IP rate limit first
    if !per_ip_limiter.check_shared(client_ip).await? {
        tracing::warn!(
            path = %path,
            ip = %client_ip,
            "Per-IP rate limit exceeded"
        );
        return Err(AppError::RateLimited);
    }

    // Also check global rate limit as a safety net
    if !global_limiter.check_shared().await? {
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
    enforce_device_code_ip_rate_limit(&limiters, request.headers(), peer, request.uri().path())
        .await?;

    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, 64 * 1024)
        .await
        .map_err(|_| AppError::BadRequest("Unable to read request body".to_string()))?;

    if let Some(pubkey) = extract_device_pubkey_for_rate_limit(limiters.db.as_ref(), &bytes).await
        && !limiters.per_pubkey.check_shared(&pubkey).await?
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

async fn enforce_device_code_ip_rate_limit(
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

    if !limiters.per_ip.check_shared(client_ip).await? {
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
    use axum::{Json, Router, http::StatusCode, middleware, routing::post};
    use mongodb::bson::doc;
    use serde::{Deserialize, Serialize};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::sync::Arc;
    use tower::ServiceExt;

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
        let _limiter = GlobalRateLimiter::new_local(10, 30);
    }

    #[test]
    fn create_per_ip_rate_limiter_does_not_panic() {
        let _limiter = PerIpRateLimiter::new(30, 1);
    }

    #[tokio::test]
    async fn direct_chat_request_window_is_per_user() {
        let limiter = Arc::new(DirectChatRateLimiter::new(2, 60, 2));
        drop(limiter.try_acquire("user-a").await.unwrap());
        drop(limiter.try_acquire("user-a").await.unwrap());
        assert!(matches!(
            limiter.try_acquire("user-a").await,
            Err(AppError::RateLimited)
        ));
        assert!(limiter.try_acquire("user-b").await.is_ok());
    }

    #[test]
    fn extract_client_ip_x_forwarded_for() {
        let req = Request::builder()
            .header("x-forwarded-for", "203.0.113.50, 70.41.3.18")
            .body(Body::empty())
            .unwrap();
        let ip = extract_client_ip(&req, &[]);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 50)));
    }

    #[test]
    fn extract_client_ip_x_real_ip() {
        let req = Request::builder()
            .header("x-real-ip", "198.51.100.22")
            .body(Body::empty())
            .unwrap();
        let ip = extract_client_ip(&req, &[]);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(198, 51, 100, 22)));
    }

    #[test]
    fn extract_client_ip_fallback_to_localhost() {
        let mut req = Request::builder().body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(10, 2, 10, 22)),
            443,
        )));
        let ip = extract_client_ip(&req, &[]);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn extract_client_ip_invalid_header_falls_through() {
        let req = Request::builder()
            .header("x-forwarded-for", "not-an-ip")
            .body(Body::empty())
            .unwrap();
        let ip = extract_client_ip(&req, &[]);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn extract_client_ip_prefers_forwarded_for_over_real_ip() {
        let req = Request::builder()
            .header("x-forwarded-for", "1.2.3.4")
            .header("x-real-ip", "5.6.7.8")
            .body(Body::empty())
            .unwrap();
        let ip = extract_client_ip(&req, &[]);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)));
    }

    #[test]
    fn extract_client_ip_switches_to_spoof_resistant_resolution_when_configured() {
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 2, 10, 22)), 443);
        let mut req = Request::builder()
            .header("x-forwarded-for", "1.2.3.4, 8.8.8.8, 10.2.10.22")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(peer));

        let ip = extract_client_ip(&req, &["10.0.0.0/8".parse().unwrap()]);

        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));
    }

    #[test]
    fn assistant_actions_has_exact_path_rate_limit_exemption() {
        assert!(is_rate_limit_exempt("/health"));
        assert!(is_rate_limit_exempt("/.well-known/openid-configuration"));
        assert!(is_rate_limit_exempt("/mcp"));
        assert!(is_rate_limit_exempt("/api/v1/assistant/actions"));
        assert!(!is_rate_limit_exempt("/api/v1/assistant/actions/extra"));
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
    fn platform_user_buckets_are_isolated() {
        let limiter = PlatformUserRateLimiter::new(1, 2);
        assert!(limiter.check("S", "A"));
        assert!(limiter.check("S", "A"));
        assert!(!limiter.check("S", "A"));
        assert!(limiter.check("S", "B"));
        assert!(limiter.check("T", "A"));
    }

    #[tokio::test]
    async fn enforce_platform_user_limit_maps_to_rate_limited() {
        let limiter = PlatformUserRateLimiter::new(1, 1);
        assert!(
            enforce_platform_user_limit_with_limiter(Some(&limiter), "S", "A")
                .await
                .is_ok()
        );
        assert!(matches!(
            enforce_platform_user_limit_with_limiter(Some(&limiter), "S", "A").await,
            Err(crate::errors::AppError::RateLimited)
        ));
        assert!(
            enforce_platform_user_limit_with_limiter(None, "S", "A")
                .await
                .is_ok()
        );
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
        let start = Instant::now();

        assert!(limiter.check_at(&pubkey, start));
        assert!(!limiter.check_at(&pubkey, start));
        assert!(limiter.check_at(&pubkey, start + std::time::Duration::from_millis(30)));
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
        let limiter = PerChannelEventLimiter::new(100, 1);
        let start = Instant::now();

        assert!(limiter.check_at("conv", start));
        assert!(!limiter.check_at("conv", start));
        assert!(limiter.check_at("conv", start + std::time::Duration::from_millis(30)));
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

    fn trusted(value: &str) -> TrustedProxyRange {
        value.parse().unwrap()
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
        let client_ip = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            format!("{client_ip}, 10.0.0.1").parse().unwrap(),
        );
        let ranges = [trusted("10.0.0.0/8")];
        let resolved = resolve_client_ip(&headers, Some(socket(proxy_ip)), &ranges);
        assert_eq!(resolved.map(|value| value.ip), Some(client_ip));
        assert_eq!(
            resolved.map(|value| value.attribution),
            Some(ClientIpAttribution::Verified)
        );
    }

    #[test]
    fn resolve_client_ip_prefers_cloudflare_connecting_ip() {
        let proxy_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", "8.8.4.4".parse().unwrap());
        headers.insert("x-forwarded-for", "1.2.3.4, 9.9.9.9".parse().unwrap());
        headers.insert("x-real-ip", "4.4.4.4".parse().unwrap());
        let ranges = [trusted("10.0.0.0/8")];

        let resolved = resolve_client_ip(&headers, Some(socket(proxy_ip)), &ranges).unwrap();
        assert_eq!(resolved.ip, "8.8.4.4".parse::<IpAddr>().unwrap());
        assert_eq!(resolved.attribution, ClientIpAttribution::Verified);
    }

    #[test]
    fn resolve_client_ip_trusts_an_ipv4_mapped_proxy_peer() {
        let proxy_ip = "::ffff:10.2.10.22".parse::<IpAddr>().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", "8.8.8.8".parse().unwrap());
        let ranges = [trusted("10.0.0.0/8")];

        let resolved = resolve_client_ip(&headers, Some(socket(proxy_ip)), &ranges).unwrap();
        assert_eq!(resolved.ip, "8.8.8.8".parse::<IpAddr>().unwrap());
        assert_eq!(resolved.attribution, ClientIpAttribution::Verified);
    }

    #[test]
    fn resolve_client_ip_normalizes_a_public_ipv4_mapped_header() {
        let proxy_ip = "10.2.10.22".parse::<IpAddr>().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", "::ffff:8.8.8.8".parse().unwrap());
        let ranges = [trusted("10.0.0.0/8")];

        let resolved = resolve_client_ip(&headers, Some(socket(proxy_ip)), &ranges).unwrap();
        assert_eq!(resolved.ip, "8.8.8.8".parse::<IpAddr>().unwrap());
        assert_eq!(resolved.attribution, ClientIpAttribution::Verified);
        assert!(is_global_unicast("::ffff:8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn resolve_client_ip_honors_x_real_ip_fallback_when_trusted() {
        let proxy_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        let client_ip = IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9));
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", client_ip.to_string().parse().unwrap());
        let resolved = resolve_client_ip_for_rate_limit(
            &headers,
            Some(socket(proxy_ip)),
            &[trusted("10.0.0.0/8")],
        );
        assert_eq!(resolved, Some(client_ip));
    }

    #[test]
    fn resolve_client_ip_ignores_xff_when_peer_not_in_allowlist() {
        let peer_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 99));
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", "8.8.8.8".parse().unwrap());
        headers.insert("x-forwarded-for", "198.51.100.55".parse().unwrap());
        let resolved = resolve_client_ip_for_rate_limit(
            &headers,
            Some(socket(peer_ip)),
            &[trusted("10.0.0.0/8")],
        );
        assert_eq!(resolved, Some(peer_ip));
    }

    #[test]
    fn resolve_client_ip_drops_malformed_xff_entry_and_uses_peer() {
        let proxy_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 4));
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "not-an-ip".parse().unwrap());
        let resolved = resolve_client_ip_for_rate_limit(
            &headers,
            Some(socket(proxy_ip)),
            &[trusted("10.0.0.0/8")],
        );
        assert_eq!(resolved, Some(proxy_ip));
    }

    #[test]
    fn resolve_client_ip_takes_rightmost_untrusted_xff_entry() {
        let proxy_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "1.2.3.4, 8.8.8.8, 10.0.0.5".parse().unwrap(),
        );
        let resolved = resolve_client_ip_for_rate_limit(
            &headers,
            Some(socket(proxy_ip)),
            &[trusted("10.0.0.0/8")],
        );
        assert_eq!(resolved, Some(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[test]
    fn resolve_client_ip_skips_ipv4_mapped_trusted_xff_hops() {
        let proxy_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "1.2.3.4, 8.8.8.8, ::ffff:10.0.0.5".parse().unwrap(),
        );
        let resolved = resolve_client_ip_for_rate_limit(
            &headers,
            Some(socket(proxy_ip)),
            &[trusted("10.0.0.0/8")],
        );
        assert_eq!(resolved, Some(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[test]
    fn xff_cloudflare_fallback_requires_every_proxy_hop_to_be_trusted() {
        let proxy_ip = IpAddr::V4(Ipv4Addr::new(10, 2, 10, 22));
        let cloudflare_edge = "104.16.10.20".parse::<IpAddr>().unwrap();
        let real_client = "8.8.8.8".parse::<IpAddr>().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "1.2.3.4, 8.8.8.8, 104.16.10.20".parse().unwrap(),
        );

        let incomplete =
            resolve_client_ip(&headers, Some(socket(proxy_ip)), &[trusted("10.0.0.0/8")]).unwrap();
        assert_eq!(incomplete.ip, cloudflare_edge);
        assert_eq!(incomplete.attribution, ClientIpAttribution::Verified);

        let complete = resolve_client_ip(
            &headers,
            Some(socket(proxy_ip)),
            &[trusted("10.0.0.0/8"), trusted("104.16.0.0/13")],
        )
        .unwrap();
        assert_eq!(complete.ip, real_client);
        assert_eq!(complete.attribution, ClientIpAttribution::Verified);
    }

    #[test]
    fn legacy_fallback_preserves_leftmost_xff_until_proxy_trust_is_configured() {
        let peer_ip = IpAddr::V4(Ipv4Addr::new(10, 2, 10, 22));
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4, 8.8.8.8".parse().unwrap());

        let resolved = resolve_client_ip_with_legacy_fallback(&headers, Some(socket(peer_ip)), &[]);
        assert_eq!(
            resolved.map(|value| value.ip),
            Some("1.2.3.4".parse().unwrap())
        );
        assert_eq!(
            resolved.map(|value| value.attribution),
            Some(ClientIpAttribution::Unverified)
        );
    }

    #[test]
    fn private_and_special_addresses_are_unavailable() {
        for value in [
            "127.0.0.1",
            "10.2.10.22",
            "169.254.1.2",
            "100.64.0.1",
            "::1",
            "fe80::1",
            "fd00::1",
        ] {
            let ip = value.parse::<IpAddr>().unwrap();
            assert!(!is_global_unicast(ip), "{value}");
            assert_eq!(classify_unverified_ip(ip), ClientIpAttribution::Unavailable);
        }
        assert!(is_global_unicast("8.8.8.8".parse().unwrap()));
        assert!(is_global_unicast("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn resolve_client_ip_handles_missing_peer() {
        // No peer means we can't make a trust decision. XFF must
        // still be ignored — returning `None` lets the caller decide
        // how to handle the ambiguity (typically: skip the per-IP
        // bucket entirely).
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.4".parse().unwrap());
        let resolved = resolve_client_ip_for_rate_limit(&headers, None, &[trusted("10.0.0.1")]);
        assert!(resolved.is_none());
    }

    #[tokio::test]
    async fn device_code_ip_limiter_skips_ip_bucket_without_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.4".parse().unwrap());
        let limiters = DeviceCodeRateLimiters {
            per_ip: Arc::new(PerIpRateLimiter::new(0, 60)),
            per_pubkey: Arc::new(PerPubkeyRateLimiter::new()),
            db: None,
            trusted_proxies: Arc::new(vec![trusted("10.0.0.1")]),
        };

        let result = enforce_device_code_ip_rate_limit(
            &limiters,
            &headers,
            None,
            "/api/v1/devices/code/poll",
        )
        .await
        .expect("missing peer should skip IP bucket");

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn device_code_ip_limiter_honors_xff_only_from_trusted_proxy() {
        let proxy_ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let client_ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 4));
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", client_ip.to_string().parse().unwrap());
        let limiters = DeviceCodeRateLimiters {
            per_ip: Arc::new(PerIpRateLimiter::new(1, 60)),
            per_pubkey: Arc::new(PerPubkeyRateLimiter::new()),
            db: None,
            trusted_proxies: Arc::new(vec![trusted("10.0.0.0/8")]),
        };

        let first = enforce_device_code_ip_rate_limit(
            &limiters,
            &headers,
            Some(socket(proxy_ip)),
            "/api/v1/devices/code/request",
        )
        .await
        .expect("first request allowed");
        let second = enforce_device_code_ip_rate_limit(
            &limiters,
            &headers,
            Some(socket(proxy_ip)),
            "/api/v1/devices/code/request",
        )
        .await
        .expect_err("second request should be rate-limited by forwarded client IP");

        assert_eq!(first, Some(client_ip));
        assert!(matches!(second, AppError::DeviceCodeRateLimited));
    }

    #[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
    struct DeviceCodeEchoBody {
        device_code: String,
        hw_id: String,
        metadata: serde_json::Value,
    }

    async fn echo_device_code_body(
        Json(body): Json<DeviceCodeEchoBody>,
    ) -> Json<DeviceCodeEchoBody> {
        Json(body)
    }

    fn device_code_test_router(limiters: DeviceCodeRateLimiters) -> Router {
        Router::new()
            .route("/api/v1/devices/code/poll", post(echo_device_code_body))
            .layer(middleware::from_fn_with_state(
                limiters,
                device_code_rate_limit_middleware,
            ))
    }

    fn request_with_json_body(path: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&body).expect("serialize body"),
            ))
            .expect("build request")
    }

    async fn json_response(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("read response body");
        serde_json::from_slice(&bytes).expect("json response")
    }

    #[tokio::test]
    async fn device_code_middleware_rebuilds_json_body_for_downstream_handler() {
        let limiters = DeviceCodeRateLimiters {
            per_ip: Arc::new(PerIpRateLimiter::new(100, 60)),
            per_pubkey: Arc::new(PerPubkeyRateLimiter::new()),
            db: None,
            trusted_proxies: Arc::new(vec![]),
        };
        let app = device_code_test_router(limiters);
        let expected = DeviceCodeEchoBody {
            device_code: "nyx_dc_body_rebuild_test".to_string(),
            hw_id: "esp32-p4-cam-1".to_string(),
            metadata: serde_json::json!({
                "nested": { "answer": 42 },
                "list": ["alpha", "beta"],
            }),
        };
        let request_body = serde_json::to_value(&expected).expect("body value");

        let response = app
            .oneshot(request_with_json_body(
                "/api/v1/devices/code/poll",
                request_body,
            ))
            .await
            .expect("middleware response");

        assert_eq!(response.status(), StatusCode::OK);
        let echoed: DeviceCodeEchoBody =
            serde_json::from_value(json_response(response).await).expect("echo body");
        assert_eq!(echoed, expected);
    }

    #[tokio::test]
    async fn device_code_middleware_rate_limits_db_derived_pubkey_when_body_omits_pubkey() {
        let Some((db, device_code, signing_key)) =
            crate::services::device_code_service::tests_support::setup_pending_row(
                "rate_limit_db_pubkey",
            )
            .await
        else {
            return;
        };
        let stored = db
            .collection::<DeviceCode>(DEVICE_CODES)
            .find_one(doc! {
                "device_code_hash": crate::crypto::token::hash_token(&device_code.device_code),
            })
            .await
            .expect("query seeded device code")
            .expect("seeded device code exists");
        assert_eq!(
            stored.device_pubkey,
            signing_key.verifying_key().to_bytes().to_vec()
        );

        let limiters = DeviceCodeRateLimiters {
            per_ip: Arc::new(PerIpRateLimiter::new(100, 60)),
            per_pubkey: Arc::new(PerPubkeyRateLimiter::new_with_rate(0.0, 1)),
            db: Some(db),
            trusted_proxies: Arc::new(vec![]),
        };
        let app = device_code_test_router(limiters);
        let request_body = serde_json::json!({
            "device_code": device_code.device_code,
            "hw_id": "esp32-p4-cam-1",
            "metadata": { "source": "db-derived-pubkey" },
        });

        let first_response = app
            .clone()
            .oneshot(request_with_json_body(
                "/api/v1/devices/code/poll",
                request_body.clone(),
            ))
            .await
            .expect("first middleware response");
        assert_eq!(first_response.status(), StatusCode::OK);
        assert_eq!(json_response(first_response).await, request_body);

        let second_response = app
            .oneshot(request_with_json_body(
                "/api/v1/devices/code/poll",
                request_body,
            ))
            .await
            .expect("second middleware response");
        assert_eq!(second_response.status(), StatusCode::TOO_MANY_REQUESTS);
        let error = json_response(second_response).await;
        assert_eq!(error["error"], "device_code_rate_limited");
        assert_eq!(error["error_code"], 9506);
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
