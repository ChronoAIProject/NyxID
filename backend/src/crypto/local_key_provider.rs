use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use rand::RngCore;
use zeroize::Zeroizing;

use crate::config::AppConfig;
use crate::errors::AppError;

use super::key_provider::{KeyProvider, WrappedKey, derive_key_id};

/// Nonce size for AES-256-GCM (96 bits / 12 bytes).
const NONCE_SIZE: usize = 12;

/// Local key provider that wraps/unwraps DEKs using in-process AES-256-GCM keys.
///
/// Key material is stored in-process using `Zeroizing` wrappers so it is
/// scrubbed from memory when the provider is dropped. Each key is identified
/// by a stable version string derived from the first byte of its SHA-256 hash.
pub struct LocalKeyProvider {
    current: Zeroizing<[u8; 32]>,
    current_id: u8,
    previous: Option<Zeroizing<[u8; 32]>>,
    previous_id: Option<u8>,
}

impl std::fmt::Debug for LocalKeyProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalKeyProvider")
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

impl LocalKeyProvider {
    /// Create a new provider from raw 32-byte keys.
    pub fn new(current: [u8; 32], previous: Option<[u8; 32]>) -> Self {
        let current_id = derive_key_id(&current);
        let previous = previous.filter(|prev| prev != &current);

        let (previous_z, previous_id) = match previous {
            Some(prev) => {
                let prev_id = derive_key_id(&prev);
                if current_id == prev_id {
                    panic!(
                        "ENCRYPTION_KEY and ENCRYPTION_KEY_PREVIOUS produce the same key id (0x{:02x}). \
                         This is a 1-in-256 hash collision. Generate a different key with: openssl rand -hex 32",
                        current_id
                    );
                }
                (Some(Zeroizing::new(prev)), Some(prev_id))
            }
            None => (None, None),
        };

        Self {
            current: Zeroizing::new(current),
            current_id,
            previous: previous_z,
            previous_id,
        }
    }

    /// Build a provider from application config, parsing hex-encoded keys.
    pub fn from_config(config: &AppConfig) -> Self {
        let current_hex = config
            .encryption_key
            .as_deref()
            .expect("ENCRYPTION_KEY must be set when KEY_PROVIDER=local");
        let current_bytes: Zeroizing<[u8; 32]> = Zeroizing::new(
            hex::decode(current_hex)
                .expect(
                    "ENCRYPTION_KEY is not valid hex (should have been caught by validate_encryption_key)",
                )
                .try_into()
                .expect("ENCRYPTION_KEY must decode to 32 bytes"),
        );

        let previous_bytes: Option<Zeroizing<[u8; 32]>> = config
            .encryption_key_previous
            .as_ref()
            .filter(|prev| prev.as_str() != current_hex)
            .map(|hex_str| {
                Zeroizing::new(
                    hex::decode(hex_str)
                        .expect(
                            "ENCRYPTION_KEY_PREVIOUS is not valid hex (should have been caught by validate_encryption_key)",
                        )
                        .try_into()
                        .expect("ENCRYPTION_KEY_PREVIOUS must decode to 32 bytes"),
                )
            });

        Self::new(*current_bytes, previous_bytes.map(|z| *z))
    }
}

impl KeyProvider for LocalKeyProvider {
    fn wrap_dek(&self, plaintext_dek: &[u8]) -> Result<WrappedKey, AppError> {
        let cipher = Aes256Gcm::new_from_slice(self.current.as_ref())
            .map_err(|e| AppError::Internal(format!("Failed to create KEK cipher: {e}")))?;

        let mut nonce_bytes = [0u8; NONCE_SIZE];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let encrypted = cipher
            .encrypt(nonce, plaintext_dek)
            .map_err(|e| AppError::Internal(format!("AES DEK wrapping failed: {e}")))?;

        // ciphertext = nonce(12) || encrypted_dek(32) || tag(16) = 60 bytes
        let mut ciphertext = Vec::with_capacity(NONCE_SIZE + encrypted.len());
        ciphertext.extend_from_slice(&nonce_bytes);
        ciphertext.extend_from_slice(&encrypted);

        Ok(WrappedKey {
            key_id: self.current_id,
            ciphertext,
        })
    }

    fn unwrap_dek(&self, wrapped: &WrappedKey) -> Result<Zeroizing<Vec<u8>>, AppError> {
        // Find the matching key for the version
        let key = if wrapped.key_id == self.current_id {
            &self.current
        } else if self.previous_id == Some(wrapped.key_id) {
            self.previous.as_ref().ok_or_else(|| {
                AppError::Internal("Previous key id matched but key is missing".to_string())
            })?
        } else {
            return Err(AppError::Internal(
                "No key available for key id".to_string(),
            ));
        };

        if wrapped.ciphertext.len() < NONCE_SIZE {
            return Err(AppError::Internal(
                "Wrapped DEK ciphertext too short".to_string(),
            ));
        }

        let (nonce_bytes, encrypted) = wrapped.ciphertext.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);

        let cipher = Aes256Gcm::new_from_slice(key.as_ref())
            .map_err(|e| AppError::Internal(format!("Failed to create KEK cipher: {e}")))?;

        cipher
            .decrypt(nonce, encrypted)
            .map(Zeroizing::new)
            .map_err(|e| AppError::Internal(format!("DEK unwrap failed: {e}")))
    }

    fn current_key_id(&self) -> u8 {
        self.current_id
    }

    fn has_key_id(&self, key_id: u8) -> bool {
        key_id == self.current_id || self.previous_id == Some(key_id)
    }

    fn has_previous_key(&self) -> bool {
        self.previous.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key_a() -> [u8; 32] {
        [0xAAu8; 32]
    }

    fn test_key_b() -> [u8; 32] {
        [0xBBu8; 32]
    }

    #[test]
    fn roundtrip_wrap_unwrap() {
        let provider = LocalKeyProvider::new(test_key_a(), None);
        let dek = [0x42u8; 32];

        let wrapped = provider.wrap_dek(&dek).unwrap();
        assert_eq!(wrapped.key_id, provider.current_key_id());

        let unwrapped = provider.unwrap_dek(&wrapped).unwrap();
        assert_eq!(unwrapped.as_slice(), &dek);
    }

    #[test]
    fn different_ciphertext_on_same_dek() {
        let provider = LocalKeyProvider::new(test_key_a(), None);
        let dek = [0x42u8; 32];

        let wrapped1 = provider.wrap_dek(&dek).unwrap();
        let wrapped2 = provider.wrap_dek(&dek).unwrap();

        // Different nonces produce different ciphertexts
        assert_ne!(wrapped1.ciphertext, wrapped2.ciphertext);

        // Both unwrap to same DEK
        assert_eq!(provider.unwrap_dek(&wrapped1).unwrap().as_slice(), &dek);
        assert_eq!(provider.unwrap_dek(&wrapped2).unwrap().as_slice(), &dek);
    }

    #[test]
    fn wrong_version_fails() {
        let provider = LocalKeyProvider::new(test_key_a(), None);
        let dek = [0x42u8; 32];

        let mut wrapped = provider.wrap_dek(&dek).unwrap();
        wrapped.key_id = 0xFF; // nonexistent key id

        assert!(provider.unwrap_dek(&wrapped).is_err());
    }

    #[test]
    fn previous_key_unwrap() {
        let provider = LocalKeyProvider::new(test_key_b(), Some(test_key_a()));
        let dek = [0x42u8; 32];

        // Wrap with key A (simulate old data)
        let provider_a = LocalKeyProvider::new(test_key_a(), None);
        let wrapped = provider_a.wrap_dek(&dek).unwrap();

        // Unwrap with provider that has key A as previous
        let unwrapped = provider.unwrap_dek(&wrapped).unwrap();
        assert_eq!(unwrapped.as_slice(), &dek);
    }

    #[test]
    fn has_previous_version_flag() {
        let no_prev = LocalKeyProvider::new(test_key_a(), None);
        assert!(!no_prev.has_previous_key());

        let with_prev = LocalKeyProvider::new(test_key_a(), Some(test_key_b()));
        assert!(with_prev.has_previous_key());
    }

    #[test]
    fn debug_redacts_keys() {
        let provider = LocalKeyProvider::new(test_key_a(), Some(test_key_b()));
        let debug_str = format!("{:?}", provider);

        assert!(debug_str.contains("REDACTED"));
        assert!(!debug_str.contains("aa"));
        assert!(!debug_str.contains("bb"));
    }

    #[test]
    fn from_config_builds_correctly() {
        let config = crate::config::AppConfig {
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
            encryption_key: Some("aa".repeat(32)),
            encryption_key_previous: Some("bb".repeat(32)),
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
            key_provider: "local".to_string(),
        };

        let provider = LocalKeyProvider::from_config(&config);
        assert!(provider.has_previous_key());

        // Verify roundtrip works
        let dek = [0x42u8; 32];
        let wrapped = provider.wrap_dek(&dek).unwrap();
        let unwrapped = provider.unwrap_dek(&wrapped).unwrap();
        assert_eq!(unwrapped.as_slice(), &dek);
    }

    #[test]
    fn wrapped_dek_size() {
        let provider = LocalKeyProvider::new(test_key_a(), None);
        let dek = [0x42u8; 32];

        let wrapped = provider.wrap_dek(&dek).unwrap();
        // nonce(12) + encrypted_dek(32) + tag(16) = 60 bytes
        assert_eq!(wrapped.ciphertext.len(), 60);
    }

    #[test]
    fn same_previous_key_is_ignored() {
        let provider = LocalKeyProvider::new(test_key_a(), Some(test_key_a()));
        assert!(!provider.has_previous_key());
    }

    #[test]
    #[should_panic(expected = "same key id")]
    fn key_id_collision_panics() {
        // Brute-force two 32-byte keys with the same SHA-256 first byte.
        use super::super::key_provider::derive_key_id;
        let base = test_key_a();
        let base_id = derive_key_id(&base);

        let mut colliding = [0u8; 32];
        for i in 0u32.. {
            colliding[..4].copy_from_slice(&i.to_le_bytes());
            if derive_key_id(&colliding) == base_id && colliding != base {
                break;
            }
        }
        // This should panic with the collision message
        LocalKeyProvider::new(base, Some(colliding));
    }
}
