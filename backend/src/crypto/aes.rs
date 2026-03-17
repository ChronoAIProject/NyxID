use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use rand::RngCore;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use zeroize::Zeroizing;

use crate::config::AppConfig;
use crate::errors::AppError;

use super::key_provider::{KeyProvider, WrappedKey, derive_key_id};
use super::local_key_provider::LocalKeyProvider;

/// Nonce size for AES-256-GCM (96 bits / 12 bytes).
const NONCE_SIZE: usize = 12;

/// AES-256-GCM authentication tag size (128 bits / 16 bytes).
const TAG_SIZE: usize = 16;

/// Version byte for the v1 envelope format.
const VERSION_V1: u8 = 0x01;

/// Version byte for the v2 envelope encryption format (per-record DEK).
const VERSION_V2: u8 = 0x02;

/// Draft key IDs used by the initial uncommitted Phase 1 implementation.
/// We keep support for these so locally written draft ciphertexts still decrypt
/// after the stable key-id fix in this patch.
const DRAFT_KEY_ID_CURRENT: u8 = 0x00;
const DRAFT_KEY_ID_PREVIOUS: u8 = 0x01;

/// Size of the v1 header: version byte + key ID byte.
const V1_HEADER_SIZE: usize = 2;

/// Minimum v1 ciphertext: header(2) + nonce(12) + tag(16) = 30 bytes.
const V1_MIN_SIZE: usize = V1_HEADER_SIZE + NONCE_SIZE + TAG_SIZE;

/// Size of the v2 header: version(1) + kek_id(1) + wrapped_dek_len(2 BE) = 4 bytes.
const V2_HEADER_SIZE: usize = 4;

/// Maximum allowed wrapped DEK size (1024 bytes).
///
/// LocalKeyProvider produces 60-byte wrapped DEKs; KMS providers ~170-200 bytes.
/// This upper bound guards against corrupted headers or unexpectedly large KMS
/// responses causing unbounded allocations.
const MAX_WRAPPED_DEK_SIZE: usize = 1024;

/// Minimum v2 ciphertext: header(4) + wrapped_dek(1+) + data_nonce(12) + tag(16).
const V2_MIN_SIZE: usize = V2_HEADER_SIZE + 1 + NONCE_SIZE + TAG_SIZE;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EncryptionDecryptStats {
    pub v2_current: u64,
    pub v2_previous: u64,
    pub v2_fallback: u64,
    pub v1_current: u64,
    pub v1_previous: u64,
    pub v0_current: u64,
    pub v0_previous: u64,
    pub unknown_key_id_failures: u64,
    pub decrypt_failures: u64,
}

#[derive(Default)]
struct DecryptCounters {
    v2_current: AtomicU64,
    v2_previous: AtomicU64,
    v2_fallback: AtomicU64,
    v1_current: AtomicU64,
    v1_previous: AtomicU64,
    v0_current: AtomicU64,
    v0_previous: AtomicU64,
    unknown_key_id_failures: AtomicU64,
    decrypt_failures: AtomicU64,
    logged_v2_previous: AtomicBool,
    logged_v2_fallback: AtomicBool,
    logged_v1_previous: AtomicBool,
    logged_v0_current: AtomicBool,
    logged_v0_previous: AtomicBool,
    logged_unknown_key_id: AtomicBool,
}

impl DecryptCounters {
    fn snapshot(&self) -> EncryptionDecryptStats {
        EncryptionDecryptStats {
            v2_current: self.v2_current.load(Ordering::Relaxed),
            v2_previous: self.v2_previous.load(Ordering::Relaxed),
            v2_fallback: self.v2_fallback.load(Ordering::Relaxed),
            v1_current: self.v1_current.load(Ordering::Relaxed),
            v1_previous: self.v1_previous.load(Ordering::Relaxed),
            v0_current: self.v0_current.load(Ordering::Relaxed),
            v0_previous: self.v0_previous.load(Ordering::Relaxed),
            unknown_key_id_failures: self.unknown_key_id_failures.load(Ordering::Relaxed),
            decrypt_failures: self.decrypt_failures.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone)]
pub(crate) struct LegacyKeys {
    current: Zeroizing<[u8; 32]>,
    current_id: u8,
    previous: Option<Zeroizing<[u8; 32]>>,
    previous_id: Option<u8>,
}

impl LegacyKeys {
    pub(crate) fn from_config(config: &AppConfig) -> Self {
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
        let current_id = derive_key_id(current_bytes.as_ref());

        let previous = config
            .encryption_key_previous
            .as_ref()
            .filter(|prev| prev.as_str() != current_hex)
            .map(|hex_str| {
                let bytes: Zeroizing<[u8; 32]> = Zeroizing::new(
                    hex::decode(hex_str)
                        .expect(
                            "ENCRYPTION_KEY_PREVIOUS is not valid hex (should have been caught by validate_encryption_key)",
                        )
                        .try_into()
                        .expect("ENCRYPTION_KEY_PREVIOUS must decode to 32 bytes"),
                );
                let id = derive_key_id(bytes.as_ref());
                (bytes, id)
            });

        if let Some((_, previous_id)) = previous.as_ref()
            && current_id == *previous_id
        {
            panic!(
                "ENCRYPTION_KEY and ENCRYPTION_KEY_PREVIOUS produce the same key id (0x{:02x}). \
                 This is a 1-in-256 hash collision. Generate a different key with: openssl rand -hex 32",
                current_id
            );
        }

        Self {
            current: Zeroizing::new(*current_bytes),
            current_id,
            previous: previous.as_ref().map(|(bytes, _)| Zeroizing::new(**bytes)),
            previous_id: previous.as_ref().map(|(_, key_id)| *key_id),
        }
    }
}

/// Holds the current and (optionally) previous encryption keys for AES-256-GCM.
///
/// New encryptions always use `current` and stamp the ciphertext with a stable
/// key id derived from the key material itself. Decryption supports the
/// currently configured key plus a single previous key.
///
/// The `provider` field delegates DEK wrap/unwrap to a [`KeyProvider`]
/// implementation (e.g. [`LocalKeyProvider`] for in-process AES-256-GCM,
/// or a KMS backend in Phase 4+). Optional raw legacy keys are retained only
/// for v0/v1 decryption fallback.
pub struct EncryptionKeys {
    /// KeyProvider for v2 envelope DEK wrap/unwrap operations.
    provider: Arc<dyn KeyProvider>,
    /// Optional fallback provider for v2 DEKs wrapped by a previous provider
    /// (e.g., local provider during migration to KMS).
    fallback_provider: Option<Arc<dyn KeyProvider>>,
    /// Raw key material used only for legacy v0/v1 decrypt fallback.
    legacy: Option<LegacyKeys>,
    counters: DecryptCounters,
}

impl std::fmt::Debug for EncryptionKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptionKeys")
            .field("provider", &self.provider)
            .field(
                "fallback_provider",
                if self.fallback_provider.is_some() {
                    &"Some(...)"
                } else {
                    &"None"
                },
            )
            .field(
                "legacy",
                if self.legacy.is_some() {
                    &"Some([REDACTED])"
                } else {
                    &"None"
                },
            )
            .finish()
    }
}

impl EncryptionKeys {
    /// Build from a [`KeyProvider`].
    ///
    /// This constructor is provider-agnostic: only v2 envelope operations are
    /// available unless legacy raw keys are attached separately.
    pub fn with_provider(provider: Arc<dyn KeyProvider>) -> Self {
        Self {
            provider,
            fallback_provider: None,
            legacy: None,
            counters: DecryptCounters::default(),
        }
    }

    /// Build with a primary and optional fallback provider.
    /// Used during migration from one provider to another (e.g., local -> KMS).
    pub fn with_provider_and_fallback(
        provider: Arc<dyn KeyProvider>,
        fallback: Option<Arc<dyn KeyProvider>>,
    ) -> Self {
        Self {
            provider,
            fallback_provider: fallback,
            legacy: None,
            counters: DecryptCounters::default(),
        }
    }

    /// Build from validated AppConfig using a [`LocalKeyProvider`].
    ///
    /// This is a convenience constructor for the common case where keys
    /// are stored in environment variables.
    pub fn from_config(config: &AppConfig) -> Self {
        let provider = Arc::new(LocalKeyProvider::from_config(config));
        let legacy = LegacyKeys::from_config(config);
        let mut keys = Self::with_provider(provider);
        keys.legacy = Some(legacy);
        keys
    }

    /// Attach legacy keys for v0/v1 decrypt fallback.
    pub(crate) fn set_legacy(&mut self, legacy: LegacyKeys) {
        self.legacy = Some(legacy);
    }

    /// Returns true if a previous key is configured.
    pub fn has_previous(&self) -> bool {
        self.provider.has_previous_key()
    }

    /// Returns counters for each decrypt path. Useful during rotation to verify
    /// whether traffic still depends on legacy or previous-key ciphertexts.
    pub fn decrypt_stats(&self) -> EncryptionDecryptStats {
        self.counters.snapshot()
    }

    /// Encrypt plaintext using the v2 envelope encryption format.
    ///
    /// A fresh random DEK encrypts the data, then the DEK is wrapped by the
    /// current KEK via the configured [`KeyProvider`]. The DEK is zeroized
    /// after use.
    ///
    /// Output: `0x02 || kek_id(1) || wrapped_dek_len(2 BE) || wrapped_dek(N) || data_nonce(12) || data_ciphertext || data_tag(16)`
    pub async fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
        // Generate random 32-byte DEK
        let mut dek = Zeroizing::new([0u8; 32]);
        rand::thread_rng().fill_bytes(dek.as_mut());

        // Encrypt plaintext with DEK (AES-256-GCM)
        let data_cipher = Aes256Gcm::new_from_slice(dek.as_ref())
            .map_err(|e| AppError::Internal(format!("Failed to create DEK cipher: {e}")))?;
        let mut data_nonce_bytes = [0u8; NONCE_SIZE];
        rand::thread_rng().fill_bytes(&mut data_nonce_bytes);
        let data_nonce = Nonce::from_slice(&data_nonce_bytes);
        let data_ciphertext = data_cipher
            .encrypt(data_nonce, plaintext)
            .map_err(|e| AppError::Internal(format!("AES data encryption failed: {e}")))?;

        // Wrap DEK with current KEK via provider
        let wrapped = self.provider.wrap_dek(dek.as_ref()).await?;
        if wrapped.ciphertext.len() > MAX_WRAPPED_DEK_SIZE {
            return Err(AppError::Internal(
                "Wrapped DEK exceeds maximum size".to_string(),
            ));
        }
        let wrapped_dek_len = wrapped.ciphertext.len() as u16;

        // Assemble v2 envelope
        let total_size =
            V2_HEADER_SIZE + wrapped_dek_len as usize + NONCE_SIZE + data_ciphertext.len();
        let mut result = Vec::with_capacity(total_size);
        result.push(VERSION_V2);
        result.push(wrapped.key_id);
        result.extend_from_slice(&wrapped_dek_len.to_be_bytes());
        result.extend_from_slice(&wrapped.ciphertext);
        result.extend_from_slice(&data_nonce_bytes);
        result.extend_from_slice(&data_ciphertext);

        // DEK is automatically zeroized when `dek` drops
        Ok(result)
    }

    /// Decrypt ciphertext, trying the fallback chain:
    ///
    /// 1. If it looks like v2: try v2 envelope with current KEK, then previous KEK
    /// 2. If it looks like v1: try v1 payload with current key, then previous key
    /// 3. Try v0 (raw `nonce || ciphertext || tag`) with current key
    /// 4. Try v0 with previous key
    /// 5. Return error if all fail
    pub async fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
        let mut unknown_key_id = None;
        let current_key_id = self.provider.current_key_id();

        // -- v2 envelope encryption --
        if looks_like_v2(ciphertext) {
            let kek_id = ciphertext[1];
            let wrapped_dek_len = u16::from_be_bytes([ciphertext[2], ciphertext[3]]) as usize;
            let required_len = V2_HEADER_SIZE + wrapped_dek_len + NONCE_SIZE + TAG_SIZE;

            // Treat oversized wrapped-DEK lengths as a malformed v2 parse and
            // fall through to the legacy v1/v0 chain. Random nonce bytes from
            // valid v0 ciphertexts can look like a v2 header.
            if wrapped_dek_len <= MAX_WRAPPED_DEK_SIZE
                && wrapped_dek_len > 0
                && ciphertext.len() >= required_len
            {
                let wrapped_dek = &ciphertext[V2_HEADER_SIZE..V2_HEADER_SIZE + wrapped_dek_len];
                let data_payload = &ciphertext[V2_HEADER_SIZE + wrapped_dek_len..];

                let wrapped = WrappedKey {
                    key_id: kek_id,
                    ciphertext: Zeroizing::new(wrapped_dek.to_vec()),
                };

                match self.provider.unwrap_dek(&wrapped).await {
                    Ok(dek_bytes) => {
                        // dek_bytes is Zeroizing<Vec<u8>> -- automatically
                        // scrubbed from memory when this scope exits.

                        // Track which counter to bump based on kek_id
                        if kek_id == current_key_id {
                            self.counters.v2_current.fetch_add(1, Ordering::Relaxed);
                        } else {
                            self.counters.v2_previous.fetch_add(1, Ordering::Relaxed);
                            self.log_once(
                                &self.counters.logged_v2_previous,
                                "Decrypted v2 envelope with ENCRYPTION_KEY_PREVIOUS; old-key ciphertexts are still in active use",
                            );
                        }

                        // Decrypt data with DEK
                        let dek =
                            Zeroizing::new(<[u8; 32]>::try_from(dek_bytes.as_slice()).map_err(
                                |_| AppError::Internal("Unwrapped DEK is not 32 bytes".to_string()),
                            )?);
                        return decrypt_raw(data_payload, dek.as_ref());
                    }
                    Err(_) => {
                        // Primary provider could not unwrap; try fallback provider
                        if let Some(ref fallback) = self.fallback_provider
                            && fallback.has_key_id(kek_id)
                            && let Ok(dek_bytes) = fallback.unwrap_dek(&wrapped).await
                        {
                            self.counters.v2_fallback.fetch_add(1, Ordering::Relaxed);
                            self.log_once(
                                &self.counters.logged_v2_fallback,
                                "Decrypted v2 envelope via fallback provider; migration from previous provider is still in progress",
                            );

                            let dek = Zeroizing::new(
                                <[u8; 32]>::try_from(dek_bytes.as_slice()).map_err(
                                    |_| {
                                        AppError::Internal(
                                            "Unwrapped DEK is not 32 bytes".to_string(),
                                        )
                                    },
                                )?,
                            );
                            return decrypt_raw(data_payload, dek.as_ref());
                        }

                        // Provider could not unwrap; if it is not the current or
                        // previous key, record the unknown key id for the error
                        // message later.
                        if !self.provider.has_key_id(kek_id) {
                            unknown_key_id = Some(kek_id);
                        }
                        // Fall through to v1/v0 fallback
                    }
                }
            }
        }

        // -- v1 versioned format --
        if looks_like_v1(ciphertext) {
            let key_id = ciphertext[1];
            let payload = &ciphertext[V1_HEADER_SIZE..];

            if let Some(legacy) = self.legacy.as_ref() {
                if key_id == legacy.current_id {
                    if let Ok(plain) = decrypt_raw(payload, legacy.current.as_ref()) {
                        self.counters.v1_current.fetch_add(1, Ordering::Relaxed);
                        return Ok(plain);
                    }
                } else if legacy.previous_id == Some(key_id) {
                    if let Some(ref prev) = legacy.previous
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
                    if let Ok(plain) = decrypt_raw(payload, legacy.current.as_ref()) {
                        self.counters.v1_current.fetch_add(1, Ordering::Relaxed);
                        return Ok(plain);
                    }

                    if let Some(ref prev) = legacy.previous
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
            } else {
                unknown_key_id = Some(key_id);
            }
        }

        if let Some(legacy) = self.legacy.as_ref() {
            // Try v0 format (full ciphertext is nonce || encrypted || tag) with current key
            if let Ok(plain) = decrypt_raw(ciphertext, legacy.current.as_ref()) {
                self.counters.v0_current.fetch_add(1, Ordering::Relaxed);
                self.log_once(
                    &self.counters.logged_v0_current,
                    "Decrypted legacy v0 ciphertext with ENCRYPTION_KEY; re-encryption is still pending",
                );
                return Ok(plain);
            }

            // Try v0 with previous key
            if let Some(ref prev) = legacy.previous
                && let Ok(plain) = decrypt_raw(ciphertext, prev.as_ref())
            {
                self.counters.v0_previous.fetch_add(1, Ordering::Relaxed);
                self.log_once(
                    &self.counters.logged_v0_previous,
                    "Decrypted legacy v0 ciphertext with ENCRYPTION_KEY_PREVIOUS; old-key ciphertexts are still in active use",
                );
                return Ok(plain);
            }
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

    /// Re-wrap a v2 ciphertext's DEK from the previous KEK to the current KEK.
    ///
    /// Only the wrapped DEK portion changes; the encrypted data is untouched.
    /// Returns the original ciphertext unchanged if it is already wrapped with
    /// the current KEK.
    pub async fn rewrap(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
        if !looks_like_v2(ciphertext) {
            return Err(AppError::Internal(
                "rewrap() only supports v2 envelope format".to_string(),
            ));
        }

        let kek_id = ciphertext[1];
        let wrapped_dek_len = u16::from_be_bytes([ciphertext[2], ciphertext[3]]) as usize;
        let required_len = V2_HEADER_SIZE + wrapped_dek_len + NONCE_SIZE + TAG_SIZE;
        if wrapped_dek_len == 0 || ciphertext.len() < required_len {
            return Err(AppError::Internal(
                "Malformed v2 ciphertext: too short".to_string(),
            ));
        }

        // Already wrapped with current KEK -- return as-is.
        if kek_id == self.provider.current_key_id() {
            return Ok(ciphertext.to_vec());
        }

        let wrapped_dek = &ciphertext[V2_HEADER_SIZE..V2_HEADER_SIZE + wrapped_dek_len];
        let data_portion = &ciphertext[V2_HEADER_SIZE + wrapped_dek_len..];

        // Unwrap DEK with previous key via provider, falling back to fallback provider.
        let old_wrapped = WrappedKey {
            key_id: kek_id,
            ciphertext: Zeroizing::new(wrapped_dek.to_vec()),
        };
        let plaintext_dek = match self.provider.unwrap_dek(&old_wrapped).await {
            Ok(dek) => dek,
            Err(_) => {
                // Try fallback for migration rewrap
                self.fallback_provider
                    .as_ref()
                    .ok_or_else(|| {
                        AppError::Internal("No provider could unwrap DEK for rewrap".into())
                    })?
                    .unwrap_dek(&old_wrapped)
                    .await?
            }
        };

        // Wrap with current key via provider (plaintext_dek is Zeroizing<Vec<u8>>).
        let new_wrapped = self.provider.wrap_dek(&plaintext_dek).await?;
        let new_wrapped_len = new_wrapped.ciphertext.len() as u16;

        // Assemble new v2 envelope with current KEK.
        let total = V2_HEADER_SIZE + new_wrapped_len as usize + data_portion.len();
        let mut result = Vec::with_capacity(total);
        result.push(VERSION_V2);
        result.push(new_wrapped.key_id);
        result.extend_from_slice(&new_wrapped_len.to_be_bytes());
        result.extend_from_slice(&new_wrapped.ciphertext);
        result.extend_from_slice(data_portion);

        Ok(result)
    }

    fn log_once(&self, flag: &AtomicBool, message: &str) {
        if !flag.swap(true, Ordering::Relaxed) {
            tracing::warn!("{message}");
        }
    }
}

/// Check if data looks like a v2 envelope.
fn looks_like_v2(data: &[u8]) -> bool {
    data.len() >= V2_MIN_SIZE && data[0] == VERSION_V2
}

/// Check if data looks like a v1 envelope.
fn looks_like_v1(data: &[u8]) -> bool {
    data.len() >= V1_MIN_SIZE && data[0] == VERSION_V1
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

/// Encrypt plaintext using AES-256-GCM with the v1 format (kept for tests).
///
/// Output: `0x01 || key_id(1) || nonce(12) || ciphertext || tag(16)`
#[cfg(test)]
fn encrypt_v1(plaintext: &[u8], key: &[u8]) -> Result<Vec<u8>, AppError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| AppError::Internal(format!("Failed to create AES cipher: {e}")))?;

    let mut nonce_bytes = [0u8; NONCE_SIZE];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| AppError::Internal(format!("AES encryption failed: {e}")))?;

    let key_id = derive_key_id(key);
    let mut result = Vec::with_capacity(V1_HEADER_SIZE + NONCE_SIZE + ciphertext.len());
    result.push(VERSION_V1);
    result.push(key_id);
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);

    Ok(result)
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
    use async_trait::async_trait;
    use zeroize::Zeroizing;

    /// Size of a wrapped DEK produced by LocalKeyProvider:
    /// dek_nonce(12) + encrypted_dek(32) + dek_tag(16) = 60 bytes.
    const WRAPPED_DEK_SIZE: usize = NONCE_SIZE + 32 + TAG_SIZE;

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
            encryption_key: Some(key_hex.to_string()),
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
            key_provider: "local".to_string(),
            aws_kms_key_arn: None,
            aws_kms_key_arn_previous: None,
            gcp_kms_key_name: None,
            gcp_kms_key_name_previous: None,
            cors_allowed_origins: vec![],
            node_heartbeat_interval_secs: 30,
            node_heartbeat_timeout_secs: 90,
            node_proxy_timeout_secs: 30,
            node_registration_token_ttl_secs: 3600,
            node_max_per_user: 10,
            node_max_ws_connections: 100,
            node_max_stream_duration_secs: 300,
            node_hmac_signing_enabled: true,
        }
    }

    #[derive(Debug)]
    struct MockKeyProvider {
        current_key_id: u8,
        current_mask: u8,
        previous: Option<(u8, u8)>,
    }

    impl MockKeyProvider {
        fn new(current_key_id: u8, current_mask: u8, previous: Option<(u8, u8)>) -> Self {
            Self {
                current_key_id,
                current_mask,
                previous,
            }
        }
    }

    #[async_trait]
    impl KeyProvider for MockKeyProvider {
        async fn wrap_dek(&self, plaintext_dek: &[u8]) -> Result<WrappedKey, AppError> {
            let mut ciphertext = Vec::with_capacity(3 + plaintext_dek.len());
            ciphertext.push(self.current_mask);
            ciphertext.push(0xA5);
            ciphertext.push(plaintext_dek.len() as u8);
            ciphertext.extend(plaintext_dek.iter().map(|byte| byte ^ self.current_mask));

            Ok(WrappedKey {
                key_id: self.current_key_id,
                ciphertext: Zeroizing::new(ciphertext),
            })
        }

        async fn unwrap_dek(&self, wrapped: &WrappedKey) -> Result<Zeroizing<Vec<u8>>, AppError> {
            let mask = if wrapped.key_id == self.current_key_id {
                self.current_mask
            } else if let Some((previous_key_id, previous_mask)) = self.previous {
                if wrapped.key_id == previous_key_id {
                    previous_mask
                } else {
                    return Err(AppError::Internal(
                        "No key available for key id".to_string(),
                    ));
                }
            } else {
                return Err(AppError::Internal(
                    "No key available for key id".to_string(),
                ));
            };

            if wrapped.ciphertext.len() < 3
                || wrapped.ciphertext[0] != mask
                || wrapped.ciphertext[1] != 0xA5
            {
                return Err(AppError::Internal("Mock unwrap failed".to_string()));
            }

            let dek_len = wrapped.ciphertext[2] as usize;
            if wrapped.ciphertext.len() != 3 + dek_len {
                return Err(AppError::Internal(
                    "Mock wrapped DEK length mismatch".to_string(),
                ));
            }

            Ok(Zeroizing::new(
                wrapped.ciphertext[3..]
                    .iter()
                    .map(|byte| byte ^ mask)
                    .collect(),
            ))
        }

        fn current_key_id(&self) -> u8 {
            self.current_key_id
        }

        fn has_key_id(&self, key_id: u8) -> bool {
            key_id == self.current_key_id
                || self.previous.map(|(previous_key_id, _)| previous_key_id) == Some(key_id)
        }

        fn has_previous_key(&self) -> bool {
            self.previous.is_some()
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

    // -- EncryptionKeys API tests --
    // These tests exercise the EncryptionKeys public API (encrypt/decrypt).
    // Originally written for v1, they now verify v2 output from encrypt() while
    // continuing to test v0/v1 backward-compatible decrypt paths. Tests named
    // "v1_decrypt_*" specifically test decryption of legacy v1 ciphertexts.

    #[tokio::test]
    async fn v1_roundtrip() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let plaintext = b"v1 encrypted data";
        let encrypted = keys.encrypt(plaintext).await.unwrap();

        // Verify v2 header (encrypt now produces v2 format)
        assert_eq!(encrypted[0], VERSION_V2);
        assert_eq!(encrypted[1], keys.provider.current_key_id());

        let decrypted = keys.decrypt(&encrypted).await.unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn v1_different_nonces() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let plaintext = b"same data";
        let enc1 = keys.encrypt(plaintext).await.unwrap();
        let enc2 = keys.encrypt(plaintext).await.unwrap();

        assert_ne!(enc1, enc2);
        assert_eq!(keys.decrypt(&enc1).await.unwrap(), plaintext);
        assert_eq!(keys.decrypt(&enc2).await.unwrap(), plaintext);
    }

    #[tokio::test]
    async fn v1_empty_plaintext() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let encrypted = keys.encrypt(b"").await.unwrap();
        let decrypted = keys.decrypt(&encrypted).await.unwrap();
        assert!(decrypted.is_empty());
    }

    #[tokio::test]
    async fn v1_large_plaintext() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let plaintext = vec![0x42u8; 10_000];
        let encrypted = keys.encrypt(&plaintext).await.unwrap();
        let decrypted = keys.decrypt(&encrypted).await.unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn v1_tamper_detection() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let mut encrypted = keys.encrypt(b"tamper test").await.unwrap();
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0xFF;

        assert!(keys.decrypt(&encrypted).await.is_err());
    }

    #[tokio::test]
    async fn v1_decrypt_v0_data_with_current_key() {
        // Simulate existing v0 data encrypted with the current key
        let key_hex = "cd".repeat(32);
        let key_bytes = hex::decode(&key_hex).unwrap();
        let config = test_config(&key_hex, None);
        let keys = EncryptionKeys::from_config(&config);

        let plaintext = b"legacy v0 data";
        let v0_encrypted = encrypt(plaintext, &key_bytes).unwrap();

        // EncryptionKeys should be able to decrypt v0 data
        let decrypted = keys.decrypt(&v0_encrypted).await.unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn v1_decrypt_v0_data_with_previous_key() {
        // Simulate key rotation: v0 data encrypted with old key, new key is now current
        let old_key_hex = "cd".repeat(32);
        let new_key_hex = "ef".repeat(32);
        let old_key_bytes = hex::decode(&old_key_hex).unwrap();
        let config = test_config(&new_key_hex, Some(&old_key_hex));
        let keys = EncryptionKeys::from_config(&config);

        let plaintext = b"old key data";
        let v0_encrypted = encrypt(plaintext, &old_key_bytes).unwrap();

        // Should decrypt using the previous key fallback
        let decrypted = keys.decrypt(&v0_encrypted).await.unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn v1_key_rotation_current_decrypts_previous_v1() {
        // Encrypt with key A as current, then rotate: key B becomes current, A becomes previous
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);
        let key_a_id = derive_key_id(&hex::decode(&key_a_hex).unwrap());

        // Phase 1: encrypt with key A (now produces v2)
        let config_a = test_config(&key_a_hex, None);
        let keys_a = EncryptionKeys::from_config(&config_a);
        let encrypted = keys_a.encrypt(b"rotation test").await.unwrap();
        assert_eq!(encrypted[1], key_a_id);

        // Phase 2: rotate - B is current, A is previous
        let config_rotated = test_config(&key_b_hex, Some(&key_a_hex));
        let keys_rotated = EncryptionKeys::from_config(&config_rotated);

        // Should still decrypt data encrypted under key A
        let decrypted = keys_rotated.decrypt(&encrypted).await.unwrap();
        assert_eq!(decrypted, b"rotation test");
    }

    #[tokio::test]
    async fn v1_rollback_scenario() {
        // Encrypt with key B as current, then rollback: A becomes current again, B becomes previous
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);

        // Phase 1: key B is current, encrypt some data
        let config_b = test_config(&key_b_hex, Some(&key_a_hex));
        let keys_b = EncryptionKeys::from_config(&config_b);
        let encrypted = keys_b.encrypt(b"rollback test").await.unwrap();

        // Phase 2: rollback - A is current again, B becomes previous
        let config_rollback = test_config(&key_a_hex, Some(&key_b_hex));
        let keys_rollback = EncryptionKeys::from_config(&config_rollback);

        let decrypted = keys_rollback.decrypt(&encrypted).await.unwrap();
        assert_eq!(decrypted, b"rollback test");
    }

    #[tokio::test]
    async fn v1_second_rotation_without_reencryption_fails_after_oldest_key_removed() {
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);
        let key_c_hex = "cc".repeat(32);

        let keys_a = EncryptionKeys::from_config(&test_config(&key_a_hex, None));
        let encrypted = keys_a.encrypt(b"still on key a").await.unwrap();

        let keys_b = EncryptionKeys::from_config(&test_config(&key_b_hex, Some(&key_a_hex)));
        assert_eq!(keys_b.decrypt(&encrypted).await.unwrap(), b"still on key a");

        let keys_c = EncryptionKeys::from_config(&test_config(&key_c_hex, Some(&key_b_hex)));
        assert!(keys_c.decrypt(&encrypted).await.is_err());
    }

    #[tokio::test]
    async fn v1_decrypt_supports_draft_phase1_header() {
        let key_hex = "ab".repeat(32);
        let key_bytes = hex::decode(&key_hex).unwrap();
        let keys = EncryptionKeys::from_config(&test_config(&key_hex, None));

        let mut encrypted = Vec::new();
        encrypted.push(VERSION_V1);
        encrypted.push(DRAFT_KEY_ID_CURRENT);
        encrypted.extend_from_slice(&encrypt(b"draft v1", &key_bytes).unwrap());

        assert_eq!(keys.decrypt(&encrypted).await.unwrap(), b"draft v1");
    }

    #[tokio::test]
    async fn v1_unknown_key_fails() {
        let key_a_hex = "aa".repeat(32);
        let key_c_hex = "cc".repeat(32);

        let config_a = test_config(&key_a_hex, None);
        let keys_a = EncryptionKeys::from_config(&config_a);
        let encrypted = keys_a.encrypt(b"secret").await.unwrap();

        // Try to decrypt with a completely different key
        let config_c = test_config(&key_c_hex, None);
        let keys_c = EncryptionKeys::from_config(&config_c);
        assert!(keys_c.decrypt(&encrypted).await.is_err());
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

    #[tokio::test]
    async fn v1_decrypt_stats_track_fallback_paths() {
        let current_hex = "ab".repeat(32);
        let previous_hex = "cd".repeat(32);
        let previous_bytes = hex::decode(&previous_hex).unwrap();
        let keys = EncryptionKeys::from_config(&test_config(&current_hex, Some(&previous_hex)));

        let v0_previous = encrypt(b"legacy previous", &previous_bytes).unwrap();
        assert_eq!(
            keys.decrypt(&v0_previous).await.unwrap(),
            b"legacy previous"
        );

        let stats = keys.decrypt_stats();
        assert_eq!(
            stats,
            EncryptionDecryptStats {
                v2_current: 0,
                v2_previous: 0,
                v2_fallback: 0,
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

    #[tokio::test]
    async fn v1_cross_version_roundtrip() {
        // Encrypt with v0 API, decrypt with EncryptionKeys (simulates migration)
        let key_hex = "dd".repeat(32);
        let key_bytes = hex::decode(&key_hex).unwrap();

        let plaintext = b"cross-version data";
        let v0_encrypted = encrypt(plaintext, &key_bytes).unwrap();

        let config = test_config(&key_hex, None);
        let keys = EncryptionKeys::from_config(&config);

        // v0 -> EncryptionKeys decrypt
        let decrypted = keys.decrypt(&v0_encrypted).await.unwrap();
        assert_eq!(decrypted, plaintext);

        // EncryptionKeys encrypt -> verify it's v2
        let v2_encrypted = keys.encrypt(plaintext).await.unwrap();
        assert_eq!(v2_encrypted[0], VERSION_V2);

        // v2 -> EncryptionKeys decrypt
        let decrypted2 = keys.decrypt(&v2_encrypted).await.unwrap();
        assert_eq!(decrypted2, plaintext);
    }

    // -- v1 backward compatibility: decrypt v1-formatted ciphertexts --

    #[tokio::test]
    async fn v1_decrypt_v1_format_ciphertext() {
        let key_hex = "ab".repeat(32);
        let key_bytes = hex::decode(&key_hex).unwrap();
        let keys = EncryptionKeys::from_config(&test_config(&key_hex, None));

        // Manually produce a v1-format ciphertext
        let v1_encrypted = encrypt_v1(b"v1 format data", &key_bytes).unwrap();
        assert_eq!(v1_encrypted[0], VERSION_V1);

        let decrypted = keys.decrypt(&v1_encrypted).await.unwrap();
        assert_eq!(decrypted, b"v1 format data");
    }

    #[tokio::test]
    async fn v1_decrypt_v1_format_with_previous_key() {
        let current_hex = "ab".repeat(32);
        let previous_hex = "cd".repeat(32);
        let previous_bytes = hex::decode(&previous_hex).unwrap();
        let keys = EncryptionKeys::from_config(&test_config(&current_hex, Some(&previous_hex)));

        let v1_encrypted = encrypt_v1(b"v1 previous key", &previous_bytes).unwrap();
        let decrypted = keys.decrypt(&v1_encrypted).await.unwrap();
        assert_eq!(decrypted, b"v1 previous key");
    }

    // -- v2 envelope encryption tests --

    #[tokio::test]
    async fn v2_roundtrip() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let plaintext = b"v2 envelope encrypted data";
        let encrypted = keys.encrypt(plaintext).await.unwrap();

        assert_eq!(encrypted[0], VERSION_V2);
        assert_eq!(encrypted[1], keys.provider.current_key_id());

        let wrapped_dek_len = u16::from_be_bytes([encrypted[2], encrypted[3]]) as usize;
        assert_eq!(wrapped_dek_len, WRAPPED_DEK_SIZE);

        let decrypted = keys.decrypt(&encrypted).await.unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn v2_different_deks() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let plaintext = b"same data";
        let enc1 = keys.encrypt(plaintext).await.unwrap();
        let enc2 = keys.encrypt(plaintext).await.unwrap();

        // Different DEKs and nonces means completely different output
        assert_ne!(enc1, enc2);

        assert_eq!(keys.decrypt(&enc1).await.unwrap(), plaintext);
        assert_eq!(keys.decrypt(&enc2).await.unwrap(), plaintext);
    }

    #[tokio::test]
    async fn v2_empty_plaintext() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let encrypted = keys.encrypt(b"").await.unwrap();
        assert_eq!(encrypted[0], VERSION_V2);

        let decrypted = keys.decrypt(&encrypted).await.unwrap();
        assert!(decrypted.is_empty());
    }

    #[tokio::test]
    async fn v2_large_plaintext() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let plaintext = vec![0x42u8; 100_000];
        let encrypted = keys.encrypt(&plaintext).await.unwrap();
        let decrypted = keys.decrypt(&encrypted).await.unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn v2_tamper_wrapped_dek() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let mut encrypted = keys.encrypt(b"tamper dek test").await.unwrap();
        // Flip a byte inside the wrapped DEK region (after header)
        encrypted[V2_HEADER_SIZE + 5] ^= 0xFF;

        assert!(keys.decrypt(&encrypted).await.is_err());
    }

    #[tokio::test]
    async fn v2_tamper_data() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let mut encrypted = keys.encrypt(b"tamper data test").await.unwrap();
        // Flip the last byte (in the data tag region)
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0xFF;

        assert!(keys.decrypt(&encrypted).await.is_err());
    }

    #[tokio::test]
    async fn v2_tamper_kek_id() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let mut encrypted = keys.encrypt(b"tamper kek_id test").await.unwrap();
        // Corrupt the kek_id byte
        encrypted[1] ^= 0xFF;

        assert!(keys.decrypt(&encrypted).await.is_err());
    }

    #[tokio::test]
    async fn v2_kek_rotation() {
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);

        // Encrypt with KEK-A
        let keys_a = EncryptionKeys::from_config(&test_config(&key_a_hex, None));
        let encrypted = keys_a.encrypt(b"kek rotation test").await.unwrap();

        // Rotate: KEK-B current, KEK-A previous
        let keys_rotated = EncryptionKeys::from_config(&test_config(&key_b_hex, Some(&key_a_hex)));
        let decrypted = keys_rotated.decrypt(&encrypted).await.unwrap();
        assert_eq!(decrypted, b"kek rotation test");

        // Verify stats show v2_previous
        let stats = keys_rotated.decrypt_stats();
        assert_eq!(stats.v2_previous, 1);
        assert_eq!(stats.v2_current, 0);
    }

    #[tokio::test]
    async fn v2_rewrap_roundtrip() {
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);

        // Encrypt with KEK-A
        let keys_a = EncryptionKeys::from_config(&test_config(&key_a_hex, None));
        let encrypted = keys_a.encrypt(b"rewrap test").await.unwrap();

        // Rotate: KEK-B current, KEK-A previous
        let keys_rotated = EncryptionKeys::from_config(&test_config(&key_b_hex, Some(&key_a_hex)));

        // Rewrap: unwrap DEK with KEK-A, re-wrap with KEK-B
        let rewrapped = keys_rotated.rewrap(&encrypted).await.unwrap();

        // Verify rewrapped ciphertext has KEK-B's id
        let key_b_id = derive_key_id(&hex::decode(&key_b_hex).unwrap());
        assert_eq!(rewrapped[0], VERSION_V2);
        assert_eq!(rewrapped[1], key_b_id);

        // Decrypt rewrapped ciphertext with KEK-B only (no previous)
        let keys_b_only = EncryptionKeys::from_config(&test_config(&key_b_hex, None));
        let decrypted = keys_b_only.decrypt(&rewrapped).await.unwrap();
        assert_eq!(decrypted, b"rewrap test");
    }

    #[tokio::test]
    async fn v2_rewrap_already_current() {
        let key_hex = "ab".repeat(32);
        let keys = EncryptionKeys::from_config(&test_config(&key_hex, None));

        let encrypted = keys.encrypt(b"already current").await.unwrap();
        let rewrapped = keys.rewrap(&encrypted).await.unwrap();

        // Should be identical (no-op)
        assert_eq!(encrypted, rewrapped);
    }

    #[tokio::test]
    async fn v2_rewrap_non_v2_fails() {
        let key_hex = "ab".repeat(32);
        let key_bytes = hex::decode(&key_hex).unwrap();
        let keys = EncryptionKeys::from_config(&test_config(&key_hex, None));

        // Try to rewrap v0 data
        let v0_data = encrypt(b"v0 data", &key_bytes).unwrap();
        assert!(keys.rewrap(&v0_data).await.is_err());

        // Try to rewrap v1 data
        let v1_data = encrypt_v1(b"v1 data", &key_bytes).unwrap();
        assert!(keys.rewrap(&v1_data).await.is_err());
    }

    #[tokio::test]
    async fn v2_rewrap_preserves_data() {
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);

        let keys_a = EncryptionKeys::from_config(&test_config(&key_a_hex, None));
        let encrypted = keys_a.encrypt(b"data must survive rewrap").await.unwrap();

        // Extract the data portion (after header + wrapped_dek)
        let wrapped_dek_len = u16::from_be_bytes([encrypted[2], encrypted[3]]) as usize;
        let data_before = &encrypted[V2_HEADER_SIZE + wrapped_dek_len..];

        // Rewrap
        let keys_rotated = EncryptionKeys::from_config(&test_config(&key_b_hex, Some(&key_a_hex)));
        let rewrapped = keys_rotated.rewrap(&encrypted).await.unwrap();

        // Data portion must be identical
        let new_wrapped_dek_len = u16::from_be_bytes([rewrapped[2], rewrapped[3]]) as usize;
        let data_after = &rewrapped[V2_HEADER_SIZE + new_wrapped_dek_len..];
        assert_eq!(data_before, data_after);
    }

    #[tokio::test]
    async fn v2_rollback() {
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);

        // Encrypt with KEK-B
        let keys_b = EncryptionKeys::from_config(&test_config(&key_b_hex, Some(&key_a_hex)));
        let encrypted = keys_b.encrypt(b"rollback v2 test").await.unwrap();

        // Rollback: KEK-A current, KEK-B previous
        let keys_rollback = EncryptionKeys::from_config(&test_config(&key_a_hex, Some(&key_b_hex)));
        let decrypted = keys_rollback.decrypt(&encrypted).await.unwrap();
        assert_eq!(decrypted, b"rollback v2 test");
    }

    #[tokio::test]
    async fn v2_decrypt_stats() {
        let current_hex = "ab".repeat(32);
        let previous_hex = "cd".repeat(32);
        let keys = EncryptionKeys::from_config(&test_config(&current_hex, Some(&previous_hex)));

        // Encrypt and decrypt with current key (v2)
        let encrypted = keys.encrypt(b"current key data").await.unwrap();
        keys.decrypt(&encrypted).await.unwrap();

        let stats = keys.decrypt_stats();
        assert_eq!(stats.v2_current, 1);
        assert_eq!(stats.v2_previous, 0);
        assert_eq!(stats.v1_current, 0);
        assert_eq!(stats.v0_current, 0);
    }

    #[tokio::test]
    async fn v2_decrypt_stats_previous() {
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);

        // Encrypt with KEK-A
        let keys_a = EncryptionKeys::from_config(&test_config(&key_a_hex, None));
        let encrypted = keys_a.encrypt(b"stats previous test").await.unwrap();

        // Rotate, decrypt with KEK-B (previous = KEK-A)
        let keys_b = EncryptionKeys::from_config(&test_config(&key_b_hex, Some(&key_a_hex)));
        keys_b.decrypt(&encrypted).await.unwrap();

        let stats = keys_b.decrypt_stats();
        assert_eq!(stats.v2_current, 0);
        assert_eq!(stats.v2_previous, 1);
    }

    #[tokio::test]
    async fn v2_cross_version_all_formats() {
        let key_hex = "dd".repeat(32);
        let key_bytes = hex::decode(&key_hex).unwrap();
        let keys = EncryptionKeys::from_config(&test_config(&key_hex, None));

        let plaintext = b"cross format data";

        // v0 decrypt
        let v0 = encrypt(plaintext, &key_bytes).unwrap();
        assert_eq!(keys.decrypt(&v0).await.unwrap(), plaintext);

        // v1 decrypt
        let v1 = encrypt_v1(plaintext, &key_bytes).unwrap();
        assert_eq!(keys.decrypt(&v1).await.unwrap(), plaintext);

        // v2 decrypt (via encrypt)
        let v2 = keys.encrypt(plaintext).await.unwrap();
        assert_eq!(keys.decrypt(&v2).await.unwrap(), plaintext);

        // Verify stats
        let stats = keys.decrypt_stats();
        assert_eq!(stats.v2_current, 1);
        assert_eq!(stats.v1_current, 1);
        assert_eq!(stats.v0_current, 1);
    }

    #[tokio::test]
    async fn v2_size_overhead() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let plaintext = b"measure overhead";
        let encrypted = keys.encrypt(plaintext).await.unwrap();

        // v2 overhead = header(4) + wrapped_dek(60) + data_nonce(12) + tag(16) = 92
        // Total = plaintext.len() + 92
        let expected_size =
            plaintext.len() + V2_HEADER_SIZE + WRAPPED_DEK_SIZE + NONCE_SIZE + TAG_SIZE;
        assert_eq!(encrypted.len(), expected_size);
    }

    #[tokio::test]
    async fn v2_rewrap_unknown_kek_id_fails() {
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);
        let key_c_hex = "cc".repeat(32);

        // Encrypt with KEK-A
        let keys_a = EncryptionKeys::from_config(&test_config(&key_a_hex, None));
        let encrypted = keys_a.encrypt(b"unknown kek test").await.unwrap();

        // Try to rewrap with KEK-C (current) + KEK-B (previous) -- neither matches KEK-A
        let keys_c = EncryptionKeys::from_config(&test_config(&key_c_hex, Some(&key_b_hex)));
        assert!(keys_c.rewrap(&encrypted).await.is_err());
    }

    #[tokio::test]
    async fn v2_rewrap_rejects_short_wrapped_dek_len() {
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);

        let keys_a = EncryptionKeys::from_config(&test_config(&key_a_hex, None));
        let mut encrypted = keys_a.encrypt(b"bad wrapped_dek_len").await.unwrap();

        // Rotate to KEK-B so rewrap() will use KEK-A as the previous key.
        let keys_b = EncryptionKeys::from_config(&test_config(&key_b_hex, Some(&key_a_hex)));

        // A wrapped DEK shorter than the nonce length previously panicked in
        // split_at(); it must now fail closed.
        encrypted[2] = 0x00;
        encrypted[3] = (NONCE_SIZE - 1) as u8;

        assert!(keys_b.rewrap(&encrypted).await.is_err());
    }

    #[tokio::test]
    async fn v2_second_rotation_without_rewrap_fails() {
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);
        let key_c_hex = "cc".repeat(32);

        // Encrypt with KEK-A
        let keys_a = EncryptionKeys::from_config(&test_config(&key_a_hex, None));
        let encrypted = keys_a.encrypt(b"needs rewrap").await.unwrap();

        // First rotation: B current, A previous -- still decryptable
        let keys_b = EncryptionKeys::from_config(&test_config(&key_b_hex, Some(&key_a_hex)));
        assert_eq!(keys_b.decrypt(&encrypted).await.unwrap(), b"needs rewrap");

        // Second rotation without rewrap: C current, B previous -- KEK-A gone
        let keys_c = EncryptionKeys::from_config(&test_config(&key_c_hex, Some(&key_b_hex)));
        assert!(keys_c.decrypt(&encrypted).await.is_err());
    }

    #[tokio::test]
    async fn v2_provider_only_roundtrip_without_legacy_keys() {
        let keys = EncryptionKeys::with_provider(Arc::new(MockKeyProvider::new(0x7A, 0x55, None)));

        let encrypted = keys.encrypt(b"provider only").await.unwrap();
        let wrapped_dek_len = u16::from_be_bytes([encrypted[2], encrypted[3]]) as usize;

        assert_eq!(encrypted[0], VERSION_V2);
        assert_eq!(encrypted[1], 0x7A);
        assert_eq!(wrapped_dek_len, 35);
        assert_eq!(keys.decrypt(&encrypted).await.unwrap(), b"provider only");
    }

    #[tokio::test]
    async fn v2_provider_only_rewrap_with_non_local_key_ids() {
        let old_keys =
            EncryptionKeys::with_provider(Arc::new(MockKeyProvider::new(0x11, 0x33, None)));
        let encrypted = old_keys.encrypt(b"provider rewrap").await.unwrap();

        let rotated_keys = EncryptionKeys::with_provider(Arc::new(MockKeyProvider::new(
            0x9C,
            0x77,
            Some((0x11, 0x33)),
        )));

        let decrypted = rotated_keys.decrypt(&encrypted).await.unwrap();
        assert_eq!(decrypted, b"provider rewrap");

        let rewrapped = rotated_keys.rewrap(&encrypted).await.unwrap();
        let wrapped_dek_len = u16::from_be_bytes([rewrapped[2], rewrapped[3]]) as usize;

        assert_eq!(rewrapped[1], 0x9C);
        assert_eq!(wrapped_dek_len, 35);

        let current_only =
            EncryptionKeys::with_provider(Arc::new(MockKeyProvider::new(0x9C, 0x77, None)));
        assert_eq!(
            current_only.decrypt(&rewrapped).await.unwrap(),
            b"provider rewrap"
        );
    }

    #[tokio::test]
    async fn v2_rewrap_then_drop_old_key() {
        let key_a_hex = "aa".repeat(32);
        let key_b_hex = "bb".repeat(32);
        let key_c_hex = "cc".repeat(32);

        // Encrypt with KEK-A
        let keys_a = EncryptionKeys::from_config(&test_config(&key_a_hex, None));
        let encrypted = keys_a.encrypt(b"rewrap chain").await.unwrap();

        // Rotate to KEK-B, rewrap
        let keys_b = EncryptionKeys::from_config(&test_config(&key_b_hex, Some(&key_a_hex)));
        let rewrapped = keys_b.rewrap(&encrypted).await.unwrap();

        // Second rotation: C current, B previous -- rewrapped data still works
        let keys_c = EncryptionKeys::from_config(&test_config(&key_c_hex, Some(&key_b_hex)));
        let decrypted = keys_c.decrypt(&rewrapped).await.unwrap();
        assert_eq!(decrypted, b"rewrap chain");
    }

    #[tokio::test]
    async fn v2_v0_collision_fallback() {
        // A valid v0 ciphertext can still look like v2 if its nonce starts
        // with 0x02 and the next bytes resemble a v2 header.
        let key_hex = "ab".repeat(32);
        let key_bytes = hex::decode(&key_hex).unwrap();
        let keys = EncryptionKeys::from_config(&test_config(&key_hex, None));

        let plaintext = vec![0x42; 80];
        let nonce_bytes = [
            VERSION_V2,
            derive_key_id(&key_bytes),
            0x00,
            WRAPPED_DEK_SIZE as u8,
            0x10,
            0x20,
            0x30,
            0x40,
            0x50,
            0x60,
            0x70,
            0x80,
        ];
        let nonce = Nonce::from_slice(&nonce_bytes);
        let cipher = Aes256Gcm::new_from_slice(&key_bytes).unwrap();
        let encrypted = cipher.encrypt(nonce, plaintext.as_slice()).unwrap();

        let mut v0 = Vec::with_capacity(NONCE_SIZE + encrypted.len());
        v0.extend_from_slice(&nonce_bytes);
        v0.extend_from_slice(&encrypted);

        assert!(looks_like_v2(&v0));
        assert_eq!(keys.decrypt(&v0).await.unwrap(), plaintext);
    }

    #[tokio::test]
    async fn v2_v0_collision_with_oversized_wrapped_len_still_falls_back() {
        // A valid v0 ciphertext can also mimic a malformed v2 header where the
        // declared wrapped-DEK length exceeds our safety bound. That must still
        // fall through to the legacy v0 decrypt path.
        let key_hex = "ab".repeat(32);
        let key_bytes = hex::decode(&key_hex).unwrap();
        let keys = EncryptionKeys::from_config(&test_config(&key_hex, None));

        let plaintext = vec![0x24; 96];
        let nonce_bytes = [
            VERSION_V2,
            derive_key_id(&key_bytes),
            0x20,
            0x00, // wrapped_dek_len = 8192 (> MAX_WRAPPED_DEK_SIZE)
            0x10,
            0x20,
            0x30,
            0x40,
            0x50,
            0x60,
            0x70,
            0x80,
        ];
        let nonce = Nonce::from_slice(&nonce_bytes);
        let cipher = Aes256Gcm::new_from_slice(&key_bytes).unwrap();
        let encrypted = cipher.encrypt(nonce, plaintext.as_slice()).unwrap();

        let mut v0 = Vec::with_capacity(NONCE_SIZE + encrypted.len());
        v0.extend_from_slice(&nonce_bytes);
        v0.extend_from_slice(&encrypted);

        assert!(looks_like_v2(&v0));
        assert_eq!(keys.decrypt(&v0).await.unwrap(), plaintext);
    }

    #[tokio::test]
    async fn v2_tamper_version_byte() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let mut encrypted = keys.encrypt(b"version byte test").await.unwrap();
        // Change version from 0x02 to 0x03
        encrypted[0] = 0x03;

        // Should fail: 0x03 is not recognized as v2 or v1, falls through to v0
        // which will also fail since the data has extra header bytes.
        assert!(keys.decrypt(&encrypted).await.is_err());
    }

    #[tokio::test]
    async fn v2_invalid_wrapped_dek_len() {
        let config = test_config(&"ab".repeat(32), None);
        let keys = EncryptionKeys::from_config(&config);

        let mut encrypted = keys.encrypt(b"bad length test").await.unwrap();
        // Set wrapped_dek_len to a value larger than the remaining ciphertext
        encrypted[2] = 0xFF;
        encrypted[3] = 0xFF;

        // Should fail: declared length exceeds actual data
        assert!(keys.decrypt(&encrypted).await.is_err());
    }

    #[tokio::test]
    async fn v2_fallback_provider_decrypt() {
        // Encrypt with "local" mock (key_id=0xAA)
        let local_mock = Arc::new(MockKeyProvider::new(0xAA, 0x33, None));
        let local_keys = EncryptionKeys::with_provider(local_mock.clone());
        let encrypted = local_keys.encrypt(b"fallback test data").await.unwrap();

        // Decrypt with "KMS" mock primary (key_id=0xBB) + "local" mock fallback (key_id=0xAA)
        let kms_mock = Arc::new(MockKeyProvider::new(0xBB, 0x77, None));
        let migration_keys = EncryptionKeys::with_provider_and_fallback(
            kms_mock,
            Some(local_mock as Arc<dyn KeyProvider>),
        );

        let decrypted = migration_keys.decrypt(&encrypted).await.unwrap();
        assert_eq!(decrypted, b"fallback test data");

        let stats = migration_keys.decrypt_stats();
        assert_eq!(stats.v2_fallback, 1);
        assert_eq!(stats.v2_current, 0);
    }

    #[tokio::test]
    async fn v2_fallback_provider_rewrap() {
        // Encrypt with "local" mock (key_id=0xAA)
        let local_mock = Arc::new(MockKeyProvider::new(0xAA, 0x33, None));
        let local_keys = EncryptionKeys::with_provider(local_mock.clone());
        let encrypted = local_keys.encrypt(b"rewrap via fallback").await.unwrap();

        // Rewrap: KMS primary (key_id=0xBB) + local fallback (key_id=0xAA)
        let kms_mock = Arc::new(MockKeyProvider::new(0xBB, 0x77, None));
        let migration_keys = EncryptionKeys::with_provider_and_fallback(
            kms_mock.clone(),
            Some(local_mock as Arc<dyn KeyProvider>),
        );

        let rewrapped = migration_keys.rewrap(&encrypted).await.unwrap();
        assert_eq!(rewrapped[1], 0xBB); // Now wrapped with KMS key_id

        // Decrypt rewrapped with KMS only (no fallback)
        let kms_only = EncryptionKeys::with_provider(kms_mock);
        let decrypted = kms_only.decrypt(&rewrapped).await.unwrap();
        assert_eq!(decrypted, b"rewrap via fallback");
    }

    #[tokio::test]
    async fn v2_no_fallback_unknown_key_fails() {
        // Encrypt with mock A
        let mock_a = Arc::new(MockKeyProvider::new(0xAA, 0x33, None));
        let keys_a = EncryptionKeys::with_provider(mock_a);
        let encrypted = keys_a.encrypt(b"no fallback").await.unwrap();

        // Decrypt with mock B (no fallback) -- should fail
        let mock_b = Arc::new(MockKeyProvider::new(0xBB, 0x77, None));
        let keys_b = EncryptionKeys::with_provider(mock_b);
        assert!(keys_b.decrypt(&encrypted).await.is_err());
    }
}
