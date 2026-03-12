use async_trait::async_trait;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::errors::AppError;

/// A DEK wrapped (encrypted) by a KEK via the KeyProvider.
///
/// The `ciphertext` field holds already-encrypted data, so zeroization is
/// defense-in-depth rather than strictly necessary. We wrap it in
/// `Zeroizing` to scrub wrapped key material from memory on drop.
#[derive(Debug, Clone)]
pub struct WrappedKey {
    /// Stable identifier stored in the ciphertext header for the wrapping key.
    pub key_id: u8,
    /// The wrapped (encrypted) DEK bytes. Zeroized on drop for defense-in-depth.
    pub ciphertext: Zeroizing<Vec<u8>>,
}

/// Abstraction over KEK wrap/unwrap operations.
///
/// Implementations provide the mechanism for protecting DEKs at rest.
/// Phase 4: async trait via `async-trait` crate for KMS network I/O.
#[async_trait]
pub trait KeyProvider: Send + Sync + std::fmt::Debug {
    /// Wrap (encrypt) a plaintext DEK with the current KEK.
    async fn wrap_dek(&self, plaintext_dek: &[u8]) -> Result<WrappedKey, AppError>;

    /// Unwrap (decrypt) a previously wrapped DEK.
    ///
    /// Returns the plaintext DEK wrapped in [`Zeroizing`] so it is automatically
    /// scrubbed from memory when the caller drops it.
    async fn unwrap_dek(&self, wrapped: &WrappedKey) -> Result<Zeroizing<Vec<u8>>, AppError>;

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

/// Derive a single-byte key ID from a string identifier (ARN, resource name)
/// via SHA-256.
///
/// Used by KMS providers (`AwsKmsProvider`, `GcpKmsProvider`) to compute a
/// stable key ID from cloud-specific key identifiers rather than raw bytes.
#[cfg(any(feature = "aws-kms", feature = "gcp-kms"))]
pub(crate) fn derive_key_id_from_str(identifier: &str) -> u8 {
    let digest = Sha256::digest(identifier.as_bytes());
    digest[0]
}
