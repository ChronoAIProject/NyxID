use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngCore;

use crate::errors::AppError;

/// Nonce size for AES-256-GCM (96 bits / 12 bytes).
const NONCE_SIZE: usize = 12;

/// Encrypt plaintext using AES-256-GCM.
///
/// The key must be exactly 32 bytes. A random 12-byte nonce is generated
/// and prepended to the ciphertext so that decryption can extract it.
///
/// Returns: `nonce || ciphertext || tag` (all concatenated).
pub fn encrypt(plaintext: &[u8], key: &[u8]) -> Result<Vec<u8>, AppError> {
    if key.len() != 32 {
        return Err(AppError::Internal(
            "AES key must be exactly 32 bytes".to_string(),
        ));
    }

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| AppError::Internal(format!("Failed to create AES cipher: {e}")))?;

    let mut nonce_bytes = [0u8; NONCE_SIZE];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| AppError::Internal(format!("AES encryption failed: {e}")))?;

    // Prepend the nonce to the ciphertext for storage
    let mut result = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);

    Ok(result)
}

/// Decrypt ciphertext that was produced by [`encrypt`].
///
/// Expects the input to be `nonce (12 bytes) || ciphertext || tag`.
pub fn decrypt(ciphertext: &[u8], key: &[u8]) -> Result<Vec<u8>, AppError> {
    if key.len() != 32 {
        return Err(AppError::Internal(
            "AES key must be exactly 32 bytes".to_string(),
        ));
    }

    if ciphertext.len() < NONCE_SIZE {
        return Err(AppError::Internal(
            "Ciphertext too short to contain nonce".to_string(),
        ));
    }

    let (nonce_bytes, encrypted) = ciphertext.split_at(NONCE_SIZE);
    let nonce = Nonce::from_slice(nonce_bytes);

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| AppError::Internal(format!("Failed to create AES cipher: {e}")))?;

    cipher
        .decrypt(nonce, encrypted)
        .map_err(|e| AppError::Internal(format!("AES decryption failed: {e}")))
}

/// Parse a hex-encoded encryption key into raw bytes.
pub fn parse_hex_key(hex_key: &str) -> Result<Vec<u8>, AppError> {
    hex::decode(hex_key).map_err(|e| {
        AppError::Internal(format!(
            "ENCRYPTION_KEY is not valid hex: {e}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = [0xABu8; 32];
        let plaintext = b"sensitive credential data";

        let encrypted = encrypt(plaintext, &key).unwrap();
        let decrypted = decrypt(&encrypted, &key).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_different_nonces() {
        let key = [0xCDu8; 32];
        let plaintext = b"same data";

        let enc1 = encrypt(plaintext, &key).unwrap();
        let enc2 = encrypt(plaintext, &key).unwrap();

        // Same plaintext should produce different ciphertexts (different nonces)
        assert_ne!(enc1, enc2);

        // Both should decrypt to the same plaintext
        assert_eq!(decrypt(&enc1, &key).unwrap(), plaintext);
        assert_eq!(decrypt(&enc2, &key).unwrap(), plaintext);
    }

    #[test]
    fn test_wrong_key_fails() {
        let key1 = [0xAAu8; 32];
        let key2 = [0xBBu8; 32];
        let plaintext = b"secret";

        let encrypted = encrypt(plaintext, &key1).unwrap();
        let result = decrypt(&encrypted, &key2);

        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_key_length() {
        let short_key = [0u8; 16]; // Too short
        let result = encrypt(b"test", &short_key);
        assert!(result.is_err());
    }
}
