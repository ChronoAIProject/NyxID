use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use rand::RngCore;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use zeroize::Zeroizing;

use crate::config::AppConfig;
use crate::errors::AppError;

/// Nonce size for AES-256-GCM (96 bits / 12 bytes).
const NONCE_SIZE: usize = 12;

/// Version byte for the v1 envelope format.
const VERSION_V1: u8 = 0x01;

/// Draft key IDs used by the initial uncommitted Phase 1 implementation.
/// We keep support for these so locally written draft ciphertexts still decrypt
/// after the stable key-id fix in this patch.
const DRAFT_KEY_ID_CURRENT: u8 = 0x00;
const DRAFT_KEY_ID_PREVIOUS: u8 = 0x01;

/// Size of the v1 header: version byte + key ID byte.
const V1_HEADER_SIZE: usize = 2;

/// Minimum v1 ciphertext: header(2) + nonce(12) + tag(16) = 30 bytes.
const V1_MIN_SIZE: usize = V1_HEADER_SIZE + NONCE_SIZE + 16;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EncryptionDecryptStats {
    pub v1_current: u64,
    pub v1_previous: u64,
    pub v0_current: u64,
    pub v0_previous: u64,
    pub unknown_key_id_failures: u64,
    pub decrypt_failures: u64,
}

#[derive(Default)]
struct DecryptCounters {
    v1_current: AtomicU64,
    v1_previous: AtomicU64,
    v0_current: AtomicU64,
    v0_previous: AtomicU64,
    unknown_key_id_failures: AtomicU64,
    decrypt_failures: AtomicU64,
    logged_v1_previous: AtomicBool,
    logged_v0_current: AtomicBool,
    logged_v0_previous: AtomicBool,
    logged_unknown_key_id: AtomicBool,
}

impl DecryptCounters {
    fn snapshot(&self) -> EncryptionDecryptStats {
        EncryptionDecryptStats {
            v1_current: self.v1_current.load(Ordering::Relaxed),
            v1_previous: self.v1_previous.load(Ordering::Relaxed),
            v0_current: self.v0_current.load(Ordering::Relaxed),
            v0_previous: self.v0_previous.load(Ordering::Relaxed),
            unknown_key_id_failures: self.unknown_key_id_failures.load(Ordering::Relaxed),
            decrypt_failures: self.decrypt_failures.load(Ordering::Relaxed),
        }
    }
}

/// Holds the current and (optionally) previous encryption keys for AES-256-GCM.
///
/// New encryptions always use `current` and stamp the ciphertext with a stable
/// key id derived from the key material itself. Decryption supports the
/// currently configured key plus a single previous key.
pub struct EncryptionKeys {
    current: Zeroizing<[u8; 32]>,
    current_id: u8,
    previous: Option<Zeroizing<[u8; 32]>>,
    previous_id: Option<u8>,
    counters: DecryptCounters,
}

impl std::fmt::Debug for EncryptionKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptionKeys")
            .field("current", &"[REDACTED]")
            .field(
                "previous",
                if self.previous.is_some() {
                    &"Some([REDACTED])"
                } else {
                    &"None"
                },
            )
            .finish()
    }
}

impl EncryptionKeys {
    /// Build from validated AppConfig. Panics on invalid keys (startup-only).
    pub fn from_config(config: &AppConfig) -> Self {
        let current_bytes: [u8; 32] = hex::decode(&config.encryption_key)
            .expect(
                "ENCRYPTION_KEY is not valid hex (should have been caught by validate_encryption_key)",
            )
            .try_into()
            .expect("ENCRYPTION_KEY must decode to 32 bytes");
        let current_id = derive_key_id(&current_bytes);

        let previous = config
            .encryption_key_previous
            .as_ref()
            .map(|hex_str| {
                let bytes: [u8; 32] = hex::decode(hex_str)
                    .expect(
                        "ENCRYPTION_KEY_PREVIOUS is not valid hex (should have been caught by validate_encryption_key)",
                    )
                    .try_into()
                    .expect("ENCRYPTION_KEY_PREVIOUS must decode to 32 bytes");
                (Zeroizing::new(bytes), derive_key_id(&bytes))
            });

        if let Some((_, previous_id)) = previous.as_ref() {
            assert_ne!(
                current_id, *previous_id,
                "ENCRYPTION_KEY and ENCRYPTION_KEY_PREVIOUS produced the same key id. Generate a different previous key."
            );
        }

        Self {
            current: Zeroizing::new(current_bytes),
            current_id,
            previous: previous.as_ref().map(|(bytes, _)| {
                let mut copied = [0u8; 32];
                copied.copy_from_slice(bytes.as_ref());
                Zeroizing::new(copied)
            }),
            previous_id: previous.as_ref().map(|(_, key_id)| *key_id),
            counters: DecryptCounters::default(),
        }
    }

    /// Returns true if a previous key is configured.
    pub fn has_previous(&self) -> bool {
        self.previous.is_some()
    }

    /// Returns counters for each decrypt path. Useful during rotation to verify
    /// whether traffic still depends on legacy or previous-key ciphertexts.
    pub fn decrypt_stats(&self) -> EncryptionDecryptStats {
        self.counters.snapshot()
    }

    /// Encrypt plaintext using AES-256-GCM with the v1 envelope format.
    ///
    /// Output: `0x01 || stable_key_id(1) || nonce(12) || ciphertext || tag(16)`
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
        let cipher = Aes256Gcm::new_from_slice(self.current.as_ref())
            .map_err(|e| AppError::Internal(format!("Failed to create AES cipher: {e}")))?;

        let mut nonce_bytes = [0u8; NONCE_SIZE];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| AppError::Internal(format!("AES encryption failed: {e}")))?;

        let mut result = Vec::with_capacity(V1_HEADER_SIZE + NONCE_SIZE + ciphertext.len());
        result.push(VERSION_V1);
        result.push(self.current_id);
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);

        Ok(result)
    }

    /// Decrypt ciphertext, trying the fallback chain:
    ///
    /// 1. If it looks like v1: try v1 payload with current key, then previous key
    /// 2. Try v0 (raw `nonce || ciphertext || tag`) with current key
    /// 3. Try v0 with previous key
    /// 4. Return error if all fail
    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
        let mut unknown_key_id = None;
        if looks_like_v1(ciphertext) {
            let key_id = ciphertext[1];
            let payload = &ciphertext[V1_HEADER_SIZE..];

            if key_id == self.current_id {
                if let Ok(plain) = decrypt_raw(payload, self.current.as_ref()) {
                    self.counters.v1_current.fetch_add(1, Ordering::Relaxed);
                    return Ok(plain);
                }
            } else if self.previous_id == Some(key_id) {
                if let Some(ref prev) = self.previous
                    && let Ok(plain) = decrypt_raw(payload, prev.as_ref())
                {
                    self.counters.v1_previous.fetch_add(1, Ordering::Relaxed);
                    self.log_once(
                        &self.counters.logged_v1_previous,
                        "Decrypted ciphertext with ENCRYPTION_KEY_PREVIOUS; old-key ciphertexts are still in active use",
                    );
                    return Ok(plain);
                }
            } else if key_id == DRAFT_KEY_ID_CURRENT || key_id == DRAFT_KEY_ID_PREVIOUS {
                if let Ok(plain) = decrypt_raw(payload, self.current.as_ref()) {
                    self.counters.v1_current.fetch_add(1, Ordering::Relaxed);
                    return Ok(plain);
                }

                if let Some(ref prev) = self.previous
                    && let Ok(plain) = decrypt_raw(payload, prev.as_ref())
                {
                    self.counters.v1_previous.fetch_add(1, Ordering::Relaxed);
                    self.log_once(
                        &self.counters.logged_v1_previous,
                        "Decrypted draft Phase 1 ciphertext with ENCRYPTION_KEY_PREVIOUS; old-key ciphertexts are still in active use",
                    );
                    return Ok(plain);
                }
            } else {
                unknown_key_id = Some(key_id);
            }
        }

        // Try v0 format (full ciphertext is nonce || encrypted || tag) with current key
        if let Ok(plain) = decrypt_raw(ciphertext, self.current.as_ref()) {
            self.counters.v0_current.fetch_add(1, Ordering::Relaxed);
            self.log_once(
                &self.counters.logged_v0_current,
                "Decrypted legacy v0 ciphertext with ENCRYPTION_KEY; re-encryption is still pending",
            );
            return Ok(plain);
        }

        // Try v0 with previous key
        if let Some(ref prev) = self.previous
            && let Ok(plain) = decrypt_raw(ciphertext, prev.as_ref())
        {
            self.counters.v0_previous.fetch_add(1, Ordering::Relaxed);
            self.log_once(
                &self.counters.logged_v0_previous,
                "Decrypted legacy v0 ciphertext with ENCRYPTION_KEY_PREVIOUS; old-key ciphertexts are still in active use",
            );
            return Ok(plain);
        }

        if let Some(key_id) = unknown_key_id {
            self.counters
                .unknown_key_id_failures
                .fetch_add(1, Ordering::Relaxed);
            self.log_once(
                &self.counters.logged_unknown_key_id,
                &format!(
                    "Encountered versioned ciphertext with unknown key id 0x{key_id:02x}; the data was likely encrypted with a key that is no longer configured"
                ),
            );
        }

        self.counters
            .decrypt_failures
            .fetch_add(1, Ordering::Relaxed);
        Err(AppError::Internal(
            "AES decryption failed: no key could decrypt the data".to_string(),
        ))
    }

    fn log_once(&self, flag: &AtomicBool, message: &str) {
        if !flag.swap(true, Ordering::Relaxed) {
            tracing::warn!("{message}");
        }
    }
}

/// Check if data looks like a v1 envelope.
fn looks_like_v1(data: &[u8]) -> bool {
    data.len() >= V1_MIN_SIZE && data[0] == VERSION_V1
}

fn derive_key_id(key: &[u8]) -> u8 {
    let digest = Sha256::digest(key);
    digest[0]
}

/// Low-level AES-256-GCM decryption: expects `nonce(12) || ciphertext || tag`.
fn decrypt_raw(data: &[u8], key: &[u8]) -> Result<Vec<u8>, AppError> {
    if data.len() < NONCE_SIZE {
        return Err(AppError::Internal(
            "Ciphertext too short to contain nonce".to_string(),
        ));
    }

    let (nonce_bytes, encrypted) = data.split_at(NONCE_SIZE);
    let nonce = Nonce::from_slice(nonce_bytes);

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| AppError::Internal(format!("Failed to create AES cipher: {e}")))?;

    cipher
        .decrypt(nonce, encrypted)
        .map_err(|e| AppError::Internal(format!("AES decryption failed: {e}")))
}

/// Encrypt plaintext using AES-256-GCM (v0 format, kept for tests).
///
/// The key must be exactly 32 bytes. A random 12-byte nonce is generated
/// and prepended to the ciphertext so that decryption can extract it.
///
/// Returns: `nonce || ciphertext || tag` (all concatenated).
#[cfg(test)]
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

/// Decrypt ciphertext that was produced by [`encrypt`] (v0 format, kept for tests).
///
/// Expects the input to be `nonce (12 bytes) || ciphertext || tag`.
#[cfg(test)]
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

/// Parse a hex-encoded encryption key into raw bytes (kept for tests).
#[cfg(test)]
pub fn parse_hex_key(hex_key: &str) -> Result<Vec<u8>, AppError> {
    hex::decode(hex_key)
        .map_err(|e| AppError::Internal(format!("ENCRYPTION_KEY is not valid hex: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(key_hex: &str, prev_hex: Option<&str>) -> AppConfig {
        AppConfig {
            port: 3001,
            base_url: "http://localhost:3001".to_string(),
            frontend_url: "http://localhost:3000".to_string(),
            database_url: "mongodb://localhost:27017/nyxid".to_string(),
            database_max_connections: 10,
            environment: "test".to_string(),
            jwt_private_key_path: "keys/private.pem".to_string(),
            jwt_public_key_path: "keys/public.pem".to_string(),
            jwt_issuer: "http://localhost:3001".to_string(),
            jwt_access_ttl_secs: 900,
            jwt_refresh_ttl_secs: 604800,
            google_client_id: None,
            google_client_secret: None,
            github_client_id: None,
            github_client_secret: None,
            apple_client_id: None,
            apple_team_id: None,
            apple_key_id: None,
            apple_private_key_path: None,
            smtp_host: None,
            smtp_port: None,
            smtp_username: None,
            smtp_password: None,
            smtp_from_address: None,
            encryption_key: key_hex.to_string(),
            encryption_key_previous: prev_hex.map(String::from),
            rate_limit_per_second: 10,
            rate_limit_burst: 30,
            sa_token_ttl_secs: 3600,
            cookie_domain: None,
            telegram_bot_token: None,
            telegram_webhook_secret: None,
            telegram_webhook_url: None,
            telegram_bot_username: None,
            approval_expiry_interval_secs: 5,
            fcm_service_account_path: None,
            fcm_project_id: None,
            apns_key_path: None,
            apns_key_id: None,
            apns_team_id: None,
            apns_topic: None,
            apns_sandbox: true,
        }
    }

    // -- Legacy v0 tests (unchanged) --

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

    #[test]
    fn test_decrypt_invalid_key_length() {
        let short_key = [0u8; 16];
        let result = decrypt(b"some-data-longer-than-12", &short_key);
        assert!(result.is_err());
    }

    #[test]
    fn test_decrypt_ciphertext_too_short() {
        let key = [0xAAu8; 32];
        let short_data = [0u8; 5]; // less than NONCE_SIZE (12)
        let result = decrypt(&short_data, &key);
        assert!(result.is_err());
    }

    #[test]
    fn test_encrypt_empty_plaintext() {
        let key = [0xBBu8; 32];
        let encrypted = encrypt(b"", &key).unwrap();
        let decrypted = decrypt(&encrypted, &key).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn test_encrypt_large_plaintext() {
        let key = [0xCCu8; 32];
        let plaintext = vec![0x42u8; 10_000];
        let encrypted = encrypt(&plaintext, &key).unwrap();
        let decrypted = decrypt(&encrypted, &key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let key = [0xDDu8; 32];
        let plaintext = b"important data";
        let mut encrypted = encrypt(plaintext, &key).unwrap();
        // Flip a byte in the ciphertext portion (after nonce)
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0xFF;
        assert!(decrypt(&encrypted, &key).is_err());
    }

    #[test]
    fn test_parse_hex_key_valid() {
        let hex_key = "ab".repeat(32); // 64 hex chars = 32 bytes
        let bytes = parse_hex_key(&hex_key).unwrap();
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn test_parse_hex_key_invalid() {
        let result = parse_hex_key("not-hex-at-all!");
        assert!(result.is_err());
    }

    // -- EncryptionKeys v1 tests --

    #[test]
    fn v1_roundtrip() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let plaintext = b"v1 encrypted data";
        let encrypted = keys.encrypt(plaintext).unwrap();

        // Verify v1 header
        assert_eq!(encrypted[0], VERSION_V1);
        assert_eq!(encrypted[1], derive_key_id(keys.current.as_ref()));

        let decrypted = keys.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn v1_different_nonces() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let plaintext = b"same data";
        let enc1 = keys.encrypt(plaintext).unwrap();
        let enc2 = keys.encrypt(plaintext).unwrap();

        assert_ne!(enc1, enc2);
        assert_eq!(keys.decrypt(&enc1).unwrap(), plaintext);
        assert_eq!(keys.decrypt(&enc2).unwrap(), plaintext);
    }

    #[test]
    fn v1_empty_plaintext() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let encrypted = keys.encrypt(b"").unwrap();
        let decrypted = keys.decrypt(&encrypted).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn v1_large_plaintext() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let plaintext = vec![0x42u8; 10_000];
        let encrypted = keys.encrypt(&plaintext).unwrap();
        let decrypted = keys.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn v1_tamper_detection() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let mut encrypted = keys.encrypt(b"tamper test").unwrap();
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0xFF;

        assert!(keys.decrypt(&encrypted).is_err());
    }

    #[test]
    fn v1_decrypt_v0_data_with_current_key() {
        // Simulate existing v0 data encrypted with the current key
        let key_hex = "cd".repeat(32);
        let key_bytes = hex::decode(&key_hex).unwrap();
        let config = test_config(&key_hex, None);
        let keys = EncryptionKeys::from_config(&config);

        let plaintext = b"legacy v0 data";
        let v0_encrypted = encrypt(plaintext, &key_bytes).unwrap();

        // EncryptionKeys should be able to decrypt v0 data
        let decrypted = keys.decrypt(&v0_encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn v1_decrypt_v0_data_with_previous_key() {
        // Simulate key rotation: v0 data encrypted with old key, new key is now current
        let old_key_hex = "cd".repeat(32);
        let new_key_hex = "ef".repeat(32);
        let old_key_bytes = hex::decode(&old_key_hex).unwrap();
        let config = test_config(&new_key_hex, Some(&old_key_hex));
        let keys = EncryptionKeys::from_config(&config);

        let plaintext = b"old key data";
        let v0_encrypted = encrypt(plaintext, &old_key_bytes).unwrap();

        // Should decrypt using the previous key fallback
        let decrypted = keys.decrypt(&v0_encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn v1_key_rotation_current_decrypts_previous_v1() {
        // Encrypt with key A as current, then rotate: key B becomes current, A becomes previous
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);
        let key_a_id = derive_key_id(&hex::decode(&key_a_hex).unwrap());

        // Phase 1: encrypt with key A
        let config_a = test_config(&key_a_hex, None);
        let keys_a = EncryptionKeys::from_config(&config_a);
        let encrypted = keys_a.encrypt(b"rotation test").unwrap();
        assert_eq!(encrypted[1], key_a_id);

        // Phase 2: rotate - B is current, A is previous
        let config_rotated = test_config(&key_b_hex, Some(&key_a_hex));
        let keys_rotated = EncryptionKeys::from_config(&config_rotated);

        // Should still decrypt data encrypted under key A
        let decrypted = keys_rotated.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, b"rotation test");
    }

    #[test]
    fn v1_rollback_scenario() {
        // Encrypt with key B as current, then rollback: A becomes current again, B becomes previous
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);

        // Phase 1: key B is current, encrypt some data
        let config_b = test_config(&key_b_hex, Some(&key_a_hex));
        let keys_b = EncryptionKeys::from_config(&config_b);
        let encrypted = keys_b.encrypt(b"rollback test").unwrap();

        // Phase 2: rollback - A is current again, B becomes previous
        let config_rollback = test_config(&key_a_hex, Some(&key_b_hex));
        let keys_rollback = EncryptionKeys::from_config(&config_rollback);

        let decrypted = keys_rollback.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, b"rollback test");
    }

    #[test]
    fn v1_second_rotation_without_reencryption_fails_after_oldest_key_removed() {
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);
        let key_c_hex = "cc".repeat(32);

        let keys_a = EncryptionKeys::from_config(&test_config(&key_a_hex, None));
        let encrypted = keys_a.encrypt(b"still on key a").unwrap();

        let keys_b = EncryptionKeys::from_config(&test_config(&key_b_hex, Some(&key_a_hex)));
        assert_eq!(keys_b.decrypt(&encrypted).unwrap(), b"still on key a");

        let keys_c = EncryptionKeys::from_config(&test_config(&key_c_hex, Some(&key_b_hex)));
        assert!(keys_c.decrypt(&encrypted).is_err());
    }

    #[test]
    fn v1_decrypt_supports_draft_phase1_header() {
        let key_hex = "ab".repeat(32);
        let key_bytes = hex::decode(&key_hex).unwrap();
        let keys = EncryptionKeys::from_config(&test_config(&key_hex, None));

        let mut encrypted = Vec::new();
        encrypted.push(VERSION_V1);
        encrypted.push(DRAFT_KEY_ID_CURRENT);
        encrypted.extend_from_slice(&encrypt(b"draft v1", &key_bytes).unwrap());

        assert_eq!(keys.decrypt(&encrypted).unwrap(), b"draft v1");
    }

    #[test]
    fn v1_unknown_key_fails() {
        let key_a_hex = "aa".repeat(32);
        let key_c_hex = "cc".repeat(32);

        let config_a = test_config(&key_a_hex, None);
        let keys_a = EncryptionKeys::from_config(&config_a);
        let encrypted = keys_a.encrypt(b"secret").unwrap();

        // Try to decrypt with a completely different key
        let config_c = test_config(&key_c_hex, None);
        let keys_c = EncryptionKeys::from_config(&config_c);
        assert!(keys_c.decrypt(&encrypted).is_err());
    }

    #[test]
    fn v1_has_previous() {
        let config_no_prev = test_config(&"ab".repeat(32), None);
        let keys_no_prev = EncryptionKeys::from_config(&config_no_prev);
        assert!(!keys_no_prev.has_previous());

        let config_with_prev = test_config(&"ab".repeat(32), Some(&"cd".repeat(32)));
        let keys_with_prev = EncryptionKeys::from_config(&config_with_prev);
        assert!(keys_with_prev.has_previous());
    }

    #[test]
    fn v1_decrypt_stats_track_fallback_paths() {
        let current_hex = "ab".repeat(32);
        let previous_hex = "cd".repeat(32);
        let previous_bytes = hex::decode(&previous_hex).unwrap();
        let keys = EncryptionKeys::from_config(&test_config(&current_hex, Some(&previous_hex)));

        let v0_previous = encrypt(b"legacy previous", &previous_bytes).unwrap();
        assert_eq!(keys.decrypt(&v0_previous).unwrap(), b"legacy previous");

        let stats = keys.decrypt_stats();
        assert_eq!(
            stats,
            EncryptionDecryptStats {
                v1_current: 0,
                v1_previous: 0,
                v0_current: 0,
                v0_previous: 1,
                unknown_key_id_failures: 0,
                decrypt_failures: 0,
            }
        );
    }

    #[test]
    fn v1_debug_redacts_keys() {
        let config = test_config(&"ab".repeat(32), Some(&"cd".repeat(32)));
        let keys = EncryptionKeys::from_config(&config);
        let debug_str = format!("{:?}", keys);

        assert!(debug_str.contains("REDACTED"));
        assert!(!debug_str.contains("ab"));
        assert!(!debug_str.contains("cd"));
    }

    #[test]
    fn v1_cross_version_roundtrip() {
        // Encrypt with v0 API, decrypt with EncryptionKeys (simulates migration)
        let key_hex = "dd".repeat(32);
        let key_bytes = hex::decode(&key_hex).unwrap();

        let plaintext = b"cross-version data";
        let v0_encrypted = encrypt(plaintext, &key_bytes).unwrap();

        let config = test_config(&key_hex, None);
        let keys = EncryptionKeys::from_config(&config);

        // v0 -> EncryptionKeys decrypt
        let decrypted = keys.decrypt(&v0_encrypted).unwrap();
        assert_eq!(decrypted, plaintext);

        // EncryptionKeys encrypt -> verify it's v1
        let v1_encrypted = keys.encrypt(plaintext).unwrap();
        assert_eq!(v1_encrypted[0], VERSION_V1);

        // v1 -> EncryptionKeys decrypt
        let decrypted2 = keys.decrypt(&v1_encrypted).unwrap();
        assert_eq!(decrypted2, plaintext);
    }
}
