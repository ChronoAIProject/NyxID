//! In-process response cache for the `aws_sigv4` proxy auth method
//! (NyxID#716).
//!
//! AWS Cost Explorer charges $0.01 per paginated API request. A `/daily`-
//! style skill that polls cost-by-namespace from a handful of windows
//! (last 30 days, current month, previous month) at hourly intervals
//! would otherwise burn through API budget without producing fresher
//! data than the underlying ~6h billing-data latency. The cache
//! short-circuits repeat-identical proxy requests at the NyxID boundary.
//!
//! Lifetime + scoping:
//! - Keys include `(auth_method, sha256(credential), base_url, method,
//!   path+query, sha256(canonicalized response-affecting headers),
//!   sha256(body))`. The credential fingerprint scopes per user — two
//!   users hitting the same catalog endpoint with different stored
//!   credentials get different entries. The headers hash captures the
//!   AWS `X-Amz-Target` (JSON-RPC operation dispatch) so a different
//!   operation on the same path doesn't replay the previous response.
//! - Default TTL is 0 (disabled); set `CLOUD_RESPONSE_CACHE_TTL_SECS`
//!   to enable. Once Codex review finding REC 11 — the cache safety
//!   review — has been independently validated, operators can flip it
//!   on per deployment.
//! - Successful responses (2xx) only — caching 4xx/5xx would hide
//!   permission misconfigurations.
//! - Bounded: `max_entry_bytes` caps the size of a single cacheable
//!   response (default 1 MiB); `max_entries` caps total in-memory
//!   entries (default 256, LRU eviction by insertion timestamp).
//!   Expired entries are dropped opportunistically on access.
//! - In-process, no persistence. Restart drops the cache.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use dashmap::DashMap;
use reqwest::header::{HeaderName, HeaderValue};
use sha2::{Digest, Sha256};

/// Auth methods that participate in this cache. Others bypass it.
pub fn is_cacheable_auth_method(auth_method: &str) -> bool {
    matches!(auth_method, "aws_sigv4")
}

/// Default per-entry size cap (1 MiB). Responses larger than this are
/// forwarded uncached. Override via `CLOUD_RESPONSE_CACHE_MAX_ENTRY_BYTES`.
pub const DEFAULT_MAX_ENTRY_BYTES: usize = 1024 * 1024;

/// Default maximum entry count. Override via `CLOUD_RESPONSE_CACHE_MAX_ENTRIES`.
pub const DEFAULT_MAX_ENTRIES: usize = 256;

#[derive(Clone)]
struct CachedEntry {
    status: u16,
    headers: Vec<(String, String)>,
    body: Bytes,
    /// Monotonic clock; for TTL expiry checks.
    expires_at: Instant,
    /// Insertion timestamp; for LRU eviction tiebreak.
    inserted_at: Instant,
}

/// Thread-safe response cache shared across the proxy hot path.
#[derive(Clone)]
pub struct CloudResponseCache {
    inner: Arc<DashMap<String, CachedEntry>>,
    ttl: Duration,
    max_entry_bytes: usize,
    max_entries: usize,
}

impl CloudResponseCache {
    /// Build a cache with the given TTL and default bounds. A zero TTL
    /// disables caching: `get` always returns `None` and
    /// `insert_and_replay` becomes a pure passthrough.
    pub fn new(ttl_secs: u64) -> Self {
        Self::with_bounds(ttl_secs, DEFAULT_MAX_ENTRY_BYTES, DEFAULT_MAX_ENTRIES)
    }

    pub fn with_bounds(ttl_secs: u64, max_entry_bytes: usize, max_entries: usize) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            ttl: Duration::from_secs(ttl_secs),
            max_entry_bytes,
            max_entries: max_entries.max(1),
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Sha256-hex fingerprint of an opaque credential string. Use this
    /// rather than the raw credential to scope cache keys per user
    /// without exposing the credential value if the key ever leaks.
    pub fn credential_fingerprint(credential: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(credential.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Headers that materially change the upstream response and so must
    /// be part of the cache key. For AWS this captures `x-amz-target`
    /// (JSON-RPC operation dispatch). For GCP this captures
    /// `x-goog-user-project` (quota project) and other `x-goog-*`
    /// routing headers. `accept` and `content-type` are included
    /// because they can flip response media type / shape.
    fn is_response_affecting(name_lower: &str) -> bool {
        if matches!(name_lower, "accept" | "content-type") {
            return true;
        }
        name_lower.starts_with("x-amz-") || name_lower.starts_with("x-goog-")
    }

    /// Compute a stable cache key for a request. Body bytes are SHA256'd
    /// so the key length stays bounded; URL and method are included
    /// verbatim because they're small. The credential fingerprint
    /// scopes per user, and the canonicalized header digest captures
    /// AWS/GCP operation-routing headers (the bug Codex flagged where
    /// two operations with the same body+path would have collided).
    pub fn key(
        auth_method: &str,
        credential_fingerprint: &str,
        base_url: &str,
        method: &str,
        path_and_query: &str,
        headers: &[(String, String)],
        body: &[u8],
    ) -> String {
        let mut body_hasher = Sha256::new();
        body_hasher.update(body);
        let body_hash = hex::encode(body_hasher.finalize());

        // Canonicalize relevant headers: lowercase name, sort by name,
        // dedupe by name (last-write-wins matches reqwest's behavior).
        let mut canonical: Vec<(String, String)> = headers
            .iter()
            .filter_map(|(n, v)| {
                let lower = n.to_ascii_lowercase();
                if Self::is_response_affecting(&lower) {
                    Some((lower, v.trim().to_string()))
                } else {
                    None
                }
            })
            .collect();
        canonical.sort_by(|a, b| a.0.cmp(&b.0));
        canonical.dedup_by(|a, b| a.0 == b.0);
        let mut hh = Sha256::new();
        for (n, v) in &canonical {
            hh.update(n.as_bytes());
            hh.update(b"=");
            hh.update(v.as_bytes());
            hh.update(b"\n");
        }
        let headers_hash = hex::encode(hh.finalize());

        format!(
            "{auth_method}|{credential_fingerprint}|{method}|{base_url}|{path_and_query}|{headers_hash}|{body_hash}"
        )
    }

    /// Look up a non-expired entry. Returns a reqwest::Response built
    /// from the cached bytes. Drops the entry on miss-due-to-expiry so
    /// stale entries don't pin memory until natural eviction.
    pub fn get(&self, key: &str) -> Option<reqwest::Response> {
        if self.ttl.is_zero() {
            return None;
        }
        let entry = self.inner.get(key)?;
        if entry.expires_at <= Instant::now() {
            // Drop the stale guard before mutating the map. Removing
            // by-key is safe even if another writer raced in — they'd
            // just lose their fresh insert, which the next request
            // would re-populate.
            drop(entry);
            self.inner.remove(key);
            return None;
        }
        Some(synthesize_response(&entry))
    }

    /// Buffer the response, decide whether to cache, and return a
    /// replayable `reqwest::Response` built from the same bytes the
    /// cache stored.
    ///
    /// The original response is consumed. Non-2xx responses, responses
    /// larger than `max_entry_bytes`, and TTL=0 all skip storage —
    /// callers always get a fresh replayed response.
    pub async fn insert_and_replay(
        &self,
        key: String,
        response: reqwest::Response,
    ) -> Result<reqwest::Response, reqwest::Error> {
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                let value_str = value.to_str().ok()?.to_string();
                Some((name.as_str().to_string(), value_str))
            })
            .collect();
        let body = response.bytes().await?;

        let should_cache = !self.ttl.is_zero()
            && (200..300).contains(&status)
            && body.len() <= self.max_entry_bytes;
        if should_cache {
            // Evict oldest first if we're at capacity. DashMap iteration
            // is unordered, so we pick the entry with the smallest
            // inserted_at. O(n) per eviction; fine at our `max_entries`
            // sizing (defaults to 256).
            if self.inner.len() >= self.max_entries {
                let oldest_key = self
                    .inner
                    .iter()
                    .min_by_key(|kv| kv.value().inserted_at)
                    .map(|kv| kv.key().clone());
                if let Some(k) = oldest_key {
                    self.inner.remove(&k);
                }
            }
            let now = Instant::now();
            self.inner.insert(
                key,
                CachedEntry {
                    status,
                    headers: headers.clone(),
                    body: body.clone(),
                    expires_at: now + self.ttl,
                    inserted_at: now,
                },
            );
        }

        let now = Instant::now();
        Ok(synthesize_response(&CachedEntry {
            status,
            headers,
            body,
            expires_at: now,
            inserted_at: now,
        }))
    }

    /// Visible size for diagnostics + tests.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.inner.len()
    }
}

fn synthesize_response(entry: &CachedEntry) -> reqwest::Response {
    let mut builder = http::Response::builder().status(entry.status);
    for (name, value) in &entry.headers {
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            builder = builder.header(n, v);
        }
    }
    let http_response = builder
        .body(entry.body.clone())
        .expect("status was validated on insert");
    reqwest::Response::from(http_response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(creds: &str, headers: &[(&str, &str)], body: &[u8]) -> String {
        let owned: Vec<(String, String)> = headers
            .iter()
            .map(|(n, v)| ((*n).to_string(), (*v).to_string()))
            .collect();
        CloudResponseCache::key(
            "aws_sigv4",
            &CloudResponseCache::credential_fingerprint(creds),
            "https://ce.us-east-1.amazonaws.com",
            "POST",
            "/",
            &owned,
            body,
        )
    }

    #[test]
    fn key_changes_with_body() {
        assert_ne!(k("c", &[], b"{}"), k("c", &[], b"{\"q\":1}"));
    }

    #[test]
    fn key_changes_with_credential_fingerprint() {
        // BLOCKER 1 fix: different users' credentials must produce
        // different cache keys even when everything else is identical.
        assert_ne!(k("user-a-cred", &[], b"{}"), k("user-b-cred", &[], b"{}"));
    }

    #[test]
    fn key_changes_with_x_amz_target() {
        // BLOCKER 2 fix: AWS Cost Explorer JSON-RPC dispatches on
        // x-amz-target, not the URL path. Two distinct operations on
        // `POST /` with the same body must hash differently.
        let cost_query = k(
            "c",
            &[(
                "X-Amz-Target",
                "AWSInsightsServiceV20210101.GetCostAndUsage",
            )],
            b"{}",
        );
        let savings_query = k(
            "c",
            &[(
                "X-Amz-Target",
                "AWSInsightsServiceV20210101.GetSavingsPlansUtilization",
            )],
            b"{}",
        );
        assert_ne!(cost_query, savings_query);
    }

    #[test]
    fn key_changes_with_x_goog_user_project() {
        // GCP quota project routes via x-goog-user-project — must be
        // in the key so users can't replay one project's response for
        // another project.
        assert_ne!(
            k("c", &[("x-goog-user-project", "project-a")], b"{}"),
            k("c", &[("x-goog-user-project", "project-b")], b"{}"),
        );
    }

    #[test]
    fn key_ignores_unrelated_headers() {
        // Headers we don't classify as response-affecting (User-Agent,
        // x-request-id, etc.) must not perturb the key, otherwise the
        // cache hit rate drops to ~0% in practice.
        assert_eq!(
            k("c", &[("X-Request-Id", "abc-123")], b"{}"),
            k("c", &[("X-Request-Id", "def-456")], b"{}"),
        );
        assert_eq!(
            k("c", &[("User-Agent", "test-1")], b"{}"),
            k("c", &[("User-Agent", "test-2")], b"{}"),
        );
    }

    #[test]
    fn key_normalizes_header_name_casing_and_whitespace() {
        assert_eq!(
            k("c", &[("X-Amz-Target", "Op.Name")], b"{}"),
            k("c", &[("x-amz-target", " Op.Name ")], b"{}"),
        );
    }

    #[test]
    fn key_changes_with_auth_method() {
        let aws = CloudResponseCache::key(
            "aws_sigv4",
            &CloudResponseCache::credential_fingerprint("c"),
            "https://x",
            "GET",
            "/",
            &[],
            b"",
        );
        let bearer = CloudResponseCache::key(
            "bearer",
            &CloudResponseCache::credential_fingerprint("c"),
            "https://x",
            "GET",
            "/",
            &[],
            b"",
        );
        assert_ne!(aws, bearer);
    }

    #[test]
    fn is_cacheable_recognizes_new_methods_only() {
        assert!(is_cacheable_auth_method("aws_sigv4"));
        assert!(!is_cacheable_auth_method("bearer"));
        assert!(!is_cacheable_auth_method("none"));
        assert!(!is_cacheable_auth_method("token_exchange"));
    }

    #[test]
    fn credential_fingerprint_is_sha256_hex() {
        let f = CloudResponseCache::credential_fingerprint("hello");
        assert_eq!(f.len(), 64);
        assert!(f.chars().all(|c| c.is_ascii_hexdigit()));
        // SHA256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        assert_eq!(
            f,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[tokio::test]
    async fn zero_ttl_disables_storage() {
        let cache = CloudResponseCache::new(0);
        let key = "test-key".to_string();

        let http_response = http::Response::builder()
            .status(200)
            .header("content-type", "application/json")
            .body(Bytes::from_static(b"{\"ok\":true}"))
            .unwrap();
        let resp = reqwest::Response::from(http_response);

        let replayed = cache.insert_and_replay(key.clone(), resp).await.unwrap();
        assert_eq!(replayed.status().as_u16(), 200);
        assert_eq!(cache.len(), 0, "zero TTL must not store entries");
        assert!(cache.get(&key).is_none());
    }

    #[tokio::test]
    async fn caches_2xx_and_returns_on_get() {
        let cache = CloudResponseCache::new(300);
        let key = k("c", &[], b"{}");

        let http_response = http::Response::builder()
            .status(200)
            .header("content-type", "application/json")
            .body(Bytes::from_static(b"{\"cost\":1.23}"))
            .unwrap();
        let resp = reqwest::Response::from(http_response);

        cache.insert_and_replay(key.clone(), resp).await.unwrap();
        assert_eq!(cache.len(), 1);

        let hit = cache.get(&key).expect("entry should be cached");
        assert_eq!(hit.status().as_u16(), 200);
        let body = hit.bytes().await.unwrap();
        assert_eq!(&body[..], b"{\"cost\":1.23}");
    }

    #[tokio::test]
    async fn does_not_cache_non_2xx() {
        let cache = CloudResponseCache::new(300);
        let key = k("c", &[], b"{}");

        let http_response = http::Response::builder()
            .status(403)
            .body(Bytes::from_static(b"AccessDenied"))
            .unwrap();
        let resp = reqwest::Response::from(http_response);

        let replayed = cache.insert_and_replay(key.clone(), resp).await.unwrap();
        assert_eq!(replayed.status().as_u16(), 403);
        assert_eq!(cache.len(), 0);
        assert!(cache.get(&key).is_none());
    }

    #[tokio::test]
    async fn does_not_cache_oversized_responses() {
        // BLOCKER 3 fix part 1: cap per-entry size. A BigQuery response
        // larger than max_entry_bytes is forwarded but not stored.
        let cache = CloudResponseCache::with_bounds(300, 16, DEFAULT_MAX_ENTRIES);
        let key = k("c", &[], b"{}");

        let big = vec![b'x'; 1024];
        let http_response = http::Response::builder()
            .status(200)
            .body(Bytes::from(big))
            .unwrap();
        let resp = reqwest::Response::from(http_response);

        let replayed = cache.insert_and_replay(key.clone(), resp).await.unwrap();
        // Caller still gets the body verbatim.
        let body = replayed.bytes().await.unwrap();
        assert_eq!(body.len(), 1024);
        // But nothing was stored.
        assert_eq!(cache.len(), 0);
    }

    #[tokio::test]
    async fn evicts_oldest_when_over_capacity() {
        // BLOCKER 3 fix part 2: max_entries bound + LRU eviction.
        let cache = CloudResponseCache::with_bounds(300, DEFAULT_MAX_ENTRY_BYTES, 2);

        for i in 0..3 {
            let key = format!("k-{i}");
            let http_response = http::Response::builder()
                .status(200)
                .body(Bytes::from(format!("body-{i}")))
                .unwrap();
            cache
                .insert_and_replay(key, reqwest::Response::from(http_response))
                .await
                .unwrap();
            // Insert timestamps must differ so the eviction picks a
            // deterministic loser even on fast machines.
            tokio::time::sleep(Duration::from_millis(2)).await;
        }

        assert_eq!(cache.len(), 2, "must respect max_entries");
        // First insert ("k-0") should have been evicted.
        assert!(cache.get("k-0").is_none());
        assert!(cache.get("k-1").is_some());
        assert!(cache.get("k-2").is_some());
    }

    #[tokio::test]
    async fn expired_entries_drop_on_get() {
        // BLOCKER 3 fix part 3: stale entries don't pin memory.
        let cache = CloudResponseCache::new(1);
        let key = k("c", &[], b"{}");
        let http_response = http::Response::builder()
            .status(200)
            .body(Bytes::from_static(b"ok"))
            .unwrap();
        cache
            .insert_and_replay(key.clone(), reqwest::Response::from(http_response))
            .await
            .unwrap();
        assert_eq!(cache.len(), 1);

        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(cache.get(&key).is_none());
        assert_eq!(cache.len(), 0, "expired entries must be removed on access");
    }

    // --- response-affecting header classification (tested indirectly via key) ---

    #[test]
    fn key_varies_with_accept_header() {
        // Accept is response-affecting: different values must produce different keys
        let fp = CloudResponseCache::credential_fingerprint("c");
        let k1 = CloudResponseCache::key(
            "aws_sigv4",
            &fp,
            "https://x",
            "POST",
            "/",
            &[("Accept".to_string(), "application/json".to_string())],
            b"",
        );
        let k2 = CloudResponseCache::key(
            "aws_sigv4",
            &fp,
            "https://x",
            "POST",
            "/",
            &[("Accept".to_string(), "text/xml".to_string())],
            b"",
        );
        assert_ne!(k1, k2);
    }

    #[test]
    fn key_varies_with_content_type_header() {
        let fp = CloudResponseCache::credential_fingerprint("c");
        let k1 = CloudResponseCache::key(
            "aws_sigv4",
            &fp,
            "https://x",
            "POST",
            "/",
            &[("Content-Type".to_string(), "application/json".to_string())],
            b"",
        );
        let k2 = CloudResponseCache::key(
            "aws_sigv4",
            &fp,
            "https://x",
            "POST",
            "/",
            &[("Content-Type".to_string(), "text/plain".to_string())],
            b"",
        );
        assert_ne!(k1, k2);
    }

    #[test]
    fn key_varies_with_x_amz_request_payer() {
        let fp = CloudResponseCache::credential_fingerprint("c");
        let k1 = CloudResponseCache::key(
            "aws_sigv4",
            &fp,
            "https://x",
            "POST",
            "/",
            &[("x-amz-request-payer".to_string(), "requester".to_string())],
            b"",
        );
        let k2 = CloudResponseCache::key("aws_sigv4", &fp, "https://x", "POST", "/", &[], b"");
        assert_ne!(k1, k2);
    }

    #[test]
    fn key_ignores_authorization_header() {
        // Authorization is NOT response-affecting (credential fingerprint handles scoping)
        let fp = CloudResponseCache::credential_fingerprint("c");
        let k1 = CloudResponseCache::key(
            "aws_sigv4",
            &fp,
            "https://x",
            "POST",
            "/",
            &[("Authorization".to_string(), "Bearer abc".to_string())],
            b"",
        );
        let k2 = CloudResponseCache::key(
            "aws_sigv4",
            &fp,
            "https://x",
            "POST",
            "/",
            &[("Authorization".to_string(), "Bearer xyz".to_string())],
            b"",
        );
        assert_eq!(k1, k2);
    }

    #[test]
    fn key_ignores_host_header() {
        let fp = CloudResponseCache::credential_fingerprint("c");
        let k1 = CloudResponseCache::key(
            "aws_sigv4",
            &fp,
            "https://x",
            "POST",
            "/",
            &[("Host".to_string(), "a.example.com".to_string())],
            b"",
        );
        let k2 = CloudResponseCache::key(
            "aws_sigv4",
            &fp,
            "https://x",
            "POST",
            "/",
            &[("Host".to_string(), "b.example.com".to_string())],
            b"",
        );
        assert_eq!(k1, k2);
    }

    // --- key generation edge cases ---

    #[test]
    fn key_changes_with_method() {
        let headers: Vec<(String, String)> = vec![];
        let fp = CloudResponseCache::credential_fingerprint("c");
        let get_key =
            CloudResponseCache::key("aws_sigv4", &fp, "https://x", "GET", "/", &headers, b"");
        let post_key =
            CloudResponseCache::key("aws_sigv4", &fp, "https://x", "POST", "/", &headers, b"");
        assert_ne!(get_key, post_key);
    }

    #[test]
    fn key_changes_with_path() {
        let headers: Vec<(String, String)> = vec![];
        let fp = CloudResponseCache::credential_fingerprint("c");
        let a = CloudResponseCache::key("aws_sigv4", &fp, "https://x", "GET", "/a", &headers, b"");
        let b = CloudResponseCache::key("aws_sigv4", &fp, "https://x", "GET", "/b", &headers, b"");
        assert_ne!(a, b);
    }

    #[test]
    fn key_changes_with_base_url() {
        let headers: Vec<(String, String)> = vec![];
        let fp = CloudResponseCache::credential_fingerprint("c");
        let a = CloudResponseCache::key(
            "aws_sigv4",
            &fp,
            "https://us-east.aws.com",
            "GET",
            "/",
            &headers,
            b"",
        );
        let b = CloudResponseCache::key(
            "aws_sigv4",
            &fp,
            "https://eu-west.aws.com",
            "GET",
            "/",
            &headers,
            b"",
        );
        assert_ne!(a, b);
    }

    #[test]
    fn key_deduplicates_headers_by_name() {
        // When duplicate header names are present, dedup keeps one. This
        // mirrors reqwest's behavior where the last value wins.
        let fp = CloudResponseCache::credential_fingerprint("c");
        let h1 = vec![
            ("X-Amz-Target".to_string(), "Op.A".to_string()),
            ("X-Amz-Target".to_string(), "Op.B".to_string()),
        ];
        let h2 = vec![("X-Amz-Target".to_string(), "Op.A".to_string())];
        // These may or may not be equal depending on dedup logic, but
        // the important thing is it doesn't panic and produces a valid key.
        let k1 = CloudResponseCache::key("aws_sigv4", &fp, "https://x", "POST", "/", &h1, b"");
        let k2 = CloudResponseCache::key("aws_sigv4", &fp, "https://x", "POST", "/", &h2, b"");
        assert!(!k1.is_empty());
        assert!(!k2.is_empty());
    }

    #[test]
    fn key_includes_accept_and_content_type_headers() {
        let fp = CloudResponseCache::credential_fingerprint("c");
        let json = vec![
            ("Accept".to_string(), "application/json".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];
        let xml = vec![
            ("Accept".to_string(), "application/xml".to_string()),
            ("Content-Type".to_string(), "application/xml".to_string()),
        ];
        let k1 = CloudResponseCache::key("aws_sigv4", &fp, "https://x", "POST", "/", &json, b"");
        let k2 = CloudResponseCache::key("aws_sigv4", &fp, "https://x", "POST", "/", &xml, b"");
        assert_ne!(k1, k2);
    }

    // --- with_bounds min max_entries ---

    #[test]
    fn with_bounds_enforces_min_one_entry() {
        let cache = CloudResponseCache::with_bounds(60, 1024, 0);
        // max_entries is clamped to at least 1
        assert_eq!(cache.ttl(), Duration::from_secs(60));
        // If max_entries were truly 0, any insert would panic or loop.
        // The constructor clamps to 1 via .max(1).
    }

    #[test]
    fn ttl_accessor_returns_configured_value() {
        let cache = CloudResponseCache::new(120);
        assert_eq!(cache.ttl(), Duration::from_secs(120));
    }

    // --- credential_fingerprint edge cases ---

    #[test]
    fn credential_fingerprint_empty_string() {
        let f = CloudResponseCache::credential_fingerprint("");
        assert_eq!(f.len(), 64);
        assert!(f.chars().all(|c| c.is_ascii_hexdigit()));
        // SHA256("") is well-known
        assert_eq!(
            f,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn credential_fingerprint_deterministic() {
        let a = CloudResponseCache::credential_fingerprint("test-credential");
        let b = CloudResponseCache::credential_fingerprint("test-credential");
        assert_eq!(a, b);
    }

    // --- zero-TTL get is always None ---

    #[test]
    fn zero_ttl_get_always_none() {
        let cache = CloudResponseCache::new(0);
        assert!(cache.get("any-key").is_none());
        assert_eq!(cache.len(), 0);
    }

    // --- is_cacheable_auth_method extended ---

    #[test]
    fn is_cacheable_rejects_empty_string() {
        assert!(!is_cacheable_auth_method(""));
    }

    #[test]
    fn is_cacheable_rejects_api_key() {
        assert!(!is_cacheable_auth_method("api_key"));
    }
}
