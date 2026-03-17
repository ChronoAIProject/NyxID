use std::io::Write;
use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::RngCore;
use zeroize::{Zeroize, Zeroizing};

use crate::error::{Error, Result};

/// AES-256-GCM encryption for local credential storage.
/// Key is loaded from `<config_dir>/.keyfile` (generated on first register).
pub struct LocalEncryption {
    key: Zeroizing<[u8; 32]>,
}

impl LocalEncryption {
    /// Load or generate the local encryption key.
    /// Creates `<config_dir>/.keyfile` with mode 0600 if it does not exist.
    pub fn load_or_generate(config_dir: &Path) -> Result<Self> {
        let keyfile = config_dir.join(".keyfile");

        let key_bytes = if keyfile.exists() {
            let mut data = std::fs::read(&keyfile)?;
            if data.len() != 32 {
                data.zeroize();
                return Err(Error::Encryption(format!(
                    "Keyfile has invalid length: expected 32 bytes, got {}",
                    data.len()
                )));
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&data);
            data.zeroize(); // M7: Zeroize source Vec after copy
            key
        } else {
            let mut key = [0u8; 32];
            OsRng.fill_bytes(&mut key);
            // C1: Create keyfile atomically with mode 0600 to avoid race condition
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&keyfile)?;
                file.write_all(&key)?;
            }
            #[cfg(not(unix))]
            {
                std::fs::write(&keyfile, &key)?;
            }
            key
        };

        Ok(Self {
            key: Zeroizing::new(key_bytes),
        })
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
