use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::errors::AppError;

/// A DEK wrapped (encrypted) by a KEK via the KeyProvider.
#[derive(Debug, Clone)]
pub struct WrappedKey {
    /// Stable identifier stored in the ciphertext header for the wrapping key.
    pub key_id: u8,
    /// The wrapped (encrypted) DEK bytes.
    pub ciphertext: Vec<u8>,
}

/// Abstraction over KEK wrap/unwrap operations.
///
/// Implementations provide the mechanism for protecting DEKs at rest.
/// Phase 3: sync trait. Phase 4+ adds async for KMS backends.
pub trait KeyProvider: Send + Sync + std::fmt::Debug {
    /// Wrap (encrypt) a plaintext DEK with the current KEK.
    fn wrap_dek(&self, plaintext_dek: &[u8]) -> Result<WrappedKey, AppError>;

    /// Unwrap (decrypt) a previously wrapped DEK.
    ///
    /// Returns the plaintext DEK wrapped in [`Zeroizing`] so it is automatically
    /// scrubbed from memory when the caller drops it.
    fn unwrap_dek(&self, wrapped: &WrappedKey) -> Result<Zeroizing<Vec<u8>>, AppError>;

    /// Stable identifier stored in the header for the current (active) KEK.
    fn current_key_id(&self) -> u8;

    /// Returns true when the provider can unwrap data for this key id.
    fn has_key_id(&self, key_id: u8) -> bool;

    /// Whether a previous key is available for unwrapping.
    fn has_previous_key(&self) -> bool;
}

/// Derive a single-byte key ID from raw key material via SHA-256.
///
/// Used by both `LocalKeyProvider` and `EncryptionKeys` to compute a stable,
/// content-derived identifier for a given key.
pub(crate) fn derive_key_id(key: &[u8]) -> u8 {
    let digest = Sha256::digest(key);
    digest[0]
}
