//! Custody acquired before registration can consume a server token. The same
//! pinned directory and lease survive the network await and local publication.
use std::path::{Path, PathBuf};

use super::config::NodeConfig;
use super::error::{Error, Result};
use super::keychain::KeychainBackend;
use super::secret_backend::SecretBackend;
use super::secret_operation::SecretOperation;
use super::secret_transaction;
use super::storage::{Directory, MAX_CONFIG_BYTES};

pub(crate) struct RegistrationCustody {
    lease: std::fs::File,
    directory: Directory,
    config_file: PathBuf,
    backend: String,
}

fn existing() -> Error {
    Error::Config("Node registration requires an absent configuration; existing custody is never replaced or repaired".into())
}

impl RegistrationCustody {
    pub(crate) fn acquire(config_dir: &Path, backend: &str) -> Result<Self> {
        if !matches!(backend, "file" | "keychain") {
            return Err(Error::Validation(
                "Unknown node storage backend; expected file or keychain".into(),
            ));
        }
        let directory = Directory::create_private(config_dir)?;
        // Refuse even an invalid existing config before creating any lock,
        // initializing a key, contacting a key service or consuming a token.
        match directory.read(std::ffi::OsStr::new("config.toml"), MAX_CONFIG_BYTES) {
            Ok(_) => return Err(existing()),
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let lease = directory.registration_lock()?;
        let config_file = config_dir.join("config.toml");
        {
            let _writer = directory.writer_lock()?;
            secret_transaction::check_observed(&directory, &config_file, None)?;
        }
        Ok(Self {
            lease,
            directory,
            config_file,
            backend: backend.into(),
        })
    }

    /// No file key is created before registration returns its node identity.
    pub(crate) fn preflight(&self) -> Result<()> {
        SecretBackend::preflight_in(&self.backend, &self.directory)
    }

    pub(crate) fn config_file(&self) -> &Path {
        &self.config_file
    }

    pub(crate) fn into_operation(
        self,
        config: &NodeConfig,
        keychain: KeychainBackend,
    ) -> Result<SecretOperation> {
        if config.storage_backend != self.backend {
            return Err(existing());
        }
        SecretOperation::registration_in(
            config,
            &self.config_file,
            self.directory,
            keychain,
            self.lease,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::secret_fields::SecretField;
    use super::*;

    #[test]
    fn existing_configuration_and_missing_key_refuse_before_any_new_inode() {
        for backend in ["file", "keychain"] {
            let dir = tempfile::tempdir().unwrap();
            let directory = Directory::open(dir.path()).unwrap();
            directory
                .publish(
                    std::ffi::OsStr::new("config.toml"),
                    b"existing-unparsed-config",
                    false,
                )
                .unwrap();
            assert!(RegistrationCustody::acquire(dir.path(), backend).is_err());
            assert_eq!(
                directory.names().unwrap(),
                [std::ffi::OsString::from("config.toml")]
            );
            assert_eq!(
                std::fs::read(dir.path().join("config.toml")).unwrap(),
                b"existing-unparsed-config"
            );
        }
    }

    #[test]
    fn invalid_backend_has_no_directory_creation_effect() {
        let dir = tempfile::tempdir().unwrap();
        assert!(RegistrationCustody::acquire(&dir.path().join("missing"), "keychian").is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn file_preflight_never_creates_or_repairs_key_material() {
        let dir = tempfile::tempdir().unwrap();
        let custody = RegistrationCustody::acquire(dir.path(), "file").unwrap();
        custody.preflight().unwrap();
        assert!(!dir.path().join(".keyfile").exists());
        let directory = Directory::open(dir.path()).unwrap();
        directory
            .publish(std::ffi::OsStr::new(".keyfile"), &[7; 31], false)
            .unwrap();
        assert!(custody.preflight().is_err());
        assert_eq!(std::fs::read(dir.path().join(".keyfile")).unwrap(), [7; 31]);
    }

    #[test]
    fn registration_lease_fences_competitors_through_operation_and_never_changes_inode() {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let custody = RegistrationCustody::acquire(dir.path(), "file").unwrap();
        let config = NodeConfig::new("wss://example.invalid".into(), "node".into(), "file".into());
        let inode = std::fs::metadata(dir.path().join(".registration.lock")).unwrap();
        assert!(RegistrationCustody::acquire(dir.path(), "file").is_err());
        let operation = custody
            .into_operation(&config, KeychainBackend::new_mock("node"))
            .unwrap();
        assert!(RegistrationCustody::acquire(dir.path(), "file").is_err());
        drop(operation);
        let retry = RegistrationCustody::acquire(dir.path(), "file").unwrap();
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(dir.path().join(".registration.lock"))
                .unwrap()
                .ino(),
            inode.ino()
        );
        drop(retry);
    }

    #[test]
    fn pinned_file_and_mock_keychain_registration_survive_path_retarget_without_foreign_effects() {
        for backend in ["file", "keychain"] {
            let dir = tempfile::tempdir().unwrap();
            let profile = dir.path().join("profile");
            let moved = dir.path().join("moved");
            let custody = RegistrationCustody::acquire(&profile, backend).unwrap();
            let original_path = custody.config_file().to_owned();
            std::fs::rename(&profile, &moved).unwrap();
            Directory::create_private(&profile).unwrap();
            let store = KeychainBackend::new_mock("node");
            let mut config = NodeConfig::new(
                "wss://example.invalid".into(),
                "node".into(),
                backend.into(),
            );
            let operation = custody.into_operation(&config, store).unwrap();
            operation.prepare(&mut config).unwrap();
            config.node.auth_token_encrypted = operation
                .store(SecretField::AuthToken, "synthetic-auth")
                .unwrap();
            config.signing.shared_secret_encrypted = Some(
                operation
                    .store(SecretField::SigningSecret, "synthetic-signing")
                    .unwrap(),
            );
            operation.publish(&config, &original_path).unwrap();
            assert_eq!(std::fs::read_dir(&profile).unwrap().count(), 0);
            assert!(
                Directory::open(&moved)
                    .unwrap()
                    .registration_lock()
                    .is_err()
            );
            let loaded = NodeConfig::load(&moved.join("config.toml")).unwrap();
            assert_eq!(
                operation
                    .load(&loaded, &SecretField::AuthToken)
                    .unwrap()
                    .unwrap()
                    .as_str(),
                "synthetic-auth"
            );
            assert_eq!(
                operation
                    .load(&loaded, &SecretField::SigningSecret)
                    .unwrap()
                    .unwrap()
                    .as_str(),
                "synthetic-signing"
            );
            operation.health(&loaded).unwrap();
        }
    }
}
