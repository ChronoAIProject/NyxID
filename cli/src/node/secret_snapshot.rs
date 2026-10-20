//! A complete, zeroizing read plan shared by ordinary writes and migration.
//! Every source read finishes before staging any target effect. Applying the
//! plan changes only a private projection until all staging has succeeded.
use std::collections::BTreeMap;
use std::path::Path;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::config::{KeychainFormat, NodeConfig};
use super::encryption::LocalEncryption;
use super::error::{Error, Result};
use super::keychain::KeychainBackend;
use super::legacy_secrets::LegacySnapshot;
use super::secret_fields::{self, SecretField};
use super::secret_transaction::{self, Attempt, ObjectStore};

fn invalid() -> Error {
    Error::Config("Complete authoritative node secret snapshot is required".into())
}

fn projection_digest(config: &NodeConfig) -> Result<[u8; 32]> {
    secret_transaction::live_references(config)?;
    let mut hasher = Sha256::new();
    hasher.update(b"nyxid-node-secret-input-v1");
    for (field, required) in secret_fields::fields(config)? {
        let encoded = Zeroizing::new(
            serde_json::to_vec(&(field.clone(), required, field.encoded(config)?))
                .map_err(|_| invalid())?,
        );
        hasher.update((encoded.len() as u64).to_be_bytes());
        hasher.update(&encoded);
    }
    hasher.update([u8::from(config.keychain_format.is_some())]);
    Ok(hasher.finalize().into())
}

pub(crate) struct SecretSnapshot {
    observed: Option<[u8; 32]>,
    node_id: String,
    source_backend: String,
    input_digest: [u8; 32],
    values: BTreeMap<SecretField, Option<Zeroizing<String>>>,
    legacy: Option<LegacySnapshot>,
}
impl std::fmt::Debug for SecretSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretSnapshot { values_locations_and_owner: [REDACTED] }")
    }
}
impl SecretSnapshot {
    pub(crate) fn read_in(
        directory: &super::storage::Directory,
        config: &NodeConfig,
        path: &Path,
        keychain: Option<&KeychainBackend>,
        previous_legacy: Option<&LegacySnapshot>,
        selected_file: Option<&LocalEncryption>,
    ) -> Result<Self> {
        let _writer = directory.writer_lock()?;
        directory.require_idle()?;
        secret_transaction::live_references(config)?;
        secret_transaction::check_observed(directory, path, config.publication.snapshot())?;
        // A valid CAS token alone cannot authorize a caller-supplied field
        // projection: ciphertext from another slot could otherwise be read
        // under the right file observation. Compare the full typed projection
        // against the installed config before any key/key-service read.
        let installed = NodeConfig::load_in(directory, path)?;
        if installed.node.id != config.node.id
            || installed.storage_backend != config.storage_backend
            || projection_digest(&installed)? != projection_digest(config)?
        {
            return Err(invalid());
        }
        Self::read_projection_in(
            directory,
            config,
            path,
            keychain,
            previous_legacy,
            selected_file,
        )
    }

    /// Migration validates the exact journal generation itself. This helper
    /// still validates the entire typed graph and key health, under its lock.
    pub(crate) fn read_projection_in(
        directory: &super::storage::Directory,
        config: &NodeConfig,
        path: &Path,
        keychain: Option<&KeychainBackend>,
        previous_legacy: Option<&LegacySnapshot>,
        selected_file: Option<&LocalEncryption>,
    ) -> Result<Self> {
        secret_transaction::live_references(config)?;
        let mut values = BTreeMap::new();
        let mut legacy = None;
        match config.storage_backend.as_str() {
            "file" => {
                let opened;
                let encryption = match selected_file {
                    Some(file) => {
                        file.check_available()?;
                        file
                    }
                    None => {
                        opened = LocalEncryption::open_in(directory)?;
                        &opened
                    }
                };
                for (field, required) in secret_fields::fields(config)? {
                    let value = field
                        .encoded(config)?
                        .map(|encoded| encryption.decrypt(encoded).map(Zeroizing::new))
                        .transpose()?;
                    if required && value.is_none() {
                        return Err(invalid());
                    }
                    values.insert(field, value);
                }
            }
            "keychain" => {
                let store = keychain.ok_or_else(invalid)?;
                if config.node.id != store.owner_id() {
                    return Err(invalid());
                }
                if config.keychain_format == Some(KeychainFormat::ImmutableV1) {
                    if previous_legacy.is_some() {
                        return Err(invalid());
                    }
                    let present =
                        secret_transaction::read_ready_references(directory, path, config, store)?;
                    for (field, required) in secret_fields::fields(config)? {
                        let value = present.get(&field).cloned();
                        if required && value.is_none() {
                            return Err(invalid());
                        }
                        values.insert(field, value);
                    }
                } else {
                    let snapshot = match previous_legacy {
                        Some(previous) => previous.reread(config, store)?,
                        None => LegacySnapshot::read(config, store)?,
                    };
                    for (field, required) in secret_fields::fields(config)? {
                        let value = snapshot.get(&field)?;
                        if required && value.is_none() {
                            return Err(invalid());
                        }
                        values.insert(field, value);
                    }
                    legacy = Some(snapshot);
                }
            }
            _ => return Err(invalid()),
        }
        Ok(Self {
            observed: config.publication.snapshot(),
            node_id: config.node.id.clone(),
            source_backend: config.storage_backend.clone(),
            input_digest: projection_digest(config)?,
            values,
            legacy,
        })
    }

    /// Includes typed slots and explicit None/Some(empty) sentinels; never
    /// printed or used as a replacement for reading authoritative custody.
    pub(crate) fn value_digest(&self, key: &[u8; 32]) -> Result<[u8; 32]> {
        let mut hash = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts 32-byte keys");
        hash.update(b"nyxid-node-migration-values-v2");
        for (field, value) in &self.values {
            let encoded =
                Zeroizing::new(serde_json::to_vec(&(field, value)).map_err(|_| invalid())?);
            hash.update(&(encoded.len() as u64).to_be_bytes());
            hash.update(&encoded);
        }
        Ok(hash.finalize().into_bytes().into())
    }

    pub(crate) fn get(&self, field: &SecretField) -> Result<Option<Zeroizing<String>>> {
        self.values.get(field).cloned().ok_or_else(invalid)
    }
    pub(crate) fn legacy_anchor(&self) -> Option<&LegacySnapshot> {
        self.legacy.as_ref()
    }

    pub(crate) fn check_projection(&self, config: &NodeConfig) -> Result<()> {
        if config.publication.snapshot() != self.observed
            || config.node.id != self.node_id
            || config.storage_backend != self.source_backend
            || projection_digest(config)? != self.input_digest
        {
            return Err(invalid());
        }
        let fields = secret_fields::fields(config)?;
        if fields.len() != self.values.len() {
            return Err(invalid());
        }
        for (field, required) in fields {
            if required && self.get(&field)?.is_none() {
                return Err(invalid());
            }
            self.get(&field)?;
        }
        Ok(())
    }

    /// The Attempt itself checks the current generation before each effect.
    /// A failed stage never publishes a partial field map or optional absence.
    pub(crate) fn project_keychain(
        &self,
        config: &NodeConfig,
        attempt: &Attempt,
        store: &impl ObjectStore,
    ) -> Result<NodeConfig> {
        self.check_projection(config)?;
        attempt.check_projection(config)?;
        if store.owner_id() != self.node_id {
            return Err(invalid());
        }
        let mut projected = config.clone();
        for (field, value) in &self.values {
            let encoded = value
                .as_ref()
                .map(|value| attempt.stage(field.clone(), value, store))
                .transpose()?;
            field.replace(&mut projected, encoded)?;
        }
        projected.storage_backend = "keychain".into();
        projected.keychain_format = Some(KeychainFormat::ImmutableV1);
        secret_transaction::live_references(&projected)?;
        Ok(projected)
    }

    /// The caller deliberately initializes the selected file target separately;
    /// reads and normal runtime use never generate or replace a missing key.
    pub(crate) fn project_file(
        &self,
        config: &NodeConfig,
        target: &LocalEncryption,
    ) -> Result<NodeConfig> {
        self.check_projection(config)?;
        let mut projected = config.clone();
        for (field, value) in &self.values {
            let encoded = value
                .as_ref()
                .map(|value| target.encrypt(value))
                .transpose()?;
            field.replace(&mut projected, encoded)?;
        }
        projected.storage_backend = "file".into();
        projected.keychain_format = None;
        secret_transaction::live_references(&projected)?;
        Ok(projected)
    }
}

#[cfg(test)]
mod tests {
    fn read_snapshot(
        config: &super::super::config::NodeConfig,
        path: &std::path::Path,
        keychain: Option<&super::super::keychain::KeychainBackend>,
        previous: Option<&super::super::legacy_secrets::LegacySnapshot>,
    ) -> super::super::error::Result<super::super::secret_snapshot::SecretSnapshot> {
        let directory = super::super::storage::Directory::open(path.parent().unwrap())?;
        super::super::secret_snapshot::SecretSnapshot::read_in(
            &directory, config, path, keychain, previous, None,
        )
    }

    use super::super::config::{CredentialConfig, PendingCryptoKeyConfig, SshKeyConfig};
    use super::super::secret_transaction::ObjectRef;
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingStore {
        inner: KeychainBackend,
        puts: AtomicUsize,
        fail_at: usize,
    }
    impl RecordingStore {
        fn new(fail_at: usize) -> Self {
            Self {
                inner: KeychainBackend::new_mock("node"),
                puts: AtomicUsize::new(0),
                fail_at,
            }
        }
    }
    impl ObjectStore for RecordingStore {
        fn owner_id(&self) -> &str {
            self.inner.owner_id()
        }
        fn get(&self, reference: ObjectRef) -> Result<Option<Zeroizing<String>>> {
            ObjectStore::get(&self.inner, reference)
        }
        fn put(&self, reference: ObjectRef, value: &str) -> Result<()> {
            let number = self.puts.fetch_add(1, Ordering::SeqCst) + 1;
            if number == self.fail_at {
                return Err(invalid());
            }
            ObjectStore::put(&self.inner, reference, value)
        }
        fn delete(&self, reference: ObjectRef) -> Result<()> {
            ObjectStore::delete(&self.inner, reference)
        }
    }

    fn source() -> (tempfile::TempDir, std::path::PathBuf, NodeConfig) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config =
            NodeConfig::new("wss://example.invalid".into(), "node".into(), "file".into());
        config.signing.shared_secret_encrypted = Some(String::new());
        let mut oauth = CredentialConfig::new_header("Authorization".into(), None, None);
        oauth.oauth_managed = true;
        config.credentials.insert("oauth".into(), oauth);
        config.credentials.insert(
            "query".into(),
            CredentialConfig::new_query_param("key".into(), None, None),
        );
        config
            .credentials
            .insert("none".into(), CredentialConfig::new_no_auth(None));
        config.ssh_keys.push(SshKeyConfig {
            service_slug: "ssh".into(),
            principal: "alice".into(),
            private_key_pem_encrypted: None,
            passphrase_encrypted: None,
            target_host: "example.invalid".into(),
            target_port: 22,
            host_key_sha256: None,
            algorithms: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        });
        config.pending_crypto_keys.insert(
            "pending".into(),
            PendingCryptoKeyConfig {
                version: "1".into(),
                service_slug: "pending-service".into(),
                injection_method: "header".into(),
                field_name: "Authorization".into(),
                target_url: None,
                expires_at: "2026-10-20T00:00:00Z".into(),
                public_key: "synthetic-public".into(),
                private_key_encrypted: None,
            },
        );
        let enc = LocalEncryption::initialize(dir.path()).unwrap();
        for (field, _) in secret_fields::fields(&config).unwrap() {
            let sentinel = Zeroizing::new(format!(
                "synthetic-field:{}",
                serde_json::to_string(&field).unwrap()
            ));
            let value = if matches!(field, SecretField::SshPassphrase { .. }) {
                None
            } else if matches!(
                field,
                SecretField::Credential {
                    part: secret_fields::CredentialPart::OAuthClientSecret,
                    ..
                }
            ) {
                Some("")
            } else {
                Some(sentinel.as_str())
            };
            field
                .replace(&mut config, value.map(|v| enc.encrypt(v).unwrap()))
                .unwrap();
        }
        config.save(&path).unwrap();
        let config = NodeConfig::load(&path).unwrap();
        (dir, path, config)
    }

    #[test]
    fn complete_round_trip_preserves_all_secret_fields_and_explicit_optional_presence() {
        let (dir, path, source) = source();
        let before = std::fs::read(&path).unwrap();
        let snapshot = read_snapshot(&source, &path, None, None).unwrap();
        let store = RecordingStore::new(usize::MAX);
        let attempt = Attempt::begin_in(
            &super::super::storage::Directory::open(path.parent().unwrap()).unwrap(),
            &path,
            source.publication.snapshot(),
            "node",
        )
        .unwrap();
        let projected = snapshot
            .project_keychain(&source, &attempt, &store)
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(projected.keychain_format == Some(KeychainFormat::ImmutableV1));
        projected.save(&path).unwrap();
        drop(attempt);
        let modern = NodeConfig::load(&path).unwrap();
        let readback = read_snapshot(&modern, &path, Some(&store.inner), None).unwrap();
        for (field, _) in secret_fields::fields(&source).unwrap() {
            assert!(snapshot.get(&field).unwrap() == readback.get(&field).unwrap());
        }
        let file = readback
            .project_file(
                &modern,
                &LocalEncryption::open_existing(dir.path()).unwrap(),
            )
            .unwrap();
        file.save(&path).unwrap();
        let reloaded = NodeConfig::load(&path).unwrap();
        let final_read = read_snapshot(&reloaded, &path, None, None).unwrap();
        for (field, _) in secret_fields::fields(&source).unwrap() {
            assert!(snapshot.get(&field).unwrap() == final_read.get(&field).unwrap());
        }
    }

    #[test]
    fn each_target_stage_failure_keeps_source_and_live_values_and_has_owned_recovery() {
        let (_dir, path, source) = source();
        let snapshot = read_snapshot(&source, &path, None, None).unwrap();
        let count = snapshot.values.values().filter(|v| v.is_some()).count();
        let before = std::fs::read(&path).unwrap();
        for fail_at in 1..=count {
            let store = RecordingStore::new(fail_at);
            let attempt = Attempt::begin_in(
                &super::super::storage::Directory::open(path.parent().unwrap()).unwrap(),
                &path,
                source.publication.snapshot(),
                "node",
            )
            .unwrap();
            let id = attempt.id().unwrap();
            assert!(
                snapshot
                    .project_keychain(&source, &attempt, &store)
                    .is_err()
            );
            assert_eq!(std::fs::read(&path).unwrap(), before);
            assert!(read_snapshot(&source, &path, None, None).is_ok());
            assert_eq!(store.puts.load(Ordering::SeqCst), fail_at);
            drop(attempt);
            let cleanup = secret_transaction::collect_one_in(
                &super::super::storage::Directory::open(path.parent().unwrap()).unwrap(),
                &path,
                id,
                &store,
            )
            .unwrap();
            assert_eq!(cleanup.retired, fail_at);
            assert_eq!(cleanup.retained, 0);
        }
    }

    #[test]
    fn full_projection_preflight_refuses_late_missing_or_modified_fields_before_target_effects() {
        let (_dir, path, source) = source();
        let mut snapshot = read_snapshot(&source, &path, None, None).unwrap();
        let store = RecordingStore::new(usize::MAX);
        let attempt = Attempt::begin_in(
            &super::super::storage::Directory::open(path.parent().unwrap()).unwrap(),
            &path,
            source.publication.snapshot(),
            "node",
        )
        .unwrap();
        let mut changed = source.clone();
        changed.ssh_keys.clear();
        assert!(
            snapshot
                .project_keychain(&changed, &attempt, &store)
                .is_err()
        );
        changed = source.clone();
        changed.node.auth_token_encrypted.push('x');
        assert!(
            snapshot
                .project_keychain(&changed, &attempt, &store)
                .is_err()
        );
        snapshot.values.pop_last();
        assert!(
            snapshot
                .project_keychain(&source, &attempt, &store)
                .is_err()
        );
        assert_eq!(store.puts.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn stale_observation_and_source_read_failure_cannot_stage_or_initialize_target() {
        let (_dir, path, source) = source();
        let snapshot = read_snapshot(&source, &path, None, None).unwrap();
        let store = RecordingStore::new(usize::MAX);
        let attempt = Attempt::begin_in(
            &super::super::storage::Directory::open(path.parent().unwrap()).unwrap(),
            &path,
            source.publication.snapshot(),
            "node",
        )
        .unwrap();
        let mut changed = source.clone();
        changed.server.url = "wss://changed.invalid".into();
        changed.save(&path).unwrap();
        assert!(
            snapshot
                .project_keychain(&source, &attempt, &store)
                .is_err()
        );
        assert!(read_snapshot(&source, &path, None, None).is_err());
        assert_eq!(store.puts.load(Ordering::SeqCst), 0);
        let mut changed = NodeConfig::load(&path).unwrap();
        changed
            .pending_crypto_keys
            .values_mut()
            .next()
            .unwrap()
            .private_key_encrypted = Some("invalid".into());
        changed.save(&path).unwrap();
        let changed = NodeConfig::load(&path).unwrap();
        assert!(read_snapshot(&changed, &path, None, None).is_err());
        assert_eq!(store.puts.load(Ordering::SeqCst), 0);
    }
}
