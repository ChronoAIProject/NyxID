use std::path::Path;

use crate::config::NodeConfig;
use crate::encryption::LocalEncryption;
use crate::error::{Error, Result};
use crate::keychain::{self, KeychainBackend};

/// Unified secret storage -- either file-based AES-GCM or OS keychain.
pub enum SecretBackend {
    File(LocalEncryption),
    Keychain(KeychainBackend),
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
    pub fn from_config(config: &NodeConfig, config_dir: &Path) -> Result<Self> {
        match config.storage_backend.as_str() {
            "keychain" => Ok(Self::Keychain(KeychainBackend::new(&config.node.id))),
            _ => Ok(Self::File(LocalEncryption::load_or_generate(config_dir)?)),
        }
    }

    /// Build during registration (before config is loaded from disk).
    pub fn new(backend: &str, node_id: &str, config_dir: &Path) -> Result<Self> {
        match backend {
            "keychain" => Ok(Self::Keychain(KeychainBackend::new(node_id))),
            _ => Ok(Self::File(LocalEncryption::load_or_generate(config_dir)?)),
        }
    }

    // -- Auth token --

    pub fn store_auth_token(&self, config: &mut NodeConfig, token: &str) -> Result<()> {
        match self {
            Self::File(enc) => config.set_auth_token(token, enc),
            Self::Keychain(kc) => {
                kc.set(keychain::KEY_AUTH_TOKEN, token)?;
                config.node.auth_token_encrypted = String::new();
                Ok(())
            }
        }
    }

    pub fn load_auth_token(&self, config: &NodeConfig) -> Result<String> {
        match self {
            Self::File(enc) => config.decrypt_auth_token(enc),
            Self::Keychain(kc) => kc.get(keychain::KEY_AUTH_TOKEN),
        }
    }

    // -- Signing secret --

    pub fn store_signing_secret(&self, config: &mut NodeConfig, secret: &str) -> Result<()> {
        match self {
            Self::File(enc) => config.set_signing_secret(secret, enc),
            Self::Keychain(kc) => {
                kc.set(keychain::KEY_SIGNING_SECRET, secret)?;
                config.signing.shared_secret_encrypted = Some(String::new());
                Ok(())
            }
        }
    }

    pub fn load_signing_secret(&self, config: &NodeConfig) -> Result<Option<String>> {
        match self {
            Self::File(enc) => config.decrypt_signing_secret(enc),
            Self::Keychain(kc) => {
                if config.signing.shared_secret_encrypted.is_some() {
                    Ok(Some(kc.get(keychain::KEY_SIGNING_SECRET)?))
                } else {
                    Ok(None)
                }
            }
        }
    }

    // -- Service credentials --

    /// Store a credential value. Returns `Some(encrypted)` for file backend,
    /// `None` for keychain (value stored externally).
    pub fn store_credential_value(
        &self,
        service_slug: &str,
        value: &str,
    ) -> Result<Option<String>> {
        match self {
            Self::File(enc) => Ok(Some(enc.encrypt(value)?)),
            Self::Keychain(kc) => {
                kc.set(&keychain::credential_key(service_slug), value)?;
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
            Self::Keychain(kc) => kc.get(&keychain::credential_key(service_slug)),
        }
    }

    /// Delete a credential from the keychain (no-op for file backend since
    /// the value is removed when the config is saved).
    pub fn delete_credential(&self, service_slug: &str) -> Result<()> {
        match self {
            Self::File(_) => Ok(()),
            Self::Keychain(kc) => kc.delete(&keychain::credential_key(service_slug)),
        }
    }

    /// Delete the stored auth token from the backend.
    pub fn delete_auth_token(&self) -> Result<()> {
        match self {
            Self::File(_) => Ok(()),
            Self::Keychain(kc) => kc.delete(keychain::KEY_AUTH_TOKEN),
        }
    }

    /// Delete the stored signing secret from the backend.
    pub fn delete_signing_secret(&self) -> Result<()> {
        match self {
            Self::File(_) => Ok(()),
            Self::Keychain(kc) => kc.delete(keychain::KEY_SIGNING_SECRET),
        }
    }
}
