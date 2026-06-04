use std::path::Path;

use super::config::NodeConfig;
use super::encryption::LocalEncryption;
use super::error::{Error, Result};
use super::keychain::{KeychainBackend, KeychainVault};
use zeroize::Zeroizing;

/// Unified secret storage -- either file-based AES-GCM or OS keychain vault.
pub enum SecretBackend {
    File(LocalEncryption),
    Keychain(KeychainVault),
}

impl SecretBackend {
    /// Verify that the selected backend is usable before registration consumes
    /// a one-time token from the server.
    pub fn preflight(backend: &str, config_dir: &Path) -> Result<()> {
        match backend {
            "keychain" => KeychainBackend::new("__preflight__").preflight(),
            _ => {
                LocalEncryption::load_or_generate(config_dir)?;
                Ok(())
            }
        }
    }

    /// Build the appropriate backend from an existing config.
    /// For keychain: loads the vault (single keychain read = single prompt).
    pub fn from_config(config: &NodeConfig, config_dir: &Path) -> Result<Self> {
        match config.storage_backend.as_str() {
            "keychain" => Ok(Self::Keychain(KeychainVault::load(
                &config.node.id,
                config,
            )?)),
            _ => Ok(Self::File(LocalEncryption::load_or_generate(config_dir)?)),
        }
    }

    /// Build during registration (before config is loaded from disk).
    pub fn new(backend: &str, node_id: &str, config_dir: &Path) -> Result<Self> {
        match backend {
            "keychain" => Ok(Self::Keychain(KeychainVault::new(node_id))),
            _ => Ok(Self::File(LocalEncryption::load_or_generate(config_dir)?)),
        }
    }

    /// Build the backend from just the storage type string and config dir.
    /// For keychain, loads the config first to get the node_id.
    pub fn from_storage_backend_str(backend: &str, config_dir: &Path) -> Result<Self> {
        match backend {
            "keychain" => {
                let config_file = config_dir.join("config.toml");
                let config = super::config::NodeConfig::load(&config_file)?;
                Ok(Self::Keychain(KeychainVault::load(
                    &config.node.id,
                    &config,
                )?))
            }
            _ => Ok(Self::File(LocalEncryption::load_or_generate(config_dir)?)),
        }
    }

    #[cfg(test)]
    pub fn new_mock_keychain(node_id: &str) -> Self {
        Self::Keychain(KeychainVault::new_mock(node_id))
    }

    // -- Auth token --

    pub fn store_auth_token(&self, config: &mut NodeConfig, token: &str) -> Result<()> {
        match self {
            Self::File(enc) => config.set_auth_token(token, enc),
            Self::Keychain(vault) => {
                vault.set_auth_token(token)?;
                config.node.auth_token_encrypted = String::new();
                Ok(())
            }
        }
    }

    pub fn load_auth_token(&self, config: &NodeConfig) -> Result<String> {
        match self {
            Self::File(enc) => config.decrypt_auth_token(enc),
            Self::Keychain(vault) => vault.get_auth_token(),
        }
    }

    // -- Signing secret --

    pub fn store_signing_secret(&self, config: &mut NodeConfig, secret: &str) -> Result<()> {
        match self {
            Self::File(enc) => config.set_signing_secret(secret, enc),
            Self::Keychain(vault) => {
                vault.set_signing_secret(secret)?;
                config.signing.shared_secret_encrypted = Some(String::new());
                Ok(())
            }
        }
    }

    pub fn load_signing_secret(&self, config: &NodeConfig) -> Result<Option<String>> {
        match self {
            Self::File(enc) => config.decrypt_signing_secret(enc),
            Self::Keychain(vault) => {
                if config.signing.shared_secret_encrypted.is_some() {
                    vault
                        .get_signing_secret()?
                        .ok_or_else(|| {
                            Error::Keychain("Signing secret missing from vault".to_string())
                        })
                        .map(Some)
                } else {
                    Ok(None)
                }
            }
        }
    }

    // -- Service credentials --

    /// Store a credential value. Returns `Some(encrypted)` for file backend,
    /// `None` for keychain (value stored in vault).
    pub fn store_credential_value(
        &self,
        service_slug: &str,
        value: &str,
    ) -> Result<Option<String>> {
        match self {
            Self::File(enc) => Ok(Some(enc.encrypt(value)?)),
            Self::Keychain(vault) => {
                vault.set_credential(service_slug, value)?;
                Ok(None)
            }
        }
    }

    /// Load a credential value from the appropriate backend.
    pub fn load_credential_value(
        &self,
        service_slug: &str,
        encrypted: Option<&str>,
    ) -> Result<String> {
        match self {
            Self::File(enc) => {
                let encrypted = encrypted.ok_or_else(|| {
                    Error::Config(format!(
                        "Missing encrypted value for credential '{service_slug}'"
                    ))
                })?;
                enc.decrypt(encrypted)
            }
            Self::Keychain(vault) => vault.get_credential(service_slug),
        }
    }

    /// Delete a credential (no-op for file backend since config.save() handles it).
    pub fn delete_credential(&self, service_slug: &str) -> Result<()> {
        match self {
            Self::File(_) => Ok(()),
            Self::Keychain(vault) => vault.delete_credential(service_slug),
        }
    }

    // -- Pending RCI private keys --

    /// Store a pending private key. Returns `Some(encrypted)` for file backend,
    /// `None` for keychain (value stored in vault).
    pub fn store_pending_crypto_key(
        &self,
        pending_id: &str,
        private_key_b64u: &Zeroizing<String>,
    ) -> Result<Option<String>> {
        match self {
            Self::File(enc) => Ok(Some(enc.encrypt(private_key_b64u)?)),
            Self::Keychain(vault) => {
                vault.set_pending_crypto_key(pending_id, private_key_b64u)?;
                Ok(None)
            }
        }
    }

    /// Load a pending private key from the appropriate backend.
    pub fn load_pending_crypto_key(
        &self,
        pending_id: &str,
        encrypted: Option<&str>,
    ) -> Result<Zeroizing<String>> {
        match self {
            Self::File(enc) => {
                let encrypted = encrypted.ok_or_else(|| {
                    Error::Config(format!(
                        "Missing encrypted private key for pending credential '{pending_id}'"
                    ))
                })?;
                enc.decrypt(encrypted).map(Zeroizing::new)
            }
            Self::Keychain(vault) => vault.get_pending_crypto_key(pending_id),
        }
    }

    /// Delete a pending private key (no-op for file backend after config save).
    pub fn delete_pending_crypto_key(&self, pending_id: &str) -> Result<()> {
        match self {
            Self::File(_) => Ok(()),
            Self::Keychain(vault) => vault.delete_pending_crypto_key(pending_id),
        }
    }

    /// Delete the stored auth token from the backend.
    pub fn delete_auth_token(&self) -> Result<()> {
        match self {
            Self::File(_) => Ok(()),
            Self::Keychain(vault) => vault.delete_auth_token(),
        }
    }

    /// Delete the stored signing secret from the backend.
    pub fn delete_signing_secret(&self) -> Result<()> {
        match self {
            Self::File(_) => Ok(()),
            Self::Keychain(vault) => vault.delete_signing_secret(),
        }
    }

    /// Re-read secrets from the backing store into memory.
    /// No-op for file backend (values are decrypted on each load).
    /// For keychain: refreshes the in-memory vault cache from the OS keychain.
    pub fn refresh(&self) -> Result<()> {
        match self {
            Self::File(_) => Ok(()),
            Self::Keychain(vault) => vault.refresh(),
        }
    }
}
