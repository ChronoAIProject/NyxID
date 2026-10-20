//! Explicit, restartable backend switch. The journal owns an exact source and
//! one verified target; its presence fences normal runtime and GC operations.
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;

use base64::Engine;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::config::{KeychainFormat, NodeConfig};
use super::encryption::LocalEncryption;
use super::error::{Error, Result};
use super::keychain::KeychainBackend;
use super::legacy_secrets::{LegacySnapshot, LegacySource};
use super::oauth::storage::RecoveryMove;
use super::secret_fields;
use super::secret_snapshot::SecretSnapshot;
use super::secret_transaction::{self, Attempt};
use super::storage::{Directory, MAX_CONFIG_BYTES, MIGRATION_NAME};

fn invalid() -> Error {
    Error::Config("node_migration_invalid: retain all storage and recovery files; restore the recorded generation/custody before resuming migration".into())
}
fn capacity_error() -> Error {
    Error::Config("node_migration_capacity_exceeded: the complete migration recovery plan exceeds its size bound; no target secrets were staged".into())
}
// Config projections contain encoded ciphertext/references, never clear values.
fn config_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
/// One random key per migration. Only its UUID-derived keychain name is public.
/// This object is not an immutable credential reference and is never collected
/// by ordinary object GC. No Debug/Serialize implementation exposes its bytes.
pub(crate) struct MigrationKey(Zeroizing<[u8; 32]>, Option<[u8; 32]>);
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyRecord {
    key: Zeroizing<String>,
    source_plan: [u8; 32],
}
impl MigrationKey {
    fn random() -> Self {
        let mut bytes = Zeroizing::new([0; 32]);
        rand::rngs::OsRng.fill_bytes(bytes.as_mut());
        Self(bytes, None)
    }
    fn name(id: Uuid) -> String {
        format!("object/v1/migration/{id}")
    }
    fn read(store: &KeychainBackend, id: Uuid) -> Result<Option<Self>> {
        store
            .get_optional(&Self::name(id))
            .map_err(|_| invalid())?
            .map(|encoded| {
                let encoded = Zeroizing::new(encoded);
                if encoded.len() > 512 {
                    return Err(invalid());
                }
                let record: KeyRecord = serde_json::from_str(&encoded).map_err(|_| invalid())?;
                if record.key.len() != 44 {
                    return Err(invalid());
                }
                let bytes = Zeroizing::new(
                    base64::engine::general_purpose::STANDARD
                        .decode(record.key.as_bytes())
                        .map_err(|_| invalid())?,
                );
                let mut key = Zeroizing::new([0; 32]);
                if bytes.len() != key.len() {
                    return Err(invalid());
                }
                key.copy_from_slice(&bytes);
                Ok(Self(key, Some(record.source_plan)))
            })
            .transpose()
    }
    fn create(&self, store: &KeychainBackend, id: Uuid, plan: &Journal) -> Result<()> {
        if Self::read(store, id)?.is_some() {
            return Err(invalid());
        }
        // The key and original complete plan commit in one OS-keychain write.
        // A crash before the active journal cannot reseal changed custody.
        let source_plan = self.plan_tag(plan)?;
        let record = KeyRecord {
            key: Zeroizing::new(
                base64::engine::general_purpose::STANDARD.encode(self.0.as_slice()),
            ),
            source_plan,
        };
        let encoded = Zeroizing::new(serde_json::to_string(&record).map_err(|_| invalid())?);
        store
            .set(&Self::name(id), &encoded)
            .map_err(|_| invalid())?;
        if Self::read(store, id)?
            .is_none_or(|actual| actual.0 != self.0 || actual.1 != Some(source_plan))
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub(crate) fn tag(&self, domain: &[u8], bytes: &[u8]) -> [u8; 32] {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(self.0.as_slice()).expect("HMAC accepts 32-byte keys");
        mac.update(b"nyxid-node-migration-v2");
        mac.update(&(domain.len() as u64).to_be_bytes());
        mac.update(domain);
        mac.update(bytes);
        mac.finalize().into_bytes().into()
    }
    fn plan_tag(&self, journal: &Journal) -> Result<[u8; 32]> {
        let mut source = journal.clone();
        source.attempt = None;
        source.prepared = None;
        source.cleaning = false;
        source.recovery = source
            .recovery
            .iter()
            .map(RecoveryMove::source_only)
            .collect();
        source.recovery.sort_by(|a, b| a.name().cmp(b.name()));
        source.retire.sort_by(|a, b| a.name.cmp(&b.name));
        let encoded = Zeroizing::new(serde_json::to_vec(&source).map_err(|_| invalid())?);
        Ok(self.tag(b"source-plan", &encoded))
    }
    fn values(&self, snapshot: &SecretSnapshot) -> Result<[u8; 32]> {
        snapshot.value_digest(&self.0)
    }
}

/// A reservation contains no secret-derived tags. Until Active is durable,
/// there are no target/source effects and no hashes authorized by this key.
/// Only this phase may create a missing key; Active always fails closed.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Boundary {
    id: Uuid,
    config_name: String,
    config: String,
    target: String,
    legacy_source: Option<LegacySource>,
}
impl Boundary {
    fn validate(
        &self,
        directory: &Directory,
        path: &Path,
        target: &str,
        store: &KeychainBackend,
    ) -> Result<NodeConfig> {
        if self.id.get_version_num() != 4
            || self.target != target
            || Some(self.config_name.as_str()) != path.file_name().and_then(OsStr::to_str)
            || directory.read(OsStr::new(&self.config_name), MAX_CONFIG_BYTES)?
                != self.config.as_bytes()
        {
            return Err(invalid());
        }
        let config = NodeConfig::from_bytes(self.config.as_bytes()).map_err(|_| invalid())?;
        validate_stored_projection(&config, self.config.as_bytes())?;
        if config.node.id != store.owner_id() {
            return Err(invalid());
        }
        Ok(config)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(
    tag = "phase",
    content = "plan",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum MigrationState {
    Reserved(Boundary),
    Active(Box<Journal>),
    /// Written only after all source retirement is durable. It authorizes only
    /// deleting this migration's key and marker, never publication/source GC.
    Retiring(Boundary),
}
impl MigrationState {
    fn read(directory: &Directory) -> Result<Option<Self>> {
        optional(directory, MIGRATION_NAME)?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| invalid()))
            .transpose()
    }
    fn persist(&self, directory: &Directory, replace: bool) -> Result<()> {
        let bytes = serde_json::to_vec(self).map_err(|_| invalid())?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(capacity_error());
        }
        directory.publish(OsStr::new(MIGRATION_NAME), &bytes, replace)
    }
}
fn finish_retirement(
    boundary: &Boundary,
    directory: &Directory,
    path: &Path,
    target: &str,
    store: &KeychainBackend,
    step: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let config = boundary.validate(directory, path, target, store)?;
    if boundary.legacy_source.is_some()
        || config.storage_backend != target
        || (target == "keychain" && config.keychain_format != Some(KeychainFormat::ImmutableV1))
    {
        return Err(invalid());
    }
    // The terminal marker has no secret tags left to validate. Re-read healthy
    // installed custody before idempotent key/marker removal, including resumes
    // after deletion. A missing ACTIVE key is never accepted through this path.
    SecretSnapshot::read_projection_in(directory, &config, path, Some(store), None, None)?;
    store
        .delete(&MigrationKey::name(boundary.id))
        .map_err(|_| invalid())?;
    step("key_deleted")?;
    if MigrationKey::read(store, boundary.id)?.is_some() {
        return Err(invalid());
    }
    directory.remove(OsStr::new(MIGRATION_NAME))
}
/// Ordinary serde readers tolerate unknown keys. Migration must not retire
/// custody after silently dropping a field that this binary cannot preserve.
fn validate_stored_projection(config: &NodeConfig, bytes: &[u8]) -> Result<()> {
    fn contained(stored: &toml::Value, known: &toml::Value) -> bool {
        match (stored, known) {
            (toml::Value::Table(stored), toml::Value::Table(known)) => {
                stored.iter().all(|(key, value)| {
                    known
                        .get(key)
                        .is_some_and(|expected| contained(value, expected))
                })
            }
            (toml::Value::Array(stored), toml::Value::Array(known)) => {
                stored.len() == known.len()
                    && stored
                        .iter()
                        .zip(known)
                        .all(|(value, expected)| contained(value, expected))
            }
            _ => stored == known,
        }
    }
    let stored: toml::Value = toml::from_str(std::str::from_utf8(bytes).map_err(|_| invalid())?)
        .map_err(|_| invalid())?;
    let known: toml::Value =
        toml::from_str(&toml::to_string(config).map_err(|_| invalid())?).map_err(|_| invalid())?;
    if !contained(&stored, &known) {
        return Err(Error::Config("node_migration_unmodelled_config: migration cannot preserve the complete stored configuration; retain it and use a compatible migration-capable CLI before retrying".into()));
    }
    Ok(())
}
fn optional(directory: &Directory, name: &str) -> Result<Option<Vec<u8>>> {
    match directory.read(OsStr::new(name), MAX_CONFIG_BYTES) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Retirement {
    file: bool,
    name: String,
    digest: [u8; 32],
}
impl Retirement {
    fn read(
        &self,
        directory: &Directory,
        keychain: &KeychainBackend,
    ) -> Result<Option<Zeroizing<Vec<u8>>>> {
        if self.file {
            if self.name != ".keyfile" {
                return Err(invalid());
            }
            Ok(optional(directory, &self.name)?.map(Zeroizing::new))
        } else {
            if self.name != "vault"
                && self.name != "auth_token"
                && self.name != "signing_secret"
                && !self.name.starts_with("cred/")
            {
                return Err(invalid());
            }
            Ok(keychain
                .get_optional(&self.name)
                .map_err(|_| invalid())?
                .map(|v| Zeroizing::new(v.into_bytes())))
        }
    }
    fn validate(
        &self,
        directory: &Directory,
        keychain: &KeychainBackend,
        cleaning: bool,
        key: &MigrationKey,
    ) -> Result<()> {
        match self.read(directory, keychain)? {
            Some(v) if key.tag(b"retirement", &v) == self.digest => Ok(()),
            None if cleaning => Ok(()),
            _ => Err(invalid()),
        }
    }
    fn remove(&self, directory: &Directory, keychain: &KeychainBackend) -> Result<()> {
        if self.file {
            if optional(directory, &self.name)?.is_some() {
                directory.remove(OsStr::new(&self.name))?;
            }
            Ok(())
        } else {
            keychain.delete(&self.name).map_err(|_| invalid())
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepared {
    config: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    id: Uuid,
    config_name: String,
    source: String,
    target: String,
    values: [u8; 32],
    legacy_source: Option<LegacySource>,
    attempt: Option<Uuid>,
    recovery: Vec<RecoveryMove>,
    retire: Vec<Retirement>,
    prepared: Option<Prepared>,
    cleaning: bool,
}
impl Journal {
    /// Reserve room for the *complete* durable plan before staging. A source
    /// that fits alone must not become permanently unresumable when its target
    /// projection and relocated recovery records are added to the journal.
    fn preflight_capacity(&self, snapshot: &SecretSnapshot) -> Result<()> {
        let mut projected = self.source()?;
        for (field, _) in secret_fields::fields(&projected)? {
            let encoded = snapshot
                .get(&field)?
                .map(|value| {
                    if self.target == "keychain" {
                        // UUIDs and reference encoding have a fixed serialized
                        // length. This placeholder never reaches a storage API.
                        Ok(format!("nyxid-key:v1:{}:{}", self.id, self.id))
                    } else {
                        let length = value
                            .len()
                            .checked_add(28)
                            .ok_or_else(invalid)?
                            .div_ceil(3)
                            .checked_mul(4)
                            .ok_or_else(invalid)?;
                        Ok::<_, Error>("A".repeat(length))
                    }
                })
                .transpose()?;
            field.replace(&mut projected, encoded)?;
        }
        projected.storage_backend = self.target.clone();
        projected.keychain_format =
            (self.target == "keychain").then_some(KeychainFormat::ImmutableV1);
        projected.storage_generation = Some(self.id);
        let config = toml::to_string_pretty(&projected).map_err(|_| invalid())?;
        if config.len() as u64 > MAX_CONFIG_BYTES {
            return Err(capacity_error());
        }
        let mut reserved = self.clone();
        reserved.prepared = Some(Prepared { config });
        reserved.attempt = (self.target == "keychain").then_some(self.id);
        // false serializes one byte longer than true.
        reserved.cleaning = false;
        for record in &mut reserved.recovery {
            record.reserve_target_capacity()?;
        }
        if serde_json::to_vec(&MigrationState::Active(Box::new(reserved)))
            .map_err(|_| invalid())?
            .len() as u64
            > MAX_CONFIG_BYTES
        {
            return Err(capacity_error());
        }
        Ok(())
    }
    #[cfg(test)]
    fn read(directory: &Directory) -> Result<Option<Self>> {
        match MigrationState::read(directory)? {
            Some(MigrationState::Active(journal)) => Ok(Some(*journal)),
            None => Ok(None),
            _ => Err(invalid()),
        }
    }
    fn persist(&self, directory: &Directory, replace: bool) -> Result<()> {
        MigrationState::Active(Box::new(self.clone())).persist(directory, replace)
    }
    fn source(&self) -> Result<NodeConfig> {
        NodeConfig::from_bytes(self.source.as_bytes()).map_err(|_| invalid())
    }
    fn validate(&self, path: &Path, target: &str) -> Result<()> {
        if self.version != 2
            || self.id.get_version_num() != 4
            || self.target != target
            || Some(self.config_name.as_str()) != path.file_name().and_then(OsStr::to_str)
            || self.recovery.len() > 4096
            || self.retire.len() > 16384
            || self
                .recovery
                .iter()
                .map(RecoveryMove::name)
                .collect::<BTreeSet<_>>()
                .len()
                != self.recovery.len()
            || (self.cleaning && self.prepared.is_none())
            || self.attempt.is_some_and(|id| id.get_version_num() != 4)
            || (target == "file" && self.attempt.is_some())
            || (target == "keychain" && self.prepared.is_some() && self.attempt.is_none())
        {
            return Err(invalid());
        }
        let source = self.source()?;
        validate_stored_projection(&source, self.source.as_bytes())?;
        let allowed: BTreeSet<String> = if source.storage_backend == "file" {
            [".keyfile".to_owned()].into_iter().collect()
        } else if source.keychain_format.is_none() {
            let mut keys: BTreeSet<_> = [
                "vault".to_owned(),
                "auth_token".to_owned(),
                "signing_secret".to_owned(),
            ]
            .into_iter()
            .collect();
            for (field, _) in secret_fields::fields(&source)? {
                if !matches!(
                    field,
                    super::secret_fields::SecretField::AuthToken
                        | super::secret_fields::SecretField::SigningSecret
                        | super::secret_fields::SecretField::PendingPrivate { .. }
                ) {
                    keys.insert(super::keychain::credential_key(&field.legacy_key()));
                }
            }
            keys
        } else {
            BTreeSet::new()
        };
        let mut seen = BTreeSet::new();
        for item in &self.retire {
            if item.file != (source.storage_backend == "file")
                || !allowed.contains(&item.name)
                || !seen.insert(&item.name)
            {
                return Err(invalid());
            }
        }
        if let Some(prepared) = &self.prepared {
            for record in &self.recovery {
                record.validate_plan(
                    config_digest(self.source.as_bytes()),
                    config_digest(prepared.config.as_bytes()),
                )?;
            }
            let target_config =
                NodeConfig::from_bytes(prepared.config.as_bytes()).map_err(|_| invalid())?;
            validate_stored_projection(&target_config, prepared.config.as_bytes())?;
            if secret_fields::fields(&source)? != secret_fields::fields(&target_config)? {
                return Err(invalid());
            }
            let mut expected = source.clone();
            for (field, _) in secret_fields::fields(&source)? {
                field.replace(
                    &mut expected,
                    field.encoded(&target_config)?.map(str::to_owned),
                )?;
            }
            expected.storage_backend = target_config.storage_backend.clone();
            expected.keychain_format = target_config.keychain_format;
            expected.storage_generation = target_config.storage_generation;
            if toml::to_string(&expected).map_err(|_| invalid())?
                != toml::to_string(&target_config).map_err(|_| invalid())?
            {
                return Err(invalid());
            }
            if source.node.id != target_config.node.id
                || target_config.storage_backend != target
                || target_config.storage_generation.is_none()
                || (target == "keychain"
                    && target_config.keychain_format != Some(KeychainFormat::ImmutableV1))
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

/// Output contains fixed states, validated backend names and counts only.
#[cfg(test)]
pub(crate) fn command(
    target: &str,
    config_path: Option<&str>,
    dry_run: bool,
    status: bool,
) -> Result<()> {
    command_with_cutover(
        target,
        config_path,
        dry_run,
        status,
        super::daemon::Cutover::default(),
    )
}

pub(crate) fn command_with_cutover(
    target: &str,
    config_path: Option<&str>,
    dry_run: bool,
    status: bool,
    cutover: super::daemon::Cutover<'_>,
) -> Result<()> {
    validate_target(target)?;
    let dir = super::config::resolve_config_dir(config_path);
    let path = dir.join("config.toml");
    let directory = Directory::open(&dir)?;
    let config = NodeConfig::load_in(&directory, &path)?;
    let keychain = KeychainBackend::new(&config.node.id);
    match (dry_run, status) {
        (false, true) => {
            let _lock = directory.writer_lock()?;
            match MigrationState::read(&directory)? {
                Some(MigrationState::Active(journal)) => {
                    journal.validate(&path, target)?;
                    let current = directory.read(OsStr::new("config.toml"), MAX_CONFIG_BYTES)?;
                    let state = if journal.cleaning {
                        "cleanup_pending"
                    } else if journal
                        .prepared
                        .as_ref()
                        .is_some_and(|p| current == p.config.as_bytes())
                    {
                        "installed_recovery_pending"
                    } else if journal.prepared.is_some() {
                        "prepared"
                    } else {
                        "preparing"
                    };
                    println!(
                        "Migration: {state}; recovery records: {}",
                        journal.recovery.len()
                    );
                }
                Some(MigrationState::Reserved(boundary)) => {
                    boundary.validate(&directory, &path, target, &keychain)?;
                    println!("Migration: key_reservation_pending");
                }
                Some(MigrationState::Retiring(boundary)) => {
                    boundary.validate(&directory, &path, target, &keychain)?;
                    println!("Migration: key_retirement_pending");
                }
                None => println!(
                    "Migration: idle; backend: {}",
                    checked_backend(&config.storage_backend)?
                ),
            }
            Ok(())
        }
        (true, false) => {
            let _lock = directory.writer_lock()?;
            directory.require_idle()?;
            validate_stored_projection(
                &config,
                &directory.read(OsStr::new("config.toml"), MAX_CONFIG_BYTES)?,
            )?;
            let snapshot = SecretSnapshot::read_projection_in(
                &directory,
                &config,
                &path,
                Some(&keychain),
                None,
                None,
            )?;
            let (recovery, _leases) = RecoveryMove::capture_all(
                &directory,
                &path,
                &config,
                &keychain,
                target == "file",
                &MigrationKey::random(),
            )?;
            check_capacity(&config, &snapshot, target)?;
            if config.storage_backend == "keychain" && config.keychain_format.is_none() {
                LegacySnapshot::retirement_keys(&config, &keychain)?;
            }
            println!(
                "Migration dry run: {} -> {target}; secret slots: {}; recovery records: {}; no target writes",
                checked_backend(&config.storage_backend)?,
                secret_fields::fields(&config)?.len(),
                recovery.len()
            );
            Ok(())
        }
        (false, false) => {
            let mut quiescence =
                super::daemon::require_migration_quiescence(&directory, &dir, cutover)?;
            quiescence.check()?;
            run(&directory, &path, target, &keychain, |_| quiescence.check())?;
            println!("Migration complete; backend: {target}. Restart the node explicitly.");
            Ok(())
        }
        (true, true) => Err(Error::Validation(
            "Choose either --dry-run or --status".into(),
        )),
    }
}
fn check_capacity(config: &NodeConfig, snapshot: &SecretSnapshot, target: &str) -> Result<()> {
    if target == "keychain" {
        let mut count = 0;
        for (field, _) in secret_fields::fields(config)? {
            if snapshot.get(&field)?.is_some() {
                count += 1;
            }
        }
        if count > secret_transaction::MAX_OBJECTS {
            return Err(invalid());
        }
    }
    Ok(())
}
fn checked_backend(value: &str) -> Result<&str> {
    validate_target(value)?;
    Ok(value)
}
fn validate_target(value: &str) -> Result<()> {
    if matches!(value, "file" | "keychain") {
        Ok(())
    } else {
        Err(Error::Validation(
            "Target must be 'keychain' or 'file'".into(),
        ))
    }
}

fn source_plan(
    directory: &Directory,
    path: &Path,
    target: &str,
    keychain: &KeychainBackend,
    config: &NodeConfig,
    id: Uuid,
    key: &MigrationKey,
) -> Result<(Journal, Vec<std::fs::File>)> {
    let source_bytes = directory.read(path.file_name().ok_or_else(invalid)?, MAX_CONFIG_BYTES)?;
    validate_stored_projection(config, &source_bytes)?;
    let snapshot =
        SecretSnapshot::read_projection_in(directory, config, path, Some(keychain), None, None)?;
    check_capacity(config, &snapshot, target)?;
    let (recovery, leases) =
        RecoveryMove::capture_all(directory, path, config, keychain, target == "file", key)?;
    let mut retire = Vec::new();
    if config.storage_backend == "file" {
        let bytes = Zeroizing::new(directory.read(OsStr::new(".keyfile"), 32)?);
        retire.push(Retirement {
            file: true,
            name: ".keyfile".into(),
            digest: key.tag(b"retirement", &bytes),
        });
    } else if config.keychain_format.is_none() {
        for name in LegacySnapshot::retirement_keys(config, keychain)? {
            if let Some(value) = keychain.get_optional(&name).map_err(|_| invalid())? {
                let value = Zeroizing::new(value);
                retire.push(Retirement {
                    file: false,
                    name,
                    digest: key.tag(b"retirement", value.as_bytes()),
                });
            }
        }
    }
    let journal = Journal {
        version: 2,
        id,
        config_name: path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(invalid)?
            .into(),
        source: String::from_utf8(source_bytes).map_err(|_| invalid())?,
        target: target.into(),
        values: key.values(&snapshot)?,
        legacy_source: snapshot.legacy_anchor().and_then(|v| v.source.clone()),
        attempt: None,
        recovery,
        retire,
        prepared: None,
        cleaning: false,
    };
    journal.validate(path, target)?;
    journal.preflight_capacity(&snapshot)?;
    Ok((journal, leases))
}

/// Failpoints model process loss at durable transitions; production uses a
/// no-op callback. No rollback runs on error or Drop.
fn run(
    directory: &Directory,
    path: &Path,
    target: &str,
    keychain: &KeychainBackend,
    mut step: impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    validate_target(target)?;
    let _registration = directory.registration_lock()?;
    let mut journal;
    let key;
    let _recovery_leases;
    {
        let _lock = directory.writer_lock()?;
        match MigrationState::read(directory)? {
            Some(MigrationState::Retiring(boundary)) => {
                return finish_retirement(&boundary, directory, path, target, keychain, &mut step);
            }
            Some(MigrationState::Reserved(boundary)) => {
                let config = boundary.validate(directory, path, target, keychain)?;
                let existing = MigrationKey::read(keychain, boundary.id)?;
                let create = existing.is_none();
                key = existing.unwrap_or_else(MigrationKey::random);
                let (planned, leases) = source_plan(
                    directory,
                    path,
                    target,
                    keychain,
                    &config,
                    boundary.id,
                    &key,
                )?;
                let plan_tag = key.plan_tag(&planned)?;
                if planned.legacy_source != boundary.legacy_source
                    || key.1.is_some_and(|expected| expected != plan_tag)
                {
                    return Err(invalid());
                }
                if create {
                    key.create(keychain, boundary.id, &planned)?;
                }
                step("key_created")?;
                planned.persist(directory, true)?;
                journal = planned;
                _recovery_leases = leases;
            }
            Some(MigrationState::Active(existing)) => {
                existing.validate(path, target)?;
                let source = existing.source()?;
                if keychain.owner_id() != source.node.id {
                    return Err(invalid());
                }
                key = MigrationKey::read(keychain, existing.id)?.ok_or_else(invalid)?;
                if key.1 != Some(key.plan_tag(&existing)?) {
                    return Err(invalid());
                }
                _recovery_leases = existing
                    .recovery
                    .iter()
                    .map(|r| r.hold(directory, path, &source.node.id))
                    .collect::<Result<Vec<_>>>()?;
                journal = *existing;
            }
            None => {
                let config = NodeConfig::load_in(directory, path)?;
                if keychain.owner_id() != config.node.id {
                    return Err(invalid());
                }
                if config.storage_backend == target
                    && (target == "file"
                        || config.keychain_format == Some(KeychainFormat::ImmutableV1))
                {
                    return Ok(());
                }
                let id = Uuid::new_v4();
                key = MigrationKey::random();
                let (planned, leases) =
                    source_plan(directory, path, target, keychain, &config, id, &key)?;
                let boundary = Boundary {
                    id,
                    config_name: planned.config_name.clone(),
                    config: planned.source.clone(),
                    target: target.into(),
                    legacy_source: planned.legacy_source.clone(),
                };
                // Establish absence before claiming this namespace. A key
                // found after the durable reservation belongs to this attempt;
                // one found before it must never be adopted or overwritten.
                if MigrationKey::read(keychain, id)?.is_some() {
                    return Err(invalid());
                }
                MigrationState::Reserved(boundary).persist(directory, false)?;
                step("key_reserved")?;
                key.create(keychain, id, &planned)?;
                step("key_created")?;
                planned.persist(directory, true)?;
                journal = planned;
                _recovery_leases = leases;
            }
        }
    }
    step("journal")?;
    let source = journal.source()?;
    if journal.prepared.is_none() {
        let snapshot;
        {
            let _lock = directory.writer_lock()?;
            if directory.read(OsStr::new(&journal.config_name), MAX_CONFIG_BYTES)?
                != journal.source.as_bytes()
            {
                return Err(invalid());
            }
            snapshot = SecretSnapshot::read_projection_in(
                directory,
                &source,
                path,
                Some(keychain),
                None,
                None,
            )?;
            if key.values(&snapshot)? != journal.values
                || snapshot.legacy_anchor().and_then(|v| v.source.clone()) != journal.legacy_source
            {
                return Err(invalid());
            }
            for item in &journal.retire {
                item.validate(directory, keychain, false, &key)?;
            }
            for record in &journal.recovery {
                record.validate_record(directory)?;
            }
        }
        let mut projected = source.clone();
        let attempt;
        if target == "keychain" {
            attempt = Some(match journal.attempt {
                Some(id) => Attempt::resume_in(
                    directory,
                    path,
                    id,
                    source.publication.snapshot(),
                    &source.node.id,
                )?,
                None => {
                    let attempt = Attempt::begin_in(
                        directory,
                        path,
                        source.publication.snapshot(),
                        &source.node.id,
                    )?;
                    journal.attempt = Some(attempt.id()?);
                    let _lock = directory.writer_lock()?;
                    journal.persist(directory, true)?;
                    attempt
                }
            });
            for (field, _) in secret_fields::fields(&source)? {
                let encoded = snapshot
                    .get(&field)?
                    .as_ref()
                    .map(|v| {
                        attempt.as_ref().ok_or_else(invalid)?.stage_migration(
                            field.clone(),
                            v,
                            keychain,
                        )
                    })
                    .transpose()?;
                field.replace(&mut projected, encoded)?;
                step("object")?;
            }
            projected.storage_backend = "keychain".into();
            projected.keychain_format = Some(KeychainFormat::ImmutableV1);
        } else {
            attempt = None;
            let file = {
                let _lock = directory.writer_lock()?;
                secret_transaction::check_observed(directory, path, source.publication.snapshot())?;
                LocalEncryption::initialize_in_locked(directory)?
            };
            step("file_key")?;
            projected = snapshot.project_file(&source, &file)?;
        }
        projected.storage_generation = Some(Uuid::new_v4());
        let target_bytes = toml::to_string_pretty(&projected).map_err(|_| invalid())?;
        {
            let _lock = directory.writer_lock()?;
            secret_transaction::check_observed(directory, path, source.publication.snapshot())?;
            verify_values(directory, path, &projected, keychain, journal.values, &key)?;
            for record in &mut journal.recovery {
                record.prepare(
                    directory,
                    keychain,
                    config_digest(journal.source.as_bytes()),
                    config_digest(target_bytes.as_bytes()),
                    &key,
                )?;
            }
            journal.prepared = Some(Prepared {
                config: target_bytes,
            });
            journal.persist(directory, true)?;
        }
        drop(attempt);
    }
    step("prepared")?;
    {
        let _lock = directory.writer_lock()?;
        let prepared = journal.prepared.as_ref().ok_or_else(invalid)?;
        let target_config = NodeConfig::from_bytes(prepared.config.as_bytes())?;
        let current = directory.read(OsStr::new(&journal.config_name), MAX_CONFIG_BYTES)?;
        let installed = current == prepared.config.as_bytes();
        if !installed && current != journal.source.as_bytes() {
            return Err(invalid());
        }
        verify_inventory(directory, &journal)?;
        verify_values(
            directory,
            path,
            &target_config,
            keychain,
            journal.values,
            &key,
        )?;
        if !installed {
            let fresh = SecretSnapshot::read_projection_in(
                directory,
                &source,
                path,
                Some(keychain),
                None,
                None,
            )?;
            if key.values(&fresh)? != journal.values
                || fresh.legacy_anchor().and_then(|v| v.source.clone()) != journal.legacy_source
            {
                return Err(invalid());
            }
            for item in &journal.retire {
                item.validate(directory, keychain, false, &key)?;
            }
            for record in &journal.recovery {
                record.stage(directory, keychain, &key)?;
            }
            step("envelopes")?;
            secret_transaction::validate_publication(
                directory,
                OsStr::new(&journal.config_name),
                Some(&current),
                &target_config,
            )?;
            // An installed-but-sync-uncertain result preserves the complete
            // journal, source and targets. Resume recognizes the exact bytes.
            directory.publish(
                OsStr::new(&journal.config_name),
                prepared.config.as_bytes(),
                true,
            )?;
        }
        step("installed")?;
        for record in &journal.recovery {
            record.validate_target(directory, keychain, &key)?;
            record.install(directory)?;
            step("recovery")?;
        }
        directory.sync()?;
        // Every target and every source artifact is validated before any
        // destructive retirement. Missing predecessors are legal only after
        // the durable cleaning transition, never during preparation.
        verify_values(
            directory,
            path,
            &target_config,
            keychain,
            journal.values,
            &key,
        )?;
        for record in &journal.recovery {
            record.validate_cleanup(directory, keychain, journal.cleaning, &key)?;
        }
        for item in &journal.retire {
            item.validate(directory, keychain, journal.cleaning, &key)?;
        }
        journal.cleaning = true;
        journal.persist(directory, true)?;
        step("cleaning")?;
        secret_transaction::collect_migration_in(directory, path, keychain)?;
        step("collected")?;
        for record in &journal.recovery {
            record.retire(directory, keychain)?;
            step("retire_recovery")?;
        }
        for item in &journal.retire {
            item.remove(directory, keychain)?;
            step("retire")?;
        }
        directory.sync()?;
        let boundary = Boundary {
            id: journal.id,
            config_name: journal.config_name.clone(),
            config: prepared.config.clone(),
            target: target.into(),
            legacy_source: None,
        };
        MigrationState::Retiring(boundary.clone()).persist(directory, true)?;
        step("key_retirement")?;
        finish_retirement(&boundary, directory, path, target, keychain, &mut step)?;
    }
    Ok(())
}
fn verify_inventory(directory: &Directory, journal: &Journal) -> Result<()> {
    let actual: BTreeSet<_> = directory
        .names()?
        .into_iter()
        .filter_map(|v| v.into_string().ok())
        .filter(|v| v.starts_with(".nyxid-storage-oauth-") && v.ends_with(".json"))
        .collect();
    if actual
        != journal
            .recovery
            .iter()
            .map(|r| r.name().to_owned())
            .collect()
    {
        return Err(invalid());
    }
    for record in &journal.recovery {
        record.validate_record(directory)?;
    }
    Ok(())
}
fn verify_values(
    directory: &Directory,
    path: &Path,
    config: &NodeConfig,
    keychain: &KeychainBackend,
    expected: [u8; 32],
    key: &MigrationKey,
) -> Result<()> {
    let snapshot =
        SecretSnapshot::read_projection_in(directory, config, path, Some(keychain), None, None)?;
    if key.values(&snapshot)? != expected {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn test_migrate(path: &Path, target: &str, store: &KeychainBackend) -> Result<()> {
    let directory = Directory::open(path.parent().ok_or_else(invalid)?)?;
    run(&directory, path, target, store, |_| Ok(()))
}

#[cfg(test)]
mod tests {
    use super::super::config::{CredentialConfig, PendingCryptoKeyConfig, SshKeyConfig};
    use super::super::secret_backend::SecretBackend;
    use super::super::secret_fields::{CredentialPart, SecretField};
    use super::*;

    pub(super) fn fixture(path: &Path) {
        let mut config =
            NodeConfig::new("wss://example.invalid".into(), "node".into(), "file".into());
        config.signing.shared_secret_encrypted = Some(String::new());
        let mut oauth = CredentialConfig::new_header("Authorization".into(), None, None);
        oauth.oauth_managed = true;
        oauth.oauth_token_url = Some("https://provider.invalid/token".into());
        config.credentials.insert("oauth".into(), oauth);
        config.credentials.insert(
            "query".into(),
            CredentialConfig::new_query_param("api_key".into(), None, None),
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
        let file = LocalEncryption::initialize(path.parent().unwrap()).unwrap();
        for (field, _) in secret_fields::fields(&config).unwrap() {
            let value = match &field {
                SecretField::SshPassphrase { .. } => None,
                SecretField::Credential {
                    part: CredentialPart::OAuthClientSecret,
                    ..
                } => Some(String::new()),
                _ => Some(format!(
                    "migration-SENTINEL:{}",
                    serde_json::to_string(&field).unwrap()
                )),
            };
            field
                .replace(&mut config, value.map(|v| file.encrypt(&v).unwrap()))
                .unwrap();
        }
        config.save(path).unwrap();
    }
    pub(super) fn values(directory: &Directory, path: &Path, store: &KeychainBackend) -> [u8; 32] {
        let _lock = directory.writer_lock().unwrap();
        let config = NodeConfig::load_in(directory, path).unwrap();
        SecretSnapshot::read_projection_in(directory, &config, path, Some(store), None, None)
            .unwrap()
            .value_digest(&[0; 32])
            .unwrap()
    }
    fn assert_no_plaintext(directory: &Directory) {
        for name in directory.names().unwrap() {
            let bytes = directory.read(&name, MAX_CONFIG_BYTES).unwrap();
            assert!(
                !bytes
                    .windows(b"migration-SENTINEL".len())
                    .any(|v| v == b"migration-SENTINEL")
            );
            assert!(
                !bytes
                    .windows(b"returned-refresh-SENTINEL".len())
                    .any(|v| v == b"returned-refresh-SENTINEL")
            );
        }
    }
    pub(super) fn forbidden_digests(
        directory: &Directory,
        path: &Path,
        store: &KeychainBackend,
    ) -> Vec<[u8; 32]> {
        fn digest(bytes: &[u8]) -> [u8; 32] {
            Sha256::digest(bytes).into()
        }
        let config = NodeConfig::load_in(directory, path).unwrap();
        let snapshot =
            SecretSnapshot::read_projection_in(directory, &config, path, Some(store), None, None)
                .unwrap();
        let mut forbidden = vec![
            digest(b"returned-access-SENTINEL"),
            digest(b"returned-refresh-SENTINEL"),
        ];
        let mut old = Sha256::new();
        old.update(b"nyxid-node-migration-values-v1");
        let fields: std::collections::BTreeMap<_, _> = secret_fields::fields(&config)
            .unwrap()
            .into_iter()
            .map(|(field, _)| {
                let value = snapshot.get(&field).unwrap();
                (field, value)
            })
            .collect();
        for (field, value) in fields {
            if let Some(value) = &value {
                forbidden.push(digest(value.as_bytes()));
            }
            let encoded = Zeroizing::new(serde_json::to_vec(&(&field, &value)).unwrap());
            old.update((encoded.len() as u64).to_be_bytes());
            old.update(&encoded);
        }
        forbidden.push(old.finalize().into());
        if let Some(key) = optional(directory, ".keyfile").unwrap() {
            forbidden.push(digest(&Zeroizing::new(key)));
        }
        for name in LegacySnapshot::retirement_keys(&config, store).unwrap_or_default() {
            if let Some(value) = store.get_optional(&name).unwrap() {
                forbidden.push(digest(Zeroizing::new(value).as_bytes()));
            }
        }
        forbidden
    }
    pub(super) fn assert_no_oracles(directory: &Directory, forbidden: &[[u8; 32]]) {
        assert_no_plaintext(directory);
        let Some(bytes) = optional(directory, MIGRATION_NAME).unwrap() else {
            return;
        };
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        fn contains(value: &serde_json::Value, needle: &serde_json::Value) -> bool {
            value == needle
                || match value {
                    serde_json::Value::Array(items) => items.iter().any(|v| contains(v, needle)),
                    serde_json::Value::Object(items) => items.values().any(|v| contains(v, needle)),
                    _ => false,
                }
        }
        for digest in forbidden {
            // Arrays are the actual digest encoding, not raw byte sequences.
            assert!(!contains(&parsed, &serde_json::to_value(digest).unwrap()));
            for encoded in [
                hex::encode(digest),
                base64::engine::general_purpose::STANDARD.encode(digest),
            ] {
                assert!(
                    !bytes
                        .windows(encoded.len())
                        .any(|v| v == encoded.as_bytes())
                );
            }
        }
    }
    #[test]
    fn unsealed_reservation_cannot_fall_back_from_vault_to_stale_individual_custody() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = NodeConfig::new(
            "wss://example.invalid".into(),
            "node".into(),
            "keychain".into(),
        );
        config.save(&path).unwrap();
        let directory = Directory::open(dir.path()).unwrap();
        let store = KeychainBackend::new_mock("node");
        let vault = r#"{"auth_token":"current-vault-SENTINEL"}"#;
        store.set("vault", vault).unwrap();
        store
            .set("auth_token", "stale-individual-SENTINEL")
            .unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(
            run(&directory, &path, "file", &store, |phase| {
                if phase == "key_reserved" {
                    Err(invalid())
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
        let Some(MigrationState::Reserved(boundary)) = MigrationState::read(&directory).unwrap()
        else {
            panic!("expected an unsealed reservation")
        };
        assert!(boundary.legacy_source == Some(LegacySource::Vault));
        store.delete("vault").unwrap();
        // A fresh legacy read *could* use this complete stale shadow. The
        // migration must preserve the already selected vault instead.
        assert!(
            LegacySnapshot::read(&config, &store).unwrap().source == Some(LegacySource::Individual)
        );
        assert!(test_migrate(&path, "file", &store).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!dir.path().join(".keyfile").exists());
        assert!(
            store
                .get_optional(&MigrationKey::name(boundary.id))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store.get("auth_token").unwrap(),
            "stale-individual-SENTINEL"
        );
        assert!(dir.path().join(MIGRATION_NAME).exists());
        store.set("vault", vault).unwrap();
        test_migrate(&path, "file", &store).unwrap();
        let installed = NodeConfig::load(&path).unwrap();
        let snapshot = SecretSnapshot::read_projection_in(
            &directory,
            &installed,
            &path,
            Some(&store),
            None,
            None,
        )
        .unwrap();
        assert!(
            snapshot
                .get(&SecretField::AuthToken)
                .unwrap()
                .unwrap()
                .as_str()
                == "current-vault-SENTINEL"
        );
    }
    #[test]
    fn key_creation_atomically_pins_source_plan_before_the_active_journal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = NodeConfig::new(
            "wss://example.invalid".into(),
            "node".into(),
            "keychain".into(),
        );
        config.save(&path).unwrap();
        let directory = Directory::open(dir.path()).unwrap();
        let store = KeychainBackend::new_mock("node");
        let original = r#"{"auth_token":"original-vault-SENTINEL"}"#;
        store.set("vault", original).unwrap();
        let config_before = std::fs::read(&path).unwrap();
        assert!(
            run(&directory, &path, "file", &store, |phase| {
                if phase == "key_created" {
                    Err(invalid())
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
        let Some(MigrationState::Reserved(boundary)) = MigrationState::read(&directory).unwrap()
        else {
            panic!("active journal must not exist yet")
        };
        let name = MigrationKey::name(boundary.id);
        let key_before = Zeroizing::new(store.get(&name).unwrap());
        for modified in [
            r#"{"auth_token":"changed-vault-SENTINEL"}"#,
            "{\n  \"auth_token\": \"original-vault-SENTINEL\"\n}",
        ] {
            store.set("vault", modified).unwrap();
            assert!(test_migrate(&path, "file", &store).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), config_before);
            assert!(!dir.path().join(".keyfile").exists());
            assert!(Zeroizing::new(store.get(&name).unwrap()).as_str() == key_before.as_str());
            assert!(dir.path().join(MIGRATION_NAME).exists());
        }
        store.set("vault", original).unwrap();
        test_migrate(&path, "file", &store).unwrap();
        assert!(store.get_optional(&name).unwrap().is_none());
        let installed = NodeConfig::load(&path).unwrap();
        let snapshot = SecretSnapshot::read_projection_in(
            &directory,
            &installed,
            &path,
            Some(&store),
            None,
            None,
        )
        .unwrap();
        assert!(
            snapshot
                .get(&SecretField::AuthToken)
                .unwrap()
                .unwrap()
                .as_str()
                == "original-vault-SENTINEL"
        );
    }
    #[test]
    fn unavailable_migration_key_custody_refuses_before_marker_or_source_effects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fixture(&path);
        let directory = Directory::open(dir.path()).unwrap();
        drop(directory.registration_lock().unwrap());
        let entries = directory.names().unwrap();
        let config = std::fs::read(&path).unwrap();
        let key = Zeroizing::new(directory.read(OsStr::new(".keyfile"), 32).unwrap());
        let store = KeychainBackend::new_failing_mock("node", "custody-error-SENTINEL");
        let error = test_migrate(&path, "keychain", &store)
            .unwrap_err()
            .to_string();
        assert!(error.contains("node_migration_invalid"));
        assert!(!error.contains("SENTINEL"));
        assert_eq!(std::fs::read(&path).unwrap(), config);
        assert!(directory.read(OsStr::new(".keyfile"), 32).unwrap() == key.as_slice());
        assert_eq!(directory.names().unwrap(), entries);
        assert!(directory.require_idle().is_ok());
    }
    #[test]
    fn missing_or_wrong_active_key_refuses_without_publication_or_retirement() {
        for target in ["file", "keychain"] {
            for phase in ["journal", "prepared", "cleaning"] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("config.toml");
                fixture(&path);
                let directory = Directory::open(dir.path()).unwrap();
                let store = KeychainBackend::new_mock("node");
                if target == "file" {
                    test_migrate(&path, "keychain", &store).unwrap();
                }
                assert!(
                    run(&directory, &path, target, &store, |s| if s == phase {
                        Err(invalid())
                    } else {
                        Ok(())
                    })
                    .is_err()
                );
                let journal = Journal::read(&directory).unwrap().unwrap();
                let name = MigrationKey::name(journal.id);
                let original = Zeroizing::new(store.get(&name).unwrap());
                let before = directory
                    .names()
                    .unwrap()
                    .into_iter()
                    .map(|n| {
                        let bytes = Zeroizing::new(directory.read(&n, MAX_CONFIG_BYTES).unwrap());
                        (n, bytes)
                    })
                    .collect::<Vec<_>>();
                let mut wrong: KeyRecord = serde_json::from_str(&original).unwrap();
                wrong.key =
                    Zeroizing::new(base64::engine::general_purpose::STANDARD.encode([42; 32]));
                for replacement in [
                    None,
                    Some(Zeroizing::new(serde_json::to_string(&wrong).unwrap())),
                ] {
                    store.delete(&name).unwrap();
                    if let Some(value) = replacement {
                        store.set(&name, &value).unwrap();
                    }
                    let error = test_migrate(&path, target, &store).unwrap_err().to_string();
                    assert!(error.contains("node_migration_invalid"));
                    assert!(!error.contains("SENTINEL"));
                    for (n, bytes) in &before {
                        let actual = Zeroizing::new(directory.read(n, MAX_CONFIG_BYTES).unwrap());
                        assert!(
                            actual.as_slice() == bytes.as_slice(),
                            "retained artifact changed"
                        );
                    }
                    assert_eq!(directory.names().unwrap().len(), before.len());
                }
                store.set(&name, &original).unwrap();
                test_migrate(&path, target, &store).unwrap();
                assert!(store.get_optional(&name).unwrap().is_none());
            }
        }
    }
    #[test]
    fn terminal_retirement_requires_healthy_exact_target_and_only_removes_its_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fixture(&path);
        let directory = Directory::open(dir.path()).unwrap();
        let store = KeychainBackend::new_mock("node");
        test_migrate(&path, "keychain", &store).unwrap();
        assert!(
            run(&directory, &path, "file", &store, |phase| {
                if phase == "key_retirement" {
                    Err(invalid())
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
        let Some(MigrationState::Retiring(boundary)) = MigrationState::read(&directory).unwrap()
        else {
            panic!("terminal state must be durable before key deletion")
        };
        let key = Zeroizing::new(directory.read(OsStr::new(".keyfile"), 32).unwrap());
        let key_name = MigrationKey::name(boundary.id);
        store.set("vault", "unowned-late-SENTINEL").unwrap();
        directory.remove(OsStr::new(".keyfile")).unwrap();
        assert!(test_migrate(&path, "file", &store).is_err());
        assert!(store.get_optional(&key_name).unwrap().is_some());
        assert!(dir.path().join(MIGRATION_NAME).exists());
        directory
            .publish(OsStr::new(".keyfile"), &key, false)
            .unwrap();
        let installed = std::fs::read(&path).unwrap();
        let mut changed = NodeConfig::load(&path).unwrap();
        changed.server.url = "wss://changed.invalid".into();
        directory
            .publish(
                OsStr::new("config.toml"),
                toml::to_string(&changed).unwrap().as_bytes(),
                true,
            )
            .unwrap();
        assert!(test_migrate(&path, "file", &store).is_err());
        assert!(store.get_optional(&key_name).unwrap().is_some());
        directory
            .publish(OsStr::new("config.toml"), &installed, true)
            .unwrap();
        test_migrate(&path, "file", &store).unwrap();
        assert!(store.get_optional(&key_name).unwrap().is_none());
        assert_eq!(store.get("vault").unwrap(), "unowned-late-SENTINEL");
        assert_eq!(std::fs::read(&path).unwrap(), installed);
        assert!(directory.require_idle().is_ok());
    }
    #[test]
    fn each_migration_uses_a_distinct_custody_key_and_retires_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fixture(&path);
        let directory = Directory::open(dir.path()).unwrap();
        let store = KeychainBackend::new_mock("node");
        let mut last = None;
        for target in ["keychain", "file", "keychain"] {
            let forbidden = forbidden_digests(&directory, &path, &store);
            assert!(
                run(&directory, &path, target, &store, |phase| {
                    if phase == "journal" {
                        Err(invalid())
                    } else {
                        Ok(())
                    }
                })
                .is_err()
            );
            let journal = Journal::read(&directory).unwrap().unwrap();
            let key = MigrationKey::read(&store, journal.id).unwrap().unwrap();
            let encoded =
                Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(key.0.as_slice()));
            let bytes = optional(&directory, MIGRATION_NAME).unwrap().unwrap();
            assert!(
                !bytes
                    .windows(encoded.len())
                    .any(|v| v == encoded.as_bytes())
            );
            assert!(!bytes.windows(32).any(|v| v == key.0.as_slice()));
            assert_no_oracles(&directory, &forbidden);
            if let Some((id, previous, tag)) = last {
                assert_ne!(journal.id, id);
                assert!(key.0 != previous);
                assert_ne!(journal.values, tag);
            }
            last = Some((journal.id, key.0, journal.values));
            test_migrate(&path, target, &store).unwrap();
            assert!(
                store
                    .get_optional(&MigrationKey::name(journal.id))
                    .unwrap()
                    .is_none()
            );
        }
    }
    fn returned(path: &Path, store: &KeychainBackend, stale: bool, unknown: bool) {
        let config = NodeConfig::load(path).unwrap();
        let reader =
            SecretBackend::reader_with_keychain(&config, path.parent().unwrap(), store.clone())
                .unwrap();
        let mut rotation = super::super::oauth::storage::Rotation::open(&reader, "oauth").unwrap();
        rotation.dispatch().unwrap();
        if !unknown {
            let token = serde_json::from_value(serde_json::json!({"access_token":"returned-access-SENTINEL", "refresh_token":"returned-refresh-SENTINEL", "expires_in":3600})).unwrap();
            rotation
                .retain(super::super::oauth::storage::Plan::Refresh, token)
                .unwrap();
        }
        drop(rotation);
        if stale {
            let mut config = NodeConfig::load(path).unwrap();
            config.server.url = "wss://changed.invalid".into();
            config.save(path).unwrap();
        }
    }
    fn recover(path: &Path, store: &KeychainBackend) -> Result<()> {
        let config = NodeConfig::load(path)?;
        let reader =
            SecretBackend::reader_with_keychain(&config, path.parent().unwrap(), store.clone())?;
        super::super::oauth::storage::recover_pending(&reader)
    }
    #[test]
    fn complete_both_directions_preserve_all_classes_and_optional_presence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fixture(&path);
        let directory = Directory::open(dir.path()).unwrap();
        let store = KeychainBackend::new_mock("node");
        let original = values(&directory, &path, &store);
        for target in ["keychain", "file", "keychain", "file"] {
            test_migrate(&path, target, &store).unwrap();
            assert!(values(&directory, &path, &store) == original);
            assert_eq!(NodeConfig::load(&path).unwrap().storage_backend, target);
            assert_eq!(dir.path().join(".keyfile").exists(), target == "file");
            assert!(directory.require_idle().is_ok());
            assert_no_plaintext(&directory);
        }
    }
    #[test]
    fn resume_after_every_durable_transition_in_both_directions() {
        for target in ["keychain", "file"] {
            let total = {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("config.toml");
                fixture(&path);
                let store = KeychainBackend::new_mock("node");
                if target == "file" {
                    test_migrate(&path, "keychain", &store).unwrap();
                }
                returned(&path, &store, false, false);
                let mut total = 0;
                run(
                    &Directory::open(dir.path()).unwrap(),
                    &path,
                    target,
                    &store,
                    |_| {
                        total += 1;
                        Ok(())
                    },
                )
                .unwrap();
                total
            };
            assert!(total > 0);
            for fail_at in 1..=total {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("config.toml");
                fixture(&path);
                let directory = Directory::open(dir.path()).unwrap();
                let store = KeychainBackend::new_mock("node");
                if target == "file" {
                    test_migrate(&path, "keychain", &store).unwrap();
                }
                returned(&path, &store, false, false);
                let original = values(&directory, &path, &store);
                let forbidden = forbidden_digests(&directory, &path, &store);
                let mut events = 0;
                let result = run(&directory, &path, target, &store, |_| {
                    events += 1;
                    if events == fail_at {
                        Err(invalid())
                    } else {
                        Ok(())
                    }
                });
                assert!(
                    result.is_err(),
                    "transition {fail_at}/{total} was not exercised"
                );
                assert_eq!(events, fail_at);
                assert!(directory.require_idle().is_err());
                let config = NodeConfig::load(&path).unwrap();
                assert!(
                    SecretBackend::reader_with_keychain(&config, dir.path(), store.clone())
                        .is_err()
                );
                assert!(config.save(&path).is_err());
                assert!(secret_transaction::collect_all_in(&directory, &path, &store).is_err());
                assert_no_oracles(&directory, &forbidden);
                let id = match MigrationState::read(&directory).unwrap().unwrap() {
                    MigrationState::Active(j) => j.id,
                    MigrationState::Reserved(b) | MigrationState::Retiring(b) => b.id,
                };
                test_migrate(&path, target, &store).unwrap();
                assert!(
                    store
                        .get_optional(&MigrationKey::name(id))
                        .unwrap()
                        .is_none()
                );
                assert!(values(&directory, &path, &store) == original);
                recover(&path, &store).unwrap();
                let config = NodeConfig::load(&path).unwrap();
                let reader =
                    SecretBackend::reader_with_keychain(&config, dir.path(), store.clone())
                        .unwrap();
                let value = reader
                    .require_field(
                        &config,
                        &SecretField::Credential {
                            slug: "oauth".into(),
                            part: CredentialPart::OAuthRefresh,
                        },
                    )
                    .unwrap();
                assert!(value.as_str() == "returned-refresh-SENTINEL");
                assert_no_plaintext(&directory);
            }
        }
    }
    #[test]
    fn stale_returns_refuse_before_staging_and_unknown_outcomes_remain_blocked() {
        for (stale, unknown) in [(true, false), (false, true)] {
            for target in ["file", "keychain"] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("config.toml");
                fixture(&path);
                let store = KeychainBackend::new_mock("node");
                if target == "file" {
                    test_migrate(&path, "keychain", &store).unwrap();
                }
                let original = std::fs::read(&path).unwrap();
                returned(&path, &store, stale, unknown);
                let before = std::fs::read(&path).unwrap();
                let directory = Directory::open(dir.path()).unwrap();
                drop(directory.registration_lock().unwrap());
                let entries = directory.names().unwrap();
                if stale {
                    let error = test_migrate(&path, target, &store).unwrap_err().to_string();
                    assert!(error.contains("node_migration_oauth_recovery_required"));
                    assert!(!error.contains("SENTINEL"));
                    assert_eq!(std::fs::read(&path).unwrap(), before);
                    assert_eq!(directory.names().unwrap(), entries);
                    assert!(!dir.path().join(MIGRATION_NAME).exists());
                } else {
                    test_migrate(&path, target, &store).unwrap();
                }
                let error = recover(&path, &store).unwrap_err().to_string();
                assert!(error.contains("oauth_recovery_pending"));
                assert!(!error.contains("SENTINEL"));
                if stale {
                    directory
                        .publish(OsStr::new("config.toml"), &original, true)
                        .unwrap();
                    recover(&path, &store).unwrap();
                    test_migrate(&path, target, &store).unwrap();
                    // The retained return remains recoverable under original
                    // authority; migration then preserves the recovered value.
                    let config = NodeConfig::load(&path).unwrap();
                    let reader =
                        SecretBackend::reader_with_keychain(&config, dir.path(), store.clone())
                            .unwrap();
                    assert!(
                        reader
                            .require_field(
                                &config,
                                &SecretField::Credential {
                                    slug: "oauth".into(),
                                    part: CredentialPart::OAuthRefresh,
                                }
                            )
                            .unwrap()
                            .as_str()
                            == "returned-refresh-SENTINEL"
                    );
                }
                assert_no_plaintext(&directory);
            }
        }
    }
    #[test]
    fn late_invalid_source_blocks_all_retirement_and_can_resume_after_repair() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fixture(&path);
        let directory = Directory::open(dir.path()).unwrap();
        let store = KeychainBackend::new_mock("node");
        let key = Zeroizing::new(std::fs::read(dir.path().join(".keyfile")).unwrap());
        assert!(
            run(&directory, &path, "keychain", &store, |state| {
                if state == "installed" {
                    Err(invalid())
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
        directory
            .publish(OsStr::new(".keyfile"), &[7; 32], true)
            .unwrap();
        assert!(test_migrate(&path, "keychain", &store).is_err());
        assert!(dir.path().join(MIGRATION_NAME).exists());
        directory
            .publish(OsStr::new(".keyfile"), &key, true)
            .unwrap();
        test_migrate(&path, "keychain", &store).unwrap();
    }
    #[test]
    fn active_oauth_lease_blocks_migration_before_marker_or_target_effects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fixture(&path);
        let store = KeychainBackend::new_mock("node");
        let config = NodeConfig::load(&path).unwrap();
        let reader =
            SecretBackend::reader_with_keychain(&config, dir.path(), store.clone()).unwrap();
        let mut rotation = super::super::oauth::storage::Rotation::open(&reader, "oauth").unwrap();
        rotation.dispatch().unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(test_migrate(&path, "keychain", &store).is_err());
        assert!(!dir.path().join(MIGRATION_NAME).exists());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    #[test]
    fn changed_generation_or_target_refuses_resume_without_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fixture(&path);
        let directory = Directory::open(dir.path()).unwrap();
        let store = KeychainBackend::new_mock("node");
        let before = std::fs::read(&path).unwrap();
        assert!(run(&directory, &path, "keychain", &store, |_| Err(invalid())).is_err());
        assert!(test_migrate(&path, "file", &store).is_err());
        directory
            .publish(OsStr::new("config.toml"), b"corrupt", true)
            .unwrap();
        assert!(test_migrate(&path, "keychain", &store).is_err());
        assert!(dir.path().join(".keyfile").exists());
        assert!(dir.path().join(MIGRATION_NAME).exists());
        directory
            .publish(OsStr::new("config.toml"), &before, true)
            .unwrap();
        test_migrate(&path, "keychain", &store).unwrap();
    }
    #[test]
    fn real_process_exit_resumes_without_an_in_memory_key_service() {
        for target in ["keychain", "file"] {
            for phase in [
                "key_reserved",
                "key_created",
                "key_retirement",
                "key_deleted",
                "journal",
                "object",
                "object-put",
                "file_key",
                "prepared",
                "envelopes",
                "envelope-put",
                "installed",
                "recovery",
                "cleaning",
                "retire_recovery",
                "retire",
                "collected",
            ] {
                if (target == "keychain" && phase == "file_key")
                    || (target == "file" && matches!(phase, "object" | "object-put"))
                {
                    continue;
                }
                let dir = tempfile::tempdir().unwrap();
                let keys = tempfile::tempdir().unwrap();
                let path = dir.path().join("config.toml");
                fixture(&path);
                let store = KeychainBackend::new_disk_mock("node", keys.path());
                if target == "file" {
                    test_migrate(&path, "keychain", &store).unwrap();
                }
                returned(&path, &store, false, false);
                let mut child = std::process::Command::new(std::env::current_exe().unwrap());
                child
                    .args([
                        "--exact",
                        "node::migration::tests::migration_process_child",
                        "--nocapture",
                    ])
                    .env("NYXID_TEST_MIGRATION_DIR", dir.path())
                    .env("NYXID_TEST_MIGRATION_KEYS", keys.path())
                    .env("NYXID_TEST_MIGRATION_TARGET", target)
                    .env("NYXID_TEST_MIGRATION_PHASE", phase);
                if phase == "object-put" {
                    child.env("NYXID_TEST_SECRET_EXIT_AFTER_PUT", "1");
                }
                if phase == "envelope-put" {
                    child.env("NYXID_TEST_MIGRATION_EXIT_AFTER_ENVELOPE_PUT", "1");
                }
                if target == "file" && phase == "retire" {
                    // Immutable predecessors retire through object GC. Exit
                    // after deletion but before its journal acknowledgement.
                    child.env("NYXID_TEST_SECRET_EXIT_AFTER_DELETE", "1");
                }
                let result = child.output().unwrap();
                assert!(
                    result.status.success(),
                    "child failed: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert!(
                    dir.path().join(MIGRATION_NAME).exists(),
                    "{target}/{phase} was not exercised"
                );
                let installed_key = if target == "file" {
                    std::fs::read(dir.path().join(".keyfile"))
                        .ok()
                        .map(Zeroizing::new)
                } else {
                    None
                };
                test_migrate(&path, target, &store).unwrap();
                if let Some(key) = installed_key {
                    assert!(std::fs::read(dir.path().join(".keyfile")).unwrap() == key.as_slice());
                }
                recover(&path, &store).unwrap();
                assert_no_plaintext(&Directory::open(dir.path()).unwrap());
            }
        }
    }
    #[test]
    fn already_installed_oauth_return_migrates_and_finishes_without_republication() {
        for target in ["keychain", "file"] {
            let dir = tempfile::tempdir().unwrap();
            let keys = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            fixture(&path);
            let store = KeychainBackend::new_disk_mock("node", keys.path());
            if target == "file" {
                test_migrate(&path, "keychain", &store).unwrap();
            }
            returned(&path, &store, false, false);
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "node::migration::tests::migration_process_child",
                    "--nocapture",
                ])
                .env("NYXID_TEST_MIGRATION_DIR", dir.path())
                .env("NYXID_TEST_MIGRATION_KEYS", keys.path())
                .env("NYXID_TEST_MIGRATION_TARGET", target)
                .env("NYXID_TEST_MIGRATION_PHASE", "oauth-install")
                .env("NYXID_TEST_OAUTH_EXIT_AFTER_PUBLICATION", "1")
                .output()
                .unwrap();
            assert!(result.status.success());
            let directory = Directory::open(dir.path()).unwrap();
            let expected = values(&directory, &path, &store);
            test_migrate(&path, target, &store).unwrap();
            let installed = std::fs::read(&path).unwrap();
            recover(&path, &store).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), installed);
            assert!(values(&directory, &path, &store) == expected);
        }
    }

    /// Explicit native probe, invoked separately with a disposable directory.
    /// Normal test discovery never contacts the host keychain/service manager.
    #[test]
    fn os_custody_process_child() {
        let Some(root) = std::env::var_os("NYXID_TEST_NODE_OS_CUSTODY_DIR") else {
            return;
        };
        let root = Path::new(&root);
        assert!(
            std::fs::read_dir(root).unwrap().next().is_none(),
            "native probe requires a fresh disposable directory; retain existing recovery data"
        );
        let path = root.join("config.toml");
        let id = format!("nyxid-custody-probe-{}", Uuid::new_v4());
        fixture(&path);
        let mut config = NodeConfig::load(&path).unwrap();
        config.node.id = id.clone();
        config.save(&path).unwrap();
        let directory = Directory::open(root).unwrap();
        let store = KeychainBackend::new(&id);
        let before = values(&directory, &path, &store);
        for target in ["keychain", "file"] {
            let result = command_with_cutover(
                target,
                root.to_str(),
                false,
                false,
                super::super::daemon::Cutover {
                    profile: Some(&id),
                    confirm_legacy_cutover: true,
                },
            );
            if let Err(error) = result {
                // Fixed errors only; never print the OS provider response or
                // node/key identifiers. Keep the caller-owned directory intact.
                assert!(!error.to_string().contains("SENTINEL"));
                let reason = match &error {
                    Error::Keychain(_) => "key_service",
                    Error::Config(message) if message.starts_with("node_migration_") => {
                        "native_quiescence"
                    }
                    Error::Config(_) => "custody_or_projection",
                    _ => "storage",
                };
                panic!(
                    "native custody/migration probe refused ({reason}); retain its disposable recovery directory"
                );
            }
            assert!(values(&directory, &path, &store) == before);
        }
        assert!(directory.require_idle().is_ok());
        assert!(store.get_optional("vault").unwrap().is_none());
        assert!(store.get_optional("auth_token").unwrap().is_none());
        assert!(store.get_optional("signing_secret").unwrap().is_none());
    }

    #[test]
    fn migration_process_child() {
        let Some(dir) = std::env::var_os("NYXID_TEST_MIGRATION_DIR") else {
            return;
        };
        let keys = std::env::var_os("NYXID_TEST_MIGRATION_KEYS").unwrap();
        let target = std::env::var("NYXID_TEST_MIGRATION_TARGET").unwrap();
        let phase = std::env::var("NYXID_TEST_MIGRATION_PHASE").unwrap();
        let directory = Directory::open(Path::new(&dir)).unwrap();
        let path = Path::new(&dir).join("config.toml");
        let store = KeychainBackend::new_disk_mock("node", Path::new(&keys));
        if matches!(phase.as_str(), "status" | "dry-run") {
            command(
                &target,
                Path::new(&dir).to_str(),
                phase == "dry-run",
                phase == "status",
            )
            .unwrap();
            return;
        }
        if phase == "oauth-install" {
            recover(&path, &store).unwrap();
            panic!("OAuth publication crash was not exercised");
        }
        run(&directory, &path, &target, &store, |state| {
            if state == phase {
                std::process::exit(0);
            }
            Ok(())
        })
        .unwrap();
        panic!("requested crash phase was not reached");
    }
    #[test]
    fn command_refuses_unproven_quiescence_without_key_init_or_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fixture(&path);
        let before = std::fs::read(&path).unwrap();
        let error = command("keychain", dir.path().to_str(), false, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("node_migration_quiescence_required"));
        assert!(!error.contains("SENTINEL"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!dir.path().join(MIGRATION_NAME).exists());
        command("file", dir.path().to_str(), false, true).unwrap();
        command("invalid", dir.path().to_str(), false, true).unwrap_err();
    }
}

#[cfg(test)]
mod additional_tests {
    use super::super::secret_fields::SecretField;
    use super::*;

    #[test]
    fn unknown_stored_fields_refuse_before_reads_are_used_to_stage_or_retire() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        super::tests::fixture(&path);
        let directory = Directory::open(dir.path()).unwrap();
        drop(directory.registration_lock().unwrap());
        let mut bytes = b"future_storage_secret = \"future-secret-SENTINEL\"\n".to_vec();
        bytes.extend_from_slice(&std::fs::read(&path).unwrap());
        directory
            .publish(OsStr::new("config.toml"), &bytes, true)
            .unwrap();
        // Ordinary serde decoding tolerates this field, which must not make
        // its silent removal safe during a destructive custody switch.
        NodeConfig::load(&path).unwrap();
        let entries = directory.names().unwrap();
        let store = KeychainBackend::new_mock("node");
        let error = test_migrate(&path, "keychain", &store)
            .unwrap_err()
            .to_string();
        assert!(error.contains("node_migration_unmodelled_config"));
        assert!(!error.contains("SENTINEL"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(directory.names().unwrap(), entries);
        assert!(command("keychain", dir.path().to_str(), true, false).is_err());
        assert!(!dir.path().join(MIGRATION_NAME).exists());
        assert!(dir.path().join(".keyfile").exists());
    }

    #[test]
    fn complete_plan_capacity_refuses_before_target_staging_or_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        super::tests::fixture(&path);
        let mut config = NodeConfig::load(&path).unwrap();
        config.credentials.get_mut("query").unwrap().target_url = Some(format!(
            "https://example.invalid/{}",
            "x".repeat(MAX_CONFIG_BYTES as usize / 2 + 1024)
        ));
        config.save(&path).unwrap();
        let directory = Directory::open(dir.path()).unwrap();
        drop(directory.registration_lock().unwrap());
        let entries = directory.names().unwrap();
        let before = std::fs::read(&path).unwrap();
        let store = KeychainBackend::new_mock("node");
        let error = test_migrate(&path, "keychain", &store)
            .unwrap_err()
            .to_string();
        assert!(error.contains("node_migration_capacity_exceeded"));
        assert!(!error.contains("SENTINEL"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(directory.names().unwrap(), entries);
        assert!(!dir.path().join(MIGRATION_NAME).exists());
        assert!(dir.path().join(".keyfile").exists());
    }

    #[test]
    fn legacy_vault_converts_in_place_with_pending_and_every_optional_class() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        super::tests::fixture(&path);
        let directory = Directory::open(dir.path()).unwrap();
        let store = KeychainBackend::new_mock("node");
        let mut config = NodeConfig::load(&path).unwrap();
        let snapshot =
            SecretSnapshot::read_in(&directory, &config, &path, None, None, None).unwrap();
        let expected = snapshot.value_digest(&[0; 32]).unwrap();
        let mut credentials = std::collections::BTreeMap::<String, Zeroizing<String>>::new();
        let mut pending = std::collections::BTreeMap::<String, Zeroizing<String>>::new();
        for (field, _) in secret_fields::fields(&config).unwrap() {
            let value = snapshot.get(&field).unwrap();
            match &field {
                SecretField::AuthToken | SecretField::SigningSecret => {}
                SecretField::PendingPrivate { pending_id } => {
                    if let Some(v) = value {
                        pending.insert(pending_id.clone(), v);
                    }
                }
                _ => {
                    if let Some(v) = value {
                        credentials.insert(field.legacy_key(), v);
                    }
                }
            }
            field
                .replace(
                    &mut config,
                    if matches!(field, SecretField::AuthToken | SecretField::SigningSecret) {
                        Some(String::new())
                    } else {
                        None
                    },
                )
                .unwrap();
        }
        #[derive(Serialize)]
        struct Vault<'a> {
            auth_token: Option<Zeroizing<String>>,
            signing_secret: Option<Zeroizing<String>>,
            credentials: &'a std::collections::BTreeMap<String, Zeroizing<String>>,
            pending_crypto_keys: &'a std::collections::BTreeMap<String, Zeroizing<String>>,
        }
        let bytes = Zeroizing::new(
            serde_json::to_string(&Vault {
                auth_token: snapshot.get(&SecretField::AuthToken).unwrap(),
                signing_secret: snapshot.get(&SecretField::SigningSecret).unwrap(),
                credentials: &credentials,
                pending_crypto_keys: &pending,
            })
            .unwrap(),
        );
        store.set("vault", &bytes).unwrap();
        config.storage_backend = "keychain".into();
        config.save(&path).unwrap();
        directory.remove(OsStr::new(".keyfile")).unwrap();
        // The source is the vault; stale individual names must never win.
        store.set("auth_token", "stale-shadow-SENTINEL").unwrap();
        let forbidden = super::tests::forbidden_digests(&directory, &path, &store);
        assert!(
            run(&directory, &path, "keychain", &store, |phase| {
                super::tests::assert_no_oracles(&directory, &forbidden);
                if phase == "prepared" {
                    Err(invalid())
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
        super::tests::assert_no_oracles(&directory, &forbidden);
        test_migrate(&path, "keychain", &store).unwrap();
        assert!(super::tests::values(&directory, &path, &store) == expected);
        assert!(
            NodeConfig::load(&path).unwrap().keychain_format == Some(KeychainFormat::ImmutableV1)
        );
        assert!(store.get_optional("vault").unwrap().is_none());
        assert!(store.get_optional("auth_token").unwrap().is_none());
        assert!(!dir.path().join(".keyfile").exists());
    }
    #[test]
    fn immutable_optional_absence_never_discovers_stale_legacy_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        super::tests::fixture(&path);
        let directory = Directory::open(dir.path()).unwrap();
        let store = KeychainBackend::new_mock("node");
        test_migrate(&path, "keychain", &store).unwrap();
        let expected = super::tests::values(&directory, &path, &store);
        let field = SecretField::SshPassphrase {
            service: "ssh".into(),
            principal: "alice".into(),
        };
        store
            .set(
                &super::super::keychain::credential_key(&field.legacy_key()),
                "stale-passphrase-SENTINEL",
            )
            .unwrap();
        store.set("vault", "invalid-vault-SENTINEL").unwrap();
        test_migrate(&path, "file", &store).unwrap();
        assert!(super::tests::values(&directory, &path, &store) == expected);
        assert!(
            NodeConfig::load(&path).unwrap().ssh_keys[0]
                .passphrase_encrypted
                .is_none()
        );
        assert_eq!(store.get("vault").unwrap(), "invalid-vault-SENTINEL");
    }
    #[test]
    fn dry_run_and_status_print_only_states_and_counts_and_do_not_initialize() {
        let dir = tempfile::tempdir().unwrap();
        let keys = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        super::tests::fixture(&path);
        let config_before = std::fs::read(&path).unwrap();
        let key_before = Zeroizing::new(std::fs::read(dir.path().join(".keyfile")).unwrap());
        let entries = Directory::open(dir.path()).unwrap().names().unwrap();
        for (phase, target, expected) in [
            ("dry-run", "file", "Migration dry run:"),
            ("status", "keychain", "Migration: idle; backend: file"),
        ] {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "node::migration::tests::migration_process_child",
                    "--nocapture",
                ])
                .env("NYXID_TEST_MIGRATION_DIR", dir.path())
                .env("NYXID_TEST_MIGRATION_KEYS", keys.path())
                .env("NYXID_TEST_MIGRATION_TARGET", target)
                .env("NYXID_TEST_MIGRATION_PHASE", phase)
                .output()
                .unwrap();
            assert!(result.status.success());
            let output = format!(
                "{}{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(output.contains(expected));
            assert!(!output.contains("SENTINEL"));
            let config = NodeConfig::load(&path).unwrap();
            for (field, _) in secret_fields::fields(&config).unwrap() {
                if let Some(encoded) = field.encoded(&config).unwrap() {
                    assert!(!output.contains(encoded));
                }
            }
        }
        assert_eq!(std::fs::read(&path).unwrap(), config_before);
        assert!(std::fs::read(dir.path().join(".keyfile")).unwrap() == key_before.as_slice());
        assert!(!dir.path().join(MIGRATION_NAME).exists());
        assert_eq!(
            Directory::open(dir.path()).unwrap().names().unwrap(),
            entries
        );
        assert!(std::fs::read_dir(keys.path()).unwrap().next().is_none());
    }
    #[test]
    fn retargeted_publication_path_never_changes_the_replacement_directory() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("node");
        let retained = root.path().join("pinned");
        std::fs::create_dir(&original).unwrap();
        let path = original.join("config.toml");
        super::tests::fixture(&path);
        let directory = Directory::open(&original).unwrap();
        let store = KeychainBackend::new_mock("node");
        let mut moved = false;
        let mut replacement = Vec::new();
        run(&directory, &path, "keychain", &store, |phase| {
            if phase == "journal" && !moved {
                std::fs::rename(&original, &retained).unwrap();
                std::fs::create_dir(&original).unwrap();
                super::tests::fixture(&path);
                replacement = std::fs::read(&path).unwrap();
                moved = true;
            }
            Ok(())
        })
        .unwrap();
        assert!(moved);
        assert_eq!(std::fs::read(&path).unwrap(), replacement);
        assert_eq!(
            NodeConfig::load(&retained.join("config.toml"))
                .unwrap()
                .storage_backend,
            "keychain"
        );
        assert!(original.join(".keyfile").exists());
        assert!(!retained.join(".keyfile").exists());
        assert!(!original.join(MIGRATION_NAME).exists());
        assert!(!retained.join(MIGRATION_NAME).exists());
    }

    #[test]
    fn corrupted_target_metadata_refuses_resume_before_install_or_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        super::tests::fixture(&path);
        let directory = Directory::open(dir.path()).unwrap();
        let store = KeychainBackend::new_mock("node");
        let source = std::fs::read(&path).unwrap();
        assert!(
            run(&directory, &path, "keychain", &store, |phase| {
                if phase == "prepared" {
                    Err(invalid())
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
        let mut journal = Journal::read(&directory).unwrap().unwrap();
        let original = journal.prepared.as_ref().unwrap().config.clone();
        let mut config = NodeConfig::from_bytes(original.as_bytes()).unwrap();
        config.server.url = "wss://foreign.invalid".into();
        journal.prepared.as_mut().unwrap().config = toml::to_string_pretty(&config).unwrap();
        journal.persist(&directory, true).unwrap();
        assert!(test_migrate(&path, "keychain", &store).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), source);
        assert!(dir.path().join(".keyfile").exists());
        journal.prepared.as_mut().unwrap().config =
            format!("future_storage_secret = \"future-secret-SENTINEL\"\n{original}");
        journal.persist(&directory, true).unwrap();
        assert!(test_migrate(&path, "keychain", &store).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), source);
        assert!(dir.path().join(".keyfile").exists());
        journal.prepared.as_mut().unwrap().config = original;
        journal.persist(&directory, true).unwrap();
        test_migrate(&path, "keychain", &store).unwrap();
    }

    #[test]
    fn installed_uncertainty_retains_source_and_owned_targets_until_resume() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        super::tests::fixture(&path);
        let directory = Directory::open(dir.path()).unwrap();
        let store = KeychainBackend::new_mock("node");
        let original = super::tests::values(&directory, &path, &store);
        let result = run(&directory, &path, "keychain", &store, |phase| {
            if phase == "installed" {
                Err(Error::StorageDurabilityUncertain(
                    std::io::Error::from_raw_os_error(libc::EIO),
                ))
            } else {
                Ok(())
            }
        });
        assert!(matches!(result, Err(Error::StorageDurabilityUncertain(_))));
        assert!(dir.path().join(".keyfile").exists());
        assert!(dir.path().join(MIGRATION_NAME).exists());
        assert!(super::tests::values(&directory, &path, &store) == original);
        test_migrate(&path, "keychain", &store).unwrap();
        assert!(super::tests::values(&directory, &path, &store) == original);
        assert!(!dir.path().join(".keyfile").exists());
    }
}
