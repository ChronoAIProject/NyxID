use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, VerifyingKey};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::errors::{AppError, AppResult};

const DEVICE_CODE_BYTES: usize = 32;
const USER_CODE_CHARS: usize = 12;
const USER_CODE_GROUP: usize = 4;
const USER_CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub const POLL_SIG_DOMAIN: &[u8] = b"nyxid:device-code:poll:v1";

pub fn generate_device_code() -> (String, String) {
    let mut bytes = [0u8; DEVICE_CODE_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    let raw = URL_SAFE_NO_PAD.encode(bytes);
    let hash = sha256_hex(raw.as_bytes());
    (raw, hash)
}

pub fn decode_device_code(device_code_raw: &str) -> AppResult<[u8; DEVICE_CODE_BYTES]> {
    let decoded = URL_SAFE_NO_PAD
        .decode(device_code_raw)
        .map_err(|_| AppError::DeviceCodeNotFound)?;
    decoded.try_into().map_err(|_| AppError::DeviceCodeNotFound)
}

pub fn generate_user_code() -> String {
    let mut random = [0u8; USER_CODE_CHARS];
    rand::thread_rng().fill_bytes(&mut random);

    let mut code = String::with_capacity(USER_CODE_CHARS + 2);
    for (idx, byte) in random.iter().enumerate() {
        if idx > 0 && idx % USER_CODE_GROUP == 0 {
            code.push('-');
        }
        code.push(USER_CODE_ALPHABET[(byte & 0b0001_1111) as usize] as char);
    }
    code
}

pub fn verify_poll_signature(
    pubkey: &[u8; 32],
    device_code_raw: &str,
    timestamp: i64,
    sig: &[u8; 64],
) -> AppResult<()> {
    let verifying_key = VerifyingKey::from_bytes(pubkey).map_err(|_| {
        AppError::DevicePollSignatureInvalid("invalid device public key".to_string())
    })?;
    let signature = Signature::from_bytes(sig);

    let message = poll_signature_message(device_code_raw, timestamp)?;
    verifying_key
        .verify_strict(&message, &signature)
        .map_err(|_| {
            AppError::DevicePollSignatureInvalid("poll signature verification failed".to_string())
        })
}

pub fn poll_signature_message(device_code_raw: &str, timestamp: i64) -> AppResult<Vec<u8>> {
    let device_code_bytes = decode_device_code_for_signature(device_code_raw)?;
    Ok(poll_signature_message_from_bytes(
        &device_code_bytes,
        timestamp,
    ))
}

fn decode_device_code_for_signature(device_code_raw: &str) -> AppResult<[u8; DEVICE_CODE_BYTES]> {
    let decoded = URL_SAFE_NO_PAD
        .decode(device_code_raw)
        .map_err(|_| AppError::DevicePollSignatureInvalid("malformed device code".to_string()))?;
    decoded
        .try_into()
        .map_err(|_| AppError::DevicePollSignatureInvalid("malformed device code".to_string()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn poll_signature_message_from_bytes(
    device_code_bytes: &[u8; DEVICE_CODE_BYTES],
    timestamp: i64,
) -> Vec<u8> {
    let mut message =
        Vec::with_capacity(POLL_SIG_DOMAIN.len() + DEVICE_CODE_BYTES + std::mem::size_of::<i64>());
    message.extend_from_slice(POLL_SIG_DOMAIN);
    message.extend_from_slice(device_code_bytes);
    message.extend_from_slice(&timestamp.to_be_bytes());
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[42u8; 32])
    }

    fn sign(device_code_raw: &str, timestamp: i64, key: &SigningKey) -> [u8; 64] {
        key.sign(&poll_signature_message(device_code_raw, timestamp).expect("signature message"))
            .to_bytes()
    }

    #[test]
    fn generate_device_code_returns_base64url_raw_and_hash() {
        let (raw, hash) = generate_device_code();

        assert_eq!(raw.len(), 43);
        assert!(
            raw.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
        assert!(!raw.contains('='));
        assert_eq!(
            URL_SAFE_NO_PAD.decode(&raw).expect("decode").len(),
            DEVICE_CODE_BYTES
        );
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, sha256_hex(raw.as_bytes()));
    }

    #[test]
    fn generate_device_code_has_entropy() {
        let (a, a_hash) = generate_device_code();
        let (b, b_hash) = generate_device_code();

        assert_ne!(a, b);
        assert_ne!(a_hash, b_hash);
    }

    #[test]
    fn generate_user_code_uses_expected_format_and_alphabet() {
        let code = generate_user_code();

        assert_eq!(code.len(), 14);
        assert_eq!(&code[4..5], "-");
        assert_eq!(&code[9..10], "-");
        for c in code.chars().filter(|c| *c != '-') {
            assert!(
                USER_CODE_ALPHABET.contains(&(c as u8)),
                "unexpected user-code character: {c}"
            );
        }
    }

    #[test]
    fn generate_user_code_has_entropy() {
        let a = generate_user_code();
        let b = generate_user_code();

        assert_ne!(a, b);
    }

    #[test]
    fn verify_poll_signature_accepts_matching_signature() {
        let key = signing_key();
        let pubkey = key.verifying_key().to_bytes();
        let timestamp: i64 = 1_761_000_000;
        let raw = URL_SAFE_NO_PAD.encode([7u8; DEVICE_CODE_BYTES]);
        let signature = sign(&raw, timestamp, &key);

        verify_poll_signature(&pubkey, &raw, timestamp, &signature)
            .expect("signature should verify");
    }

    #[test]
    fn verify_poll_signature_rejects_wrong_device_code() {
        let key = signing_key();
        let pubkey = key.verifying_key().to_bytes();
        let timestamp: i64 = 1_761_000_000;
        let raw = URL_SAFE_NO_PAD.encode([7u8; DEVICE_CODE_BYTES]);
        let signature = sign(&raw, timestamp, &key);

        let wrong_raw = URL_SAFE_NO_PAD.encode([8u8; DEVICE_CODE_BYTES]);
        let error = verify_poll_signature(&pubkey, &wrong_raw, timestamp, &signature)
            .expect_err("signature should fail");
        assert!(matches!(error, AppError::DevicePollSignatureInvalid(_)));
    }

    #[test]
    fn verify_poll_signature_rejects_wrong_timestamp() {
        let key = signing_key();
        let pubkey = key.verifying_key().to_bytes();
        let timestamp = 1_761_000_000;
        let raw = URL_SAFE_NO_PAD.encode([7u8; DEVICE_CODE_BYTES]);
        let signature = sign(&raw, timestamp, &key);

        let error = verify_poll_signature(&pubkey, &raw, timestamp + 1, &signature)
            .expect_err("signature should fail");
        assert!(matches!(error, AppError::DevicePollSignatureInvalid(_)));
    }

    #[test]
    fn verify_poll_signature_rejects_legacy_unprefixed_message() {
        let key = signing_key();
        let pubkey = key.verifying_key().to_bytes();
        let timestamp: i64 = 1_761_000_000;
        let raw = URL_SAFE_NO_PAD.encode([7u8; DEVICE_CODE_BYTES]);
        let device_code_bytes = decode_device_code(&raw).expect("valid device code");
        let mut legacy_message = Vec::with_capacity(DEVICE_CODE_BYTES + std::mem::size_of::<i64>());
        legacy_message.extend_from_slice(&device_code_bytes);
        legacy_message.extend_from_slice(&timestamp.to_be_bytes());
        let signature = key.sign(&legacy_message).to_bytes();

        let error = verify_poll_signature(&pubkey, &raw, timestamp, &signature)
            .expect_err("legacy signature should fail");
        assert!(matches!(error, AppError::DevicePollSignatureInvalid(_)));
    }

    #[test]
    fn verify_poll_signature_rejects_small_subgroup_public_key() {
        let key = signing_key();
        let timestamp = 1_761_000_000;
        let raw = URL_SAFE_NO_PAD.encode([7u8; DEVICE_CODE_BYTES]);
        let signature = sign(&raw, timestamp, &key);
        let mut small_order_pubkey = [0u8; 32];
        // Compressed identity point, one of ed25519-dalek's low-order
        // public-key test vectors.
        small_order_pubkey[0] = 1;

        let error = verify_poll_signature(&small_order_pubkey, &raw, timestamp, &signature)
            .expect_err("small-subgroup public key should fail");
        assert!(matches!(error, AppError::DevicePollSignatureInvalid(_)));
    }

    #[test]
    fn verify_poll_signature_rejects_malformed_device_code() {
        let key = signing_key();
        let pubkey = key.verifying_key().to_bytes();
        let timestamp = 1_761_000_000;
        let raw = URL_SAFE_NO_PAD.encode([7u8; DEVICE_CODE_BYTES]);
        let signature = sign(&raw, timestamp, &key);

        let error = verify_poll_signature(&pubkey, "not-valid-device-code", timestamp, &signature)
            .expect_err("signature should fail");
        assert!(matches!(error, AppError::DevicePollSignatureInvalid(_)));
    }
}
