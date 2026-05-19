use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::authorization_code::ExternalSubjectRef;

pub const COLLECTION_NAME: &str = "pushed_authorization_requests";

/// Request-URI prefix per RFC 9126 §2.2:
/// `urn:ietf:params:oauth:request_uri:<opaque>`
pub const REQUEST_URI_PREFIX: &str = "urn:ietf:params:oauth:request_uri:";

/// Length of the random opaque suffix on request_uri values (22 hex chars).
pub const REQUEST_URI_RANDOM_HEX_LEN: usize = 22;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PushedAuthorizationRequest {
    /// SHA-256 hex of the request_uri. The raw URI is returned once to the
    /// client and is never persisted.
    #[serde(rename = "_id")]
    pub id: String,

    pub client_id: String,
    pub response_type: String,
    pub redirect_uri: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_challenge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_challenge_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_subject: Option<ExternalSubjectRef>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

pub fn generate_request_uri() -> String {
    use rand::RngCore;

    let mut bytes = [0u8; REQUEST_URI_RANDOM_HEX_LEN / 2 + 1];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut hex = hex::encode(bytes);
    hex.truncate(REQUEST_URI_RANDOM_HEX_LEN);
    format!("{REQUEST_URI_PREFIX}{hex}")
}

pub fn hash_request_uri(uri: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut h = Sha256::new();
    h.update(uri.as_bytes());
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "pushed_authorization_requests");
    }

    #[test]
    fn generate_request_uri_format() {
        let uri = generate_request_uri();
        assert!(uri.starts_with(REQUEST_URI_PREFIX));
        let suffix = &uri[REQUEST_URI_PREFIX.len()..];
        assert_eq!(suffix.len(), REQUEST_URI_RANDOM_HEX_LEN);
        assert!(
            suffix
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn generate_request_uri_uniqueness() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(generate_request_uri()));
        }
    }

    #[test]
    fn hash_request_uri_stable() {
        let h1 = hash_request_uri("urn:ietf:params:oauth:request_uri:abc");
        let h2 = hash_request_uri("urn:ietf:params:oauth:request_uri:abc");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }
}
