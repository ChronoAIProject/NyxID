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
//! `{base_url}::{sha256(credential_json)}`. The credential hash makes keys
//! opaque and provider-agnostic: two users with the same Lark app share a
//! cache entry (the resulting token is identical anyway), and different
//! credentials cannot collide across providers.
//!
//! ## Lifetime
//!
//! In-memory only. Backend restart pays one exchange latency per distinct
//! credential on the next request. No background sweeper; stale entries
//! are recomputed lazily on read.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, RwLock};

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{CredentialFieldSpec, TokenExchangeConfig};

/// A single cached access token with its computed expiry time.
#[derive(Clone, Debug)]
pub struct CachedToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Per-key slot combining the stored value and the fetch serializer.
#[derive(Default)]
struct TokenSlot {
    cached: RwLock<Option<CachedToken>>,
    fetch_lock: Mutex<()>,
}

/// Process-wide cache of exchanged tokens keyed by
/// `{base_url}::sha256({credential_json})`.
///
/// Clone is cheap -- all state lives behind `Arc`/`DashMap`.
#[derive(Default, Clone)]
pub struct TokenExchangeCache {
    entries: Arc<DashMap<String, Arc<TokenSlot>>>,
}

impl TokenExchangeCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a cached token or fetch a fresh one via `fetch_fn`.
    ///
    /// The `fetch_fn` closure is invoked at most once per concurrent burst
    /// per cache key. Other waiters block on the per-key fetch lock, then
    /// observe the fresh value through the cache without calling `fetch_fn`
    /// themselves. Cache hits bypass the fetch lock entirely.
    pub async fn get_or_fetch<F, Fut>(&self, key: &str, fetch_fn: F) -> AppResult<String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = AppResult<CachedToken>>,
    {
        // DashMap::entry is atomic: races between concurrent first-touches on
        // a brand-new key produce exactly one TokenSlot.
        let slot = self
            .entries
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(TokenSlot::default()))
            .clone();

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
        let fresh = fetch_fn().await?;
        let token_value = fresh.token.clone();
        *slot.cached.write().await = Some(fresh);
        Ok(token_value)
    }

    /// Read the cached token if it exists and is still considered fresh
    /// (more than 10 minutes remaining before expiry).
    async fn read_fresh(slot: &TokenSlot) -> Option<String> {
        let guard = slot.cached.read().await;
        let cached = guard.as_ref()?;
        if cached.expires_at > Utc::now() + Duration::minutes(10) {
            Some(cached.token.clone())
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
        *slot.cached.write().await = Some(entry);
    }
}

/// Build the cache key for a given base URL and credential blob.
///
/// Credential hashing makes keys opaque -- the raw secret never appears in
/// cache metadata. Hash is stable across restarts and across users, so two
/// users holding the same credential transparently share one cache entry.
pub fn cache_key(base_url: &str, credential_json: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(base_url.trim_end_matches('/').as_bytes());
    hasher.update(b"::");
    hasher.update(credential_json.as_bytes());
    format!("{:x}", hasher.finalize())
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
    match injection {
        "bearer" => Ok(request.bearer_auth(token)),
        "bot_bearer" => Ok(request.header("Authorization", format!("Bot {token}"))),
        "token" => Ok(request.header("Authorization", format!("token {token}"))),
        other => {
            if let Some(header_name) = other.strip_prefix("header:") {
                let trimmed = header_name.trim();
                if trimmed.is_empty() {
                    return Err(AppError::Internal(
                        "token_exchange injection 'header:' requires a non-empty header name"
                            .to_string(),
                    ));
                }
                Ok(request.header(trimmed, token))
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

    let response: serde_json::Value = request
        .send()
        .await
        .map_err(|e| AppError::ChannelPlatformError(format!("token_exchange request failed: {e}")))?
        .json()
        .await
        .map_err(|e| {
            AppError::ChannelPlatformError(format!(
                "token_exchange response parse failed (expected JSON): {e}"
            ))
        })?;

    // Error check (provider-specific field like Lark's `code`).
    if let Some(error_path) = config.error_code_path.as_deref()
        && let Some(error_value) = extract_json_path(&response, error_path)
        && !is_success_value(error_value)
    {
        let msg = config
            .error_message_path
            .as_deref()
            .and_then(|p| extract_json_path(&response, p))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(AppError::ChannelPlatformError(format!(
            "token_exchange failed (code={error_value}, msg={msg})"
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

    Ok(CachedToken {
        token,
        expires_at: Utc::now() + Duration::seconds(ttl_secs),
    })
}

/// Get a cached token or perform a fresh exchange, using the single-flight
/// cache keyed by `(base_url, hash(credential_json))`.
pub async fn get_cached_exchange_token(
    cache: &TokenExchangeCache,
    http: &reqwest::Client,
    base_url: &str,
    credential_json: &str,
    config: &TokenExchangeConfig,
    credential: &serde_json::Map<String, serde_json::Value>,
) -> AppResult<String> {
    let key = cache_key(base_url, credential_json);
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

    // ─── cache_key ────────────────────────────────────────────────────

    #[test]
    fn cache_key_differs_per_base_url() {
        let a = cache_key("https://open.larksuite.com", r#"{"app_id":"x"}"#);
        let b = cache_key("https://open.feishu.cn", r#"{"app_id":"x"}"#);
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_differs_per_credential() {
        let a = cache_key("https://api.example.com", r#"{"id":"a"}"#);
        let b = cache_key("https://api.example.com", r#"{"id":"b"}"#);
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_normalises_trailing_slash() {
        assert_eq!(
            cache_key("https://api.example.com/", "cred"),
            cache_key("https://api.example.com", "cred")
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
            token: name.to_string(),
            expires_at: Utc::now() + Duration::seconds(ttl_secs),
        }
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

        assert_eq!(token, "cached-token");
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

        assert_eq!(token, "fresh");
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
            assert_eq!(handle.await.unwrap().unwrap(), "only-fetch");
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
        assert_eq!(second, "recovered");
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    // ─── apply_injection ──────────────────────────────────────────────

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
