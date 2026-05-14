#![allow(dead_code)]

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::errors::{AppError, AppResult};

const DEVICE_CODE_BYTES: usize = 32;
const USER_CODE_CHARS: usize = 12;
const USER_CODE_GROUP: usize = 4;
const USER_CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

pub fn generate_device_code() -> (String, String) {
    let mut bytes = [0u8; DEVICE_CODE_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    let raw = hex::encode(bytes);
    let hash = sha256_hex(raw.as_bytes());
    (raw, hash)
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

    let message = poll_signature_message(device_code_raw, timestamp);
    verifying_key.verify(&message, &signature).map_err(|_| {
        AppError::DevicePollSignatureInvalid("poll signature verification failed".to_string())
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn poll_signature_message(device_code_raw: &str, timestamp: i64) -> Vec<u8> {
    let mut message = Vec::with_capacity(device_code_raw.len() + std::mem::size_of::<i64>());
    message.extend_from_slice(device_code_raw.as_bytes());
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
        key.sign(&poll_signature_message(device_code_raw, timestamp))
            .to_bytes()
    }

    #[test]
    fn generate_device_code_returns_hex_raw_and_hash() {
        let (raw, hash) = generate_device_code();

        assert_eq!(raw.len(), DEVICE_CODE_BYTES * 2);
        assert!(raw.chars().all(|c| c.is_ascii_hexdigit()));
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
        let timestamp = 1_761_000_000;
        let raw = "aabbccdd".repeat(8);
        let signature = sign(&raw, timestamp, &key);

        verify_poll_signature(&pubkey, &raw, timestamp, &signature)
            .expect("signature should verify");
    }

    #[test]
    fn verify_poll_signature_rejects_wrong_device_code() {
        let key = signing_key();
        let pubkey = key.verifying_key().to_bytes();
        let timestamp = 1_761_000_000;
        let raw = "aabbccdd".repeat(8);
        let signature = sign(&raw, timestamp, &key);

        let error = verify_poll_signature(&pubkey, "00112233", timestamp, &signature)
            .expect_err("signature should fail");
        assert!(matches!(error, AppError::DevicePollSignatureInvalid(_)));
    }

    #[test]
    fn verify_poll_signature_rejects_wrong_timestamp() {
        let key = signing_key();
        let pubkey = key.verifying_key().to_bytes();
        let timestamp = 1_761_000_000;
        let raw = "aabbccdd".repeat(8);
        let signature = sign(&raw, timestamp, &key);

        let error = verify_poll_signature(&pubkey, &raw, timestamp + 1, &signature)
            .expect_err("signature should fail");
        assert!(matches!(error, AppError::DevicePollSignatureInvalid(_)));
    }
}
