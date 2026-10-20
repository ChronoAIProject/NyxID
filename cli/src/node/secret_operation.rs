//! One observed node secret operation. Callers retain the authoritative
//! baseline across awaits; all target effects belong to that observation.
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zeroize::Zeroizing;

use super::config::{KeychainFormat, NodeConfig};
use super::encryption::LocalEncryption;
use super::error::{Error, Result};
use super::keychain::KeychainBackend;
use super::secret_fields::{self, SecretField};
use super::secret_snapshot::SecretSnapshot;
use super::secret_transaction::{self, Attempt};
use super::storage::Directory;

fn invalid() -> Error {
    Error::Config("Node secret operation requires its original authoritative configuration; reload before retrying".into())
}

#[derive(Default)]
struct State {
    attempt: Option<Attempt>,
    prepared: bool,
    failed: bool,
    published: bool,
}

pub(crate) struct SecretOperation {
    path: PathBuf,
    directory: Directory,
    baseline: NodeConfig,
    source: Option<SecretSnapshot>,
    file: Option<LocalEncryption>,
    keychain: KeychainBackend,
    _registration_lease: Option<std::fs::File>,
    state: Mutex<State>,
}
impl std::fmt::Debug for SecretOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretOperation { authority_and_material: [REDACTED] }")
    }
}
impl SecretOperation {
    pub(crate) fn open(
        config: &NodeConfig,
        path: &Path,
        keychain: KeychainBackend,
    ) -> Result<Self> {
        // The caller may not substitute a mutated in-memory secret projection
        // while retaining a valid Publication observation from an earlier load.
        let directory = Directory::open(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        let baseline = NodeConfig::load_in(&directory, path)?;
        let supplied = Zeroizing::new(toml::to_string(config).map_err(|_| invalid())?);
        let actual = Zeroizing::new(toml::to_string(&baseline).map_err(|_| invalid())?);
        if config.publication.snapshot() != baseline.publication.snapshot()
            || supplied.as_str() != actual.as_str()
        {
            return Err(invalid());
        }
        let source =
            SecretSnapshot::read_in(&directory, &baseline, path, Some(&keychain), None, None)?;
        Self::build(
            &baseline,
            path,
            directory,
            keychain,
            Some(source),
            false,
            None,
        )
    }

    /// Registration uses a missing-config observation and explicit target
    /// initialization only after the caller has acquired registration custody.
    #[cfg(test)]
    pub(crate) fn registration(
        config: &NodeConfig,
        path: &Path,
        keychain: KeychainBackend,
    ) -> Result<Self> {
        if config.publication.snapshot().is_some() {
            return Err(invalid());
        }
        let directory = Directory::open(path.parent().unwrap_or(Path::new(".")))?;
        Self::build(config, path, directory, keychain, None, true, None)
    }

    pub(crate) fn registration_in(
        config: &NodeConfig,
        path: &Path,
        directory: Directory,
        keychain: KeychainBackend,
        lease: std::fs::File,
    ) -> Result<Self> {
        if config.publication.snapshot().is_some() {
            return Err(invalid());
        }
        Self::build(config, path, directory, keychain, None, true, Some(lease))
    }

    fn build(
        config: &NodeConfig,
        path: &Path,
        directory: Directory,
        keychain: KeychainBackend,
        source: Option<SecretSnapshot>,
        initialize: bool,
        registration_lease: Option<std::fs::File>,
    ) -> Result<Self> {
        if config.node.id != keychain.owner_id() {
            return Err(invalid());
        }
        let file = {
            let _writer = directory.writer_lock()?;
            secret_transaction::check_observed(&directory, path, config.publication.snapshot())?;
            match config.storage_backend.as_str() {
                "file" => Some(if initialize {
                    LocalEncryption::initialize_in_locked(&directory)?
                } else {
                    LocalEncryption::open_in(&directory)?
                }),
                "keychain" => None,
                _ => return Err(invalid()),
            }
        };
        Ok(Self {
            path: path.into(),
            directory,
            baseline: config.clone(),
            source,
            file,
            keychain,
            _registration_lease: registration_lease,
            state: Mutex::new(State::default()),
        })
    }

    /// Must run before the caller edits the secret projection. In legacy
    /// keychain mode it stages the entire read plan into attempt-owned refs.
    pub(crate) fn prepare(&self, config: &mut NodeConfig) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| invalid())?;
        if state.failed
            || state.published
            || config.publication.snapshot() != self.baseline.publication.snapshot()
            || config.node.id != self.baseline.node.id
            || config.storage_backend != self.baseline.storage_backend
        {
            return Err(invalid());
        }
        if state.prepared {
            return Ok(());
        }
        let result = (|| {
            if let Some(source) = &self.source {
                source.check_projection(config)?;
            }
            {
                let _writer = self.directory.writer_lock()?;
                secret_transaction::check_observed(
                    &self.directory,
                    &self.path,
                    self.baseline.publication.snapshot(),
                )?;
            }
            if config.storage_backend == "keychain" {
                state.attempt = Some(Attempt::begin_in(
                    &self.directory,
                    &self.path,
                    self.baseline.publication.snapshot(),
                    &self.baseline.node.id,
                )?);
                if config.keychain_format.is_none() {
                    if let Some(source) = &self.source {
                        let projected = source.project_keychain(
                            &self.baseline,
                            state.attempt.as_ref().ok_or_else(invalid)?,
                            &self.keychain,
                        )?;
                        // Copy only source secret slots; caller metadata edits remain.
                        for (field, _) in secret_fields::fields(&self.baseline)? {
                            field.replace(config, field.encoded(&projected)?.map(str::to_owned))?;
                        }
                    }
                    config.keychain_format = Some(KeychainFormat::ImmutableV1);
                }
            }
            state.prepared = true;
            Ok(())
        })();
        if result.is_err() {
            state.failed = true;
        }
        result
    }

    pub(crate) fn store(&self, field: SecretField, value: &str) -> Result<String> {
        let mut state = self.state.lock().map_err(|_| invalid())?;
        if state.failed || state.published || !state.prepared {
            return Err(invalid());
        }
        let result = (|| match &self.file {
            Some(file) => {
                let _writer = self.directory.writer_lock()?;
                secret_transaction::check_observed(
                    &self.directory,
                    &self.path,
                    self.baseline.publication.snapshot(),
                )?;
                file.encrypt(value)
            }
            None => state
                .attempt
                .as_ref()
                .ok_or_else(invalid)?
                .stage(field, value, &self.keychain),
        })();
        if result.is_err() {
            state.failed = true;
        }
        result
    }

    pub(crate) fn load(
        &self,
        config: &NodeConfig,
        field: &SecretField,
    ) -> Result<Option<Zeroizing<String>>> {
        if config.node.id != self.baseline.node.id
            || config.storage_backend != self.baseline.storage_backend
        {
            return Err(invalid());
        }
        // An anchor is relevant only while this exact legacy source remains
        // selected. A successful modern publication has no legacy fallback.
        let previous = if config.keychain_format.is_none() {
            self.source.as_ref().and_then(SecretSnapshot::legacy_anchor)
        } else {
            None
        };
        let snapshot = SecretSnapshot::read_in(
            &self.directory,
            config,
            &self.path,
            Some(&self.keychain),
            previous,
            self.file.as_ref(),
        )?;
        // A missing signing configuration means signing is disabled, even if
        // an obsolete legacy keychain entry remains. Still validate the whole
        // selected store before returning that absence.
        if matches!(field, SecretField::SigningSecret)
            && config.signing.shared_secret_encrypted.is_none()
        {
            return Ok(None);
        }
        snapshot.get(field)
    }

    pub(crate) fn health(&self, config: &NodeConfig) -> Result<()> {
        if config.node.id != self.baseline.node.id
            || config.storage_backend != self.baseline.storage_backend
        {
            return Err(invalid());
        }
        let previous = if config.keychain_format.is_none() {
            self.source.as_ref().and_then(SecretSnapshot::legacy_anchor)
        } else {
            None
        };
        SecretSnapshot::read_in(
            &self.directory,
            config,
            &self.path,
            Some(&self.keychain),
            previous,
            self.file.as_ref(),
        )?;
        Ok(())
    }

    pub(crate) fn publish(&self, config: &NodeConfig, path: &Path) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| invalid())?;
        if state.failed
            || state.published
            || !state.prepared
            || path != self.path
            || config.publication.snapshot() != self.baseline.publication.snapshot()
            || config.node.id != self.baseline.node.id
            || config.storage_backend != self.baseline.storage_backend
        {
            return Err(invalid());
        }
        // Every required file value must decrypt; every keychain slot must be
        // a typed reference/explicit absence. No partial staging is publishable.
        let result =
            config.save_with_secret_validation_in(&self.directory, path, |directory, next| {
                match &self.file {
                    Some(file) => {
                        file.check_available()?;
                        for (field, required) in secret_fields::fields(next)? {
                            match field.encoded(next)? {
                                Some(encoded) => {
                                    let _value = Zeroizing::new(file.decrypt(encoded)?);
                                }
                                None if required => return Err(invalid()),
                                None => {}
                            }
                        }
                    }
                    None => {
                        secret_transaction::read_ready_references(
                            directory,
                            path,
                            next,
                            &self.keychain,
                        )?;
                    }
                }
                Ok(())
            });
        // Retain the attempt on every error, including installed/uncertain.
        // No rollback or key-service deletion follows an ambiguous publication.
        if result.is_ok() || matches!(result, Err(Error::StorageDurabilityUncertain(_))) {
            state.published = true;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::super::config::CredentialConfig;
    use super::super::secret_fields::CredentialPart;
    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf, NodeConfig, KeychainBackend) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config =
            NodeConfig::new("wss://example.invalid".into(), "node".into(), "file".into());
        let enc = LocalEncryption::initialize(dir.path()).unwrap();
        config
            .set_auth_token("synthetic-original-auth", &enc)
            .unwrap();
        config
            .add_header_credential(
                "service",
                "Authorization",
                "synthetic-original-header",
                &enc,
            )
            .unwrap();
        config.save(&path).unwrap();
        let config = NodeConfig::load(&path).unwrap();
        (dir, path, config, KeychainBackend::new_mock("node"))
    }
    fn header(slug: &str) -> SecretField {
        SecretField::Credential {
            slug: slug.into(),
            part: CredentialPart::Header,
        }
    }

    #[test]
    fn opening_requires_unmodified_baseline_before_any_target_effect() {
        let (dir, path, config, store) = fixture();
        let before = std::fs::read(&path).unwrap();
        let mut mutated = config.clone();
        mutated.node.auth_token_encrypted = mutated.credentials["service"]
            .header_value_encrypted
            .clone()
            .unwrap();
        assert!(SecretOperation::open(&mutated, &path, store.clone()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(SecretOperation::open(&config, &path, store).is_ok());
        assert!(!std::fs::read_dir(dir.path()).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".secret-intent-")
        }));
    }

    #[test]
    fn typed_file_write_publishes_once_and_refuses_a_foreign_target_directory() {
        let (_dir, path, mut config, store) = fixture();
        let operation = SecretOperation::open(&config, &path, store).unwrap();
        let foreign = tempfile::tempdir().unwrap();
        operation.prepare(&mut config).unwrap();
        let encoded = operation
            .store(header("service"), "synthetic-replacement-header")
            .unwrap();
        header("service")
            .replace(&mut config, Some(encoded))
            .unwrap();
        assert!(
            operation
                .publish(&config, &foreign.path().join("config.toml"))
                .is_err()
        );
        assert!(std::fs::read_dir(foreign.path()).unwrap().next().is_none());
        operation.publish(&config, &path).unwrap();
        assert!(operation.publish(&config, &path).is_err());
        let loaded = NodeConfig::load(&path).unwrap();
        let value = operation
            .load(&loaded, &header("service"))
            .unwrap()
            .unwrap();
        assert_eq!(value.as_str(), "synthetic-replacement-header");
    }

    #[test]
    fn metadata_winner_fences_a_prepared_file_operation() {
        let (_dir, path, config, store) = fixture();
        let operation = SecretOperation::open(&config, &path, store).unwrap();
        let mut writer = config.clone();
        operation.prepare(&mut writer).unwrap();
        let encoded = operation
            .store(header("service"), "synthetic-losing-value")
            .unwrap();
        header("service")
            .replace(&mut writer, Some(encoded))
            .unwrap();
        let mut metadata = config.clone();
        metadata.server.url = "wss://metadata-winner.invalid".into();
        metadata.save(&path).unwrap();
        let winning = std::fs::read(&path).unwrap();
        assert!(operation.publish(&writer, &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), winning);
        let loaded = NodeConfig::load(&path).unwrap();
        assert_eq!(
            operation
                .load(&loaded, &header("service"))
                .unwrap()
                .unwrap()
                .as_str(),
            "synthetic-original-header"
        );
    }

    #[test]
    fn any_staging_failure_poisons_operation_even_after_keyfile_restoration() {
        let (dir, path, mut config, store) = fixture();
        let operation = SecretOperation::open(&config, &path, store).unwrap();
        operation.prepare(&mut config).unwrap();
        let key = Zeroizing::new(std::fs::read(dir.path().join(".keyfile")).unwrap());
        std::fs::remove_file(dir.path().join(".keyfile")).unwrap();
        assert!(
            operation
                .store(header("service"), "synthetic-unpublished")
                .is_err()
        );
        Directory::open(dir.path())
            .unwrap()
            .publish(std::ffi::OsStr::new(".keyfile"), &key, false)
            .unwrap();
        assert!(operation.publish(&config, &path).is_err());
        let loaded = NodeConfig::load(&path).unwrap();
        assert_eq!(
            operation
                .load(&loaded, &header("service"))
                .unwrap()
                .unwrap()
                .as_str(),
            "synthetic-original-header"
        );
    }

    #[test]
    fn legacy_read_has_no_effect_and_first_write_converts_all_values_without_overwriting_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = NodeConfig::new(
            "wss://example.invalid".into(),
            "node".into(),
            "keychain".into(),
        );
        config.credentials.insert(
            "service".into(),
            CredentialConfig::new_header("Authorization".into(), None, None),
        );
        config.save(&path).unwrap();
        let mut config = NodeConfig::load(&path).unwrap();
        let store = KeychainBackend::new_mock("node");
        let vault = r#"{"auth_token":"synthetic-legacy-auth","credentials":{"service":"synthetic-legacy-header"}}"#;
        store.set("vault", vault).unwrap();
        let before = std::fs::read(&path).unwrap();
        let operation = SecretOperation::open(&config, &path, store.clone()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(store.get("vault").unwrap(), vault);
        operation.prepare(&mut config).unwrap();
        let encoded = operation
            .store(header("service"), "synthetic-new-header")
            .unwrap();
        header("service")
            .replace(&mut config, Some(encoded))
            .unwrap();
        assert_eq!(store.get("vault").unwrap(), vault);
        operation.publish(&config, &path).unwrap();
        let loaded = NodeConfig::load(&path).unwrap();
        assert!(loaded.keychain_format == Some(KeychainFormat::ImmutableV1));
        assert_eq!(
            operation
                .load(&loaded, &SecretField::AuthToken)
                .unwrap()
                .unwrap()
                .as_str(),
            "synthetic-legacy-auth"
        );
        assert_eq!(
            operation
                .load(&loaded, &header("service"))
                .unwrap()
                .unwrap()
                .as_str(),
            "synthetic-new-header"
        );
        assert_eq!(store.get("vault").unwrap(), vault);
    }

    #[test]
    fn generation_loser_cannot_stage_or_overwrite_winning_keychain_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = NodeConfig::new(
            "wss://example.invalid".into(),
            "node".into(),
            "keychain".into(),
        );
        let store = KeychainBackend::new_mock("node");
        let registration = SecretOperation::registration(&config, &path, store.clone()).unwrap();
        registration.prepare(&mut config).unwrap();
        config.node.auth_token_encrypted = registration
            .store(SecretField::AuthToken, "synthetic-first-auth")
            .unwrap();
        registration.publish(&config, &path).unwrap();
        drop(registration);
        let config = NodeConfig::load(&path).unwrap();
        let winner = SecretOperation::open(&config, &path, store.clone()).unwrap();
        let loser = SecretOperation::open(&config, &path, store.clone()).unwrap();
        let mut winning = config.clone();
        let mut losing = config.clone();
        winner.prepare(&mut winning).unwrap();
        loser.prepare(&mut losing).unwrap();
        winning.node.auth_token_encrypted = winner
            .store(SecretField::AuthToken, "synthetic-winning-auth")
            .unwrap();
        winner.publish(&winning, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            loser
                .store(SecretField::AuthToken, "synthetic-losing-auth")
                .is_err()
        );
        assert!(loser.publish(&losing, &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let loaded = NodeConfig::load(&path).unwrap();
        assert_eq!(
            winner
                .load(&loaded, &SecretField::AuthToken)
                .unwrap()
                .unwrap()
                .as_str(),
            "synthetic-winning-auth"
        );
    }
    #[test]
    fn missing_or_unavailable_staged_object_refuses_replacement_under_publication_lock() {
        for missing in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            let config = NodeConfig::new(
                "wss://example.invalid".into(),
                "node".into(),
                "keychain".into(),
            );
            config.save(&path).unwrap();
            let mut config = NodeConfig::load(&path).unwrap();
            let store = KeychainBackend::new_mock("node");
            store
                .set("vault", r#"{"auth_token":"synthetic-initial-auth"}"#)
                .unwrap();
            let mut operation = SecretOperation::open(&config, &path, store.clone()).unwrap();
            operation.prepare(&mut config).unwrap();
            config.node.auth_token_encrypted = operation
                .store(SecretField::AuthToken, "synthetic-staged-auth")
                .unwrap();
            let before = std::fs::read(&path).unwrap();
            if missing {
                let reference =
                    secret_transaction::ObjectRef::parse(&config.node.auth_token_encrypted)
                        .unwrap()
                        .unwrap();
                secret_transaction::ObjectStore::delete(&store, reference).unwrap();
            } else {
                operation.keychain =
                    KeychainBackend::new_failing_mock("node", "injected store unavailable");
            }
            assert!(operation.publish(&config, &path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), before);
            assert_eq!(
                store.get("vault").unwrap(),
                r#"{"auth_token":"synthetic-initial-auth"}"#
            );
        }
    }

    #[test]
    fn concurrent_registration_rechecks_missing_observation_before_key_initialization() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let directory = Directory::open(dir.path()).unwrap();
        let lock = directory.writer_lock().unwrap();
        let thread_path = path.clone();
        let (ready, receiver) = std::sync::mpsc::channel();
        let child = std::thread::spawn(move || {
            let config =
                NodeConfig::new("wss://loser.invalid".into(), "node".into(), "file".into());
            ready.send(()).unwrap();
            SecretOperation::registration(&config, &thread_path, KeychainBackend::new_mock("node"))
                .is_err()
        });
        receiver.recv().unwrap();
        let winner = NodeConfig::new(
            "wss://winner.invalid".into(),
            "winner".into(),
            "keychain".into(),
        );
        let bytes = toml::to_string(&winner).unwrap();
        directory
            .publish(std::ffi::OsStr::new("config.toml"), bytes.as_bytes(), false)
            .unwrap();
        drop(lock);
        assert!(child.join().unwrap());
        assert!(!dir.path().join(".keyfile").exists());
        assert_eq!(std::fs::read_to_string(path).unwrap(), bytes);
    }
}
