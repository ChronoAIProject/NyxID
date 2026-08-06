//! Continuous Access Evaluation webhook delivery for OAuth broker
//! binding revocations.
//!
//! Delivery is intentionally best-effort: revoke commits are never rolled
//! back if the receiver is unavailable. The webhook is HMAC-SHA256 signed so
//! clients can verify the event came from NyxID and was not tampered with.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::models::oauth_client::OauthClient;
use crate::services::webhook_delivery_service::{self, SignatureContract};

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
        match webhook_delivery_service::deliver_signed_body(
            &http_client,
            &url,
            raw_hmac_secret.as_bytes(),
            event.event_type,
            &delivery_id,
            &body,
            SignatureContract::BodyOnly,
        )
        .await
        {
            Ok(()) => tracing::debug!(
                delivery_id = %delivery_id,
                client_id = %event.client_id,
                "CAE webhook delivered"
            ),
            Err(failure) => tracing::error!(
                delivery_id = %delivery_id,
                client_id = %event.client_id,
                attempts = failure.attempts,
                reason = failure.reason,
                last_status = failure.last_status,
                "CAE webhook delivery exhausted retries"
            ),
        }
    });
}

#[cfg(test)]
fn compute_signature(secret: &str, body: &[u8]) -> String {
    webhook_delivery_service::compute_body_signature(secret.as_bytes(), body)
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
