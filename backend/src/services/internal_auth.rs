use std::sync::Arc;
use std::time::Duration;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::services::coordination_service::ReplayStore;

type HmacSha256 = Hmac<Sha256>;

pub const VERSION_HEADER: &str = "x-nyxid-internal-version";
pub const TIMESTAMP_HEADER: &str = "x-nyxid-internal-timestamp";
pub const NONCE_HEADER: &str = "x-nyxid-internal-nonce";
pub const BODY_SHA256_HEADER: &str = "x-nyxid-internal-body-sha256";
pub const SIGNATURE_HEADER: &str = "x-nyxid-internal-signature";
pub const PROTOCOL_VERSION: &str = "v1";
const REPLAY_NAMESPACE: &str = "internal-dispatch";

pub struct AuthenticatedBody {
    digest: String,
}

impl AuthenticatedBody {
    pub fn matches(&self, body: &[u8]) -> bool {
        self.digest
            .as_bytes()
            .ct_eq(sha256_hex(body).as_bytes())
            .into()
    }
}

#[derive(Clone)]
pub struct InternalAuth {
    db: mongodb::Database,
    key: Arc<Zeroizing<[u8; 32]>>,
    max_skew: Duration,
    nonce_ttl: Duration,
}

impl std::fmt::Debug for InternalAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InternalAuth")
            .field("key", &"[REDACTED]")
            .field("max_skew", &self.max_skew)
            .field("nonce_ttl", &self.nonce_ttl)
            .finish_non_exhaustive()
    }
}

impl InternalAuth {
    pub fn new(
        db: mongodb::Database,
        key: Zeroizing<[u8; 32]>,
        max_skew: Duration,
        nonce_ttl: Duration,
    ) -> Self {
        Self {
            db,
            key: Arc::new(key),
            max_skew,
            nonce_ttl,
        }
    }

    pub fn signed_headers(&self, method: &str, path: &str, body: &[u8]) -> axum::http::HeaderMap {
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let nonce = uuid::Uuid::new_v4().to_string();
        let body_digest = sha256_hex(body);
        let signature = sign(
            self.key.as_slice(),
            method,
            path,
            &timestamp,
            &nonce,
            &body_digest,
        );
        let mut headers = axum::http::HeaderMap::new();
        for (name, value) in [
            (VERSION_HEADER, PROTOCOL_VERSION),
            (TIMESTAMP_HEADER, timestamp.as_str()),
            (NONCE_HEADER, nonce.as_str()),
            (BODY_SHA256_HEADER, body_digest.as_str()),
            (SIGNATURE_HEADER, signature.as_str()),
        ] {
            headers.insert(
                axum::http::HeaderName::from_static(name),
                axum::http::HeaderValue::from_str(value)
                    .expect("internal authentication headers are ASCII"),
            );
        }
        headers
    }

    pub async fn authenticate(
        &self,
        headers: &axum::http::HeaderMap,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> bool {
        self.authenticate_headers(headers, method, path)
            .await
            .is_some_and(|authenticated| authenticated.matches(body))
    }

    pub async fn authenticate_headers(
        &self,
        headers: &axum::http::HeaderMap,
        method: &str,
        path: &str,
    ) -> Option<AuthenticatedBody> {
        let version = header(headers, VERSION_HEADER)?;
        let timestamp = header(headers, TIMESTAMP_HEADER)?;
        let nonce = header(headers, NONCE_HEADER)?;
        let supplied_digest = header(headers, BODY_SHA256_HEADER)?;
        let signature = header(headers, SIGNATURE_HEADER)?;
        if version != PROTOCOL_VERSION {
            return None;
        }
        let Ok(timestamp_secs) = timestamp.parse::<i64>() else {
            return None;
        };
        let skew = chrono::Utc::now()
            .timestamp()
            .saturating_sub(timestamp_secs)
            .unsigned_abs();
        if skew > self.max_skew.as_secs() {
            return None;
        }
        if !verify(
            self.key.as_slice(),
            method,
            path,
            timestamp,
            nonce,
            supplied_digest,
            signature,
        ) {
            return None;
        }
        match ReplayStore::claim(&self.db, REPLAY_NAMESPACE, nonce, self.nonce_ttl).await {
            Ok(true) => Some(AuthenticatedBody {
                digest: supplied_digest.to_string(),
            }),
            Ok(false) => None,
            Err(error) => {
                tracing::error!(%error, "Failed to claim internal request nonce");
                None
            }
        }
    }
}

fn header<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

pub fn derive_key(
    explicit_hex: Option<&str>,
    encryption_key: Option<&[u8]>,
    jwt_private_pem: &[u8],
) -> Zeroizing<[u8; 32]> {
    if let Some(raw) = explicit_hex.map(str::trim).filter(|raw| !raw.is_empty()) {
        let bytes = hex::decode(raw)
            .unwrap_or_else(|_| panic!("INTERNAL_DISPATCH_HMAC_KEY must be 64 hex characters"));
        let key: [u8; 32] = bytes
            .try_into()
            .unwrap_or_else(|_| panic!("INTERNAL_DISPATCH_HMAC_KEY must be 64 hex characters"));
        return Zeroizing::new(key);
    }
    crate::crypto::hmac_keys::derive_hmac_key("internal-dispatch", encryption_key, jwt_private_pem)
}

pub fn sha256_hex(body: &[u8]) -> String {
    hex::encode(Sha256::digest(body))
}

pub fn sign(
    key: &[u8],
    method: &str,
    path: &str,
    timestamp: &str,
    nonce: &str,
    body_digest: &str,
) -> String {
    hex::encode(signature_bytes(
        key,
        method,
        path,
        timestamp,
        nonce,
        body_digest,
    ))
}

fn signature_bytes(
    key: &[u8],
    method: &str,
    path: &str,
    timestamp: &str,
    nonce: &str,
    body_digest: &str,
) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC-SHA256 accepts any key length");
    mac.update(canonical(method, path, timestamp, nonce, body_digest).as_bytes());
    mac.finalize().into_bytes().into()
}

pub fn verify(
    key: &[u8],
    method: &str,
    path: &str,
    timestamp: &str,
    nonce: &str,
    body_digest: &str,
    supplied_signature: &str,
) -> bool {
    let expected = signature_bytes(key, method, path, timestamp, nonce, body_digest);
    let Ok(supplied) = hex::decode(supplied_signature) else {
        return false;
    };
    if supplied.len() != 32 {
        return false;
    }
    expected.as_slice().ct_eq(supplied.as_slice()).into()
}

fn canonical(method: &str, path: &str, timestamp: &str, nonce: &str, body_digest: &str) -> String {
    format!(
        "{PROTOCOL_VERSION}\n{}\n{path}\n{timestamp}\n{nonce}\n{body_digest}",
        method.to_ascii_uppercase()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_binds_method_path_timestamp_nonce_and_body() {
        let key = [0x42_u8; 32];
        let body_digest = sha256_hex(br#"{"node_id":"node-a"}"#);
        let nonce = uuid::Uuid::new_v4().to_string();
        let other_nonce = uuid::Uuid::new_v4().to_string();
        let signature = sign(
            &key,
            "POST",
            "/internal/v1/nodes/node-a/proxy",
            "1725100000",
            &nonce,
            &body_digest,
        );

        assert!(verify(
            &key,
            "POST",
            "/internal/v1/nodes/node-a/proxy",
            "1725100000",
            &nonce,
            &body_digest,
            &signature,
        ));
        assert!(!verify(
            &key,
            "POST",
            "/internal/v1/nodes/node-b/proxy",
            "1725100000",
            &nonce,
            &body_digest,
            &signature,
        ));
        assert!(!verify(
            &key,
            "POST",
            "/internal/v1/nodes/node-a/proxy",
            "1725100000",
            &nonce,
            &sha256_hex(b"different"),
            &signature,
        ));
        assert!(!verify(
            &key,
            "POST",
            "/internal/v1/nodes/node-a/proxy",
            "1725100000",
            &other_nonce,
            &body_digest,
            &signature,
        ));
    }

    #[test]
    fn signature_verification_accepts_mixed_case_hex() {
        let key = [0x42_u8; 32];
        let body_digest = sha256_hex(br#"{"node_id":"node-a"}"#);
        let nonce = uuid::Uuid::new_v4().to_string();
        let signature = sign(
            &key,
            "POST",
            "/internal/v1/nodes/node-a/proxy",
            "1725100000",
            &nonce,
            &body_digest,
        );
        let mixed_case_signature: String = signature
            .chars()
            .enumerate()
            .map(|(index, character)| {
                if index % 2 == 0 {
                    character.to_ascii_uppercase()
                } else {
                    character
                }
            })
            .collect();

        assert_ne!(signature, mixed_case_signature);
        assert!(verify(
            &key,
            "POST",
            "/internal/v1/nodes/node-a/proxy",
            "1725100000",
            &nonce,
            &body_digest,
            &mixed_case_signature,
        ));
    }

    #[test]
    fn explicit_key_override_and_domain_derivation_are_deterministic() {
        let override_hex = hex::encode([0x11_u8; 32]);
        let jwt = [0x77_u8; 64];
        let explicit = derive_key(Some(&override_hex), Some(&[0x33_u8; 32]), &jwt);
        assert_eq!(explicit.as_slice(), &[0x11_u8; 32]);

        let first = derive_key(None, Some(&[0x33_u8; 32]), &jwt);
        let second = derive_key(None, Some(&[0x33_u8; 32]), &jwt);
        let other_domain =
            crate::crypto::hmac_keys::derive_hmac_key("auth-device", Some(&[0x33_u8; 32]), &jwt);
        assert_eq!(first.as_slice(), second.as_slice());
        assert_ne!(first.as_slice(), other_domain.as_slice());
    }

    #[tokio::test]
    async fn shared_nonce_claim_rejects_replay() {
        let Some(db) = crate::test_utils::connect_test_database("internal_auth_replay").await
        else {
            return;
        };
        crate::services::coordination_service::ensure_indexes(&db)
            .await
            .unwrap();
        let auth = InternalAuth::new(
            db,
            Zeroizing::new([0x42; 32]),
            Duration::from_secs(30),
            Duration::from_secs(60),
        );
        let method = "POST";
        let path = "/internal/v1/nodes/node-a/proxy";
        let body = br#"{"node_id":"node-a"}"#;
        let headers = auth.signed_headers(method, path, body);

        assert!(auth.authenticate(&headers, method, path, body).await);
        assert!(!auth.authenticate(&headers, method, path, body).await);
    }
}
