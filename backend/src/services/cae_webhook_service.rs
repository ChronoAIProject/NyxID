//! Continuous Access Evaluation webhook delivery for OAuth broker
//! binding revocations.
//!
//! Notification intent commits with revocation; revoke commits are never rolled
//! back if the receiver is unavailable. Dispatch outcomes are durable, and an
//! ambiguous send is parked because receivers have no deduplication contract.

mod intents;
pub(super) use intents::deliver_intent;

#[cfg(test)]
use crate::services::webhook_delivery_service;
use chrono::{DateTime, Utc};
use serde::Serialize;

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
