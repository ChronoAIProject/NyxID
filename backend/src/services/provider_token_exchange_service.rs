//! Generic server-side token exchange with single-flight caching.
//!
//! Many downstream providers require a one-shot credential-to-token handshake
//! before their APIs can be called: Lark/Feishu tenant tokens, OAuth 2.0
//! client_credentials flows, Slack app-level tokens, and so on. The shape
//! is always the same -- POST secrets to an endpoint, parse the token out
//! of the response, cache it until near expiry, then inject it on every
//! outbound request.
//!
//! This module provides the infrastructure for that flow as pure data.
//! Per-provider behaviour lives entirely in [`TokenExchangeConfig`] on the
//! service record:
//!
//! - endpoint URL (with `{base_url}` substitution)
//! - request encoding (`json` or `form`)
//! - request template with `$field` placeholders resolved from the user's
//!   credential JSON blob
//! - JSON paths to extract the token / TTL / error code from the response
//! - injection format (`bearer`, `bot_bearer`, `token`, `header:X-Custom`)
//!
//! Adding a new provider means seeding a `TokenExchangeConfig`, not writing
//! a new auth method. The only exceptions are providers that need stateful
//! signing (GitHub App JWT, AWS SigV4), which still take specialised code.
//!
//! ## Concurrency
//!
//! A per-key `TokenSlot` holds the cached value behind an `RwLock` (fast
//! path: concurrent reads, no contention) and serialises fetches behind a
//! separate `Mutex` (slow path: exactly one exchange in flight per key).
//! Waiters acquire the fetch lock, double-check the cache, and see the
//! freshly-populated value without re-fetching. Tested with 100 concurrent
//! callers on a cold cache hitting `fetch_fn` exactly once.
//!
//! ## Cache key
//!
//! `{base_url}::{execution_config_fingerprint}::{credential_json}`, hashed as
//! one opaque cache key. The config fingerprint prevents a token fetched
//! under one exchange contract from being reused after an execution-relevant
//! config change. Two users with the same credential and the same contract
//! still share a cache entry (the resulting token is identical anyway).
//!
//! ## Lifetime
//!
//! In-memory only. Backend restart pays one exchange latency per distinct
//! credential on the next request. A bounded cache evicts expired values every
//! 30 seconds, including credentials never requested again. In-flight slots
//! remain stable until their last fetcher/waiter releases them.
//! The supported cache lifetime is 60 seconds through 30 days. Larger
//! provider/configured lifetimes are rejected rather than overflowing dates.

use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, LazyLock, Weak};
use zeroize::Zeroizing;

const MAX_TOKEN_TTL_SECS: i64 = 30 * 24 * 60 * 60;

use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, RwLock};

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{CredentialFieldSpec, TokenExchangeConfig};
use crate::services::mcp_service;

/// A single cached access token with its computed expiry time.
#[derive(Clone)]
pub struct CachedToken {
    pub token: Zeroizing<String>,
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for CachedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedToken")
            .field("token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Per-key slot combining the stored value and the fetch serializer.
#[derive(Default)]
struct TokenSlot {
    cached: RwLock<Option<StoredToken>>,
    fetch_lock: Mutex<()>,
    generation: AtomicU64,
}

struct StoredToken {
    value: CachedToken,
    retained_until: std::time::Instant,
}

type CacheEntries = DashMap<String, Arc<TokenSlot>>;
// Weak references keep no credentials or caches alive. Dead registrations are
// pruned on construction and retirement; production uses one shared cache.
static PROCESS_CACHES: LazyLock<std::sync::Mutex<Vec<Weak<CacheEntries>>>> =
    LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// Drop locally cached exchange results for exactly this source material.
/// This is memory cleanup, not the cross-replica authorization boundary: live
/// credential/owner admission remains mandatory on every execution.
pub(crate) async fn retire_credential(credential: &[u8]) {
    let prefix = format!("{:x}:", Sha256::digest(credential));
    let caches = {
        let mut registry = PROCESS_CACHES.lock().unwrap_or_else(|e| e.into_inner());
        registry.retain(|entry| entry.strong_count() > 0);
        registry
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>()
    };
    for entries in caches {
        let slots = entries
            .iter()
            .filter(|entry| entry.key().starts_with(&prefix))
            .map(|entry| entry.value().clone())
            .collect::<Vec<_>>();
        for slot in slots {
            let mut cached = slot.cached.write().await;
            slot.generation.fetch_add(1, AtomicOrdering::AcqRel);
            *cached = None;
        }
        TokenExchangeCache::prune_entries(&entries);
    }
}

const MAX_CACHE_ENTRIES: usize = 4096;
const MAX_TOKEN_RESIDENCY: std::time::Duration = std::time::Duration::from_secs(600);

/// Process-wide cache of exchanged tokens keyed by the base URL, canonical
/// execution-config fingerprint, and credential hash.
///
/// Clone is cheap -- all state lives behind `Arc`/`DashMap`.
#[derive(Clone)]
pub struct TokenExchangeCache {
    entries: Arc<DashMap<String, Arc<TokenSlot>>>,
    admission: Arc<std::sync::Mutex<()>>,
}

impl Default for TokenExchangeCache {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenExchangeCache {
    pub fn new() -> Self {
        let cache = Self {
            entries: Arc::new(DashMap::new()),
            admission: Arc::new(std::sync::Mutex::new(())),
        };
        {
            let mut registry = PROCESS_CACHES.lock().unwrap_or_else(|e| e.into_inner());
            registry.retain(|entry| entry.strong_count() > 0);
            registry.push(Arc::downgrade(&cache.entries));
        }
        if tokio::runtime::Handle::try_current().is_ok() {
            cache.spawn_expiry_worker(std::time::Duration::from_secs(30));
        }
        cache
    }

    fn spawn_expiry_worker(&self, cadence: std::time::Duration) {
        let entries = Arc::downgrade(&self.entries);
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(cadence);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                timer.tick().await;
                let Some(entries) = entries.upgrade() else {
                    break;
                };
                Self::prune_entries(&entries);
            }
        });
    }

    fn prune_entries(entries: &DashMap<String, Arc<TokenSlot>>) {
        let now = Utc::now();
        let monotonic = std::time::Instant::now();
        entries.retain(|_, slot| {
            // Never wait for a reader/fetcher while holding a map shard.
            let Ok(mut cached) = slot.cached.try_write() else {
                return true;
            };
            if cached.as_ref().is_some_and(|entry| {
                entry.value.expires_at <= now || entry.retained_until <= monotonic
            }) {
                *cached = None; // drops/zeroizes the owned token immediately
            }
            // Arc count is checked under the map write lock. New readers cannot
            // acquire this slot between the check and removal; existing fetchers
            // and queued waiters each own a reference, so cannot be split.
            cached.is_some() || Arc::strong_count(slot) > 1
        });
    }

    fn slot(&self, key: &str) -> AppResult<Arc<TokenSlot>> {
        if let Some(slot) = self.entries.get(key) {
            return Ok(slot.clone());
        }
        let _admission = self
            .admission
            .lock()
            .map_err(|_| AppError::Internal("Token cache admission unavailable".into()))?;
        if let Some(slot) = self.entries.get(key) {
            return Ok(slot.clone());
        }
        if self.entries.len() >= MAX_CACHE_ENTRIES {
            Self::prune_entries(&self.entries);
            if self.entries.len() >= MAX_CACHE_ENTRIES {
                return Err(AppError::CredentialUnavailable(
                    "Token exchange cache is at capacity; retry later".into(),
                ));
            }
        }
        Ok(self
            .entries
            .entry(key.to_owned())
            .or_insert_with(|| Arc::new(TokenSlot::default()))
            .clone())
    }

    /// Get a cached token or fetch a fresh one via `fetch_fn`.
    ///
    /// The `fetch_fn` closure is invoked at most once per concurrent burst
    /// per cache key. Other waiters block on the per-key fetch lock, then
    /// observe the fresh value through the cache without calling `fetch_fn`
    /// themselves. Cache hits bypass the fetch lock entirely.
    pub async fn get_or_fetch<F, Fut>(&self, key: &str, fetch_fn: F) -> AppResult<Zeroizing<String>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = AppResult<CachedToken>>,
    {
        // DashMap::entry is atomic: races between concurrent first-touches on
        // a brand-new key produce exactly one TokenSlot.
        let slot = self.slot(key)?;

        // Fast path: the vast majority of calls land here. Concurrent readers
        // on a valid cache entry don't contend with each other.
        if let Some(token) = Self::read_fresh(&slot).await {
            return Ok(token);
        }

        // Slow path: serialize fetches per key.
        let _fetch_guard = slot.fetch_lock.lock().await;

        // Double-check after acquiring the fetch lock. If another task
        // already populated the cache while we were waiting, we skip the
        // network round-trip entirely.
        if let Some(token) = Self::read_fresh(&slot).await {
            return Ok(token);
        }

        // Perform the fetch. The fetch_lock is still held, blocking any
        // other fetcher for THIS key. Other keys are unaffected.
        let generation = slot.generation.load(AtomicOrdering::Acquire);
        let fresh = fetch_fn().await?;
        let mut cached = slot.cached.write().await;
        if slot.generation.load(AtomicOrdering::Acquire) != generation {
            return Err(AppError::CredentialUnavailable(
                "Source credential changed during token exchange; retry with current authority"
                    .into(),
            ));
        }
        let token_value = fresh.token.clone();
        *cached = Some(StoredToken {
            value: fresh,
            retained_until: std::time::Instant::now() + MAX_TOKEN_RESIDENCY,
        });
        Ok(token_value)
    }

    /// Read the cached token if it exists and is still considered fresh
    /// (more than 10 minutes remaining before expiry).
    async fn read_fresh(slot: &TokenSlot) -> Option<Zeroizing<String>> {
        let guard = slot.cached.read().await;
        let cached = guard.as_ref()?;
        if cached.value.expires_at > Utc::now() + Duration::minutes(10)
            && cached.retained_until > std::time::Instant::now()
        {
            Some(cached.value.token.clone())
        } else {
            None
        }
    }

    /// Test-only: insert a cache entry directly. Used to exercise the
    /// expired/near-expiry branches without going through `fetch_fn`.
    #[cfg(test)]
    async fn insert_for_test(&self, key: &str, entry: CachedToken) {
        let slot = self
            .entries
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(TokenSlot::default()))
            .clone();
        *slot.cached.write().await = Some(StoredToken {
            value: entry,
            retained_until: std::time::Instant::now() + MAX_TOKEN_RESIDENCY,
        });
    }
}

/// Build the canonical fingerprint for the execution-relevant token-exchange
/// contract.
///
/// Credential labels, placeholders, and secret-rendering hints are UI-only
/// and deliberately excluded. Field names are sorted and deduplicated so UI
/// ordering cannot change the execution identity.
pub fn execution_config_fingerprint(config: &TokenExchangeConfig) -> String {
    let mut credential_field_names = config
        .credential_fields
        .iter()
        .map(|field| field.name.clone())
        .collect::<Vec<_>>();
    credential_field_names.sort_unstable();
    credential_field_names.dedup();
    mcp_service::canonical_sha256(serde_json::json!({
        "contract_version": "nyxid-token-exchange-execution.v1",
        "endpoint": config.endpoint,
        "request_encoding": config.request_encoding,
        "request_template": config.request_template,
        "token_response_path": config.token_response_path,
        "ttl_response_path": config.ttl_response_path,
        "default_ttl_secs": config.default_ttl_secs,
        "injection": config.injection,
        "error_code_path": config.error_code_path,
        "error_message_path": config.error_message_path,
        "credential_field_names": credential_field_names,
    }))
}

/// Build the cache key for a base URL, credential blob, and token-exchange
/// execution contract.
///
/// Credential hashing makes keys opaque -- the raw secret never appears in
/// cache metadata. Hash is stable across restarts and across users, so users
/// holding the same credential under the same config share one cache entry.
pub fn cache_key(base_url: &str, credential_json: &str, config: &TokenExchangeConfig) -> String {
    let mut hasher = Sha256::new();
    hasher.update(base_url.trim_end_matches('/').as_bytes());
    hasher.update(b"::");
    hasher.update(execution_config_fingerprint(config).as_bytes());
    hasher.update(b"::");
    hasher.update(credential_json.as_bytes());
    format!(
        "{:x}:{:x}",
        Sha256::digest(credential_json.as_bytes()),
        hasher.finalize()
    )
}

/// Expand `{base_url}` placeholders in an endpoint template.
pub fn resolve_endpoint(template: &str, base_url: &str) -> String {
    template.replace("{base_url}", base_url.trim_end_matches('/'))
}

/// Walk a dot-separated JSON path through a value. Returns `None` if any
/// intermediate segment is missing or not an object.
pub fn extract_json_path<'a>(
    value: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

/// Walk a JSON template and substitute every string of the form `$field`
/// with the value of that field in the credential map. Non-placeholder
/// strings, numbers, booleans, nulls, and arrays pass through unchanged.
/// Placeholder strings inside arrays are still substituted.
///
/// Returns an error if any `$field` placeholder has no matching credential
/// key -- surfaces misconfiguration loudly instead of silently sending
/// `null`.
pub fn substitute_template(
    template: &serde_json::Value,
    credential: &serde_json::Map<String, serde_json::Value>,
) -> AppResult<serde_json::Value> {
    match template {
        serde_json::Value::String(s) => {
            if let Some(field_name) = s.strip_prefix('$') {
                credential.get(field_name).cloned().ok_or_else(|| {
                    AppError::BadRequest(format!(
                        "token_exchange credential missing required field: {field_name}"
                    ))
                })
            } else {
                Ok(template.clone())
            }
        }
        serde_json::Value::Object(map) => {
            let mut result = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                result.insert(k.clone(), substitute_template(v, credential)?);
            }
            Ok(serde_json::Value::Object(result))
        }
        serde_json::Value::Array(items) => {
            let mut result = Vec::with_capacity(items.len());
            for item in items {
                result.push(substitute_template(item, credential)?);
            }
            Ok(serde_json::Value::Array(result))
        }
        _ => Ok(template.clone()),
    }
}

/// Parse a token-exchange credential from its stored JSON form.
///
/// The credential is stored as a JSON object (e.g.
/// `{"app_id":"cli_xxx","app_secret":"yyy"}`) so the same code path works
/// for any provider regardless of how many secrets they require.
/// `declared_fields` is the `TokenExchangeConfig::credential_fields` list
/// from the service record; missing or blank values produce a clear error.
pub fn parse_credential(
    credential: &str,
    declared_fields: &[CredentialFieldSpec],
) -> AppResult<serde_json::Map<String, serde_json::Value>> {
    let value: serde_json::Value = serde_json::from_str(credential).map_err(|_| {
        AppError::BadRequest(
            "token_exchange credential must be a JSON object matching the service's \
             declared credential fields"
                .to_string(),
        )
    })?;

    let mut map = match value {
        serde_json::Value::Object(m) => m,
        _ => {
            return Err(AppError::BadRequest(
                "token_exchange credential must be a JSON object".to_string(),
            ));
        }
    };

    for spec in declared_fields {
        let present = match map.get(&spec.name) {
            Some(serde_json::Value::String(s)) => !s.trim().is_empty(),
            Some(serde_json::Value::Null) | None => false,
            _ => true,
        };
        if !present {
            return Err(AppError::BadRequest(format!(
                "token_exchange credential is missing required field '{}'",
                spec.name
            )));
        }
    }

    // Trim string values so the template sees clean input.
    for (_, value) in map.iter_mut() {
        if let serde_json::Value::String(s) = value {
            *s = s.trim().to_string();
        }
    }

    Ok(map)
}

/// Apply a [`TokenExchangeConfig::injection`] directive to a reqwest
/// [`RequestBuilder`], layering the exchanged token onto the outbound
/// request to the downstream service.
pub fn apply_injection(
    request: reqwest::RequestBuilder,
    injection: &str,
    token: &str,
) -> AppResult<reqwest::RequestBuilder> {
    fn secret_header(value: &str) -> AppResult<reqwest::header::HeaderValue> {
        let mut value = reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| AppError::Internal("Invalid exchanged authentication header".into()))?;
        value.set_sensitive(true);
        Ok(value)
    }
    match injection {
        "bearer" => Ok(request.bearer_auth(token)),
        "bot_bearer" => {
            let value = Zeroizing::new(format!("Bot {token}"));
            Ok(request.header("Authorization", secret_header(&value)?))
        }
        "token" => {
            let value = Zeroizing::new(format!("token {token}"));
            Ok(request.header("Authorization", secret_header(&value)?))
        }
        other => {
            if let Some(header_name) = other.strip_prefix("header:") {
                let trimmed = header_name.trim();
                if trimmed.is_empty() {
                    return Err(AppError::Internal(
                        "token_exchange injection 'header:' requires a non-empty header name"
                            .to_string(),
                    ));
                }
                Ok(request.header(trimmed, secret_header(token)?))
            } else {
                Err(AppError::Internal(format!(
                    "Unknown token_exchange injection format: {other}"
                )))
            }
        }
    }
}

/// Perform one token exchange round-trip against the configured endpoint.
///
/// Does NOT touch the cache. Callers usually want
/// [`get_cached_exchange_token`] instead.
pub async fn fetch_exchange_token(
    http: &reqwest::Client,
    base_url: &str,
    config: &TokenExchangeConfig,
    credential: &serde_json::Map<String, serde_json::Value>,
) -> AppResult<CachedToken> {
    let url = resolve_endpoint(&config.endpoint, base_url);
    let body = substitute_template(&config.request_template, credential)?;

    let request = match config.request_encoding.as_str() {
        "json" => http.post(&url).json(&body),
        "form" => {
            // Flatten to Vec<(String, String)>. Only top-level keys are
            // supported for form encoding -- that covers every OAuth-style
            // exchange I know of.
            let map = body.as_object().ok_or_else(|| {
                AppError::Internal(
                    "token_exchange form encoding requires a JSON object template".to_string(),
                )
            })?;
            let pairs: Vec<(String, String)> = map
                .iter()
                .map(|(k, v)| (k.clone(), stringify_form_value(v)))
                .collect();
            http.post(&url).form(&pairs)
        }
        other => {
            return Err(AppError::Internal(format!(
                "Unknown token_exchange request_encoding: {other}"
            )));
        }
    };

    let response = request
        .send()
        .await
        .map_err(|_| AppError::ChannelPlatformError("token_exchange transport failed".into()))?;
    let status = response.status();
    let response: serde_json::Value = response.json().await.map_err(|_| {
        AppError::ChannelPlatformError("Invalid token_exchange JSON response".into())
    })?;

    // Detection remains configurable; only finite known provider categories
    // cross the error boundary. Neither the configured code value nor message
    // path is an authority to disclose provider response text.
    let configured_rejection = config
        .error_code_path
        .as_deref()
        .and_then(|path| extract_json_path(&response, path))
        .is_some_and(|value| !is_success_value(value));
    if !status.is_success() || configured_rejection {
        let code = super::provider_diagnostics::rejection(&response)
            .map_or("provider_rejected", |error| error.code);
        return Err(AppError::ChannelPlatformError(format!(
            "token_exchange rejected (HTTP {}, {code})",
            status.as_u16()
        )));
    }

    // Extract the token.
    let token = extract_json_path(&response, &config.token_response_path)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            AppError::ChannelPlatformError(format!(
                "token_exchange response missing token at path '{}'",
                config.token_response_path
            ))
        })?;

    // Extract TTL (in seconds) or fall back.
    let ttl_secs = config
        .ttl_response_path
        .as_deref()
        .and_then(|p| extract_json_path(&response, p))
        .and_then(json_value_to_i64)
        .unwrap_or(config.default_ttl_secs)
        .max(60);
    if ttl_secs > MAX_TOKEN_TTL_SECS {
        return Err(AppError::ChannelPlatformError(
            "token_exchange returned an unsupported token lifetime".into(),
        ));
    }
    let expires_at = Duration::try_seconds(ttl_secs)
        .and_then(|ttl| Utc::now().checked_add_signed(ttl))
        .ok_or_else(|| {
            AppError::ChannelPlatformError(
                "token_exchange returned an unsupported token lifetime".into(),
            )
        })?;

    Ok(CachedToken {
        token: Zeroizing::new(token),
        expires_at,
    })
}

/// Get a cached token or perform a fresh exchange, using the single-flight
/// cache keyed by `(base_url, execution_config_fingerprint,
/// hash(credential_json))`.
pub async fn get_cached_exchange_token(
    cache: &TokenExchangeCache,
    http: &reqwest::Client,
    base_url: &str,
    credential_json: &str,
    config: &TokenExchangeConfig,
    credential: &serde_json::Map<String, serde_json::Value>,
) -> AppResult<Zeroizing<String>> {
    let key = cache_key(base_url, credential_json, config);
    cache
        .get_or_fetch(&key, || async {
            fetch_exchange_token(http, base_url, config, credential).await
        })
        .await
}

fn stringify_form_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn is_success_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => true,
        serde_json::Value::Number(n) => n.as_i64() == Some(0) || n.as_f64() == Some(0.0),
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            trimmed.is_empty() || trimmed == "0" || trimmed.eq_ignore_ascii_case("ok")
        }
        serde_json::Value::Bool(b) => *b,
        _ => false,
    }
}

fn json_value_to_i64(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        serde_json::Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn configured_exchange_rejection_and_malformed_success_have_safe_http_diagnostics() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
        let secret = "distinctive-configurable-exchange-secret-f149";
        let server = MockServer::start().await;
        let mut config = exchange_config();
        config.error_code_path = Some("result.code".into());
        config.error_message_path = Some("result.msg".into());
        let credential = parse_credential(
            r#"{"app_id":"fixture","app_secret":"fixture"}"#,
            &lark_field_specs(),
        )
        .unwrap();
        for (status, body, category) in [
            (
                200,
                serde_json::json!({"result":{"code":secret,"msg":secret},"token_a":"must-not-be-accepted"}),
                "provider_rejected",
            ),
            (
                200,
                serde_json::json!({"code":99991663,"result":{"code":99991663,"msg":secret},"token_a":"must-not-be-accepted"}),
                "lark_app_ticket_invalid",
            ),
            (
                403,
                serde_json::json!({"result":{"code":0,"msg":secret},"token_a":"must-not-be-accepted"}),
                "provider_rejected",
            ),
            (
                200,
                serde_json::json!({"result":{"code":0},"token_a":{"secret":secret}}),
                "missing token",
            ),
        ] {
            server.reset().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(status).set_body_json(body))
                .mount(&server)
                .await;
            let error =
                fetch_exchange_token(&reqwest::Client::new(), &server.uri(), &config, &credential)
                    .await
                    .err()
                    .unwrap();
            assert!(
                matches!(&error, AppError::ChannelPlatformError(message) if message.contains(category)),
                "wrong rejection category"
            );
            assert!(!error.to_string().contains(secret));
            let response = axum::response::IntoResponse::into_response(error);
            assert_eq!(response.status(), axum::http::StatusCode::BAD_GATEWAY);
            let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains(secret));
        }
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"result":{"code":0,"msg":secret},"token_a":"valid-token"}),
            ))
            .mount(&server)
            .await;
        assert_eq!(
            fetch_exchange_token(&reqwest::Client::new(), &server.uri(), &config, &credential)
                .await
                .unwrap()
                .token
                .as_str(),
            "valid-token"
        );
    }

    #[tokio::test]
    async fn extreme_provider_ttl_is_rejected_and_normal_lifetime_is_preserved() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
        let server = MockServer::start().await;
        let config = exchange_config();
        let credential = parse_credential(
            r#"{"app_id":"fixture","app_secret":"fixture"}"#,
            &lark_field_specs(),
        )
        .unwrap();
        for ttl in [
            serde_json::json!(i64::MAX),
            serde_json::json!(u64::MAX),
            serde_json::json!(MAX_TOKEN_TTL_SECS + 1),
        ] {
            server.reset().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"token_a":"distinctive-extreme-ttl-token", "expires_in":ttl}))).mount(&server).await;
            let error =
                fetch_exchange_token(&reqwest::Client::new(), &server.uri(), &config, &credential)
                    .await
                    .err()
                    .unwrap();
            assert!(
                matches!(&error, AppError::ChannelPlatformError(message) if message.contains("unsupported token lifetime"))
            );
            assert!(!error.to_string().contains("distinctive-extreme-ttl-token"));
        }
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"token_a":"valid-token", "expires_in":3600})),
            )
            .mount(&server)
            .await;
        let before = Utc::now();
        let cached =
            fetch_exchange_token(&reqwest::Client::new(), &server.uri(), &config, &credential)
                .await
                .unwrap();
        assert_eq!(cached.token.as_str(), "valid-token");
        assert!(cached.expires_at >= before + Duration::seconds(3600));
        assert!(cached.expires_at <= Utc::now() + Duration::seconds(3600));
    }

    #[test]
    fn cached_token_debug_redacts_secret() {
        let cached = CachedToken {
            token: Zeroizing::new("distinctive-exchanged-access-token".to_owned()),
            expires_at: Utc::now(),
        };
        for output in [format!("{cached:?}"), format!("{cached:#?}")] {
            assert!(!output.contains(cached.token.as_str()));
            assert!(output.contains("[REDACTED]"));
        }
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    fn lark_field_specs() -> Vec<CredentialFieldSpec> {
        vec![
            CredentialFieldSpec {
                name: "app_id".to_string(),
                label: "App ID".to_string(),
                placeholder: None,
                secret: false,
            },
            CredentialFieldSpec {
                name: "app_secret".to_string(),
                label: "App Secret".to_string(),
                placeholder: None,
                secret: true,
            },
        ]
    }

    fn exchange_config() -> TokenExchangeConfig {
        TokenExchangeConfig {
            endpoint: "{base_url}/token".to_string(),
            request_encoding: "json".to_string(),
            request_template: serde_json::json!({
                "app_id": "$app_id",
                "app_secret": "$app_secret",
            }),
            token_response_path: "token_a".to_string(),
            ttl_response_path: Some("expires_in".to_string()),
            default_ttl_secs: 3600,
            injection: "bearer".to_string(),
            error_code_path: None,
            error_message_path: None,
            credential_fields: lark_field_specs(),
        }
    }

    // ─── cache_key ────────────────────────────────────────────────────

    #[test]
    fn cache_key_differs_per_base_url() {
        let config = exchange_config();
        let a = cache_key("https://open.larksuite.com", r#"{"app_id":"x"}"#, &config);
        let b = cache_key("https://open.feishu.cn", r#"{"app_id":"x"}"#, &config);
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_differs_per_credential() {
        let config = exchange_config();
        let a = cache_key("https://api.example.com", r#"{"id":"a"}"#, &config);
        let b = cache_key("https://api.example.com", r#"{"id":"b"}"#, &config);
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_normalises_trailing_slash() {
        let config = exchange_config();
        assert_eq!(
            cache_key("https://api.example.com/", "cred", &config),
            cache_key("https://api.example.com", "cred", &config)
        );
    }

    #[test]
    fn execution_config_fingerprint_ignores_ui_hints_and_field_order() {
        let mut first = exchange_config();
        first.credential_fields.push(CredentialFieldSpec {
            name: "app_id".to_string(),
            label: "Duplicate UI label".to_string(),
            placeholder: Some("duplicate".to_string()),
            secret: true,
        });

        let mut second = exchange_config();
        second.credential_fields.reverse();
        for field in &mut second.credential_fields {
            field.label = format!("Changed {} label", field.name);
            field.placeholder = Some(format!("changed-{}", field.name));
            field.secret = !field.secret;
        }

        assert_eq!(
            execution_config_fingerprint(&first),
            execution_config_fingerprint(&second)
        );
    }

    #[test]
    fn execution_config_fingerprint_changes_with_execution_contract() {
        let first = exchange_config();
        let mut second = first.clone();
        second.token_response_path = "token_b".to_string();

        assert_ne!(
            execution_config_fingerprint(&first),
            execution_config_fingerprint(&second)
        );
        assert_ne!(
            cache_key("https://api.example.com", "credential", &first),
            cache_key("https://api.example.com", "credential", &second)
        );
    }

    // ─── resolve_endpoint ─────────────────────────────────────────────

    #[test]
    fn resolve_endpoint_substitutes_placeholder() {
        assert_eq!(
            resolve_endpoint(
                "{base_url}/open-apis/auth/v3/tenant_access_token/internal",
                "https://open.larksuite.com"
            ),
            "https://open.larksuite.com/open-apis/auth/v3/tenant_access_token/internal"
        );
    }

    #[test]
    fn resolve_endpoint_passes_absolute_url_through() {
        assert_eq!(
            resolve_endpoint("https://slack.com/api/oauth.v2.access", "anything"),
            "https://slack.com/api/oauth.v2.access"
        );
    }

    // ─── extract_json_path ────────────────────────────────────────────

    #[test]
    fn extract_json_path_top_level() {
        let v = serde_json::json!({"access_token": "abc"});
        assert_eq!(
            extract_json_path(&v, "access_token"),
            Some(&serde_json::json!("abc"))
        );
    }

    #[test]
    fn extract_json_path_nested() {
        let v = serde_json::json!({"data": {"token": "abc"}});
        assert_eq!(
            extract_json_path(&v, "data.token"),
            Some(&serde_json::json!("abc"))
        );
    }

    #[test]
    fn extract_json_path_missing_returns_none() {
        let v = serde_json::json!({"data": {}});
        assert!(extract_json_path(&v, "data.token").is_none());
        assert!(extract_json_path(&v, "missing").is_none());
    }

    // ─── substitute_template ──────────────────────────────────────────

    #[test]
    fn substitute_template_replaces_placeholders() {
        let tmpl = serde_json::json!({
            "app_id": "$app_id",
            "app_secret": "$app_secret",
            "literal": "unchanged",
            "number": 42,
        });
        let mut cred = serde_json::Map::new();
        cred.insert("app_id".into(), serde_json::json!("cli_xxx"));
        cred.insert("app_secret".into(), serde_json::json!("s3cret"));
        let result = substitute_template(&tmpl, &cred).unwrap();
        assert_eq!(
            result,
            serde_json::json!({
                "app_id": "cli_xxx",
                "app_secret": "s3cret",
                "literal": "unchanged",
                "number": 42,
            })
        );
    }

    #[test]
    fn substitute_template_errors_on_missing_field() {
        let tmpl = serde_json::json!({"x": "$missing"});
        let cred = serde_json::Map::new();
        let err = substitute_template(&tmpl, &cred).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn substitute_template_walks_arrays_and_nested_objects() {
        let tmpl = serde_json::json!({
            "top": {
                "nested": ["$a", "literal", {"deep": "$b"}]
            }
        });
        let mut cred = serde_json::Map::new();
        cred.insert("a".into(), serde_json::json!("A"));
        cred.insert("b".into(), serde_json::json!("B"));
        let result = substitute_template(&tmpl, &cred).unwrap();
        assert_eq!(result["top"]["nested"][0], serde_json::json!("A"));
        assert_eq!(result["top"]["nested"][1], serde_json::json!("literal"));
        assert_eq!(result["top"]["nested"][2]["deep"], serde_json::json!("B"));
    }

    // ─── parse_credential ─────────────────────────────────────────────

    #[test]
    fn parse_credential_lark_happy_path() {
        let specs = lark_field_specs();
        let map = parse_credential(r#"{"app_id":"cli_xxx","app_secret":"yyy"}"#, &specs).unwrap();
        assert_eq!(map["app_id"].as_str(), Some("cli_xxx"));
        assert_eq!(map["app_secret"].as_str(), Some("yyy"));
    }

    #[test]
    fn parse_credential_trims_string_values() {
        let specs = lark_field_specs();
        let map =
            parse_credential(r#"{"app_id":"  cli_xxx  ","app_secret":"  yyy  "}"#, &specs).unwrap();
        assert_eq!(map["app_id"].as_str(), Some("cli_xxx"));
    }

    #[test]
    fn parse_credential_rejects_missing_field() {
        let specs = lark_field_specs();
        assert!(parse_credential(r#"{"app_id":"x"}"#, &specs).is_err());
    }

    #[test]
    fn parse_credential_rejects_empty_string_field() {
        let specs = lark_field_specs();
        let err = parse_credential(r#"{"app_id":"","app_secret":"y"}"#, &specs).unwrap_err();
        assert!(err.to_string().contains("app_id"));
    }

    #[test]
    fn parse_credential_rejects_non_object() {
        let specs = lark_field_specs();
        assert!(parse_credential("[1,2,3]", &specs).is_err());
        assert!(parse_credential("\"not object\"", &specs).is_err());
    }

    // ─── is_success_value ─────────────────────────────────────────────

    #[test]
    fn is_success_value_recognises_common_success_encodings() {
        assert!(is_success_value(&serde_json::json!(0)));
        assert!(is_success_value(&serde_json::json!("0")));
        assert!(is_success_value(&serde_json::json!("ok")));
        assert!(is_success_value(&serde_json::json!("OK")));
        assert!(is_success_value(&serde_json::json!(null)));
        assert!(is_success_value(&serde_json::json!(true)));
    }

    #[test]
    fn is_success_value_rejects_error_encodings() {
        assert!(!is_success_value(&serde_json::json!(1)));
        assert!(!is_success_value(&serde_json::json!(99991661)));
        assert!(!is_success_value(&serde_json::json!("error")));
        assert!(!is_success_value(&serde_json::json!(false)));
    }

    // ─── cache + single-flight ────────────────────────────────────────

    fn fresh_token(name: &str, ttl_secs: i64) -> CachedToken {
        CachedToken {
            token: Zeroizing::new(name.to_string()),
            expires_at: Utc::now() + Duration::seconds(ttl_secs),
        }
    }

    #[tokio::test]
    async fn cache_worker_evicts_idle_expired_and_residency_limited_values() {
        let cache = TokenExchangeCache::new();
        cache
            .insert_for_test("expired-idle", fresh_token("expired-secret", -1))
            .await;
        cache
            .insert_for_test("residency-idle", fresh_token("long-lived-secret", 86400))
            .await;
        {
            let slot = cache.entries.get("residency-idle").unwrap().clone();
            slot.cached.write().await.as_mut().unwrap().retained_until = std::time::Instant::now();
        }
        cache.spawn_expiry_worker(std::time::Duration::from_millis(5));
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !cache.entries.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // No get_or_fetch of either retired key triggered this cleanup.
    }

    #[tokio::test]
    async fn credential_retirement_clears_matching_caches_and_fences_held_fetch() {
        let credential = format!("distinctive-retired-{}", uuid::Uuid::new_v4());
        let config = exchange_config();
        let key = cache_key("https://provider.example", &credential, &config);
        let other_key = cache_key("https://provider.example", "unrelated-credential", &config);
        let first = TokenExchangeCache::new();
        let second = TokenExchangeCache::new();
        first
            .insert_for_test(&key, fresh_token("retired-token", 3600))
            .await;
        first
            .insert_for_test(&other_key, fresh_token("unrelated-token", 3600))
            .await;
        let entered = tokio::sync::Semaphore::new(0);
        let release = tokio::sync::Semaphore::new(0);
        let fetch = second.get_or_fetch(&key, || async {
            entered.add_permits(1);
            release.acquire().await.unwrap().forget();
            Ok(fresh_token("late-retired-token", 3600))
        });
        let retire = async {
            entered.acquire().await.unwrap().forget();
            retire_credential(credential.as_bytes()).await;
            assert!(!first.entries.contains_key(&key));
            assert!(first.entries.contains_key(&other_key));
            release.add_permits(1);
        };
        let (result, _) = tokio::join!(fetch, retire);
        assert!(matches!(result, Err(AppError::CredentialUnavailable(_))));
        TokenExchangeCache::prune_entries(&second.entries);
        assert!(!second.entries.contains_key(&key));
    }

    #[tokio::test]
    async fn shared_source_retirement_permits_immediate_authorized_refetch() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token_a":"shared-source-token", "expires_in":3600
            })))
            .expect(2)
            .mount(&server)
            .await;
        let cache = TokenExchangeCache::new();
        let config = exchange_config();
        let raw = format!(
            r#"{{"app_id":"{}","app_secret":"shared"}}"#,
            uuid::Uuid::new_v4()
        );
        let credential = parse_credential(&raw, &lark_field_specs()).unwrap();
        let http = reqwest::Client::new();
        let key = cache_key(&server.uri(), &raw, &config);
        for _authorized_owner in ["owner-a", "owner-b"] {
            let token =
                get_cached_exchange_token(&cache, &http, &server.uri(), &raw, &config, &credential)
                    .await
                    .unwrap();
            assert_eq!(token.as_str(), "shared-source-token");
        }
        // Keep a stale Arc alive: cleanup must not make owner B unavailable
        // until that Arc drops or the next periodic sweep runs.
        let held = cache.slot(&key).unwrap();
        retire_credential(raw.as_bytes()).await;
        assert!(cache.entries.contains_key(&key));
        assert!(held.cached.read().await.is_none());
        let token =
            get_cached_exchange_token(&cache, &http, &server.uri(), &raw, &config, &credential)
                .await
                .unwrap();
        assert_eq!(token.as_str(), "shared-source-token");
        assert!(Arc::ptr_eq(&held, &cache.slot(&key).unwrap()));
    }

    #[tokio::test]
    async fn pruning_preserves_live_single_flight_and_bounds_admission() {
        let cache = TokenExchangeCache::new();
        let entered = tokio::sync::Semaphore::new(0);
        let release = tokio::sync::Semaphore::new(0);
        let calls = AtomicUsize::new(0);
        let first = cache.get_or_fetch("held", || async {
            calls.fetch_add(1, Ordering::SeqCst);
            entered.add_permits(1);
            release.acquire().await.unwrap().forget();
            Ok(fresh_token("single-flight", 3600))
        });
        let waiter = async {
            entered.acquire().await.unwrap().forget();
            TokenExchangeCache::prune_entries(&cache.entries);
            assert!(cache.entries.contains_key("held"));
            let slot = cache.slot("held").unwrap();
            let queued = cache.get_or_fetch("held", || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(fresh_token("duplicate", 3600))
            });
            release.add_permits(1);
            let result = queued.await.unwrap();
            drop(slot);
            result
        };
        let (first, second) = tokio::join!(first, waiter);
        assert_eq!(first.unwrap().as_str(), "single-flight");
        assert_eq!(second.as_str(), "single-flight");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        for i in 1..MAX_CACHE_ENTRIES {
            cache
                .insert_for_test(&format!("entry-{i}"), fresh_token("bounded", 3600))
                .await;
        }
        assert!(matches!(
            cache.slot("over-capacity"),
            Err(AppError::CredentialUnavailable(_))
        ));
        {
            let slot = cache.entries.get("entry-1").unwrap().clone();
            slot.cached.write().await.as_mut().unwrap().value.expires_at = Utc::now();
        }
        assert!(cache.slot("after-idle-expiry").is_ok());
        assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
    }

    #[tokio::test]
    async fn cache_hit_skips_fetch() {
        let cache = TokenExchangeCache::new();
        cache
            .insert_for_test("key", fresh_token("cached-token", 3600))
            .await;

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();
        let token = cache
            .get_or_fetch("key", || {
                let counter = counter_clone.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(fresh_token("fresh-token", 3600))
                }
            })
            .await
            .unwrap();

        assert_eq!(token.as_str(), "cached-token");
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn near_expiry_triggers_refresh() {
        let cache = TokenExchangeCache::new();
        cache.insert_for_test("key", fresh_token("stale", 60)).await;

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();
        let token = cache
            .get_or_fetch("key", || {
                let counter = counter_clone.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(fresh_token("fresh", 3600))
                }
            })
            .await
            .unwrap();

        assert_eq!(token.as_str(), "fresh");
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cold_cache_single_flight_with_100_concurrent_callers() {
        let cache = TokenExchangeCache::new();
        let counter = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..100 {
            let cache = cache.clone();
            let counter = counter.clone();
            handles.push(tokio::spawn(async move {
                cache
                    .get_or_fetch("shared-key", || {
                        let counter = counter.clone();
                        async move {
                            counter.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            Ok(fresh_token("only-fetch", 3600))
                        }
                    })
                    .await
            }));
        }

        for handle in handles {
            assert_eq!(handle.await.unwrap().unwrap().as_str(), "only-fetch");
        }
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "fetch_fn must be called exactly once"
        );
    }

    #[tokio::test]
    async fn distinct_keys_fetch_independently() {
        let cache = TokenExchangeCache::new();
        let counter = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for i in 0..100 {
            let cache = cache.clone();
            let counter = counter.clone();
            let key = if i % 2 == 0 { "key-a" } else { "key-b" };
            handles.push(tokio::spawn(async move {
                cache
                    .get_or_fetch(key, || {
                        let counter = counter.clone();
                        async move {
                            counter.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            Ok(fresh_token("token", 3600))
                        }
                    })
                    .await
            }));
        }

        for handle in handles {
            handle.await.unwrap().unwrap();
        }
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn fetch_error_propagates_without_poisoning_cache() {
        let cache = TokenExchangeCache::new();
        let counter = Arc::new(AtomicUsize::new(0));

        let counter_clone = counter.clone();
        let first = cache
            .get_or_fetch("key", || {
                let counter = counter_clone.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Err(AppError::ChannelPlatformError("boom".to_string()))
                }
            })
            .await;
        assert!(first.is_err());

        let counter_clone = counter.clone();
        let second = cache
            .get_or_fetch("key", || {
                let counter = counter_clone.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(fresh_token("recovered", 3600))
                }
            })
            .await
            .unwrap();
        assert_eq!(second.as_str(), "recovered");
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn cached_exchange_is_scoped_to_the_execution_config() {
        use axum::{Json, Router, extract::State, routing::post};
        use tokio::net::TcpListener;

        async fn exchange_handler(
            State(fetch_count): State<Arc<AtomicUsize>>,
        ) -> Json<serde_json::Value> {
            fetch_count.fetch_add(1, Ordering::SeqCst);
            Json(serde_json::json!({
                "token_a": "token-from-config-a",
                "token_b": "token-from-config-b",
                "expires_in": 3600,
            }))
        }

        let fetch_count = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/token", post(exchange_handler))
            .with_state(fetch_count.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let base_url = format!("http://{addr}");
        let credential_json = r#"{"app_id":"same","app_secret":"same"}"#;
        let credential = parse_credential(credential_json, &lark_field_specs()).unwrap();
        let config_a = exchange_config();
        let mut config_b = config_a.clone();
        config_b.token_response_path = "token_b".to_string();
        let cache = TokenExchangeCache::new();
        let http = reqwest::Client::new();

        let first_a = get_cached_exchange_token(
            &cache,
            &http,
            &base_url,
            credential_json,
            &config_a,
            &credential,
        )
        .await
        .unwrap();
        let second_a = get_cached_exchange_token(
            &cache,
            &http,
            &base_url,
            credential_json,
            &config_a,
            &credential,
        )
        .await
        .unwrap();
        let first_b = get_cached_exchange_token(
            &cache,
            &http,
            &base_url,
            credential_json,
            &config_b,
            &credential,
        )
        .await
        .unwrap();
        let second_b = get_cached_exchange_token(
            &cache,
            &http,
            &base_url,
            credential_json,
            &config_b,
            &credential,
        )
        .await
        .unwrap();

        assert_eq!(first_a.as_str(), "token-from-config-a");
        assert_eq!(second_a, first_a, "the same config must remain cached");
        assert_eq!(first_b.as_str(), "token-from-config-b");
        assert_eq!(second_b, first_b, "the second config must remain cached");
        assert_eq!(cache.entries.len(), 2, "config A and B need distinct slots");
        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            2,
            "each config must fetch once while repeated calls hit its cache"
        );

        server.abort();
    }

    // ─── apply_injection ──────────────────────────────────────────────

    #[test]
    fn injected_request_and_header_debug_do_not_expose_exchanged_token() {
        let secret = "distinctive-exchange-request-secret";
        for injection in ["bearer", "bot_bearer", "token", "header:X-Custom-Key"] {
            let request = apply_injection(
                reqwest::Client::new().get("https://provider.example"),
                injection,
                secret,
            )
            .unwrap()
            .build()
            .unwrap();
            let name = if injection.starts_with("header:") {
                "x-custom-key"
            } else {
                "authorization"
            };
            let header = request.headers().get(name).unwrap();
            assert!(header.is_sensitive());
            assert!(
                header.to_str().unwrap().contains(secret),
                "wire value remains exact"
            );
            for diagnostic in [
                format!("{request:?}"),
                format!("{:?}", request.headers()),
                format!("{header:?}"),
            ] {
                assert!(!diagnostic.contains(secret));
            }
        }
    }

    #[tokio::test]
    async fn apply_injection_supports_common_formats() {
        // Use a mock server to inspect the resulting headers
        use axum::{Router, http::HeaderMap, routing::get};
        use std::sync::Mutex as StdMutex;
        use tokio::net::TcpListener;

        #[derive(Clone, Default)]
        struct CaptureState {
            auth: Arc<StdMutex<Option<String>>>,
            custom: Arc<StdMutex<Option<String>>>,
        }

        async fn handler(
            axum::extract::State(state): axum::extract::State<CaptureState>,
            headers: HeaderMap,
        ) -> axum::http::StatusCode {
            *state.auth.lock().unwrap() = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .map(ToString::to_string);
            *state.custom.lock().unwrap() = headers
                .get("x-my-key")
                .and_then(|v| v.to_str().ok())
                .map(ToString::to_string);
            axum::http::StatusCode::OK
        }

        let state = CaptureState::default();
        let app = Router::new()
            .route("/", get(handler))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let client = reqwest::Client::new();
        let url = format!("http://{addr}/");

        // bearer
        apply_injection(client.get(&url), "bearer", "tkn")
            .unwrap()
            .send()
            .await
            .unwrap();
        assert_eq!(state.auth.lock().unwrap().as_deref(), Some("Bearer tkn"));

        // bot_bearer (Discord style)
        apply_injection(client.get(&url), "bot_bearer", "tkn")
            .unwrap()
            .send()
            .await
            .unwrap();
        assert_eq!(state.auth.lock().unwrap().as_deref(), Some("Bot tkn"));

        // token (GitHub style)
        apply_injection(client.get(&url), "token", "tkn")
            .unwrap()
            .send()
            .await
            .unwrap();
        assert_eq!(state.auth.lock().unwrap().as_deref(), Some("token tkn"));

        // custom header
        apply_injection(client.get(&url), "header:X-My-Key", "tkn")
            .unwrap()
            .send()
            .await
            .unwrap();
        assert_eq!(state.custom.lock().unwrap().as_deref(), Some("tkn"));

        server.abort();
    }

    #[test]
    fn apply_injection_rejects_unknown_format() {
        let client = reqwest::Client::new();
        let result = apply_injection(client.get("http://example.com"), "weird", "tkn");
        assert!(result.is_err());
    }

    #[test]
    fn apply_injection_rejects_empty_header_name() {
        let client = reqwest::Client::new();
        let result = apply_injection(client.get("http://example.com"), "header:", "tkn");
        assert!(result.is_err());
    }
}
