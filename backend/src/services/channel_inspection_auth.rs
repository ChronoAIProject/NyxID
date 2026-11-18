use std::sync::Arc;
use std::time::Duration;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::services::coordination_service::ReplayStore;

type HmacSha256 = Hmac<Sha256>;

pub const VERSION_HEADER: &str = "x-nyxid-inspection-version";
pub const TIMESTAMP_HEADER: &str = "x-nyxid-inspection-timestamp";
pub const NONCE_HEADER: &str = "x-nyxid-inspection-nonce";
pub const BODY_SHA256_HEADER: &str = "x-nyxid-inspection-body-sha256";
pub const SIGNATURE_HEADER: &str = "x-nyxid-inspection-signature";
pub const PROTOCOL_VERSION: &str = "v1";

const REPLAY_NAMESPACE: &str = "channel-inspection-v1";
const MAX_SKEW: Duration = Duration::from_secs(30);
const NONCE_TTL: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub struct InspectionAuth {
    db: mongodb::Database,
    key: Arc<Zeroizing<[u8; 32]>>,
}

impl std::fmt::Debug for InspectionAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectionAuth")
            .field("key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl InspectionAuth {
    pub fn from_hex(db: mongodb::Database, raw: &str) -> Result<Self, &'static str> {
        let decoded = hex::decode(raw).map_err(|_| "must be 64 hexadecimal characters")?;
        let key: [u8; 32] = decoded
            .try_into()
            .map_err(|_| "must be 64 hexadecimal characters")?;
        Ok(Self {
            db,
            key: Arc::new(Zeroizing::new(key)),
        })
    }

    pub async fn authenticate(
        &self,
        headers: &axum::http::HeaderMap,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> bool {
        let Some(version) = header(headers, VERSION_HEADER) else {
            return false;
        };
        let Some(timestamp) = header(headers, TIMESTAMP_HEADER) else {
            return false;
        };
        let Some(nonce) = header(headers, NONCE_HEADER) else {
            return false;
        };
        let Some(body_digest) = header(headers, BODY_SHA256_HEADER) else {
            return false;
        };
        let Some(signature) = header(headers, SIGNATURE_HEADER) else {
            return false;
        };
        if version != PROTOCOL_VERSION || !valid_nonce(nonce) {
            return false;
        }
        let Ok(timestamp_secs) = timestamp.parse::<i64>() else {
            return false;
        };
        if chrono::Utc::now()
            .timestamp()
            .saturating_sub(timestamp_secs)
            .unsigned_abs()
            > MAX_SKEW.as_secs()
        {
            return false;
        }
        let actual_digest = sha256_hex(body);
        if !bool::from(body_digest.as_bytes().ct_eq(actual_digest.as_bytes()))
            || !verify(
                self.key.as_slice(),
                method,
                path,
                timestamp,
                nonce,
                body_digest,
                signature,
            )
        {
            return false;
        }
        match ReplayStore::claim(&self.db, REPLAY_NAMESPACE, nonce, NONCE_TTL).await {
            Ok(claimed) => claimed,
            Err(_error) => {
                tracing::error!("Inspection replay check failed");
                false
            }
        }
    }
}

fn header<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

fn valid_nonce(value: &str) -> bool {
    value.len() == 36 && uuid::Uuid::parse_str(value).is_ok()
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
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC-SHA256 accepts any key length");
    mac.update(canonical(method, path, timestamp, nonce, body_digest).as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn verify(
    key: &[u8],
    method: &str,
    path: &str,
    timestamp: &str,
    nonce: &str,
    body_digest: &str,
    supplied_signature: &str,
) -> bool {
    let expected = sign(key, method, path, timestamp, nonce, body_digest);
    let Ok(supplied) = hex::decode(supplied_signature) else {
        return false;
    };
    let Ok(expected) = hex::decode(expected) else {
        return false;
    };
    bool::from(expected.as_slice().ct_eq(supplied.as_slice()))
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
    fn signature_binds_every_request_component() {
        let key = [0x4a; 32];
        let path = "/api/v1/channel-inspections/telegram_private/dispatch";
        let body_digest = sha256_hex(br#"{"protocol_version":1}"#);
        let nonce = uuid::Uuid::new_v4().to_string();
        let signature = sign(&key, "POST", path, "1794990000", &nonce, &body_digest);
        assert!(verify(
            &key,
            "POST",
            path,
            "1794990000",
            &nonce,
            &body_digest,
            &signature
        ));
        assert!(!verify(
            &key,
            "GET",
            path,
            "1794990000",
            &nonce,
            &body_digest,
            &signature
        ));
        assert!(!verify(
            &key,
            "POST",
            "/api/v1/channel-inspections/other/dispatch",
            "1794990000",
            &nonce,
            &body_digest,
            &signature
        ));
    }

    #[tokio::test]
    async fn invalid_key_material_is_rejected_without_debug_exposure() {
        let client = mongodb::Client::with_uri_str("mongodb://127.0.0.1:27017")
            .await
            .expect("client");
        let error = InspectionAuth::from_hex(client.database("inspection_auth_test"), "not-hex");
        assert_eq!(error.unwrap_err(), "must be 64 hexadecimal characters");
    }
}
