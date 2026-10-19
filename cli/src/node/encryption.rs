use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::RngCore;
use zeroize::Zeroizing;

use super::error::{Error, Result};

/// AES-256-GCM encryption for local credential storage.
/// Key is loaded from `<config_dir>/.keyfile` (generated on first register).
pub struct LocalEncryption {
    key: Zeroizing<[u8; 32]>,
}

impl LocalEncryption {
    /// Open an existing key without creation, repair, or backend fallback.
    pub fn open_existing(config_dir: &Path) -> Result<Self> {
        let directory = super::storage::Directory::open(config_dir)?;
        Self::open_in(&directory)
    }

    fn open_in(directory: &super::storage::Directory) -> Result<Self> {
        let bytes = Zeroizing::new(directory.read(std::ffi::OsStr::new(".keyfile"), 32)?);
        if bytes.len() != 32 {
            return Err(Error::Encryption(
                "Node keyfile must contain exactly 32 bytes".into(),
            ));
        }
        let mut key = Zeroizing::new([0u8; 32]);
        key.copy_from_slice(&bytes);
        Ok(Self { key })
    }

    /// Deliberate registration/migration initialization only. Never replaces
    /// an existing key, even if corrupt, unprotected, or concurrently created.
    pub fn initialize(config_dir: &Path) -> Result<Self> {
        let directory = super::storage::Directory::open(config_dir)?;
        let _writer = directory.writer_lock()?;
        match Self::open_in(&directory) {
            Ok(existing) => Ok(existing),
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut key = Zeroizing::new([0u8; 32]);
                OsRng.fill_bytes(key.as_mut());
                directory.publish(std::ffi::OsStr::new(".keyfile"), key.as_ref(), false)?;
                Ok(Self { key })
            }
            Err(error) => Err(error),
        }
    }

    /// Test fixtures explicitly create their own disposable local store.
    #[cfg(test)]
    pub fn load_or_generate(config_dir: &Path) -> Result<Self> {
        Self::initialize(config_dir)
    }

    /// Encrypt a plaintext string. Returns base64-encoded `nonce || ciphertext`.
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        let cipher = Aes256Gcm::new_from_slice(&*self.key)
            .map_err(|e| Error::Encryption(format!("Failed to create cipher: {e}")))?;

        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| Error::Encryption(format!("Encryption failed: {e}")))?;

        // nonce (12 bytes) || ciphertext
        let mut output = Vec::with_capacity(12 + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);

        Ok(base64::engine::general_purpose::STANDARD.encode(&output))
    }

    /// Decrypt a base64-encoded `nonce || ciphertext`. Returns plaintext string.
    pub fn decrypt(&self, encoded: &str) -> Result<String> {
        let data = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| Error::Encryption(format!("Invalid base64: {e}")))?;

        if data.len() < 13 {
            return Err(Error::Encryption(
                "Ciphertext too short (must be at least 13 bytes)".to_string(),
            ));
        }

        let (nonce_bytes, ciphertext) = data.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        let cipher = Aes256Gcm::new_from_slice(&*self.key)
            .map_err(|e| Error::Encryption(format!("Failed to create cipher: {e}")))?;

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| Error::Encryption(format!("Decryption failed: {e}")))?;

        String::from_utf8(plaintext)
            .map_err(|e| Error::Encryption(format!("Decrypted data is not valid UTF-8: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_missing_read_has_no_filesystem_side_effects() {
        let dir = tempfile::tempdir().unwrap();
        assert!(LocalEncryption::open_existing(dir.path()).is_err());
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[test]
    fn initialization_preserves_existing_key_and_refuses_corrupt_key() {
        let dir = tempfile::tempdir().unwrap();
        let first = LocalEncryption::initialize(dir.path()).unwrap();
        let ciphertext = first.encrypt("disposable-test-value").unwrap();
        let before = std::fs::read(dir.path().join(".keyfile")).unwrap();
        let second = LocalEncryption::initialize(dir.path()).unwrap();
        assert!(second.decrypt(&ciphertext).is_ok());
        assert!(before == std::fs::read(dir.path().join(".keyfile")).unwrap());
        std::fs::write(dir.path().join(".keyfile"), b"corrupt").unwrap();
        assert!(LocalEncryption::initialize(dir.path()).is_err());
        assert!(LocalEncryption::open_existing(dir.path()).is_err());
        assert_eq!(
            std::fs::read(dir.path().join(".keyfile")).unwrap(),
            b"corrupt"
        );
    }

    #[cfg(unix)]
    #[test]
    fn strict_lookup_refuses_unprotected_key_without_changing_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        LocalEncryption::initialize(dir.path()).unwrap();
        let path = dir.path().join(".keyfile");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(LocalEncryption::open_existing(dir.path()).is_err());
        assert!(LocalEncryption::initialize(dir.path()).is_err());
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let enc = LocalEncryption::load_or_generate(dir.path()).unwrap();

        let plaintext = "Bearer sk-test-secret-key-12345";
        let encrypted = enc.encrypt(plaintext).unwrap();
        let decrypted = enc.decrypt(&encrypted).unwrap();

        assert_eq!(plaintext, decrypted);
        assert_ne!(plaintext, encrypted);
    }

    #[test]
    fn different_nonces_produce_different_ciphertexts() {
        let dir = tempfile::tempdir().unwrap();
        let enc = LocalEncryption::load_or_generate(dir.path()).unwrap();

        let plaintext = "same-plaintext";
        let a = enc.encrypt(plaintext).unwrap();
        let b = enc.encrypt(plaintext).unwrap();

        assert_ne!(a, b);
        assert_eq!(enc.decrypt(&a).unwrap(), enc.decrypt(&b).unwrap());
    }

    #[test]
    fn keyfile_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let enc1 = LocalEncryption::load_or_generate(dir.path()).unwrap();
        let encrypted = enc1.encrypt("test").unwrap();

        let enc2 = LocalEncryption::load_or_generate(dir.path()).unwrap();
        let decrypted = enc2.decrypt(&encrypted).unwrap();

        assert_eq!("test", decrypted);
    }

    #[test]
    fn decrypt_invalid_base64_fails() {
        let dir = tempfile::tempdir().unwrap();
        let enc = LocalEncryption::load_or_generate(dir.path()).unwrap();
        assert!(enc.decrypt("not-valid-base64!!!").is_err());
    }

    #[test]
    fn decrypt_too_short_fails() {
        let dir = tempfile::tempdir().unwrap();
        let enc = LocalEncryption::load_or_generate(dir.path()).unwrap();
        let short = base64::engine::general_purpose::STANDARD.encode([0u8; 5]);
        assert!(enc.decrypt(&short).is_err());
    }
}
