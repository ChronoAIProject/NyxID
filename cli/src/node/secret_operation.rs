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
use super::secret_transaction::{self, Attempt, CleanupReport, ObjectStore};
use super::storage::Directory;

fn invalid() -> Error {
    Error::Config("Node secret operation requires its original authoritative configuration; reload before retrying".into())
}

/// The publication phase is independent of credential storage. In particular,
/// Unpublished never proves that a caller's preceding store had no effect.
#[must_use]
pub enum PublicationOutcome {
    Unpublished(Error),
    PublicationUncertain(Error),
    /// An error leaves durable debt; a successful report can instead describe
    /// active leases or explicit recovery that this pass correctly retained.
    PublishedCleanupPending(Result<CleanupReport>),
    Complete(CleanupReport),
}
impl std::fmt::Debug for PublicationOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unpublished(_) => f.write_str("Unpublished([REDACTED])"),
            Self::PublicationUncertain(_) => f.write_str("PublicationUncertain([REDACTED])"),
            Self::PublishedCleanupPending(_) => f.write_str("PublishedCleanupPending"),
            Self::Complete(report) => f.debug_tuple("Complete").field(report).finish(),
        }
    }
}
impl PublicationOutcome {
    /// Preserve the legacy API: successful GC passes that defer active/recovery
    /// owners were already accepted. Typed consumers must inspect the outcome
    /// to distinguish that retained debt from complete retirement.
    pub(crate) fn into_result(self) -> Result<()> {
        match self {
            Self::Unpublished(error) | Self::PublicationUncertain(error) => Err(error),
            Self::PublishedCleanupPending(result) => result.map(|_| ()),
            Self::Complete(_) => Ok(()),
        }
    }
}

#[derive(Default, PartialEq, Eq)]
enum PublicationState {
    #[default]
    Open,
    Unpublished,
    Uncertain,
    Published,
}

#[derive(Default)]
struct State {
    attempt: Option<Attempt>,
    prepared: bool,
    failed: bool,
    publication: PublicationState,
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
    pub(crate) fn oauth_custody(&self) -> Result<(Directory, PathBuf, KeychainBackend)> {
        Ok((
            self.directory.try_clone()?,
            self.path.clone(),
            self.keychain.clone(),
        ))
    }
    /// Observe a new generation through the original directory and custody.
    /// Credential edits may advance; runtime identity/backend changes require
    /// a stopped daemon and explicit restart rather than cached-key fallback.
    pub(crate) fn current(&self) -> Result<(NodeConfig, Self)> {
        self.directory.require_idle()?;
        let baseline = NodeConfig::load_in(&self.directory, &self.path)?;
        if baseline.node.id != self.baseline.node.id
            || baseline.storage_backend != self.baseline.storage_backend
            || baseline.keychain_format != self.baseline.keychain_format
            || baseline.server.url != self.baseline.server.url
        {
            return Err(Error::Config(
                "Node storage or identity changed; stop the daemon and restart it with the selected configuration".into(),
            ));
        }
        let snapshot = SecretSnapshot::read_in(
            &self.directory,
            &baseline,
            &self.path,
            Some(&self.keychain),
            self.source.as_ref().and_then(SecretSnapshot::legacy_anchor),
            self.file.as_ref(),
        )?;
        let source = self.source.as_ref().ok_or_else(invalid)?;
        for field in [SecretField::AuthToken, SecretField::SigningSecret] {
            let original = if field.encoded(&self.baseline)?.is_some() {
                source.get(&field)?
            } else {
                None
            };
            let current = if field.encoded(&baseline)?.is_some() {
                snapshot.get(&field)?
            } else {
                None
            };
            if current != original {
                return Err(Error::Config(
                    "Node authentication material changed; restart the daemon before serving requests".into(),
                ));
            }
        }
        let operation = Self {
            path: self.path.clone(),
            directory: self.directory.try_clone()?,
            baseline: baseline.clone(),
            source: Some(snapshot),
            file: self
                .file
                .as_ref()
                .map(LocalEncryption::try_clone)
                .transpose()?,
            keychain: self.keychain.clone(),
            _registration_lease: None,
            state: Mutex::new(State::default()),
        };
        Ok((baseline, operation))
    }

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
        Self::open_in(config, path, directory, keychain)
    }

    /// Managed bootstrap already holds directory custody across its network
    /// await. Observe that same handle instead of resolving the path again.
    pub(crate) fn open_in(
        config: &NodeConfig,
        path: &Path,
        directory: Directory,
        keychain: KeychainBackend,
    ) -> Result<Self> {
        directory.require_idle()?;
        let baseline = NodeConfig::load_in(&directory, path)?;
        let supplied = Zeroizing::new(toml::to_string(config).map_err(|_| invalid())?);
        let actual = Zeroizing::new(toml::to_string(&baseline).map_err(|_| invalid())?);
        if config.publication.snapshot() != baseline.publication.snapshot()
            || supplied.as_str() != actual.as_str()
        {
            return Err(invalid());
        }
        let mut operation = Self::build(&baseline, path, directory, keychain, None, false, None)?;
        // Retain the exact selected file key through source reads and target
        // projection. Reopening between them could accidentally re-encrypt a
        // captured source under a concurrently replaced keyfile.
        operation.source = Some(SecretSnapshot::read_in(
            &operation.directory,
            &operation.baseline,
            path,
            Some(&operation.keychain),
            None,
            operation.file.as_ref(),
        )?);
        Ok(operation)
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
            directory.require_idle()?;
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
            || state.publication != PublicationState::Open
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
                self.directory.require_idle()?;
                secret_transaction::check_observed(
                    &self.directory,
                    &self.path,
                    self.baseline.publication.snapshot(),
                )?;
            }
            if let (Some(file), Some(source)) = (&self.file, &self.source) {
                let _writer = self.directory.writer_lock()?;
                self.directory.require_idle()?;
                secret_transaction::check_observed(
                    &self.directory,
                    &self.path,
                    self.baseline.publication.snapshot(),
                )?;
                file.check_available()?;
                let projected = source.project_file(&self.baseline, file)?;
                for (field, _) in secret_fields::fields(&self.baseline)? {
                    field.replace(config, field.encoded(&projected)?.map(str::to_owned))?;
                }
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
        if state.failed || state.publication != PublicationState::Open || !state.prepared {
            return Err(invalid());
        }
        let result = (|| match &self.file {
            Some(file) => {
                let _writer = self.directory.writer_lock()?;
                self.directory.require_idle()?;
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

    /// A writer can read its private prepared projection before publication.
    /// Validate the full graph under the same pinned lock; an ordinary reader
    /// must continue to require the installed projection instead.
    pub(crate) fn load_projection(
        &self,
        config: &NodeConfig,
        field: &SecretField,
    ) -> Result<Option<Zeroizing<String>>> {
        let state = self.state.lock().map_err(|_| invalid())?;
        if state.failed
            || !state.prepared
            || config.node.id != self.baseline.node.id
            || config.storage_backend != self.baseline.storage_backend
        {
            return Err(invalid());
        }
        let _writer = self.directory.writer_lock()?;
        self.directory.require_idle()?;
        secret_transaction::check_observed(
            &self.directory,
            &self.path,
            config.publication.snapshot(),
        )?;
        let bytes = self.directory.read(
            self.path.file_name().ok_or_else(invalid)?,
            super::storage::MAX_CONFIG_BYTES,
        )?;
        secret_transaction::validate_publication(
            &self.directory,
            self.path.file_name().ok_or_else(invalid)?,
            Some(&bytes),
            config,
        )?;
        match &self.file {
            Some(file) => {
                file.check_available()?;
                let mut selected = None;
                for (location, required) in secret_fields::fields(config)? {
                    let value = location
                        .encoded(config)?
                        .map(|value| file.decrypt(value).map(Zeroizing::new))
                        .transpose()?;
                    if required && value.is_none() {
                        return Err(invalid());
                    }
                    if &location == field {
                        selected = Some(value);
                    }
                }
                if matches!(field, SecretField::SigningSecret)
                    && config.signing.shared_secret_encrypted.is_none()
                {
                    Ok(None)
                } else {
                    selected.ok_or_else(invalid)
                }
            }
            None => {
                let values = secret_transaction::read_ready_references(
                    &self.directory,
                    &self.path,
                    config,
                    &self.keychain,
                )?;
                // Enumerating the typed projection also distinguishes an absent
                // optional field from a location that no longer exists.
                field.encoded(config)?;
                Ok(values.get(field).cloned())
            }
        }
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
        self.publish_with_intent(config, path, |_, _| Ok(()))
    }

    pub(crate) fn publish_with_intent(
        &self,
        config: &NodeConfig,
        path: &Path,
        intent: impl FnOnce(&Directory, &[u8]) -> Result<()>,
    ) -> Result<()> {
        self.publish_with_intent_outcome(config, path, intent)?
            .into_result()
    }

    pub(crate) fn publish_outcome(
        &self,
        config: &NodeConfig,
        path: &Path,
    ) -> Result<PublicationOutcome> {
        self.publish_with_intent_outcome(config, path, |_, _| Ok(()))
    }

    pub(crate) fn publish_with_intent_outcome(
        &self,
        config: &NodeConfig,
        path: &Path,
        intent: impl FnOnce(&Directory, &[u8]) -> Result<()>,
    ) -> Result<PublicationOutcome> {
        self.publish_using_store(config, path, intent, &self.keychain)
    }

    fn publish_using_store(
        &self,
        config: &NodeConfig,
        path: &Path,
        intent: impl FnOnce(&Directory, &[u8]) -> Result<()>,
        store: &impl ObjectStore,
    ) -> Result<PublicationOutcome> {
        let mut state = self.state.lock().map_err(|_| invalid())?;
        if state.failed
            || state.publication != PublicationState::Open
            || !state.prepared
            || path != self.path
            || config.publication.snapshot() != self.baseline.publication.snapshot()
            || config.node.id != self.baseline.node.id
            || config.storage_backend != self.baseline.storage_backend
        {
            return Err(invalid());
        }
        // This operation owns one publication attempt, including definite
        // failure. A caller must observe afresh rather than repeat its intent.
        state.publication = PublicationState::Unpublished;
        // Every required file value must decrypt; every keychain slot must be
        // a typed reference/explicit absence. No partial staging is publishable.
        let result = config.save_with_secret_intent_in(
            &self.directory,
            path,
            |directory, next| {
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
                        secret_transaction::read_ready_references(directory, path, next, store)?;
                    }
                }
                Ok(())
            },
            intent,
        );
        match result {
            Ok(()) => {
                state.publication = PublicationState::Published;
                drop(state);
                self.finalize_using_store(store, 1)
            }
            Err(error @ Error::StorageDurabilityUncertain(_)) => {
                // Keep the attempt lease. Neither retry nor error conversion
                // may authorize retirement or pretend this was committed.
                state.publication = PublicationState::Uncertain;
                Ok(PublicationOutcome::PublicationUncertain(error))
            }
            Err(error) => Ok(PublicationOutcome::Unpublished(error)),
        }
    }

    /// At most two cleanup passes. This never decrypts/stores credentials,
    /// invokes publication intent, or retries configuration publication.
    pub(crate) fn resume_finalize(&self) -> Result<PublicationOutcome> {
        self.finalize_using_store(&self.keychain, 2)
    }

    fn finalize_using_store(
        &self,
        store: &impl ObjectStore,
        passes: usize,
    ) -> Result<PublicationOutcome> {
        let mut state = self.state.lock().map_err(|_| invalid())?;
        match state.publication {
            PublicationState::Uncertain => {
                return Ok(PublicationOutcome::PublicationUncertain(
                    Error::StorageDurabilityUncertain(std::io::Error::other(
                        "Publication requires a fresh authoritative observation",
                    )),
                ));
            }
            PublicationState::Published => {}
            PublicationState::Open | PublicationState::Unpublished => return Err(invalid()),
        }
        // Only a confirmed publication releases its own lease for collection.
        // The journal remains durable on every failed/deferred cleanup pass.
        let attempt_id = match state.attempt.as_ref().map(Attempt::id).transpose() {
            Ok(id) => id,
            Err(_) => {
                return Ok(PublicationOutcome::PublishedCleanupPending(Err(
                    cleanup_pending(),
                )));
            }
        };
        drop(state.attempt.take());
        let mut retired = if let Some(id) = attempt_id {
            match secret_transaction::collect_one_in(&self.directory, &self.path, id, store) {
                Ok(cleanup) => cleanup.retired,
                Err(_) => {
                    return Ok(PublicationOutcome::PublishedCleanupPending(Err(
                        cleanup_pending(),
                    )));
                }
            }
        } else {
            0
        };
        let mut last = Err(cleanup_pending());
        for _ in 0..passes.min(2) {
            last = secret_transaction::collect_all_in(&self.directory, &self.path, store)
                .map(|mut report| {
                    retired += report.retired;
                    report.retired = retired;
                    report
                })
                .map_err(|_| cleanup_pending());
            if let Ok(report) = &last
                && !report.deferred()
            {
                return Ok(PublicationOutcome::Complete(last?));
            }
        }
        Ok(PublicationOutcome::PublishedCleanupPending(last))
    }

    /// A fresh reader may reconcile durable journals after restart. The
    /// collector obtains current authority and resolves parent-sync uncertainty
    /// before any delete; this does not confirm an earlier publication outcome.
    pub(crate) fn reconcile_retirement(&self) -> Result<CleanupReport> {
        let state = self.state.lock().map_err(|_| invalid())?;
        if state.prepared || state.publication != PublicationState::Open {
            return Err(invalid());
        }
        secret_transaction::collect_all_in(&self.directory, &self.path, &self.keychain)
    }
}

fn cleanup_pending() -> Error {
    Error::Config(
        "Node configuration was saved; old secret cleanup is incomplete and will retry on the next update".into(),
    )
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

    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RetirementFault {
        store: KeychainBackend,
        failures: AtomicUsize,
        deletes: AtomicUsize,
    }
    impl ObjectStore for RetirementFault {
        fn owner_id(&self) -> &str {
            self.store.owner_id()
        }
        fn get(
            &self,
            reference: secret_transaction::ObjectRef,
        ) -> Result<Option<Zeroizing<String>>> {
            ObjectStore::get(&self.store, reference)
        }
        fn put(&self, _: secret_transaction::ObjectRef, _: &str) -> Result<()> {
            panic!("Finalization must not repeat a secret store")
        }
        fn delete(&self, reference: secret_transaction::ObjectRef) -> Result<()> {
            self.deletes.fetch_add(1, Ordering::SeqCst);
            if self
                .failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                    left.checked_sub(1)
                })
                .is_ok()
            {
                return Err(Error::Keychain("injected retirement failure".into()));
            }
            ObjectStore::delete(&self.store, reference)
        }
    }

    fn pending_fixture() -> (
        tempfile::TempDir,
        PathBuf,
        NodeConfig,
        KeychainBackend,
        secret_transaction::ObjectRef,
    ) {
        use super::super::config::PendingCryptoKeyConfig;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = NodeConfig::new(
            "wss://example.invalid".into(),
            "node".into(),
            "keychain".into(),
        );
        config.pending_crypto_keys.insert(
            "pending".into(),
            PendingCryptoKeyConfig {
                version: "v1".into(),
                service_slug: "service".into(),
                injection_method: "header".into(),
                field_name: "Authorization".into(),
                target_url: None,
                expires_at: "2026-01-01T00:00:00Z".into(),
                public_key: "synthetic-public-key".into(),
                private_key_encrypted: None,
            },
        );
        let store = KeychainBackend::new_mock("node");
        let registration = SecretOperation::registration(&config, &path, store.clone()).unwrap();
        registration.prepare(&mut config).unwrap();
        config.node.auth_token_encrypted = registration
            .store(SecretField::AuthToken, "synthetic-auth")
            .unwrap();
        let pending = SecretField::PendingPrivate {
            pending_id: "pending".into(),
        };
        let reference = registration
            .store(pending.clone(), "synthetic-private-key")
            .unwrap();
        pending
            .replace(&mut config, Some(reference.clone()))
            .unwrap();
        registration.publish(&config, &path).unwrap();
        drop(registration);
        let config = NodeConfig::load(&path).unwrap();
        (
            dir,
            path,
            config,
            store,
            secret_transaction::ObjectRef::parse(&reference)
                .unwrap()
                .unwrap(),
        )
    }

    #[test]
    fn typed_publication_and_bounded_finalize_preserve_one_store_and_one_publication() {
        let (_dir, path, mut config, store, pending) = pending_fixture();
        let operation = SecretOperation::open(&config, &path, store.clone()).unwrap();
        operation.prepare(&mut config).unwrap();
        config.credentials.insert(
            "service".into(),
            CredentialConfig::new_header("Authorization".into(), None, None),
        );
        // Exactly one credential store occurs before publication. The injected
        // ObjectStore panics if any finalize pass tries to stage another value.
        let encoded = operation
            .store(header("service"), "synthetic-delivered")
            .unwrap();
        header("service")
            .replace(&mut config, Some(encoded))
            .unwrap();
        config.pending_crypto_keys.remove("pending");
        let faults = RetirementFault {
            store: store.clone(),
            failures: AtomicUsize::new(1),
            deletes: AtomicUsize::new(0),
        };
        let publications = AtomicUsize::new(0);
        let outcome = operation
            .publish_using_store(
                &config,
                &path,
                |_, _| {
                    publications.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
                &faults,
            )
            .unwrap();
        assert!(matches!(
            outcome,
            PublicationOutcome::PublishedCleanupPending(Err(_))
        ));
        assert!(ObjectStore::get(&store, pending).unwrap().is_some());
        let published = std::fs::read(&path).unwrap();
        assert!(
            !NodeConfig::load(&path)
                .unwrap()
                .pending_crypto_keys
                .contains_key("pending")
        );
        assert!(matches!(
            operation.finalize_using_store(&faults, 2).unwrap(),
            PublicationOutcome::Complete(_)
        ));
        assert!(ObjectStore::get(&store, pending).unwrap().is_none());
        assert_eq!(faults.deletes.load(Ordering::SeqCst), 2);
        assert_eq!(publications.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read(&path).unwrap(), published);
        assert!(
            operation
                .store(header("service"), "synthetic-repeat")
                .is_err()
        );
        assert!(
            operation
                .publish_with_intent_outcome(&config, &path, |_, _| {
                    publications.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(publications.load(Ordering::SeqCst), 1);
        let current = NodeConfig::load(&path).unwrap();
        assert_eq!(
            operation
                .load(&current, &header("service"))
                .unwrap()
                .unwrap()
                .as_str(),
            "synthetic-delivered"
        );
    }

    #[test]
    fn published_retirement_debt_survives_a_fresh_reader_without_republishing() {
        let (_dir, path, mut config, store, pending) = pending_fixture();
        let operation = SecretOperation::open(&config, &path, store.clone()).unwrap();
        operation.prepare(&mut config).unwrap();
        config.pending_crypto_keys.remove("pending");
        let faults = RetirementFault {
            store: store.clone(),
            failures: AtomicUsize::new(3),
            deletes: AtomicUsize::new(0),
        };
        assert!(matches!(
            operation
                .publish_using_store(&config, &path, |_, _| Ok(()), &faults)
                .unwrap(),
            PublicationOutcome::PublishedCleanupPending(Err(_))
        ));
        // Both retry passes fail. Durable ownership still retains the usable key.
        assert!(matches!(
            operation.finalize_using_store(&faults, 2).unwrap(),
            PublicationOutcome::PublishedCleanupPending(Err(_))
        ));
        assert_eq!(faults.deletes.load(Ordering::SeqCst), 3);
        assert!(ObjectStore::get(&store, pending).unwrap().is_some());
        let published = std::fs::read(&path).unwrap();
        drop(operation);
        let current = NodeConfig::load(&path).unwrap();
        let restarted = SecretOperation::open(&current, &path, store.clone()).unwrap();
        let report = restarted.reconcile_retirement().unwrap();
        assert_eq!(report.retired, 1);
        assert!(!report.deferred());
        assert!(ObjectStore::get(&store, pending).unwrap().is_none());
        assert_eq!(std::fs::read(&path).unwrap(), published);
        assert_eq!(
            restarted
                .load(&current, &SecretField::AuthToken)
                .unwrap()
                .unwrap()
                .as_str(),
            "synthetic-auth"
        );
    }

    #[test]
    fn uncertain_publication_retains_lease_and_never_authorizes_finalize() {
        for installed in [false, true] {
            let (_dir, path, mut config, store, pending) = pending_fixture();
            let operation = SecretOperation::open(&config, &path, store.clone()).unwrap();
            operation.prepare(&mut config).unwrap();
            config.pending_crypto_keys.remove("pending");
            let before = std::fs::read(&path).unwrap();
            // Exercise the operation boundary both before and after installation.
            // The latter installs the exact prepared bytes under the real writer
            // lock, then injects the durability-uncertain return. Storage's own
            // sync-fault control separately checks its observation update.
            let outcome = operation
                .publish_with_intent_outcome(&config, &path, |directory, next| {
                    if installed {
                        directory.publish(path.file_name().unwrap(), next, true)?;
                    }
                    Err(Error::StorageDurabilityUncertain(
                        std::io::Error::from_raw_os_error(libc::EIO),
                    ))
                })
                .unwrap();
            assert!(matches!(
                &outcome,
                PublicationOutcome::PublicationUncertain(_)
            ));
            assert!(outcome.into_result().is_err());
            let faults = RetirementFault {
                store: store.clone(),
                failures: AtomicUsize::new(0),
                deletes: AtomicUsize::new(0),
            };
            assert!(matches!(
                operation.finalize_using_store(&faults, 2).unwrap(),
                PublicationOutcome::PublicationUncertain(_)
            ));
            assert_eq!(faults.deletes.load(Ordering::SeqCst), 0);
            assert!(operation.state.lock().unwrap().attempt.is_some());
            assert!(operation.publish_outcome(&config, &path).is_err());
            assert!(operation.reconcile_retirement().is_err());
            assert_eq!(std::fs::read(&path).unwrap() != before, installed);
            assert!(ObjectStore::get(&store, pending).unwrap().is_some());
            // Only a separate fresh authoritative pass may sync the installed
            // projection and retire its removed key; the uncertain lease remains.
            let directory = Directory::open(path.parent().unwrap()).unwrap();
            let report = secret_transaction::collect_all_in(&directory, &path, &store).unwrap();
            assert_eq!(report.active_attempts, 1);
            assert!(report.deferred());
            assert_eq!(report.retired, usize::from(installed));
            assert_eq!(
                ObjectStore::get(&store, pending).unwrap().is_none(),
                installed
            );
        }
    }

    #[test]
    fn unpublished_intent_failure_consumes_only_that_publication_attempt() {
        let (_dir, path, mut config, store) = fixture();
        let operation = SecretOperation::open(&config, &path, store).unwrap();
        operation.prepare(&mut config).unwrap();
        let before = std::fs::read(&path).unwrap();
        let publications = AtomicUsize::new(0);
        let outcome = operation
            .publish_with_intent_outcome(&config, &path, |_, _| {
                publications.fetch_add(1, Ordering::SeqCst);
                Err(Error::Config("synthetic-intent-failure".into()))
            })
            .unwrap();
        assert!(matches!(outcome, PublicationOutcome::Unpublished(_)));
        assert!(
            operation
                .publish_with_intent_outcome(&config, &path, |_, _| {
                    publications.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
                .is_err()
        );
        assert!(operation.resume_finalize().is_err());
        assert_eq!(publications.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn typed_deferred_active_ownership_is_reported_without_changing_legacy_success() {
        let (_dir, path, config, store, _) = pending_fixture();
        let active = SecretOperation::open(&config, &path, store.clone()).unwrap();
        active.prepare(&mut config.clone()).unwrap();
        let operation = SecretOperation::open(&config, &path, store).unwrap();
        let mut next = config.clone();
        operation.prepare(&mut next).unwrap();
        let outcome = operation.publish_outcome(&next, &path).unwrap();
        let PublicationOutcome::PublishedCleanupPending(Ok(report)) = &outcome else {
            panic!("Active ownership must remain explicit: {outcome:?}");
        };
        assert_eq!(report.active_attempts, 1);
        outcome.into_result().unwrap();
        drop(active);
        assert!(matches!(
            operation.resume_finalize().unwrap(),
            PublicationOutcome::Complete(_)
        ));
    }

    #[test]
    fn typed_outcome_debug_never_exposes_failure_material() {
        let outcome =
            PublicationOutcome::Unpublished(Error::Config("synthetic-secret-and-reference".into()));
        assert!(!format!("{outcome:?}").contains("synthetic-secret-and-reference"));
        let outcome = PublicationOutcome::PublishedCleanupPending(Err(Error::Config(
            "synthetic-secret-and-reference".into(),
        )));
        assert!(!format!("{outcome:?}").contains("synthetic-secret-and-reference"));
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
    fn complete_file_projection_refuses_replaced_key_before_preparing_or_publishing() {
        let (dir, path, mut config, store) = fixture();
        let before = std::fs::read(&path).unwrap();
        let original = Zeroizing::new(std::fs::read(dir.path().join(".keyfile")).unwrap());
        let operation = SecretOperation::open(&config, &path, store).unwrap();
        let directory = Directory::open(dir.path()).unwrap();
        let replacement = Zeroizing::new([129u8; 32]);
        directory
            .publish(
                std::ffi::OsStr::new(".keyfile"),
                replacement.as_slice(),
                true,
            )
            .unwrap();
        assert!(operation.prepare(&mut config).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(std::fs::read(dir.path().join(".keyfile")).unwrap() == replacement.as_slice());
        directory
            .publish(std::ffi::OsStr::new(".keyfile"), &original, true)
            .unwrap();
        assert!(operation.prepare(&mut config).is_err());
        assert!(operation.publish(&config, &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(
            !directory
                .names()
                .unwrap()
                .iter()
                .any(|name| name.to_string_lossy().starts_with(".secret-intent-"))
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
