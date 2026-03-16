use hmac::{Hmac, Mac};
use sha2::Sha256;

/// Maximum allowed timestamp skew in seconds.
/// Requests older than this are rejected for replay protection.
const MAX_TIMESTAMP_SKEW_SECS: i64 = 300;

/// Maximum number of nonces to track for replay protection.
const MAX_NONCE_SET_SIZE: usize = 10_000;

/// Replay protection state: tracks recently seen nonces.
pub struct ReplayGuard {
    /// Set of (nonce, timestamp) pairs
    seen: std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
}

impl ReplayGuard {
    pub fn new() -> Self {
        Self {
            seen: std::collections::HashMap::new(),
        }
    }

    /// Check if a request should be accepted based on timestamp and nonce.
    /// Returns `true` if the request is valid (not replayed), `false` if it should be rejected.
    pub fn check(&mut self, timestamp: &str, nonce: &str) -> bool {
        let request_time = match chrono::DateTime::parse_from_rfc3339(timestamp) {
            Ok(t) => t.with_timezone(&chrono::Utc),
            Err(_) => return false,
        };

        let now = chrono::Utc::now();
        let skew = (now - request_time).num_seconds().abs();

        if skew > MAX_TIMESTAMP_SKEW_SECS {
            return false;
        }

        // Evict old nonces before checking
        self.evict_old_nonces();

        // Check for duplicate nonce
        if self.seen.contains_key(nonce) {
            return false;
        }

        self.seen.insert(nonce.to_string(), request_time);
        true
    }

    /// Remove nonces older than the timestamp skew window.
    /// Always runs time-based eviction, then enforces a hard cap to prevent
    /// unbounded memory growth under high request rates.
    fn evict_old_nonces(&mut self) {
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(MAX_TIMESTAMP_SKEW_SECS);
        self.seen.retain(|_, ts| *ts > cutoff);

        // Hard cap: if still over max after time-based eviction, drop oldest entries
        if self.seen.len() > MAX_NONCE_SET_SIZE {
            let mut entries: Vec<(String, chrono::DateTime<chrono::Utc>)> =
                self.seen.drain().collect();
            entries.sort_by_key(|(_, ts)| *ts);
            let keep_from = entries.len() - MAX_NONCE_SET_SIZE;
            self.seen = entries.into_iter().skip(keep_from).collect();
        }
    }
}

/// Verify the HMAC-SHA256 signature on a proxy request.
pub fn verify_request_signature(
    request: &serde_json::Value,
    secret_hex: &str,
    expected_signature: &str,
) -> bool {
    let secret_bytes = match hex::decode(secret_hex) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let timestamp = request["timestamp"].as_str().unwrap_or("");
    let nonce = request["nonce"].as_str().unwrap_or("");
    let method = request["method"].as_str().unwrap_or("");
    let path = request["path"].as_str().unwrap_or("");
    let query = request["query"].as_str().unwrap_or("");
    let body = request["body"].as_str().unwrap_or("");

    let message = format!("{timestamp}\n{nonce}\n{method}\n{path}\n{query}\n{body}");

    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(&secret_bytes) else {
        return false;
    };
    mac.update(message.as_bytes());

    let expected_bytes = match hex::decode(expected_signature) {
        Ok(b) => b,
        Err(_) => return false,
    };

    // Constant-time comparison
    mac.verify_slice(&expected_bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compute_signature(secret_hex: &str, request: &serde_json::Value) -> String {
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let timestamp = request["timestamp"].as_str().unwrap_or("");
        let nonce = request["nonce"].as_str().unwrap_or("");
        let method = request["method"].as_str().unwrap_or("");
        let path = request["path"].as_str().unwrap_or("");
        let query = request["query"].as_str().unwrap_or("");
        let body = request["body"].as_str().unwrap_or("");

        let message = format!("{timestamp}\n{nonce}\n{method}\n{path}\n{query}\n{body}");

        let mut mac = Hmac::<Sha256>::new_from_slice(&secret_bytes).unwrap();
        mac.update(message.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    #[test]
    fn valid_signature_passes() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "method": "POST",
            "path": "/v1/chat/completions",
            "query": "",
            "body": "dGVzdA==",
        });

        let sig = compute_signature(&secret, &request);
        assert!(verify_request_signature(&request, &secret, &sig));
    }

    #[test]
    fn tampered_body_fails() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "method": "POST",
            "path": "/v1/chat/completions",
            "query": "",
            "body": "dGVzdA==",
        });

        let sig = compute_signature(&secret, &request);

        let tampered = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "method": "POST",
            "path": "/v1/chat/completions",
            "query": "",
            "body": "dGFtcGVyZWQ=",
        });

        assert!(!verify_request_signature(&tampered, &secret, &sig));
    }

    #[test]
    fn wrong_secret_fails() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "method": "GET",
            "path": "/health",
        });

        let sig = compute_signature(&secret, &request);
        let wrong_secret = "cd".repeat(32);
        assert!(!verify_request_signature(&request, &wrong_secret, &sig));
    }

    #[test]
    fn replay_guard_accepts_fresh_request() {
        let mut guard = ReplayGuard::new();
        let ts = chrono::Utc::now().to_rfc3339();
        assert!(guard.check(&ts, "nonce-1"));
    }

    #[test]
    fn replay_guard_rejects_duplicate_nonce() {
        let mut guard = ReplayGuard::new();
        let ts = chrono::Utc::now().to_rfc3339();
        assert!(guard.check(&ts, "nonce-1"));
        assert!(!guard.check(&ts, "nonce-1"));
    }

    #[test]
    fn replay_guard_rejects_old_timestamp() {
        let mut guard = ReplayGuard::new();
        let old = (chrono::Utc::now() - chrono::Duration::seconds(600)).to_rfc3339();
        assert!(!guard.check(&old, "nonce-old"));
    }
}
