use std::path::Path;

use super::config::{KeychainFormat, NodeConfig};
use super::encryption::LocalEncryption;
use super::error::{Error, Result};
use super::keychain::{KeychainBackend, KeychainVault};
use super::secret_fields::SecretField;
use super::secret_operation::SecretOperation;
use zeroize::Zeroizing;

/// A pinned, read-only observation. Mutation requires a separate guarded
/// operation; legacy writer APIs cannot reinterpret immutable references.
pub struct ObservedSecrets {
    operation: SecretOperation,
    baseline: NodeConfig,
}

/// Unified secret storage -- either file-based AES-GCM or OS keychain vault.
pub enum SecretBackend {
    File(LocalEncryption),
    Keychain(KeychainVault),
    Observed(Box<ObservedSecrets>),
    Writer(Box<ObservedSecrets>),
}

impl SecretBackend {
    /// Verify that the selected backend is usable before registration consumes
    /// a one-time token from the server.
    pub(crate) fn preflight_in(backend: &str, directory: &super::storage::Directory) -> Result<()> {
        match backend {
            "keychain" => KeychainBackend::new("__preflight__").preflight(),
            "file" => match LocalEncryption::open_in(directory) {
                Ok(_) => Ok(()),
                Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            },
            _ => Err(unknown_backend()),
        }
    }

    /// Build the appropriate backend from an existing config.
    /// For keychain: loads the vault (single keychain read = single prompt).
    pub fn from_config(config: &NodeConfig, config_dir: &Path) -> Result<Self> {
        if config.keychain_format == Some(KeychainFormat::ImmutableV1) {
            // Legacy writer entry points must never open an immutable store.
            // Read-only callers explicitly request a snapshot reader.
            return Err(guarded_write_required());
        }
        match config.storage_backend.as_str() {
            "keychain" => Ok(Self::Keychain(KeychainVault::load(
                &config.node.id,
                config,
            )?)),
            "file" => Ok(Self::File(LocalEncryption::open_existing(config_dir)?)),
            _ => Err(unknown_backend()),
        }
    }

    /// Read all fields from one authoritative configuration without staging,
    /// legacy consolidation, key generation or backend repair.
    pub(crate) fn reader(config: &NodeConfig, config_dir: &Path) -> Result<Self> {
        Self::reader_with_keychain(config, config_dir, KeychainBackend::new(&config.node.id))
    }

    pub(crate) fn reader_with_keychain(
        config: &NodeConfig,
        config_dir: &Path,
        keychain: KeychainBackend,
    ) -> Result<Self> {
        let operation = SecretOperation::open(config, &config_dir.join("config.toml"), keychain)?;
        Ok(Self::Observed(Box::new(ObservedSecrets {
            operation,
            baseline: config.clone(),
        })))
    }

    /// One mutation owns one exact observation and one pinned directory.
    /// Legacy keychain conversion requires the explicit migration/quiescence
    /// boundary; ordinary commands cannot silently upgrade a running daemon.
    pub(crate) fn writer(config: &mut NodeConfig, config_dir: &Path) -> Result<Self> {
        let keychain = KeychainBackend::new(&config.node.id);
        Self::writer_with_keychain(config, config_dir, keychain)
    }

    pub(crate) fn writer_with_keychain(
        config: &mut NodeConfig,
        config_dir: &Path,
        keychain: KeychainBackend,
    ) -> Result<Self> {
        if config.storage_backend == "keychain" && config.keychain_format.is_none() {
            return Err(legacy_migration_required());
        }
        let baseline = config.clone();
        let operation = SecretOperation::open(config, &config_dir.join("config.toml"), keychain)?;
        operation.prepare(config)?;
        Ok(Self::Writer(Box::new(ObservedSecrets {
            operation,
            baseline,
        })))
    }

    pub(crate) fn current_reader(&self) -> Result<(NodeConfig, Self)> {
        let Self::Observed(reader) = self else {
            return Err(guarded_write_required());
        };
        let (config, operation) = reader.operation.current()?;
        let backend = Self::Observed(Box::new(ObservedSecrets {
            operation,
            baseline: config.clone(),
        }));
        Ok((config, backend))
    }

    pub(crate) fn current_writer(&self) -> Result<(NodeConfig, Self)> {
        if let Self::Observed(reader) = self
            && reader.baseline.storage_backend == "keychain"
            && reader.baseline.keychain_format.is_none()
        {
            return Err(legacy_migration_required());
        }
        let (mut config, backend) = self.current_reader()?;
        let Self::Observed(writer) = backend else {
            return Err(guarded_write_required());
        };
        writer.operation.prepare(&mut config)?;
        Ok((config, Self::Writer(writer)))
    }

    pub(crate) fn publish(&self, config: &NodeConfig, path: &Path) -> Result<()> {
        match self {
            Self::Writer(writer) => writer.operation.publish(config, path),
            Self::File(_) | Self::Keychain(_) | Self::Observed(_) => Err(guarded_write_required()),
        }
    }

    pub(crate) fn oauth_custody(
        &self,
    ) -> Result<(
        super::storage::Directory,
        std::path::PathBuf,
        KeychainBackend,
        NodeConfig,
    )> {
        match self {
            Self::Observed(reader) => {
                let (directory, path, keychain) = reader.operation.oauth_custody()?;
                Ok((directory, path, keychain, reader.baseline.clone()))
            }
            _ => Err(guarded_write_required()),
        }
    }

    pub(crate) fn publish_with_intent(
        &self,
        config: &NodeConfig,
        path: &Path,
        intent: impl FnOnce(&super::storage::Directory, &[u8]) -> Result<()>,
    ) -> Result<()> {
        match self {
            Self::Writer(writer) => writer.operation.publish_with_intent(config, path, intent),
            _ => Err(guarded_write_required()),
        }
    }

    pub(crate) fn store_field(&self, field: SecretField, value: &str) -> Result<Option<String>> {
        match self {
            Self::Writer(writer) => writer.operation.store(field, value).map(Some),
            Self::File(file) => file.encrypt(value).map(Some),
            Self::Keychain(vault) => {
                vault.set_credential(&field.legacy_key(), value)?;
                Ok(None)
            }
            Self::Observed(_) => Err(guarded_write_required()),
        }
    }

    pub(crate) fn load_field(
        &self,
        config: &NodeConfig,
        field: &SecretField,
    ) -> Result<Option<Zeroizing<String>>> {
        match self {
            Self::Observed(reader) => reader.operation.load(config, field),
            Self::Writer(writer) => writer.operation.load_projection(config, field),
            Self::File(file) => field
                .encoded(config)?
                .map(|encoded| file.decrypt(encoded).map(Zeroizing::new))
                .transpose(),
            Self::Keychain(vault) => match field {
                SecretField::AuthToken => vault.get_auth_token().map(Zeroizing::new).map(Some),
                SecretField::SigningSecret => vault
                    .get_signing_secret()
                    .map(|value| value.map(Zeroizing::new)),
                SecretField::PendingPrivate { pending_id } => {
                    vault.get_pending_crypto_key(pending_id).map(Some)
                }
                _ => vault
                    .get_credential(&field.legacy_key())
                    .map(Zeroizing::new)
                    .map(Some),
            },
        }
    }

    pub(crate) fn require_field(
        &self,
        config: &NodeConfig,
        field: &SecretField,
    ) -> Result<Zeroizing<String>> {
        self.load_field(config, field)?
            .ok_or_else(|| Error::Config("Required node secret is unavailable".into()))
    }

    /// Build during registration (before config is loaded from disk).
    pub fn new(backend: &str, node_id: &str, config_dir: &Path) -> Result<Self> {
        match backend {
            "keychain" => Ok(Self::Keychain(KeychainVault::new(node_id))),
            "file" => Ok(Self::File(LocalEncryption::initialize(config_dir)?)),
            _ => Err(unknown_backend()),
        }
    }

    /// Build the backend from just the storage type string and config dir.
    /// For keychain, loads the config first to get the node_id.
    #[cfg(test)]
    pub fn from_storage_backend_str(backend: &str, config_dir: &Path) -> Result<Self> {
        match backend {
            "keychain" => {
                let config_file = config_dir.join("config.toml");
                let config = super::config::NodeConfig::load(&config_file)?;
                Self::from_config(&config, config_dir)
            }
            "file" => Ok(Self::File(LocalEncryption::open_existing(config_dir)?)),
            _ => Err(unknown_backend()),
        }
    }

    #[cfg(test)]
    pub fn new_mock_keychain(node_id: &str) -> Self {
        Self::Keychain(KeychainVault::new_mock(node_id))
    }

    // -- Auth token --

    pub fn store_auth_token(&self, config: &mut NodeConfig, token: &str) -> Result<()> {
        match self {
            Self::Writer(writer) => {
                config.node.auth_token_encrypted =
                    writer.operation.store(SecretField::AuthToken, token)?;
                Ok(())
            }
            Self::File(enc) => config.set_auth_token(token, enc),
            Self::Keychain(vault) => {
                vault.set_auth_token(token)?;
                config.node.auth_token_encrypted = String::new();
                Ok(())
            }
            Self::Observed(_) => Err(guarded_write_required()),
        }
    }

    pub fn load_auth_token(&self, config: &NodeConfig) -> Result<String> {
        match self {
            Self::File(enc) => config.decrypt_auth_token(enc),
            Self::Keychain(vault) => vault.get_auth_token(),
            Self::Observed(_) | Self::Writer(_) => self
                .require_field(config, &SecretField::AuthToken)
                .map(|value| value.to_string()),
        }
    }

    // -- Signing secret --

    pub fn store_signing_secret(&self, config: &mut NodeConfig, secret: &str) -> Result<()> {
        match self {
            Self::Writer(writer) => {
                config.signing.shared_secret_encrypted =
                    Some(writer.operation.store(SecretField::SigningSecret, secret)?);
                Ok(())
            }
            Self::File(enc) => config.set_signing_secret(secret, enc),
            Self::Keychain(vault) => {
                vault.set_signing_secret(secret)?;
                config.signing.shared_secret_encrypted = Some(String::new());
                Ok(())
            }
            Self::Observed(_) => Err(guarded_write_required()),
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
            Self::Observed(_) | Self::Writer(_) => self
                .load_field(config, &SecretField::SigningSecret)
                .map(|value| value.map(|value| value.to_string())),
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
            Self::Observed(_) | Self::Writer(_) => Err(guarded_write_required()),
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
            Self::Observed(_) | Self::Writer(_) => Err(Error::Config(
                "Typed node secret location is required".into(),
            )),
        }
    }

    /// Delete a credential (no-op for file backend since config.save() handles it).
    pub fn delete_credential(&self, service_slug: &str) -> Result<()> {
        match self {
            Self::File(_) | Self::Writer(_) => Ok(()),
            Self::Keychain(vault) => vault.delete_credential(service_slug),
            Self::Observed(_) => Err(guarded_write_required()),
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
            Self::Writer(writer) => writer
                .operation
                .store(
                    SecretField::PendingPrivate {
                        pending_id: pending_id.into(),
                    },
                    private_key_b64u,
                )
                .map(Some),
            Self::File(enc) => Ok(Some(enc.encrypt(private_key_b64u)?)),
            Self::Keychain(vault) => {
                vault.set_pending_crypto_key(pending_id, private_key_b64u)?;
                Ok(None)
            }
            Self::Observed(_) => Err(guarded_write_required()),
        }
    }

    /// Delete a pending private key (no-op for file backend after config save).
    pub fn delete_pending_crypto_key(&self, pending_id: &str) -> Result<()> {
        match self {
            Self::File(_) | Self::Writer(_) => Ok(()),
            Self::Keychain(vault) => vault.delete_pending_crypto_key(pending_id),
            Self::Observed(_) => Err(guarded_write_required()),
        }
    }

    /// Delete the stored auth token from the backend.
    pub fn delete_auth_token(&self) -> Result<()> {
        match self {
            Self::File(_) | Self::Writer(_) => Ok(()),
            Self::Keychain(vault) => vault.delete_auth_token(),
            Self::Observed(_) => Err(guarded_write_required()),
        }
    }

    /// Delete the stored signing secret from the backend.
    pub fn delete_signing_secret(&self) -> Result<()> {
        match self {
            Self::File(_) | Self::Writer(_) => Ok(()),
            Self::Keychain(vault) => vault.delete_signing_secret(),
            Self::Observed(_) => Err(guarded_write_required()),
        }
    }

    /// Re-read secrets from the backing store into memory.
    /// No-op for file backend (values are decrypted on each load).
    /// For keychain: refreshes the in-memory vault cache from the OS keychain.
    pub fn refresh(&self) -> Result<()> {
        match self {
            Self::File(_) => Ok(()),
            Self::Keychain(vault) => vault.refresh(),
            Self::Observed(reader) | Self::Writer(reader) => {
                reader.operation.health(&reader.baseline)
            }
        }
    }
}

fn guarded_write_required() -> Error {
    Error::Config("Immutable node secret storage requires a guarded write operation".into())
}

fn legacy_migration_required() -> Error {
    Error::Config(
        "Legacy keychain storage requires an explicit stopped-daemon migration. Upgrade to the release supporting resumable node storage migration, stop the node daemon, then run `nyxid node migrate --to keychain` with the same --profile/--config.".into(),
    )
}

fn unknown_backend() -> Error {
    Error::Validation("Unknown node storage backend; expected file or keychain".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_file_lookup_never_initializes_a_missing_store() {
        let dir = tempfile::tempdir().unwrap();
        let config = NodeConfig::new(String::new(), "test-node".into(), "file".into());
        assert!(SecretBackend::from_config(&config, dir.path()).is_err());
        assert!(SecretBackend::from_storage_backend_str("file", dir.path()).is_err());
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[test]
    fn all_unknown_backend_entry_points_refuse_without_side_effects() {
        let dir = tempfile::tempdir().unwrap();
        let directory = super::super::storage::Directory::open(dir.path()).unwrap();
        for backend in ["", "FILE", "keychian", "file "] {
            let config = NodeConfig::new(String::new(), "test-node".into(), backend.into());
            assert!(SecretBackend::preflight_in(backend, &directory).is_err());
            assert!(SecretBackend::from_config(&config, dir.path()).is_err());
            assert!(SecretBackend::new(backend, "test-node", dir.path()).is_err());
            assert!(SecretBackend::from_storage_backend_str(backend, dir.path()).is_err());
        }
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }
}
