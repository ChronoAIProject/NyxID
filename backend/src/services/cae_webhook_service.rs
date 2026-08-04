//! Continuous Access Evaluation webhook delivery for OAuth broker
//! binding revocations.
//!
//! Delivery is intentionally best-effort: revoke commits are never rolled
//! back if the receiver is unavailable. The webhook is HMAC-SHA256 signed so
//! clients can verify the event came from NyxID and was not tampered with.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::models::oauth_client::OauthClient;

const MAX_ATTEMPTS: u32 = 3;
const BASE_BACKOFF_MS: u64 = 1_000;
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Serialize, Clone)]
pub struct RevocationEvent {
    pub event_type: &'static str,
    pub binding_hash: String,
    pub client_id: String,
    pub revoke_source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub revoked_at: DateTime<Utc>,
}

impl RevocationEvent {
    pub fn new_at(
        binding_hash: String,
        client_id: String,
        revoke_source: &str,
        reason: Option<String>,
        revoked_at: DateTime<Utc>,
    ) -> Self {
        Self {
            event_type: "oauth_broker_binding.revoked",
            binding_hash,
            client_id,
            revoke_source: revoke_source.to_string(),
            reason,
            revoked_at,
        }
    }
}

/// Spawn a background task that delivers `event` to
/// `client.revocation_webhook_url` if webhook delivery is enabled.
///
/// `raw_hmac_secret` is the client's raw webhook secret, decrypted by the
/// caller immediately before dispatch. The secret is never logged.
pub fn dispatch_revocation_event(
    http_client: reqwest::Client,
    client: OauthClient,
    raw_hmac_secret: String,
    event: RevocationEvent,
) {
    let url = match client.revocation_webhook_url.clone() {
        Some(url) if !url.trim().is_empty() => url,
        _ => return,
    };
    if raw_hmac_secret.is_empty() {
        return;
    }
    // Wrap the raw HMAC key so the String buffer is zeroed when the spawned
    // task drops it. Best-effort -- the String allocation may be reused by
    // the allocator before the Drop runs, but this closes the obvious window.
    let raw_hmac_secret = Zeroizing::new(raw_hmac_secret);
    let delivery_id = Uuid::new_v4().to_string();

    tokio::spawn(async move {
        let body = match serde_json::to_vec(&event) {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(error = %error, "failed to serialize CAE event");
                return;
            }
        };
        let signature = compute_signature(&raw_hmac_secret, &body);

        for attempt in 0..MAX_ATTEMPTS {
            let request = http_client
                .post(&url)
                .timeout(HTTP_TIMEOUT)
                .header("Content-Type", "application/json")
                .header("X-NyxID-Event", event.event_type)
                .header("X-NyxID-Delivery-Id", &delivery_id)
                .header("X-NyxID-Signature", format!("sha256={signature}"))
                .body(body.clone());

            match request.send().await {
                Ok(response) if response.status().is_success() => {
                    tracing::debug!(
                        delivery_id = %delivery_id,
                        client_id = %event.client_id,
                        attempt = attempt + 1,
                        "CAE webhook delivered"
                    );
                    return;
                }
                Ok(response) => {
                    tracing::warn!(
                        delivery_id = %delivery_id,
                        status = %response.status(),
                        attempt = attempt + 1,
                        "CAE webhook returned non-2xx"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        delivery_id = %delivery_id,
                        error = %error,
                        attempt = attempt + 1,
                        "CAE webhook send failed"
                    );
                }
            }

            if attempt + 1 < MAX_ATTEMPTS {
                let backoff_ms = BASE_BACKOFF_MS * 4_u64.pow(attempt);
                sleep(Duration::from_millis(backoff_ms)).await;
            }
        }

        tracing::error!(
            delivery_id = %delivery_id,
            client_id = %event.client_id,
            "CAE webhook delivery exhausted retries"
        );
    });
}

fn compute_signature(secret: &str, body: &[u8]) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any length key");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_stable() {
        let s1 = compute_signature("secret", b"payload");
        let s2 = compute_signature("secret", b"payload");
        assert_eq!(s1, s2);
        assert_eq!(s1.len(), 64);
    }

    #[test]
    fn different_secrets_yield_different_signatures() {
        let s1 = compute_signature("a", b"payload");
        let s2 = compute_signature("b", b"payload");
        assert_ne!(s1, s2);
    }

    #[test]
    fn event_serializes_with_required_fields() {
        let event = RevocationEvent::new_at(
            "abcdef".to_string(),
            "client-x".to_string(),
            "user",
            Some("user_revoked".to_string()),
            Utc::now(),
        );
        let json = serde_json::to_value(&event).expect("serialize event");
        assert_eq!(json["event_type"], "oauth_broker_binding.revoked");
        assert_eq!(json["binding_hash"], "abcdef");
        assert_eq!(json["client_id"], "client-x");
        assert_eq!(json["revoke_source"], "user");
        assert_eq!(json["reason"], "user_revoked");
        assert!(json["revoked_at"].is_string());
    }
}
