use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

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

impl Default for ReplayGuard {
    fn default() -> Self {
        Self::new()
    }
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
    let timestamp = request["timestamp"].as_str().unwrap_or("");
    let nonce = request["nonce"].as_str().unwrap_or("");
    let method = request["method"].as_str().unwrap_or("");
    let path = request["path"].as_str().unwrap_or("");
    let query = request["query"].as_str().unwrap_or("");
    let body = request["body"].as_str().unwrap_or("");

    let message = format!("{timestamp}\n{nonce}\n{method}\n{path}\n{query}\n{body}");
    verify_signature(secret_hex, expected_signature, &message)
}

/// Verify the HMAC-SHA256 signature on an SSH tunnel open request.
pub fn verify_ssh_tunnel_signature(
    request: &serde_json::Value,
    secret_hex: &str,
    expected_signature: &str,
) -> bool {
    let timestamp = request["timestamp"].as_str().unwrap_or("");
    let nonce = request["nonce"].as_str().unwrap_or("");
    let session_id = request["session_id"].as_str().unwrap_or("");
    let service_id = request["service_id"].as_str().unwrap_or("");
    let host = request["host"].as_str().unwrap_or("");
    let port = request["port"]
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .map(|value| value.to_string())
        .unwrap_or_default();

    let message = format!("{timestamp}\n{nonce}\n{session_id}\n{service_id}\n{host}\n{port}");
    verify_signature(secret_hex, expected_signature, &message)
}

/// Verify the HMAC-SHA256 signature on an SSH exec request.
/// Message format:
/// `{timestamp}\n{nonce}\n{request_id}\n{host}\n{port}\n{principal}\n{auth_mode}\n{sha256(command)}\n{sha256(certificate_openssh)}`
pub fn verify_ssh_exec_signature(
    request: &serde_json::Value,
    secret_hex: &str,
    expected_signature: &str,
) -> bool {
    let timestamp = request["timestamp"].as_str().unwrap_or("");
    let nonce = request["nonce"].as_str().unwrap_or("");
    let request_id = request["request_id"].as_str().unwrap_or("");
    let host = request["host"].as_str().unwrap_or("");
    let port = request["port"]
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .map(|value| value.to_string())
        .unwrap_or_default();
    let principal = request["principal"].as_str().unwrap_or("");
    let auth_mode = request["auth_mode"].as_str().unwrap_or("");
    let command = request["command"].as_str().unwrap_or("");
    let certificate_openssh = request["certificate_openssh"].as_str().unwrap_or("");
    let command_hash = hex::encode(Sha256::digest(command.as_bytes()));
    let certificate_hash = hex::encode(Sha256::digest(certificate_openssh.as_bytes()));

    let message = format!(
        "{timestamp}\n{nonce}\n{request_id}\n{host}\n{port}\n{principal}\n{auth_mode}\n{command_hash}\n{certificate_hash}"
    );
    verify_signature(secret_hex, expected_signature, &message)
}

/// Verify the HMAC-SHA256 signature on an SSH node-key exec request.
/// Message format:
/// `{timestamp}\n{nonce}\n{request_id}\n{service_slug}\n{principal}\n{auth_mode}\n{sha256(command)}`
pub fn verify_ssh_node_exec_signature(
    request: &serde_json::Value,
    secret_hex: &str,
    expected_signature: &str,
) -> bool {
    let timestamp = request["timestamp"].as_str().unwrap_or("");
    let nonce = request["nonce"].as_str().unwrap_or("");
    let request_id = request["request_id"].as_str().unwrap_or("");
    let service_slug = request["service_slug"].as_str().unwrap_or("");
    let principal = request["principal"].as_str().unwrap_or("");
    let auth_mode = request["auth_mode"].as_str().unwrap_or("");
    let command = request["command"].as_str().unwrap_or("");
    let command_hash = hex::encode(Sha256::digest(command.as_bytes()));

    let message = format!(
        "{timestamp}\n{nonce}\n{request_id}\n{service_slug}\n{principal}\n{auth_mode}\n{command_hash}"
    );
    verify_signature(secret_hex, expected_signature, &message)
}

/// Verify the HMAC-SHA256 signature on a web terminal open request.
/// Message format:
/// `{timestamp}\n{nonce}\n{session_id}\n{host}\n{port}\n{principal}\n{auth_mode}\n{service_slug}\n{sha256(auth_material)}`
pub fn verify_web_terminal_signature(
    request: &serde_json::Value,
    secret_hex: &str,
    expected_signature: &str,
) -> bool {
    let timestamp = request["timestamp"].as_str().unwrap_or("");
    let nonce = request["nonce"].as_str().unwrap_or("");
    let session_id = request["session_id"].as_str().unwrap_or("");
    let host = request["host"].as_str().unwrap_or("");
    let port = request["port"]
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .map(|value| value.to_string())
        .unwrap_or_default();
    let principal = request["principal"].as_str().unwrap_or("");
    let auth_mode = request["auth_mode"].as_str().unwrap_or("");
    let service_slug = request["service_slug"].as_str().unwrap_or("");
    let auth_material = if auth_mode == "cert" {
        request["certificate_openssh"].as_str().unwrap_or("")
    } else {
        ""
    };
    let auth_material_hash = hex::encode(Sha256::digest(auth_material.as_bytes()));

    let message = format!(
        "{timestamp}\n{nonce}\n{session_id}\n{host}\n{port}\n{principal}\n{auth_mode}\n{service_slug}\n{auth_material_hash}"
    );
    verify_signature(secret_hex, expected_signature, &message)
}

/// Verify the HMAC-SHA256 signature on a WS proxy open request.
/// Message format:
/// `{timestamp}\n{nonce}\n{session_id}\n{service_slug}\n{base_url}\n{path}\n{query}`
pub fn verify_ws_proxy_signature(
    request: &serde_json::Value,
    secret_hex: &str,
    expected_signature: &str,
) -> bool {
    let timestamp = request["timestamp"].as_str().unwrap_or("");
    let nonce = request["nonce"].as_str().unwrap_or("");
    let session_id = request["session_id"].as_str().unwrap_or("");
    let service_slug = request["service_slug"].as_str().unwrap_or("");
    let base_url = request["base_url"].as_str().unwrap_or("");
    let path = request["path"].as_str().unwrap_or("");
    let query = request["query"].as_str().unwrap_or("");

    let message =
        format!("{timestamp}\n{nonce}\n{session_id}\n{service_slug}\n{base_url}\n{path}\n{query}");
    verify_signature(secret_hex, expected_signature, &message)
}

fn verify_signature(secret_hex: &str, expected_signature: &str, message: &str) -> bool {
    let secret_bytes = match hex::decode(secret_hex) {
        Ok(b) => b,
        Err(_) => return false,
    };

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
        let timestamp = request["timestamp"].as_str().unwrap_or("");
        let nonce = request["nonce"].as_str().unwrap_or("");
        let method = request["method"].as_str().unwrap_or("");
        let path = request["path"].as_str().unwrap_or("");
        let query = request["query"].as_str().unwrap_or("");
        let body = request["body"].as_str().unwrap_or("");

        compute_signature_for_message(
            secret_hex,
            &format!("{timestamp}\n{nonce}\n{method}\n{path}\n{query}\n{body}"),
        )
    }

    fn compute_ssh_tunnel_signature(secret_hex: &str, request: &serde_json::Value) -> String {
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let timestamp = request["timestamp"].as_str().unwrap_or("");
        let nonce = request["nonce"].as_str().unwrap_or("");
        let session_id = request["session_id"].as_str().unwrap_or("");
        let service_id = request["service_id"].as_str().unwrap_or("");
        let host = request["host"].as_str().unwrap_or("");
        let port = request["port"]
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .map(|value| value.to_string())
            .unwrap_or_default();

        let message = format!("{timestamp}\n{nonce}\n{session_id}\n{service_id}\n{host}\n{port}");

        let mut mac = Hmac::<Sha256>::new_from_slice(&secret_bytes).unwrap();
        mac.update(message.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    fn compute_signature_for_message(secret_hex: &str, message: &str) -> String {
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let mut mac = Hmac::<Sha256>::new_from_slice(&secret_bytes).unwrap();
        mac.update(message.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    fn compute_ws_proxy_signature(secret_hex: &str, request: &serde_json::Value) -> String {
        let timestamp = request["timestamp"].as_str().unwrap_or("");
        let nonce = request["nonce"].as_str().unwrap_or("");
        let session_id = request["session_id"].as_str().unwrap_or("");
        let service_slug = request["service_slug"].as_str().unwrap_or("");
        let base_url = request["base_url"].as_str().unwrap_or("");
        let path = request["path"].as_str().unwrap_or("");
        let query = request["query"].as_str().unwrap_or("");

        compute_signature_for_message(
            secret_hex,
            &format!(
                "{timestamp}\n{nonce}\n{session_id}\n{service_slug}\n{base_url}\n{path}\n{query}"
            ),
        )
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
    fn valid_ssh_tunnel_signature_passes() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "session_id": "sess-1",
            "service_id": "svc-1",
            "host": "ssh.internal",
            "port": 22,
        });

        let sig = compute_ssh_tunnel_signature(&secret, &request);
        assert!(verify_ssh_tunnel_signature(&request, &secret, &sig));
    }

    #[test]
    fn tampered_ssh_tunnel_signature_fails() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "session_id": "sess-1",
            "service_id": "svc-1",
            "host": "ssh.internal",
            "port": 22,
        });

        let sig = compute_ssh_tunnel_signature(&secret, &request);
        let tampered = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "session_id": "sess-1",
            "service_id": "svc-1",
            "host": "other.internal",
            "port": 22,
        });

        assert!(!verify_ssh_tunnel_signature(&tampered, &secret, &sig));
    }

    fn compute_web_terminal_signature(secret_hex: &str, request: &serde_json::Value) -> String {
        let timestamp = request["timestamp"].as_str().unwrap_or("");
        let nonce = request["nonce"].as_str().unwrap_or("");
        let session_id = request["session_id"].as_str().unwrap_or("");
        let host = request["host"].as_str().unwrap_or("");
        let port = request["port"]
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .map(|value| value.to_string())
            .unwrap_or_default();
        let principal = request["principal"].as_str().unwrap_or("");
        let auth_mode = request["auth_mode"].as_str().unwrap_or("");
        let service_slug = request["service_slug"].as_str().unwrap_or("");
        let auth_material = if auth_mode == "cert" {
            request["certificate_openssh"].as_str().unwrap_or("")
        } else {
            ""
        };
        let auth_material_hash = hex::encode(Sha256::digest(auth_material.as_bytes()));

        compute_signature_for_message(
            secret_hex,
            &format!(
                "{timestamp}\n{nonce}\n{session_id}\n{host}\n{port}\n{principal}\n{auth_mode}\n{service_slug}\n{auth_material_hash}"
            ),
        )
    }

    #[test]
    fn valid_web_terminal_signature_passes() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "session_id": "wt-sess-1",
            "host": "10.0.0.5",
            "port": 22,
            "principal": "ubuntu",
            "auth_mode": "cert",
            "service_slug": "linux-host",
            "certificate_openssh": "ssh-rsa-cert-v01@openssh.com AAAATEST user-cert",
        });

        let sig = compute_web_terminal_signature(&secret, &request);
        assert!(verify_web_terminal_signature(&request, &secret, &sig));
    }

    #[test]
    fn tampered_web_terminal_signature_fails() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "session_id": "wt-sess-1",
            "host": "10.0.0.5",
            "port": 22,
            "principal": "ubuntu",
            "auth_mode": "cert",
            "service_slug": "linux-host",
            "certificate_openssh": "ssh-rsa-cert-v01@openssh.com AAAATEST user-cert",
        });

        let sig = compute_web_terminal_signature(&secret, &request);
        let tampered = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "session_id": "wt-sess-1",
            "host": "10.0.0.5",
            "port": 22,
            "principal": "root",
            "auth_mode": "cert",
            "service_slug": "linux-host",
            "certificate_openssh": "ssh-rsa-cert-v01@openssh.com AAAATEST user-cert",
        });

        assert!(!verify_web_terminal_signature(&tampered, &secret, &sig));
    }

    fn compute_ssh_exec_signature(secret_hex: &str, request: &serde_json::Value) -> String {
        let timestamp = request["timestamp"].as_str().unwrap_or("");
        let nonce = request["nonce"].as_str().unwrap_or("");
        let request_id = request["request_id"].as_str().unwrap_or("");
        let host = request["host"].as_str().unwrap_or("");
        let port = request["port"]
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .map(|value| value.to_string())
            .unwrap_or_default();
        let principal = request["principal"].as_str().unwrap_or("");
        let auth_mode = request["auth_mode"].as_str().unwrap_or("");
        let command = request["command"].as_str().unwrap_or("");
        let certificate_openssh = request["certificate_openssh"].as_str().unwrap_or("");
        let command_hash = hex::encode(Sha256::digest(command.as_bytes()));
        let certificate_hash = hex::encode(Sha256::digest(certificate_openssh.as_bytes()));

        compute_signature_for_message(
            secret_hex,
            &format!(
                "{timestamp}\n{nonce}\n{request_id}\n{host}\n{port}\n{principal}\n{auth_mode}\n{command_hash}\n{certificate_hash}"
            ),
        )
    }

    fn compute_ssh_node_exec_signature(secret_hex: &str, request: &serde_json::Value) -> String {
        let timestamp = request["timestamp"].as_str().unwrap_or("");
        let nonce = request["nonce"].as_str().unwrap_or("");
        let request_id = request["request_id"].as_str().unwrap_or("");
        let service_slug = request["service_slug"].as_str().unwrap_or("");
        let principal = request["principal"].as_str().unwrap_or("");
        let auth_mode = request["auth_mode"].as_str().unwrap_or("");
        let command = request["command"].as_str().unwrap_or("");
        let command_hash = hex::encode(Sha256::digest(command.as_bytes()));

        compute_signature_for_message(
            secret_hex,
            &format!(
                "{timestamp}\n{nonce}\n{request_id}\n{service_slug}\n{principal}\n{auth_mode}\n{command_hash}"
            ),
        )
    }

    #[test]
    fn valid_ssh_exec_signature_passes() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "request_id": "req-exec-1",
            "host": "10.0.0.5",
            "port": 22,
            "principal": "ubuntu",
            "auth_mode": "cert",
            "command": "uptime",
            "certificate_openssh": "ssh-rsa-cert-v01@openssh.com AAAATEST user-cert",
        });

        let sig = compute_ssh_exec_signature(&secret, &request);
        assert!(verify_ssh_exec_signature(&request, &secret, &sig));
    }

    #[test]
    fn tampered_ssh_exec_signature_fails() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "request_id": "req-exec-1",
            "host": "10.0.0.5",
            "port": 22,
            "principal": "ubuntu",
            "auth_mode": "cert",
            "command": "uptime",
            "certificate_openssh": "ssh-rsa-cert-v01@openssh.com AAAATEST user-cert",
        });

        let sig = compute_ssh_exec_signature(&secret, &request);
        let tampered = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "request_id": "req-exec-1",
            "host": "10.0.0.5",
            "port": 22,
            "principal": "root",
            "auth_mode": "cert",
            "command": "uptime",
            "certificate_openssh": "ssh-rsa-cert-v01@openssh.com AAAATEST user-cert",
        });

        assert!(!verify_ssh_exec_signature(&tampered, &secret, &sig));
    }

    #[test]
    fn tampered_ssh_exec_command_replay_fails() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "request_id": "req-exec-1",
            "host": "10.0.0.5",
            "port": 22,
            "principal": "ubuntu",
            "auth_mode": "cert",
            "command": "A",
            "certificate_openssh": "ssh-rsa-cert-v01@openssh.com AAAATEST user-cert",
        });

        let sig = compute_ssh_exec_signature(&secret, &request);
        let tampered = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "request_id": "req-exec-1",
            "host": "10.0.0.5",
            "port": 22,
            "principal": "ubuntu",
            "auth_mode": "cert",
            "command": "B",
            "certificate_openssh": "ssh-rsa-cert-v01@openssh.com AAAATEST user-cert",
        });

        assert!(!verify_ssh_exec_signature(&tampered, &secret, &sig));
    }

    #[test]
    fn valid_ssh_node_exec_signature_passes() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "request_id": "req-exec-1",
            "service_slug": "routeros",
            "principal": "nyxid-ro",
            "auth_mode": "node_key",
            "command": "/system identity print",
        });

        let sig = compute_ssh_node_exec_signature(&secret, &request);
        assert!(verify_ssh_node_exec_signature(&request, &secret, &sig));
    }

    #[test]
    fn tampered_ssh_node_exec_signature_fails() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "request_id": "req-exec-1",
            "service_slug": "routeros",
            "principal": "nyxid-ro",
            "auth_mode": "node_key",
            "command": "/system identity print",
        });

        let sig = compute_ssh_node_exec_signature(&secret, &request);
        let tampered = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "request_id": "req-exec-1",
            "service_slug": "routeros",
            "principal": "nyxid-admin",
            "auth_mode": "node_key",
            "command": "/system identity print",
        });

        assert!(!verify_ssh_node_exec_signature(&tampered, &secret, &sig));
    }

    #[test]
    fn tampered_ssh_node_exec_command_fails() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "request_id": "req-exec-1",
            "service_slug": "routeros",
            "principal": "nyxid-ro",
            "auth_mode": "node_key",
            "command": "/system identity print",
        });

        let sig = compute_ssh_node_exec_signature(&secret, &request);
        let tampered = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "request_id": "req-exec-1",
            "service_slug": "routeros",
            "principal": "nyxid-ro",
            "auth_mode": "node_key",
            "command": "/system reboot",
        });

        assert!(!verify_ssh_node_exec_signature(&tampered, &secret, &sig));
    }

    #[test]
    fn valid_ws_proxy_signature_passes() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "session_id": "ws-sess-1",
            "service_slug": "openclaw",
            "base_url": "https://gateway.example.com",
            "path": "/socket",
            "query": "stream=true&api_key=abc",
        });

        let sig = compute_ws_proxy_signature(&secret, &request);
        assert!(verify_ws_proxy_signature(&request, &secret, &sig));
    }

    #[test]
    fn tampered_ws_proxy_query_fails() {
        let secret = "ab".repeat(32);
        let request = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "session_id": "ws-sess-1",
            "service_slug": "openclaw",
            "base_url": "https://gateway.example.com",
            "path": "/socket",
            "query": "stream=true&api_key=abc",
        });

        let sig = compute_ws_proxy_signature(&secret, &request);
        let tampered = serde_json::json!({
            "timestamp": "2026-03-12T10:30:00.000Z",
            "nonce": "test-nonce",
            "session_id": "ws-sess-1",
            "service_slug": "openclaw",
            "base_url": "https://gateway.example.com",
            "path": "/socket",
            "query": "stream=false&api_key=abc",
        });

        assert!(!verify_ws_proxy_signature(&tampered, &secret, &sig));
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
