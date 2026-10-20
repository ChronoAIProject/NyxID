//! Immutable key-service objects and durable, commit-aware ownership journals.
//! This module never rewrites a live object or deletes by a logical secret name.
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::File;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::config::{KeychainFormat, NodeConfig};
use super::error::{Error, Result};
use super::keychain::KeychainBackend;
use super::secret_fields::{self, SecretField};
use super::storage::{Directory, MAX_CONFIG_BYTES};

const PREFIX: &str = "nyxid-key:v1:";
pub(crate) const MAX_OBJECTS: usize = 4096;

fn invalid() -> Error {
    Error::Config(
        "Node secret transaction requires authoritative state and an owned, ready reference".into(),
    )
}
fn stale() -> Error {
    Error::Config("Node configuration changed; reload before staging or publishing secrets".into())
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObjectRef {
    attempt: Uuid,
    object: Uuid,
}
impl std::fmt::Debug for ObjectRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ObjectRef([REDACTED])")
    }
}
impl ObjectRef {
    pub(crate) fn encode(self) -> String {
        format!("{PREFIX}{}:{}", self.attempt, self.object)
    }
    pub(crate) fn parse(value: &str) -> Result<Option<Self>> {
        let Some(suffix) = value.strip_prefix(PREFIX) else {
            // Unknown future formats must not be mistaken for legacy values.
            return if value.starts_with("nyxid-key:") {
                Err(invalid())
            } else {
                Ok(None)
            };
        };
        let (attempt, object) = suffix.split_once(':').ok_or_else(invalid)?;
        let reference = Self {
            attempt: Uuid::parse_str(attempt).map_err(|_| invalid())?,
            object: Uuid::parse_str(object).map_err(|_| invalid())?,
        };
        if reference.attempt.get_version_num() != 4
            || reference.object.get_version_num() != 4
            || reference.encode() != value
        {
            return Err(invalid());
        }
        Ok(Some(reference))
    }
    fn key(self) -> String {
        format!("object/v1/{}/{}", self.attempt, self.object)
    }
}

/// OS effects are explicit and injectable for crash/failure controls. Errors
/// crossing this boundary must not contain values or opaque object identifiers.
pub(crate) trait ObjectStore {
    fn owner_id(&self) -> &str;
    fn get(&self, reference: ObjectRef) -> Result<Option<Zeroizing<String>>>;
    fn put(&self, reference: ObjectRef, value: &str) -> Result<()>;
    fn delete(&self, reference: ObjectRef) -> Result<()>;
}
impl ObjectStore for KeychainBackend {
    fn owner_id(&self) -> &str {
        KeychainBackend::owner_id(self)
    }
    fn get(&self, reference: ObjectRef) -> Result<Option<Zeroizing<String>>> {
        self.get_optional(&reference.key())
            .map(|v| v.map(Zeroizing::new))
            .map_err(|_| Error::Keychain("Cannot read node secret object".into()))
    }
    fn put(&self, reference: ObjectRef, value: &str) -> Result<()> {
        self.set(&reference.key(), value)
            .map_err(|_| Error::Keychain("Cannot stage node secret object".into()))
    }
    fn delete(&self, reference: ObjectRef) -> Result<()> {
        KeychainBackend::delete(self, &reference.key())
            .map_err(|_| Error::Keychain("Cannot retire node secret object".into()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Purpose {
    Ordinary,
    ReturnedRotationRecovery,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Planned,
    Ready,
    Retired,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectIntent {
    reference: ObjectRef,
    field: SecretField,
    phase: Phase,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    attempt: Uuid,
    node_id: String,
    config_name: String,
    observed: Option<[u8; 32]>,
    purpose: Purpose,
    objects: Vec<ObjectIntent>,
}
fn journal_name(id: Uuid) -> String {
    format!(".secret-intent-{id}.json")
}
fn lease_name(id: Uuid) -> String {
    format!(".secret-attempt-{id}.lock")
}
fn current_bytes(directory: &Directory, name: &OsStr) -> Result<Option<Vec<u8>>> {
    match directory.read(name, MAX_CONFIG_BYTES) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
fn fingerprint(bytes: Option<&[u8]>) -> Option<[u8; 32]> {
    bytes.map(|bytes| Sha256::digest(bytes).into())
}
/// Caller holds the pinned directory's writer lock throughout source reads.
pub(crate) fn check_observed(
    directory: &Directory,
    path: &Path,
    observed: Option<[u8; 32]>,
) -> Result<()> {
    let name = path.file_name().ok_or_else(invalid)?;
    if fingerprint(current_bytes(directory, name)?.as_deref()) != observed {
        return Err(stale());
    }
    Ok(())
}
fn config(bytes: &[u8]) -> Result<NodeConfig> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    toml::from_str(text).map_err(|_| invalid())
}
impl Journal {
    fn read(directory: &Directory, id: Uuid) -> Result<Self> {
        // Ownership includes the original private lease inode, even when its
        // writer is no longer running. Never repair missing metadata by read.
        directory.read(OsStr::new(&lease_name(id)), 0)?;
        let bytes = directory.read(OsStr::new(&journal_name(id)), MAX_CONFIG_BYTES)?;
        let journal: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if journal.version != 1
            || journal.attempt != id
            || journal.objects.len() > MAX_OBJECTS
            || journal.attempt.get_version_num() != 4
            || journal
                .objects
                .iter()
                .any(|o| o.reference.attempt != id || o.reference.object.get_version_num() != 4)
            || journal
                .objects
                .iter()
                .map(|o| o.reference)
                .collect::<BTreeSet<_>>()
                .len()
                != journal.objects.len()
        {
            return Err(invalid());
        }
        Ok(journal)
    }
    fn persist(&self, directory: &Directory, replace: bool) -> Result<()> {
        let bytes = serde_json::to_vec(self).map_err(|_| invalid())?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(invalid());
        }
        directory.publish(OsStr::new(&journal_name(self.attempt)), &bytes, replace)
    }
}

/// One observed operation. A live lease prevents recovery from deleting an
/// object between durable intent, key-service effect and config publication.
/// No Drop rollback: process loss or uncertain installation requires recovery.
pub(crate) struct Attempt {
    directory: Directory,
    _lease: File,
    journal: Mutex<Journal>,
}
impl Attempt {
    pub(crate) fn id(&self) -> Result<Uuid> {
        Ok(self.journal.lock().map_err(|_| invalid())?.attempt)
    }

    pub(crate) fn check_projection(&self, config: &NodeConfig) -> Result<()> {
        let journal = self.journal.lock().map_err(|_| invalid())?;
        if journal.node_id != config.node.id
            || journal.observed != config.publication.snapshot()
            || journal.purpose != Purpose::Ordinary
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub(crate) fn begin_in(
        directory: &Directory,
        path: &Path,
        observed: Option<[u8; 32]>,
        node_id: &str,
    ) -> Result<Self> {
        Self::begin_in_with_purpose(directory, path, observed, node_id, Purpose::Ordinary)
    }
    fn begin_in_with_purpose(
        directory: &Directory,
        path: &Path,
        observed: Option<[u8; 32]>,
        node_id: &str,
        purpose: Purpose,
    ) -> Result<Self> {
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(invalid)?;
        let _writer = directory.writer_lock()?;
        if purpose == Purpose::Ordinary
            && fingerprint(current_bytes(directory, OsStr::new(name))?.as_deref()) != observed
        {
            return Err(stale());
        }
        let id = Uuid::new_v4();
        let lease = directory.create_lease(OsStr::new(&lease_name(id)))?;
        let journal = Journal {
            version: 1,
            attempt: id,
            node_id: node_id.into(),
            config_name: name.into(),
            observed,
            purpose,
            objects: Vec::new(),
        };
        journal.persist(directory, false)?;
        drop(_writer);
        Ok(Self {
            directory: directory.try_clone()?,
            _lease: lease,
            journal: Mutex::new(journal),
        })
    }
    pub(crate) fn resume_in(
        directory: &Directory,
        path: &Path,
        id: Uuid,
        observed: Option<[u8; 32]>,
        node_id: &str,
    ) -> Result<Self> {
        let _writer = directory.writer_lock()?;
        check_observed(directory, path, observed)?;
        let lease = directory
            .try_existing_lease(OsStr::new(&lease_name(id)))?
            .ok_or_else(invalid)?;
        let journal = Journal::read(directory, id)?;
        if journal.observed != observed
            || journal.node_id != node_id
            || Some(journal.config_name.as_str()) != path.file_name().and_then(OsStr::to_str)
            || journal.purpose != Purpose::Ordinary
            || journal.objects.iter().any(|o| o.phase == Phase::Retired)
            || journal
                .objects
                .iter()
                .map(|o| &o.field)
                .collect::<BTreeSet<_>>()
                .len()
                != journal.objects.len()
        {
            return Err(invalid());
        }
        Ok(Self {
            directory: directory.try_clone()?,
            _lease: lease,
            journal: Mutex::new(journal),
        })
    }

    /// Migration reuses its durably owned slot after a crash, verifying the
    /// exact source value instead of allocating a replacement or overwriting.
    pub(crate) fn stage_migration(
        &self,
        field: SecretField,
        value: &str,
        store: &impl ObjectStore,
    ) -> Result<String> {
        {
            let _writer = self.directory.writer_lock()?;
            let mut journal = self.journal.lock().map_err(|_| invalid())?;
            check_observed(
                &self.directory,
                Path::new(&journal.config_name),
                journal.observed,
            )?;
            if store.owner_id() != journal.node_id {
                return Err(invalid());
            }
            if let Some(index) = journal.objects.iter().position(|o| o.field == field) {
                let intent = &journal.objects[index];
                if intent.phase == Phase::Retired {
                    return Err(invalid());
                }
                let actual = store.get(intent.reference)?;
                match actual {
                    Some(actual) if actual.as_str() == value => {}
                    None if intent.phase == Phase::Planned => {
                        store.put(intent.reference, value)?;
                        if store.get(intent.reference)?.as_deref().map(String::as_str)
                            != Some(value)
                        {
                            return Err(invalid());
                        }
                    }
                    _ => return Err(invalid()),
                }
                let encoded = intent.reference.encode();
                journal.objects[index].phase = Phase::Ready;
                journal.persist(&self.directory, true)?;
                return Ok(encoded);
            }
        }
        self.stage_inner(field, value, store, true)
    }

    pub(crate) fn stage(
        &self,
        field: SecretField,
        value: &str,
        store: &impl ObjectStore,
    ) -> Result<String> {
        self.stage_inner(field, value, store, false)
    }
    fn stage_inner(
        &self,
        field: SecretField,
        value: &str,
        store: &impl ObjectStore,
        migration: bool,
    ) -> Result<String> {
        let _writer = self.directory.writer_lock()?;
        if !migration {
            self.directory.require_idle()?;
        }
        let mut journal = self.journal.lock().map_err(|_| invalid())?;
        if store.owner_id() != journal.node_id {
            return Err(invalid());
        }
        if journal.objects.len() >= MAX_OBJECTS {
            return Err(invalid());
        }
        if journal.purpose == Purpose::Ordinary
            && fingerprint(
                current_bytes(&self.directory, OsStr::new(&journal.config_name))?.as_deref(),
            ) != journal.observed
        {
            return Err(stale());
        }
        let reference = ObjectRef {
            attempt: journal.attempt,
            object: Uuid::new_v4(),
        };
        // A pre-existing target is never owned by this attempt and cannot be
        // overwritten or added to its GC manifest, even on a UUID collision.
        if store.get(reference)?.is_some() {
            return Err(invalid());
        }
        journal.objects.push(ObjectIntent {
            reference,
            field,
            phase: Phase::Planned,
        });
        journal.persist(&self.directory, true)?;
        store.put(reference, value)?;
        #[cfg(test)]
        if std::env::var_os("NYXID_TEST_SECRET_EXIT_AFTER_PUT").is_some() {
            std::process::exit(0);
        }
        let actual = store.get(reference)?.ok_or_else(invalid)?;
        if actual.as_str() != value {
            return Err(invalid());
        }
        journal.objects.last_mut().ok_or_else(invalid)?.phase = Phase::Ready;
        journal.persist(&self.directory, true)?;
        Ok(reference.encode())
    }
}

/// Complete typed liveness, including optional slots. Malformed/unknown config
/// or mixed reference formats refuse cleanup rather than dropping a live object.
pub(crate) fn live_references(config: &NodeConfig) -> Result<BTreeMap<ObjectRef, SecretField>> {
    if !matches!(config.storage_backend.as_str(), "file" | "keychain") {
        return Err(invalid());
    }
    let modern = config.keychain_format == Some(KeychainFormat::ImmutableV1);
    if config.storage_backend == "file" && config.keychain_format.is_some() {
        return Err(invalid());
    }
    let mut result = BTreeMap::new();
    for (field, required) in secret_fields::fields(config)? {
        let encoded = field.encoded(config)?;
        let reference = encoded.map(ObjectRef::parse).transpose()?.flatten();
        if config.storage_backend == "file" {
            if reference.is_some() {
                return Err(invalid());
            }
        } else if modern {
            if let Some(reference) = reference {
                if result.insert(reference, field).is_some() {
                    return Err(invalid());
                }
            } else if required || encoded.is_some() {
                return Err(invalid());
            }
        } else if reference.is_some() || encoded.is_some_and(|value| !value.is_empty()) {
            // Mixed or unmarked modern refs cannot preserve optional absence.
            return Err(invalid());
        }
    }
    Ok(result)
}

/// Called under the same directory/writer lock as the observed config CAS.
/// A previously live reference may persist only in the same typed location.
/// New references require ready ordinary intents bound to this exact snapshot.
pub(crate) fn validate_publication(
    directory: &Directory,
    name: &OsStr,
    current: Option<&[u8]>,
    next: &NodeConfig,
) -> Result<()> {
    let name = name.to_str().ok_or_else(invalid)?;
    let prior = current.map(config).transpose()?;
    let old = prior
        .as_ref()
        .map(live_references)
        .transpose()?
        .unwrap_or_default();
    let next_refs = live_references(next)?;
    for (reference, field) in next_refs {
        let journal = Journal::read(directory, reference.attempt)?;
        if journal.node_id != next.node.id || journal.config_name != name {
            return Err(invalid());
        }
        let intent = journal
            .objects
            .iter()
            .find(|o| o.reference == reference)
            .ok_or_else(invalid)?;
        if intent.phase != Phase::Ready || intent.field != field {
            return Err(invalid());
        }
        if let Some(previous_field) = old.get(&reference) {
            if previous_field != &field {
                return Err(invalid());
            }
        } else if journal.observed != fingerprint(current)
            || journal.purpose != Purpose::Ordinary
            || journal.objects.iter().any(|o| o.phase != Phase::Ready)
        {
            return Err(stale());
        }
    }
    Ok(())
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Cleanup {
    pub retired: usize,
    pub retained: usize,
    pub active: bool,
    pub recovery_pending: bool,
}

/// Validate the complete authoritative reference graph before authorizing any
/// cleanup effect, including references owned by other attempts. A syntactic
/// reference alone cannot establish liveness, field binding or ownership.
fn authoritative_references(
    directory: &Directory,
    name: &OsStr,
    current: &NodeConfig,
    store: &impl ObjectStore,
) -> Result<BTreeMap<ObjectRef, SecretField>> {
    if store.owner_id() != current.node.id {
        return Err(invalid());
    }
    let live = live_references(current)?;
    for (reference, field) in &live {
        let owner = Journal::read(directory, reference.attempt)?;
        if owner.node_id != current.node.id
            || Some(owner.config_name.as_str()) != name.to_str()
            || owner.purpose != Purpose::Ordinary
        {
            return Err(invalid());
        }
        let intent = owner
            .objects
            .iter()
            .find(|o| o.reference == *reference)
            .ok_or_else(invalid)?;
        if intent.phase != Phase::Ready || intent.field != *field {
            return Err(invalid());
        }
        // An unavailable live object invalidates authoritative storage. Refuse
        // all retirement instead of treating a damaged store as GC authority.
        store.get(*reference)?.ok_or_else(invalid)?;
    }
    Ok(live)
}

/// Complete modern read under the caller's writer lock. The same ownership
/// preflight used by recovery guards normal reads; no orphan or fallback read.
pub(crate) fn read_ready_references(
    directory: &Directory,
    path: &Path,
    config: &NodeConfig,
    store: &impl ObjectStore,
) -> Result<BTreeMap<SecretField, Zeroizing<String>>> {
    let live = authoritative_references(
        directory,
        path.file_name().ok_or_else(invalid)?,
        config,
        store,
    )?;
    live.into_iter()
        .map(|(reference, field)| Ok((field, store.get(reference)?.ok_or_else(invalid)?)))
        .collect()
}

/// Reconcile one owned manifest. Current config is authoritative even if an
/// earlier publish returned installed/uncertain. Live ownership is retained
/// durably so a later metadata removal can still retire those exact objects.
pub(crate) fn collect_one_in(
    directory: &Directory,
    path: &Path,
    id: Uuid,
    store: &impl ObjectStore,
) -> Result<Cleanup> {
    let name = path.file_name().ok_or_else(invalid)?;
    let _writer = directory.writer_lock()?;
    directory.require_idle()?;
    let Some(_lease) = directory.try_existing_lease(OsStr::new(&lease_name(id)))? else {
        return Ok(Cleanup {
            active: true,
            ..Default::default()
        });
    };
    let authority = cleanup_authority(directory, name, store)?;
    collect_locked(directory, name, id, store, &authority)
}

struct CleanupAuthority {
    config: NodeConfig,
    live: BTreeMap<ObjectRef, SecretField>,
}

fn cleanup_authority(
    directory: &Directory,
    name: &OsStr,
    store: &impl ObjectStore,
) -> Result<CleanupAuthority> {
    // Missing/corrupt authority never authorizes object deletion.
    let bytes = current_bytes(directory, name)?.ok_or_else(invalid)?;
    let current = config(&bytes)?;
    let live = authoritative_references(directory, name, &current, store)?;
    if current.storage_backend == "file" {
        let file = super::encryption::LocalEncryption::open_in(directory)?;
        for (field, required) in secret_fields::fields(&current)? {
            match field.encoded(&current)? {
                Some(encoded) => {
                    let _value = Zeroizing::new(file.decrypt(encoded)?);
                }
                None if required => return Err(invalid()),
                None => {}
            }
        }
    }
    // Resolve any earlier parent-sync uncertainty before destructive effects.
    directory.sync()?;
    Ok(CleanupAuthority {
        config: current,
        live,
    })
}

/// The caller retains both the writer lock and this attempt's existing lease.
fn collect_locked(
    directory: &Directory,
    name: &OsStr,
    id: Uuid,
    store: &impl ObjectStore,
    authority: &CleanupAuthority,
) -> Result<Cleanup> {
    let current = &authority.config;
    let live = &authority.live;
    let mut journal = Journal::read(directory, id)?;
    if journal.node_id != current.node.id || Some(journal.config_name.as_str()) != name.to_str() {
        return Err(invalid());
    }
    if store.owner_id() != journal.node_id {
        return Err(invalid());
    }
    if journal.purpose == Purpose::ReturnedRotationRecovery {
        // Rotation may have invalidated the remote predecessor. An unreachable
        // returned token is recovery state, not an abandoned ordinary write.
        // Explicit recovery/reauthorization must resolve it before retirement.
        return Ok(Cleanup {
            retained: journal.objects.len(),
            recovery_pending: true,
            ..Default::default()
        });
    }
    // Finish the plan before the first delete. No later live entry, even in
    // another manifest, may turn an earlier deletion into a partial preflight.
    let mut result = Cleanup {
        retained: journal
            .objects
            .iter()
            .filter(|object| live.contains_key(&object.reference))
            .count(),
        ..Default::default()
    };
    let retire: Vec<_> = journal
        .objects
        .iter()
        .enumerate()
        .filter(|(_, object)| {
            !live.contains_key(&object.reference) && object.phase != Phase::Retired
        })
        .map(|(index, _)| index)
        .collect();
    for index in retire {
        store.delete(journal.objects[index].reference)?;
        #[cfg(test)]
        if std::env::var_os("NYXID_TEST_SECRET_EXIT_AFTER_DELETE").is_some() {
            std::process::exit(0);
        }
        journal.objects[index].phase = Phase::Retired;
        journal.persist(directory, true)?;
        result.retired += 1;
    }
    if result.retained == 0 {
        // Both deletion and any future recovery open are writer-locked. A
        // missing lease is never recreated; after journal removal this unique
        // attempt cannot gain new references or new active users.
        directory.remove(OsStr::new(&journal_name(id)))?;
        directory.remove(OsStr::new(&lease_name(id)))?;
    }
    Ok(result)
}

/// Bounded recovery also visits old owners after a replacement/removal. A live
/// attempt is left alone, and returned rotations retain their recovery purpose.
pub(crate) fn collect_all_in(
    directory: &Directory,
    path: &Path,
    store: &impl ObjectStore,
) -> Result<()> {
    let _writer = directory.writer_lock()?;
    directory.require_idle()?;
    collect_migration_in(directory, path, store)
}

/// Only the migration journal authorizes cleanup while ordinary GC is fenced.
pub(crate) fn collect_migration_in(
    directory: &Directory,
    path: &Path,
    store: &impl ObjectStore,
) -> Result<()> {
    let config_name = path.file_name().ok_or_else(invalid)?;
    let mut owners = Vec::new();
    for name in directory.names()? {
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(id) = name
            .strip_prefix(".secret-intent-")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        let id = Uuid::parse_str(id).map_err(|_| invalid())?;
        if journal_name(id) != name {
            return Err(invalid());
        }
        let Some(_lease) = directory.try_existing_lease(OsStr::new(&lease_name(id)))? else {
            continue;
        };
        let journal = Journal::read(directory, id)?;
        if journal.node_id != store.owner_id()
            || Some(journal.config_name.as_str()) != config_name.to_str()
        {
            return Err(invalid());
        }
        owners.push((id, _lease));
    }
    if !owners.is_empty() {
        // Read and validate every ownership artifact before the first delete,
        // retaining all original inactive lease inodes throughout the pass.
        let authority = cleanup_authority(directory, config_name, store)?;
        for (id, _lease) in owners {
            collect_locked(directory, config_name, id, store, &authority)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn collect_one(path: &Path, id: Uuid, store: &impl ObjectStore) -> Result<Cleanup> {
        let directory = Directory::open(path.parent().unwrap()).unwrap();
        collect_one_in(&directory, path, id, store)
    }
    fn begin(
        path: &Path,
        observed: Option<[u8; 32]>,
        node_id: &str,
        purpose: Purpose,
    ) -> Result<Attempt> {
        let directory = Directory::open(path.parent().unwrap()).unwrap();
        Attempt::begin_in_with_purpose(&directory, path, observed, node_id, purpose)
    }

    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Store {
        owner: String,
        objects: Mutex<BTreeMap<ObjectRef, Zeroizing<String>>>,
        fail_put: AtomicBool,
        bad_readback: AtomicBool,
        puts: AtomicUsize,
        deletes: AtomicUsize,
        directory: std::path::PathBuf,
    }
    impl Store {
        fn new(directory: &Path) -> Self {
            Self {
                owner: "node".into(),
                objects: Mutex::new(BTreeMap::new()),
                fail_put: AtomicBool::new(false),
                bad_readback: AtomicBool::new(false),
                puts: AtomicUsize::new(0),
                deletes: AtomicUsize::new(0),
                directory: directory.into(),
            }
        }
    }
    impl ObjectStore for Store {
        fn owner_id(&self) -> &str {
            &self.owner
        }
        fn get(&self, reference: ObjectRef) -> Result<Option<Zeroizing<String>>> {
            let value = self.objects.lock().unwrap().get(&reference).cloned();
            if value.is_some() && self.bad_readback.load(Ordering::SeqCst) {
                Ok(Some(Zeroizing::new("different".into())))
            } else {
                Ok(value)
            }
        }
        fn put(&self, reference: ObjectRef, value: &str) -> Result<()> {
            // Actual effects must see a durable planned owner record first.
            let directory = Directory::open(&self.directory)?;
            let journal = Journal::read(&directory, reference.attempt)?;
            assert!(
                journal
                    .objects
                    .iter()
                    .any(|o| o.reference == reference && o.phase == Phase::Planned)
            );
            assert!(
                !std::fs::read_to_string(self.directory.join(journal_name(reference.attempt)))
                    .unwrap()
                    .contains(value)
            );
            self.puts.fetch_add(1, Ordering::SeqCst);
            if self.fail_put.load(Ordering::SeqCst) {
                return Err(Error::Keychain("injected staging failure".into()));
            }
            assert!(
                self.objects
                    .lock()
                    .unwrap()
                    .insert(reference, Zeroizing::new(value.into()))
                    .is_none()
            );
            Ok(())
        }
        fn delete(&self, reference: ObjectRef) -> Result<()> {
            self.deletes.fetch_add(1, Ordering::SeqCst);
            self.objects.lock().unwrap().remove(&reference);
            Ok(())
        }
    }
    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, NodeConfig, Store) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = NodeConfig::new(
            "wss://example.invalid".into(),
            "node".into(),
            "keychain".into(),
        );
        config.save(&path).unwrap();
        let mut config = NodeConfig::load(&path).unwrap();
        let store = Store::new(dir.path());
        let initial = begin(
            &path,
            config.publication.snapshot(),
            "node",
            Purpose::Ordinary,
        )
        .unwrap();
        config.node.auth_token_encrypted = initial
            .stage(SecretField::AuthToken, "synthetic-initial-auth", &store)
            .unwrap();
        config.keychain_format = Some(KeychainFormat::ImmutableV1);
        config.save(&path).unwrap();
        drop(initial);
        store.puts.store(0, Ordering::SeqCst);
        let config = NodeConfig::load(&path).unwrap();
        (dir, path, config, store)
    }
    fn reference(encoded: &str) -> ObjectRef {
        ObjectRef::parse(encoded).unwrap().unwrap()
    }

    #[test]
    fn intent_precedes_effect_and_ready_reference_binds_publication() {
        let (dir, path, mut config, store) = fixture();
        let attempt = begin(
            &path,
            config.publication.snapshot(),
            "node",
            Purpose::Ordinary,
        )
        .unwrap();
        let encoded = attempt
            .stage(SecretField::AuthToken, "synthetic-auth-value", &store)
            .unwrap();
        config.node.auth_token_encrypted = encoded.clone();
        config.save(&path).unwrap();
        let loaded = NodeConfig::load(&path).unwrap();
        assert_eq!(loaded.node.auth_token_encrypted, encoded);
        assert_eq!(
            store.get(reference(&encoded)).unwrap().unwrap().as_str(),
            "synthetic-auth-value"
        );
        assert_eq!(
            Directory::open(dir.path())
                .unwrap()
                .names()
                .unwrap()
                .iter()
                .filter(|p| p.to_string_lossy().starts_with(".secret-intent-"))
                .count(),
            2
        );
    }

    #[test]
    fn stale_staging_refuses_before_any_key_service_effect() {
        let (_dir, path, config, store) = fixture();
        let snapshot = config.publication.snapshot();
        let attempt = begin(&path, snapshot, "node", Purpose::Ordinary).unwrap();
        let newer = NodeConfig::load(&path).unwrap();
        newer.save(&path).unwrap();
        assert!(begin(&path, snapshot, "node", Purpose::Ordinary).is_err());
        assert!(
            attempt
                .stage(SecretField::AuthToken, "synthetic-stale-value", &store)
                .is_err()
        );
        assert_eq!(store.puts.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn immutable_format_requires_complete_refs_and_refuses_unmarked_or_file_refs() {
        let (_dir, path, mut config, store) = fixture();
        let original = config.node.auth_token_encrypted.clone();
        for value in ["", "legacy-ciphertext", "nyxid-key:v2:unsupported"] {
            config.node.auth_token_encrypted = value.into();
            assert!(config.save(&path).is_err());
        }
        config.node.auth_token_encrypted = original;
        config.keychain_format = None;
        assert!(config.save(&path).is_err());
        config.storage_backend = "file".into();
        assert!(config.save(&path).is_err());
        config.keychain_format = Some(KeychainFormat::ImmutableV1);
        assert!(config.save(&path).is_err());
        let text = toml::to_string(&config)
            .unwrap()
            .replace("immutable_v1", "unknown_v2");
        assert!(toml::from_str::<NodeConfig>(&text).is_err());
        assert_eq!(store.puts.load(Ordering::SeqCst), 0);
        assert_eq!(store.deletes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn active_and_committed_objects_survive_gc_until_metadata_retires_them() {
        let (dir, path, mut config, store) = fixture();
        let attempt = begin(
            &path,
            config.publication.snapshot(),
            "node",
            Purpose::Ordinary,
        )
        .unwrap();
        let encoded = attempt
            .stage(SecretField::SigningSecret, "synthetic-live-value", &store)
            .unwrap();
        let reference = reference(&encoded);
        assert!(
            collect_one(&path, reference.attempt, &store)
                .unwrap()
                .active
        );
        config.signing.shared_secret_encrypted = Some(encoded);
        config.save(&path).unwrap();
        drop(attempt);
        let result = collect_one(&path, reference.attempt, &store).unwrap();
        assert_eq!(result.retained, 1);
        assert!(dir.path().join(journal_name(reference.attempt)).exists());
        // Keeping the committed manifest is essential: later ordinary metadata
        // removal still has exact durable ownership for the old object.
        let mut config = NodeConfig::load(&path).unwrap();
        config.signing.shared_secret_encrypted = None;
        config.save(&path).unwrap();
        assert_eq!(
            collect_one(&path, reference.attempt, &store)
                .unwrap()
                .retired,
            1
        );
        assert!(!dir.path().join(journal_name(reference.attempt)).exists());
        assert!(!dir.path().join(lease_name(reference.attempt)).exists());
        assert!(store.get(reference).unwrap().is_none());
    }

    #[test]
    fn two_observed_writers_keep_winner_and_collect_only_loser_objects() {
        let (_dir, path, mut winner, store) = fixture();
        let mut loser = winner.clone();
        let first = begin(
            &path,
            winner.publication.snapshot(),
            "node",
            Purpose::Ordinary,
        )
        .unwrap();
        let second = begin(
            &path,
            loser.publication.snapshot(),
            "node",
            Purpose::Ordinary,
        )
        .unwrap();
        winner.node.auth_token_encrypted = first
            .stage(SecretField::AuthToken, "synthetic-winner", &store)
            .unwrap();
        loser.node.auth_token_encrypted = second
            .stage(SecretField::AuthToken, "synthetic-loser", &store)
            .unwrap();
        let winning_ref = reference(&winner.node.auth_token_encrypted);
        let losing_ref = reference(&loser.node.auth_token_encrypted);
        winner.save(&path).unwrap();
        assert!(loser.save(&path).is_err());
        drop(first);
        drop(second);
        assert_eq!(
            collect_one(&path, losing_ref.attempt, &store)
                .unwrap()
                .retired,
            1
        );
        assert!(store.get(winning_ref).unwrap().is_some());
        assert_eq!(
            collect_one(&path, winning_ref.attempt, &store)
                .unwrap()
                .retained,
            1
        );
    }

    #[test]
    fn stage_failure_and_bad_readback_never_authorize_partial_publication() {
        for bad_readback in [false, true] {
            let (_dir, path, mut config, store) = fixture();
            let attempt = begin(
                &path,
                config.publication.snapshot(),
                "node",
                Purpose::Ordinary,
            )
            .unwrap();
            let good = attempt
                .stage(SecretField::AuthToken, "synthetic-ready", &store)
                .unwrap();
            store.bad_readback.store(bad_readback, Ordering::SeqCst);
            store.fail_put.store(!bad_readback, Ordering::SeqCst);
            assert!(
                attempt
                    .stage(SecretField::SigningSecret, "synthetic-failed", &store)
                    .is_err()
            );
            config.node.auth_token_encrypted = good.clone();
            assert!(config.save(&path).is_err());
            drop(attempt);
            let cleaned = collect_one(&path, reference(&good).attempt, &store).unwrap();
            assert_eq!(cleaned.retired, 2);
            assert_eq!(store.objects.lock().unwrap().len(), 1);
        }
    }

    #[test]
    fn missing_or_corrupt_authority_refuses_all_gc_effects() {
        for missing in [true, false] {
            let (_dir, path, config, store) = fixture();
            let attempt = begin(
                &path,
                config.publication.snapshot(),
                "node",
                Purpose::Ordinary,
            )
            .unwrap();
            let encoded = attempt
                .stage(SecretField::AuthToken, "synthetic-preserved", &store)
                .unwrap();
            drop(attempt);
            if missing {
                std::fs::remove_file(&path).unwrap();
            } else {
                std::fs::write(&path, b"invalid toml = [[").unwrap();
            }
            assert!(collect_one(&path, reference(&encoded).attempt, &store).is_err());
            assert_eq!(store.deletes.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn missing_lease_is_unready_without_deletion_or_inode_recreation() {
        let (dir, path, config, store) = fixture();
        let attempt = begin(
            &path,
            config.publication.snapshot(),
            "node",
            Purpose::Ordinary,
        )
        .unwrap();
        let owned = reference(
            &attempt
                .stage(SecretField::AuthToken, "synthetic-owned", &store)
                .unwrap(),
        );
        assert!(collect_one(&path, owned.attempt, &store).unwrap().active);
        drop(attempt);
        let lease = dir.path().join(lease_name(owned.attempt));
        std::fs::remove_file(&lease).unwrap();
        assert!(collect_one(&path, owned.attempt, &store).is_err());
        assert_eq!(store.deletes.load(Ordering::SeqCst), 0);
        assert!(!lease.exists());
        assert!(store.get(owned).unwrap().is_some());
        assert!(dir.path().join(journal_name(owned.attempt)).exists());
    }

    #[test]
    fn gc_preflights_later_live_intent_before_deleting_an_earlier_orphan() {
        for fault in [
            "none",
            "field",
            "planned",
            "retired",
            "owner",
            "missing-object",
        ] {
            let (dir, path, mut config, store) = fixture();
            let attempt = begin(
                &path,
                config.publication.snapshot(),
                "node",
                Purpose::Ordinary,
            )
            .unwrap();
            let orphan = reference(
                &attempt
                    .stage(SecretField::AuthToken, "synthetic-first-orphan", &store)
                    .unwrap(),
            );
            let encoded = attempt
                .stage(SecretField::SigningSecret, "synthetic-later-live", &store)
                .unwrap();
            let live = reference(&encoded);
            config.signing.shared_secret_encrypted = Some(encoded);
            config.save(&path).unwrap();
            drop(attempt);
            let directory = Directory::open(dir.path()).unwrap();
            let mut journal = Journal::read(&directory, orphan.attempt).unwrap();
            assert_eq!(journal.objects[0].reference, orphan);
            assert_eq!(journal.objects[1].reference, live);
            match fault {
                "field" => journal.objects[1].field = SecretField::AuthToken,
                "planned" => journal.objects[1].phase = Phase::Planned,
                "retired" => journal.objects[1].phase = Phase::Retired,
                "owner" => journal.node_id = "different-node".into(),
                "missing-object" => {
                    store.objects.lock().unwrap().remove(&live);
                }
                _ => (),
            }
            journal.persist(&directory, true).unwrap();
            let result = collect_one(&path, orphan.attempt, &store);
            if fault == "none" {
                let result = result.unwrap();
                assert_eq!(result.retired, 1);
                assert_eq!(result.retained, 1);
                assert_eq!(store.deletes.load(Ordering::SeqCst), 1);
                assert!(store.get(orphan).unwrap().is_none());
                assert!(store.get(live).unwrap().is_some());
            } else {
                assert!(result.is_err(), "fault: {fault}");
                assert_eq!(store.deletes.load(Ordering::SeqCst), 0, "fault: {fault}");
                assert!(store.get(orphan).unwrap().is_some());
            }
        }
    }

    #[test]
    fn gc_refuses_invalid_live_metadata_owned_by_another_manifest_before_any_delete() {
        for fault in [
            "missing", "corrupt", "owner", "path", "field", "phase", "purpose", "unowned", "lease",
        ] {
            let (dir, path, mut config, store) = fixture();
            let abandoned = begin(
                &path,
                config.publication.snapshot(),
                "node",
                Purpose::Ordinary,
            )
            .unwrap();
            let orphan = reference(
                &abandoned
                    .stage(SecretField::AuthToken, "synthetic-owned-orphan", &store)
                    .unwrap(),
            );
            let committed = begin(
                &path,
                config.publication.snapshot(),
                "node",
                Purpose::Ordinary,
            )
            .unwrap();
            let encoded = committed
                .stage(SecretField::AuthToken, "synthetic-foreign-live", &store)
                .unwrap();
            let live = reference(&encoded);
            config.node.auth_token_encrypted = encoded;
            config.save(&path).unwrap();
            drop(abandoned);
            drop(committed);
            let directory = Directory::open(dir.path()).unwrap();
            let mut journal = Journal::read(&directory, live.attempt).unwrap();
            match fault {
                "lease" => directory
                    .remove(OsStr::new(&lease_name(live.attempt)))
                    .unwrap(),
                "missing" => directory
                    .remove(OsStr::new(&journal_name(live.attempt)))
                    .unwrap(),
                "corrupt" => directory
                    .publish(
                        OsStr::new(&journal_name(live.attempt)),
                        b"invalid json",
                        true,
                    )
                    .unwrap(),
                _ => {
                    match fault {
                        "owner" => journal.node_id = "different-node".into(),
                        "path" => journal.config_name = "different.toml".into(),
                        "field" => journal.objects[0].field = SecretField::SigningSecret,
                        "phase" => journal.objects[0].phase = Phase::Planned,
                        "purpose" => journal.purpose = Purpose::ReturnedRotationRecovery,
                        "unowned" => journal.objects.clear(),
                        _ => unreachable!(),
                    }
                    journal.persist(&directory, true).unwrap();
                }
            }
            assert!(
                collect_one(&path, orphan.attempt, &store).is_err(),
                "fault: {fault}"
            );
            assert_eq!(store.deletes.load(Ordering::SeqCst), 0, "fault: {fault}");
            assert!(store.get(orphan).unwrap().is_some());
            assert!(store.get(live).unwrap().is_some());
        }
    }

    #[test]
    fn file_cleanup_refuses_missing_or_corrupt_selected_custody_before_retiring_objects() {
        use super::super::encryption::LocalEncryption;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let file = LocalEncryption::initialize(dir.path()).unwrap();
        let mut current =
            NodeConfig::new("wss://example.invalid".into(), "node".into(), "file".into());
        current
            .set_auth_token("synthetic-live-file-auth", &file)
            .unwrap();
        current.save(&path).unwrap();
        let current = NodeConfig::load(&path).unwrap();
        let store = Store::new(dir.path());
        let attempt = begin(
            &path,
            current.publication.snapshot(),
            "node",
            Purpose::Ordinary,
        )
        .unwrap();
        let orphan = reference(
            &attempt
                .stage(
                    SecretField::AuthToken,
                    "synthetic-unpublished-object",
                    &store,
                )
                .unwrap(),
        );
        drop(attempt);
        let directory = Directory::open(dir.path()).unwrap();
        let key = Zeroizing::new(std::fs::read(dir.path().join(".keyfile")).unwrap());
        std::fs::remove_file(dir.path().join(".keyfile")).unwrap();
        assert!(collect_all_in(&directory, &path, &store).is_err());
        assert_eq!(store.deletes.load(Ordering::SeqCst), 0);
        assert!(store.get(orphan).unwrap().is_some());
        directory
            .publish(OsStr::new(".keyfile"), &key, false)
            .unwrap();
        let mut damaged = NodeConfig::load(&path).unwrap();
        damaged.node.auth_token_encrypted = "invalid-ciphertext".into();
        damaged.save(&path).unwrap();
        assert!(collect_all_in(&directory, &path, &store).is_err());
        assert_eq!(store.deletes.load(Ordering::SeqCst), 0);
        assert!(store.get(orphan).unwrap().is_some());
        let mut repaired = NodeConfig::load(&path).unwrap();
        repaired
            .set_auth_token("synthetic-live-file-auth", &file)
            .unwrap();
        repaired.save(&path).unwrap();
        collect_all_in(&directory, &path, &store).unwrap();
        assert_eq!(store.deletes.load(Ordering::SeqCst), 1);
        assert!(store.get(orphan).unwrap().is_none());
        assert_eq!(
            NodeConfig::load(&path)
                .unwrap()
                .decrypt_auth_token(&file)
                .unwrap(),
            "synthetic-live-file-auth"
        );
    }

    #[test]
    fn returned_rotation_is_retained_after_stale_await_without_authorizing_publish() {
        let (_dir, path, old, store) = fixture();
        let mut current = NodeConfig::load(&path).unwrap();
        current.server.proxy_max_body_size += 1;
        current.save(&path).unwrap();
        assert!(begin(&path, old.publication.snapshot(), "node", Purpose::Ordinary).is_err());
        let recovery = begin(
            &path,
            old.publication.snapshot(),
            "node",
            Purpose::ReturnedRotationRecovery,
        )
        .unwrap();
        let encoded = recovery
            .stage(
                SecretField::AuthToken,
                "synthetic-returned-rotation",
                &store,
            )
            .unwrap();
        let mut current = NodeConfig::load(&path).unwrap();
        current.node.auth_token_encrypted = encoded.clone();
        assert!(current.save(&path).is_err());
        drop(recovery);
        let cleanup = collect_one(&path, reference(&encoded).attempt, &store).unwrap();
        assert!(cleanup.recovery_pending);
        assert_eq!(cleanup.retained, 1);
        assert_eq!(store.deletes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn references_cannot_move_fields_or_cross_nodes_and_debug_redacts() {
        let (_dir, path, mut config, store) = fixture();
        let attempt = begin(
            &path,
            config.publication.snapshot(),
            "node",
            Purpose::Ordinary,
        )
        .unwrap();
        let encoded = attempt
            .stage(SecretField::AuthToken, "synthetic-value", &store)
            .unwrap();
        let reference = reference(&encoded);
        assert!(!format!("{reference:?}").contains(&reference.object.to_string()));
        config.signing.shared_secret_encrypted = Some(encoded.clone());
        assert!(config.save(&path).is_err());
        config.signing.shared_secret_encrypted = None;
        config.node.auth_token_encrypted = encoded.clone();
        config.node.id = "different-node".into();
        assert!(config.save(&path).is_err());
        let mut wrong_store = Store::new(&store.directory);
        wrong_store.owner = "different-node".into();
        assert!(
            attempt
                .stage(SecretField::AuthToken, "synthetic-other", &wrong_store)
                .is_err()
        );
        assert_eq!(wrong_store.puts.load(Ordering::SeqCst), 0);
        for debug in [
            format!("{config:?}"),
            format!("{:?}", config.node),
            format!("{:?}", config.signing),
        ] {
            assert!(!debug.contains(&encoded));
        }
    }

    #[test]
    fn migration_resume_reuses_planned_and_ready_ownership_without_overwrite() {
        let (dir, path, config, store) = fixture();
        let directory = Directory::open(dir.path()).unwrap();
        let attempt =
            Attempt::begin_in(&directory, &path, config.publication.snapshot(), "node").unwrap();
        let id = attempt.id().unwrap();
        store.fail_put.store(true, Ordering::SeqCst);
        assert!(
            attempt
                .stage_migration(SecretField::AuthToken, "migration-value", &store)
                .is_err()
        );
        let journal = Journal::read(&directory, id).unwrap();
        assert!(journal.objects[0].phase == Phase::Planned);
        let planned = journal.objects[0].reference;
        drop(attempt);
        store.fail_put.store(false, Ordering::SeqCst);
        let resumed =
            Attempt::resume_in(&directory, &path, id, config.publication.snapshot(), "node")
                .unwrap();
        let encoded = resumed
            .stage_migration(SecretField::AuthToken, "migration-value", &store)
            .unwrap();
        assert!(reference(&encoded) == planned);
        assert_eq!(store.puts.load(Ordering::SeqCst), 2);
        drop(resumed);
        let resumed =
            Attempt::resume_in(&directory, &path, id, config.publication.snapshot(), "node")
                .unwrap();
        assert_eq!(
            resumed
                .stage_migration(SecretField::AuthToken, "migration-value", &store)
                .unwrap(),
            encoded
        );
        assert!(
            resumed
                .stage_migration(SecretField::AuthToken, "wrong-value", &store)
                .is_err()
        );
        assert_eq!(store.puts.load(Ordering::SeqCst), 2);
        assert_eq!(store.deletes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn migration_resume_requires_original_inactive_lease_and_exact_generation() {
        let (dir, path, config, store) = fixture();
        let directory = Directory::open(dir.path()).unwrap();
        let attempt =
            Attempt::begin_in(&directory, &path, config.publication.snapshot(), "node").unwrap();
        let id = attempt.id().unwrap();
        assert!(
            Attempt::resume_in(&directory, &path, id, config.publication.snapshot(), "node")
                .is_err()
        );
        drop(attempt);
        directory.remove(OsStr::new(&lease_name(id))).unwrap();
        assert!(
            Attempt::resume_in(&directory, &path, id, config.publication.snapshot(), "node")
                .is_err()
        );
        assert!(!dir.path().join(lease_name(id)).exists());
        assert_eq!(store.puts.load(Ordering::SeqCst), 0);
        assert_eq!(store.deletes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn reference_parser_rejects_unknown_versions_noncanonical_and_trailing_data() {
        let r = ObjectRef {
            attempt: Uuid::parse_str("a1234567-1234-4234-8234-123456789abc").unwrap(),
            object: Uuid::parse_str("b1234567-1234-4234-8234-123456789abc").unwrap(),
        };
        assert_eq!(ObjectRef::parse(&r.encode()).unwrap(), Some(r));
        for invalid in [
            r.encode() + ":extra",
            r.encode().replacen("v1", "v2", 1),
            r.encode()
                .to_uppercase()
                .replacen("NYXID-KEY:V1:", PREFIX, 1),
            format!("{PREFIX}{}:{}", Uuid::nil(), r.object),
        ] {
            assert!(ObjectRef::parse(&invalid).is_err());
        }
        assert!(
            ObjectRef::parse("legacy-file-ciphertext")
                .unwrap()
                .is_none()
        );
    }
}
