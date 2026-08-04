use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::authorization_code::ExternalSubjectRef;
use super::bson_datetime;

pub const COLLECTION_NAME: &str = "oauth_broker_bindings";

/// Length of the hex-encoded random suffix on `binding_id` strings.
/// 32 hex chars = 128 bits of entropy, matching `crate::crypto::token::generate_random_token`.
pub const BINDING_ID_RANDOM_HEX_LEN: usize = 32;
pub const BINDING_ID_PREFIX: &str = "bnd_";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OauthBrokerBinding {
    /// SHA-256 hex (lowercase) of the raw binding_id. The raw value never leaves the issuance
    /// response or the holding client.
    #[serde(rename = "_id")]
    pub id: String,

    /// OAuthClient._id that owns this binding. Only this client may exchange or revoke it.
    pub client_id: String,

    /// nyx_subject (User._id) bound to this credential.
    pub user_id: String,

    /// Links to `RefreshToken.jti` so revocation cascades from one to the other.
    pub refresh_token_jti: String,

    /// AES-256-GCM v2 envelope ciphertext of the active refresh_token string.
    /// AAD is intentionally omitted in v1 to match the existing `UserProviderToken`
    /// pattern; binding-hash AAD is a v2 hardening.
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub refresh_token_encrypted: Option<Vec<u8>>,

    /// Granted scopes at issuance time. Token-exchange may scope-down but never up.
    #[serde(default)]
    pub scopes: Vec<String>,

    /// Optional external-subject reference captured at /oauth/authorize time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_subject: Option<ExternalSubjectRef>,

    /// Optimistic-concurrency counter. Increments on each refresh-token rotation.
    /// Concurrent token-exchange callers race on this field via conditional update.
    #[serde(default)]
    pub rotation_version: u32,

    #[serde(default)]
    pub revoked: bool,

    #[serde(default, with = "bson_datetime::optional")]
    pub last_used_at: Option<DateTime<Utc>>,

    #[serde(default, with = "bson_datetime::optional")]
    pub revoked_at: Option<DateTime<Utc>>,

    /// One of: "user", "client", "admin", "reuse_detected", "rotation_failed".
    #[serde(default)]
    pub revoke_reason: Option<String>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// Generate a new opaque binding_id of the form `bnd_<32-hex>`.
///
/// 128 bits of entropy. The raw value is returned ONCE to the OAuth client at issuance
/// time and is never persisted server-side -- only its SHA-256 hash is stored.
pub fn generate_binding_id() -> String {
    let mut bytes = [0u8; BINDING_ID_RANDOM_HEX_LEN / 2];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{BINDING_ID_PREFIX}{}", hex::encode(bytes))
}

/// Compute the canonical lookup key for a binding_id.
///
/// SHA-256 hex (lowercase). This is `OauthBrokerBinding._id`. Use this for every Mongo
/// query so callers never see the raw binding_id and the database leak surface is bounded.
pub fn hash_binding_id(binding_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(binding_id.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "oauth_broker_bindings");
    }

    #[test]
    fn generate_binding_id_format() {
        let id = generate_binding_id();
        assert!(id.starts_with(BINDING_ID_PREFIX));
        let suffix = &id[BINDING_ID_PREFIX.len()..];
        assert_eq!(suffix.len(), BINDING_ID_RANDOM_HEX_LEN);
        assert!(
            suffix
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn generate_binding_id_uniqueness() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(generate_binding_id()));
        }
    }

    #[test]
    fn hash_binding_id_is_stable_lowercase_hex() {
        let h1 = hash_binding_id("bnd_deadbeef");
        let h2 = hash_binding_id("bnd_deadbeef");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
        assert!(
            h1.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn hash_binding_id_distinct() {
        assert_ne!(hash_binding_id("bnd_a"), hash_binding_id("bnd_b"));
    }

    #[test]
    fn bson_roundtrip_full() {
        let now = Utc::now();
        let binding = OauthBrokerBinding {
            id: hash_binding_id("bnd_test"),
            client_id: "client-1".to_string(),
            user_id: "user-1".to_string(),
            refresh_token_jti: "jti-1".to_string(),
            refresh_token_encrypted: Some(vec![1, 2, 3, 4]),
            scopes: vec!["openid".to_string(), "profile".to_string()],
            external_subject: Some(ExternalSubjectRef {
                platform: "lark".to_string(),
                tenant: Some("t1".to_string()),
                external_user_id: "u1".to_string(),
            }),
            rotation_version: 2,
            revoked: false,
            last_used_at: Some(now),
            revoked_at: None,
            revoke_reason: None,
            created_at: now,
        };
        let doc = bson::to_document(&binding).expect("serialize");
        let restored: OauthBrokerBinding = bson::from_document(doc).expect("deserialize");
        assert_eq!(binding.id, restored.id);
        assert_eq!(binding.scopes, restored.scopes);
        assert_eq!(binding.external_subject, restored.external_subject);
        assert_eq!(binding.rotation_version, restored.rotation_version);
        assert_eq!(
            binding.refresh_token_encrypted,
            restored.refresh_token_encrypted
        );
    }

    #[test]
    fn bson_default_for_legacy_doc() {
        // Minimal doc with only required fields. Future on-disk additions
        // must not break decoding of older docs.
        let now = Utc::now();
        let doc = bson::doc! {
            "_id": "abcdef",
            "client_id": "c1",
            "user_id": "u1",
            "refresh_token_jti": "j1",
            "created_at": bson::DateTime::from_chrono(now),
        };
        let restored: OauthBrokerBinding = bson::from_document(doc).expect("deserialize legacy");
        assert!(restored.refresh_token_encrypted.is_none());
        assert!(restored.scopes.is_empty());
        assert!(restored.external_subject.is_none());
        assert_eq!(restored.rotation_version, 0);
        assert!(!restored.revoked);
        assert!(restored.last_used_at.is_none());
        assert!(restored.revoked_at.is_none());
        assert!(restored.revoke_reason.is_none());
    }
}
