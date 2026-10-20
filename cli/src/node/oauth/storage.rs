//! OAuth response recovery is separate from permission to publish node config.
//! A durable dispatch barrier prevents refresh replay after an ambiguous result.
//! Returned material uses a key reserved before dispatch, independent of the
//! selected node key's continued availability. Nothing here repairs a lookup.
use std::ffi::OsStr;
use std::fs::File;
use std::path::PathBuf;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::super::config::{NodeConfig, OAuthRequestOptions};
use super::super::error::{Error, Result};
use super::super::keychain::KeychainBackend;
use super::super::secret_backend::SecretBackend;
use super::super::storage::{Directory, MAX_CONFIG_BYTES};
use super::{OAuthConfig, TokenResponse};

fn pending() -> Error {
    Error::Config("oauth_recovery_pending: retained OAuth state must be recovered before another provider request; restore the original node configuration and custody, then restart or rerun the credential command. An unknown provider outcome with no retained response requires fresh authorization via `nyxid node credentials add-oauth --service <slug>` with the original OAuth options and profile/config; never retry its refresh token.".into())
}
fn invalid() -> Error {
    Error::Config("oauth_recovery_invalid: retain node storage and recovery files; no provider request was retried".into())
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Plan {
    Refresh,
    Login(Box<LoginPlan>),
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginPlan {
    token_url: String,
    client_id: Zeroizing<String>,
    client_secret: Option<Zeroizing<String>>,
    scopes: Option<String>,
    auth_method: String,
    client_id_param_name: Option<String>,
    options: OAuthRequestOptions,
    target_url: Option<String>,
}

impl Plan {
    pub(crate) fn login(
        config: &OAuthConfig,
        client_id: &str,
        client_secret: Option<&str>,
        scopes: &str,
        target_url: Option<&str>,
    ) -> Self {
        Self::Login(Box::new(LoginPlan {
            token_url: config.token_url.clone(),
            client_id: Zeroizing::new(client_id.into()),
            client_secret: client_secret.map(|value| Zeroizing::new(value.into())),
            scopes: (!scopes.is_empty()).then(|| scopes.into()),
            auth_method: config.token_endpoint_auth_method.clone(),
            client_id_param_name: config.client_id_param_name.clone(),
            options: config.request_options.clone(),
            target_url: target_url.map(str::to_owned),
        }))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Returned {
    plan: Plan,
    token: TokenResponse,
    received_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Reserved,
    Dispatched,
    Returned,
    Complete,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    id: Uuid,
    binding: String,
    service: String,
    file_key: bool,
    observed: [u8; 32],
    installed: Option<[u8; 32]>,
    phase: Phase,
    ciphertext: Option<String>,
}

pub(crate) struct Rotation {
    directory: Directory,
    path: PathBuf,
    keychain: KeychainBackend,
    reader: SecretBackend,
    config: NodeConfig,
    service: String,
    binding: String,
    name: String,
    _lease: File,
    record: Option<Record>,
    recovery_key: Option<Zeroizing<[u8; 32]>>,
}
impl std::fmt::Debug for Rotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OAuthRotation { authority_and_material: [REDACTED] }")
    }
}

fn optional_read(directory: &Directory, name: &str) -> Result<Option<Vec<u8>>> {
    match directory.read(OsStr::new(name), MAX_CONFIG_BYTES) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
fn remove_if_present(directory: &Directory, name: &str) -> Result<()> {
    match directory.remove(OsStr::new(name)) {
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

impl Rotation {
    pub(crate) fn open(backend: &SecretBackend, service: &str) -> Result<Self> {
        let (directory, path, keychain, original) = backend.oauth_custody()?;
        let (config, reader) = backend.current_reader()?;
        if config.publication.snapshot() != original.publication.snapshot() {
            return Err(pending());
        }
        let encoded = serde_json::to_vec(&(
            "nyxid-node-oauth-recovery-v1",
            path.file_name()
                .and_then(OsStr::to_str)
                .ok_or_else(invalid)?,
            &config.node.id,
            service,
        ))
        .map_err(|_| invalid())?;
        let binding = hex::encode(Sha256::digest(encoded));
        let name = format!(".nyxid-storage-oauth-{binding}.json");
        let lock_name = format!(".nyxid-storage-oauth-{binding}.lock");
        let writer = directory.writer_lock()?;
        let record: Option<Record> = optional_read(&directory, &name)?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| invalid()))
            .transpose()?;
        if let Some(record) = &record
            && (record.version != 1
                || record.binding != binding
                || record.service != service
                || record.id.get_version_num() != 4
                || record.file_key != (config.storage_backend == "file"))
        {
            return Err(invalid());
        }
        let lease = match directory.try_existing_lease(OsStr::new(&lock_name)) {
            Ok(Some(lease)) => lease,
            Ok(None) => return Err(pending()),
            Err(Error::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound && record.is_none() =>
            {
                directory.create_lease(OsStr::new(&lock_name))?
            }
            Err(error) => return Err(error),
        };
        drop(writer);
        Ok(Self {
            directory,
            path,
            keychain,
            reader,
            config,
            service: service.into(),
            binding,
            name,
            _lease: lease,
            record,
            recovery_key: None,
        })
    }

    fn persist(&self) -> Result<()> {
        let bytes =
            serde_json::to_vec(self.record.as_ref().ok_or_else(invalid)?).map_err(|_| invalid())?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(invalid());
        }
        self.directory.publish(OsStr::new(&self.name), &bytes, true)
    }

    fn key_name(record: &Record) -> String {
        if record.file_key {
            format!(".nyxid-storage-oauth-{}.key", record.id)
        } else {
            format!("object/v1/oauth-recovery/{}", record.id)
        }
    }

    fn key(&self) -> Result<Zeroizing<[u8; 32]>> {
        if let Some(key) = &self.recovery_key {
            return Ok(key.clone());
        }
        let record = self.record.as_ref().ok_or_else(invalid)?;
        let bytes = if record.file_key {
            Zeroizing::new(
                self.directory
                    .read(OsStr::new(&Self::key_name(record)), 32)?,
            )
        } else {
            let encoded = Zeroizing::new(
                self.keychain
                    .get(&Self::key_name(record))
                    .map_err(|_| invalid())?,
            );
            Zeroizing::new(
                base64::engine::general_purpose::STANDARD
                    .decode(encoded.as_bytes())
                    .map_err(|_| invalid())?,
            )
        };
        if bytes.len() != 32 {
            return Err(invalid());
        }
        let mut key = Zeroizing::new([0; 32]);
        key.copy_from_slice(&bytes);
        Ok(key)
    }

    fn encrypt(&self, plaintext: &[u8]) -> Result<String> {
        let key = self.key()?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| invalid())?;
        let mut nonce = [0; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let mut encrypted = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &self.aad()?,
                },
            )
            .map_err(|_| invalid())?;
        let mut output = nonce.to_vec();
        output.append(&mut encrypted);
        Ok(base64::engine::general_purpose::STANDARD.encode(output))
    }

    fn aad(&self) -> Result<Vec<u8>> {
        let record = self.record.as_ref().ok_or_else(invalid)?;
        serde_json::to_vec(&(
            1,
            &record.binding,
            record.id,
            record.file_key,
            record.observed,
        ))
        .map_err(|_| invalid())
    }

    fn returned(&self) -> Result<Returned> {
        let record = self.record.as_ref().ok_or_else(invalid)?;
        let encoded = record.ciphertext.as_ref().ok_or_else(invalid)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| invalid())?;
        if bytes.len() < 28 {
            return Err(invalid());
        }
        let key = self.key()?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| invalid())?;
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&bytes[..12]),
                    Payload {
                        msg: &bytes[12..],
                        aad: &self.aad()?,
                    },
                )
                .map_err(|_| invalid())?,
        );
        serde_json::from_slice(&plaintext).map_err(|_| invalid())
    }

    /// Reserve recovery custody and persist the replay barrier BEFORE awaiting
    /// a provider. Failure here never consumes or rotates a remote token.
    pub(crate) fn dispatch(&mut self) -> Result<()> {
        if self.record.is_some() {
            return Err(pending());
        }
        // Refuse legacy conversion or stale/unhealthy writers before dispatch.
        let _writer = self.reader.current_writer()?;
        let lock = self.directory.writer_lock()?;
        super::super::secret_transaction::check_observed(
            &self.directory,
            &self.path,
            self.config.publication.snapshot(),
        )?;
        self.record = Some(Record {
            version: 1,
            id: Uuid::new_v4(),
            binding: self.binding.clone(),
            service: self.service.clone(),
            file_key: self.config.storage_backend == "file",
            observed: self.config.publication.snapshot().ok_or_else(invalid)?,
            installed: None,
            phase: Phase::Reserved,
            ciphertext: None,
        });
        self.persist()?;
        let record = self.record.as_ref().ok_or_else(invalid)?;
        let mut key = Zeroizing::new([0; 32]);
        rand::rngs::OsRng.fill_bytes(key.as_mut());
        if record.file_key {
            self.directory
                .publish(OsStr::new(&Self::key_name(record)), key.as_ref(), false)?;
        } else {
            let name = Self::key_name(record);
            if self
                .keychain
                .get_optional(&name)
                .map_err(|_| invalid())?
                .is_some()
            {
                return Err(invalid());
            }
            let encoded =
                Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(key.as_ref()));
            self.keychain.set(&name, &encoded).map_err(|_| invalid())?;
        }
        if self.key()?.as_ref() != key.as_ref() {
            return Err(invalid());
        }
        self.recovery_key = Some(key);
        self.record.as_mut().ok_or_else(invalid)?.phase = Phase::Dispatched;
        self.persist()?;
        drop(lock);
        Ok(())
    }

    /// This is intentionally independent of config CAS and node-key health.
    /// The reserved envelope survives publication denial and selected-key loss.
    pub(crate) fn retain(&mut self, plan: Plan, token: TokenResponse) -> Result<()> {
        if self
            .record
            .as_ref()
            .is_none_or(|r| r.phase != Phase::Dispatched)
        {
            return Err(invalid());
        }
        let returned = Returned {
            plan,
            token,
            received_at: chrono::Utc::now(),
        };
        let plaintext = Zeroizing::new(serde_json::to_vec(&returned).map_err(|_| invalid())?);
        let ciphertext = self.encrypt(&plaintext)?;
        let record = self.record.as_mut().ok_or_else(invalid)?;
        record.ciphertext = Some(ciphertext);
        record.phase = Phase::Returned;
        self.persist_returned()
    }

    fn persist_returned(&self) -> Result<()> {
        let _writer = self.directory.writer_lock()?;
        self.persist()
    }

    /// Keep the returned response and its lease alive while disk is unavailable.
    /// No publication, live use or second provider request can pass this wait.
    /// Forced process termination still leaves the durable dispatch barrier.
    pub(crate) async fn retain_durable(&mut self, plan: Plan, token: TokenResponse) -> Result<()> {
        let mut result = self.retain(plan, token);
        while result.is_err() {
            if self
                .record
                .as_ref()
                .is_none_or(|record| record.phase != Phase::Returned)
            {
                return result;
            }
            tracing::error!(
                reason = "oauth_recovery_write_failed",
                "Returned OAuth tokens remain held in memory; restore storage without stopping this process"
            );
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            result = self.persist_returned();
        }
        result
    }

    fn finish(&mut self) -> Result<()> {
        let _writer = self.directory.writer_lock()?;
        let record = self.record.as_mut().ok_or_else(invalid)?;
        record.phase = Phase::Complete;
        self.persist()?;
        self.cleanup_locked()
    }
    fn cleanup_locked(&mut self) -> Result<()> {
        // Complete is durable before deleting its envelope key. Missing keys
        // are tolerated only on this idempotent completion/reservation path.
        self.directory.sync()?;
        let record = self.record.as_ref().ok_or_else(invalid)?;
        if record.file_key {
            remove_if_present(&self.directory, &Self::key_name(record))?;
        } else {
            self.keychain
                .delete(&Self::key_name(record))
                .map_err(|_| invalid())?;
        }
        remove_if_present(&self.directory, &self.name)?;
        self.record = None;
        self.recovery_key = None;
        Ok(())
    }

    /// Resume only the exact observed generation; never rebase returned secrets
    /// onto a changed owner, backend, credential definition or policy.
    pub(crate) fn recover(&mut self) -> Result<bool> {
        let Some(record) = &self.record else {
            return Ok(false);
        };
        match record.phase {
            Phase::Reserved | Phase::Complete => {
                let _writer = self.directory.writer_lock()?;
                self.cleanup_locked()?;
                return Ok(false);
            }
            Phase::Dispatched => return Err(pending()),
            Phase::Returned => {}
        }
        let (mut config, reader) = self.reader.current_reader()?;
        let record = self.record.as_ref().ok_or_else(invalid)?;
        if record.installed.is_some() && config.publication.snapshot() == record.installed {
            // A process may have died after install but before completion.
            // current_reader validated the entire live graph/key health.
            self.finish()?;
            return Ok(true);
        }
        if config.publication.snapshot() != Some(record.observed) {
            return Err(pending());
        }
        let returned = self.returned()?;
        let (prepared, writer) = reader.current_writer()?;
        if prepared.publication.snapshot() != Some(record.observed) {
            return Err(pending());
        }
        config = prepared;
        apply(&mut config, &writer, &self.service, returned)?;
        let name = self.name.clone();
        let record = self.record.as_mut().ok_or_else(invalid)?;
        writer.publish_with_intent(&config, &self.path, |directory, bytes| {
            record.installed = Some(Sha256::digest(bytes).into());
            let bytes = serde_json::to_vec(record).map_err(|_| invalid())?;
            directory.publish(OsStr::new(&name), &bytes, true)
        })?;
        #[cfg(test)]
        if std::env::var_os("NYXID_TEST_OAUTH_EXIT_AFTER_PUBLICATION").is_some() {
            std::process::exit(0);
        }
        self.finish()?;
        Ok(true)
    }

    /// An explicit add-oauth command can obtain a fresh authorization grant
    /// after a lost response. It never retries the predecessor refresh token,
    /// and cannot discard any durably returned response or an active lease.
    pub(crate) fn recover_for_login(&mut self) -> Result<bool> {
        if self
            .record
            .as_ref()
            .is_some_and(|record| record.phase == Phase::Dispatched)
        {
            self.finish()?;
            return Ok(false);
        }
        self.recover()
    }
}

pub(crate) fn recover_pending(backend: &SecretBackend) -> Result<()> {
    let (directory, _, _, _) = backend.oauth_custody()?;
    for name in directory.names()? {
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(".nyxid-storage-oauth-") || !name.ends_with(".json") {
            continue;
        }
        let bytes = directory.read(OsStr::new(name), MAX_CONFIG_BYTES)?;
        let record: Record = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        let (_, current) = backend.current_reader()?;
        let mut rotation = Rotation::open(&current, &record.service)?;
        if rotation.name != name {
            return Err(invalid());
        }
        rotation.recover()?;
    }
    Ok(())
}

/// An encrypted recovery record moves with the backend. Neither capture nor
/// relocation contacts a provider. The original service lease is held by the
/// migration from validation through publication and cleanup.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryMove {
    name: String,
    source_bytes: String,
    target_bytes: Option<String>,
    key_digest: Option<[u8; 32]>,
    target_file: bool,
}

fn binding_for(path: &std::path::Path, node: &str, service: &str) -> Result<String> {
    let bytes = serde_json::to_vec(&(
        "nyxid-node-oauth-recovery-v1",
        path.file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(invalid)?,
        node,
        service,
    ))
    .map_err(|_| invalid())?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
fn read_envelope(
    directory: &Directory,
    keychain: &KeychainBackend,
    record: &Record,
) -> Result<Option<Zeroizing<Vec<u8>>>> {
    let name = Rotation::key_name(record);
    let bytes = if record.file_key {
        optional_read(directory, &name)?.map(Zeroizing::new)
    } else {
        keychain
            .get_optional(&name)
            .map_err(|_| invalid())?
            .map(|v| {
                let v = Zeroizing::new(v);
                if v.len() != 44 {
                    return Err(invalid());
                }
                base64::engine::general_purpose::STANDARD
                    .decode(v.as_bytes())
                    .map(Zeroizing::new)
                    .map_err(|_| invalid())
            })
            .transpose()?
    };
    if bytes.as_ref().is_some_and(|v| v.len() != 32) {
        return Err(invalid());
    }
    Ok(bytes)
}
fn aad_for(record: &Record) -> Result<Vec<u8>> {
    serde_json::to_vec(&(
        1,
        &record.binding,
        record.id,
        record.file_key,
        record.observed,
    ))
    .map_err(|_| invalid())
}
fn decrypt_record(record: &Record, key: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(record.ciphertext.as_ref().ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    if bytes.len() < 28 {
        return Err(invalid());
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| invalid())?;
    let plain = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&bytes[..12]),
                Payload {
                    msg: &bytes[12..],
                    aad: &aad_for(record)?,
                },
            )
            .map_err(|_| invalid())?,
    );
    // Validate the typed payload as well as authenticating its bytes.
    let _: Returned = serde_json::from_slice(&plain).map_err(|_| invalid())?;
    Ok(plain)
}
fn encrypt_record(record: &Record, key: &[u8], plain: &[u8]) -> Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| invalid())?;
    let mut nonce = [0; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let mut encrypted = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: &aad_for(record)?,
            },
        )
        .map_err(|_| invalid())?;
    let mut bytes = nonce.to_vec();
    bytes.append(&mut encrypted);
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}
impl RecoveryMove {
    /// A size-only upper bound, never persisted or used as authority. AES-GCM
    /// relocation preserves ciphertext length; decimal hash bytes may grow.
    pub(crate) fn reserve_target_capacity(&mut self) -> Result<()> {
        let mut record = self.source()?;
        record.file_key = self.target_file;
        record.observed = [255; 32];
        record.installed = record.installed.map(|_| [255; 32]);
        self.target_bytes = Some(serde_json::to_string(&record).map_err(|_| invalid())?);
        Ok(())
    }
    fn source(&self) -> Result<Record> {
        serde_json::from_str(&self.source_bytes).map_err(|_| invalid())
    }
    fn target(&self) -> Result<Record> {
        serde_json::from_str(self.target_bytes.as_ref().ok_or_else(invalid)?).map_err(|_| invalid())
    }
    pub(crate) fn capture_all(
        directory: &Directory,
        path: &std::path::Path,
        config: &NodeConfig,
        keychain: &KeychainBackend,
        target_file: bool,
    ) -> Result<(Vec<Self>, Vec<File>)> {
        let mut records = Vec::new();
        let mut leases = Vec::new();
        for name in directory.names()? {
            let Some(name) = name.to_str() else {
                continue;
            };
            if !name.starts_with(".nyxid-storage-oauth-") || !name.ends_with(".json") {
                continue;
            }
            let bytes = directory.read(OsStr::new(name), MAX_CONFIG_BYTES)?;
            let record: Record = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            let binding = binding_for(path, &config.node.id, &record.service)?;
            if record.version != 1
                || record.binding != binding
                || record.id.get_version_num() != 4
                || record.file_key != (config.storage_backend == "file")
                || name != format!(".nyxid-storage-oauth-{binding}.json")
                || (record.phase == Phase::Returned && record.ciphertext.is_none())
                || (matches!(record.phase, Phase::Reserved | Phase::Dispatched)
                    && record.ciphertext.is_some())
            {
                return Err(invalid());
            }
            leases.push(
                directory
                    .try_existing_lease(OsStr::new(&format!(
                        ".nyxid-storage-oauth-{binding}.lock"
                    )))?
                    .ok_or_else(pending)?,
            );
            let key = read_envelope(directory, keychain, &record)?;
            if matches!(record.phase, Phase::Dispatched | Phase::Returned) && key.is_none() {
                return Err(invalid());
            }
            if record.phase == Phase::Returned {
                let _ = decrypt_record(&record, key.as_ref().ok_or_else(invalid)?)?;
                if config.publication.snapshot() != Some(record.observed)
                    && config.publication.snapshot() != record.installed
                {
                    // Retiring original custody would remove the existing
                    // restore-and-recover path. Resolve this return first;
                    // migration must never authorize it for a stale baseline.
                    return Err(Error::Config("node_migration_oauth_recovery_required: restore the original node configuration and custody, recover the retained OAuth response, then rerun migration; no target secrets were staged".into()));
                }
            }
            if record.file_key != target_file {
                let mut target = record.clone();
                target.file_key = target_file;
                // Ownership is reserved in the migration journal before any
                // target write; an existing target is never adopted.
                if read_envelope(directory, keychain, &target)?.is_some() {
                    return Err(invalid());
                }
            }
            records.push(Self {
                name: name.into(),
                source_bytes: String::from_utf8(bytes).map_err(|_| invalid())?,
                target_bytes: None,
                key_digest: key.as_ref().map(|k| Sha256::digest(k.as_slice()).into()),
                target_file,
            });
        }
        Ok((records, leases))
    }
    pub(crate) fn hold(
        &self,
        directory: &Directory,
        path: &std::path::Path,
        node: &str,
    ) -> Result<File> {
        let source = self.source()?;
        let binding = binding_for(path, node, &source.service)?;
        if source.version != 1
            || source.id.get_version_num() != 4
            || source.binding != binding
            || self.name != format!(".nyxid-storage-oauth-{binding}.json")
        {
            return Err(invalid());
        }
        self.validate_record(directory)?;
        directory
            .try_existing_lease(OsStr::new(&format!(".nyxid-storage-oauth-{binding}.lock")))?
            .ok_or_else(pending)
    }
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
    pub(crate) fn validate_record(&self, directory: &Directory) -> Result<()> {
        let actual = directory.read(OsStr::new(&self.name), MAX_CONFIG_BYTES)?;
        if actual != self.source_bytes.as_bytes()
            && self
                .target_bytes
                .as_ref()
                .is_none_or(|v| actual != v.as_bytes())
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub(crate) fn prepare(
        &mut self,
        directory: &Directory,
        keychain: &KeychainBackend,
        observed: [u8; 32],
        installed: [u8; 32],
    ) -> Result<()> {
        let source = self.source()?;
        let key = read_envelope(directory, keychain, &source)?;
        if key
            .as_ref()
            .map(|v| <[u8; 32]>::from(Sha256::digest(v.as_slice())))
            != self.key_digest
        {
            return Err(invalid());
        }
        let mut target = source.clone();
        target.file_key = self.target_file;
        // Only a return authorized for the exact pre-migration generation may
        // follow its unchanged complete projection to the installed generation.
        // A stale return stays stale. Unknown dispatches always stay barriers.
        if target.observed == observed {
            target.observed = installed;
        }
        if target.installed == Some(observed) {
            target.installed = Some(installed);
        }
        if matches!(source.phase, Phase::Reserved | Phase::Complete) {
            target.ciphertext = None;
        }
        if source.phase == Phase::Returned {
            let key = key.as_ref().ok_or_else(invalid)?;
            let plain = decrypt_record(&source, key)?;
            target.ciphertext = Some(encrypt_record(&target, key, &plain)?);
        }
        self.target_bytes = Some(serde_json::to_string(&target).map_err(|_| invalid())?);
        Ok(())
    }
    pub(crate) fn validate_plan(&self, source_hash: [u8; 32], target_hash: [u8; 32]) -> Result<()> {
        let source = self.source()?;
        if self.target_bytes.is_none() {
            return Ok(());
        }
        let target = self.target()?;
        let mut expected = source.clone();
        expected.file_key = self.target_file;
        if expected.observed == source_hash {
            expected.observed = target_hash;
        }
        if expected.installed == Some(source_hash) {
            expected.installed = Some(target_hash);
        }
        expected.ciphertext = if matches!(source.phase, Phase::Reserved | Phase::Complete) {
            None
        } else {
            target.ciphertext.clone()
        };
        if serde_json::to_vec(&expected).map_err(|_| invalid())?
            != serde_json::to_vec(&target).map_err(|_| invalid())?
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub(crate) fn stage(&self, directory: &Directory, keychain: &KeychainBackend) -> Result<()> {
        let source = self.source()?;
        let target = self.target()?;
        if source.file_key == target.file_key || self.key_digest.is_none() {
            return Ok(());
        }
        let key = read_envelope(directory, keychain, &source)?.ok_or_else(invalid)?;
        if Some(Sha256::digest(key.as_slice()).into()) != self.key_digest {
            return Err(invalid());
        }
        match read_envelope(directory, keychain, &target)? {
            Some(actual) if actual == key => {}
            Some(_) => return Err(invalid()),
            None if target.file_key => {
                directory.publish(OsStr::new(&Rotation::key_name(&target)), &key, false)?
            }
            None => {
                let encoded = Zeroizing::new(
                    base64::engine::general_purpose::STANDARD.encode(key.as_slice()),
                );
                keychain
                    .set(&Rotation::key_name(&target), &encoded)
                    .map_err(|_| invalid())?;
            }
        }
        #[cfg(test)]
        if std::env::var_os("NYXID_TEST_MIGRATION_EXIT_AFTER_ENVELOPE_PUT").is_some() {
            std::process::exit(0);
        }
        self.validate_target(directory, keychain)
    }
    pub(crate) fn validate_target(
        &self,
        directory: &Directory,
        keychain: &KeychainBackend,
    ) -> Result<()> {
        let target = self.target()?;
        let source = self.source()?;
        if source.id != target.id
            || source.binding != target.binding
            || source.service != target.service
            || source.phase != target.phase
            || target.file_key != self.target_file
        {
            return Err(invalid());
        }
        let key = read_envelope(directory, keychain, &target)?;
        if key
            .as_ref()
            .map(|v| <[u8; 32]>::from(Sha256::digest(v.as_slice())))
            != self.key_digest
        {
            return Err(invalid());
        }
        if target.phase == Phase::Returned {
            let _ = decrypt_record(&target, key.as_ref().ok_or_else(invalid)?)?;
        }
        Ok(())
    }
    pub(crate) fn install(&self, directory: &Directory) -> Result<()> {
        self.validate_record(directory)?;
        let target = self.target_bytes.as_ref().ok_or_else(invalid)?;
        directory.publish(OsStr::new(&self.name), target.as_bytes(), true)
    }
    pub(crate) fn validate_cleanup(
        &self,
        directory: &Directory,
        keychain: &KeychainBackend,
        cleaning: bool,
    ) -> Result<()> {
        self.validate_target(directory, keychain)?;
        let source = self.source()?;
        if source.file_key == self.target_file || self.key_digest.is_none() {
            return Ok(());
        }
        let actual = read_envelope(directory, keychain, &source)?;
        if actual.is_none() && cleaning {
            return Ok(());
        }
        if actual
            .as_ref()
            .map(|v| <[u8; 32]>::from(Sha256::digest(v.as_slice())))
            != self.key_digest
        {
            return Err(invalid());
        }
        if source.phase == Phase::Returned {
            let key = actual.as_ref().ok_or_else(invalid)?;
            let target = self.target()?;
            if decrypt_record(&source, key)? != decrypt_record(&target, key)? {
                return Err(invalid());
            }
        }
        Ok(())
    }
    pub(crate) fn retire(&self, directory: &Directory, keychain: &KeychainBackend) -> Result<()> {
        let source = self.source()?;
        if source.file_key == self.target_file || self.key_digest.is_none() {
            return Ok(());
        }
        if source.file_key {
            remove_if_present(directory, &Rotation::key_name(&source))
        } else {
            keychain
                .delete(&Rotation::key_name(&source))
                .map_err(|_| invalid())
        }
    }
}

fn apply(
    config: &mut NodeConfig,
    writer: &SecretBackend,
    service: &str,
    returned: Returned,
) -> Result<()> {
    use super::super::secret_fields::{CredentialPart, SecretField};
    let field = |part| SecretField::Credential {
        slug: service.into(),
        part,
    };
    let Returned {
        plan,
        token,
        received_at,
    } = returned;
    if let Plan::Login(plan) = plan {
        let LoginPlan {
            token_url,
            client_id,
            client_secret,
            scopes,
            auth_method,
            client_id_param_name,
            options,
            target_url,
        } = *plan;
        config.add_header_credential_via(
            service,
            "Authorization",
            "",
            target_url.as_deref(),
            writer,
        )?;
        let credential = config.credentials.get_mut(service).ok_or_else(invalid)?;
        credential.oauth_managed = true;
        credential.oauth_token_url = Some(token_url);
        credential.oauth_client_id_encrypted =
            writer.store_field(field(CredentialPart::OAuthClientId), &client_id)?;
        credential.oauth_client_secret_encrypted = client_secret
            .as_ref()
            .map(|secret| writer.store_field(field(CredentialPart::OAuthClientSecret), secret))
            .transpose()?
            .flatten();
        credential.oauth_scopes = scopes;
        credential.oauth_token_endpoint_auth_method = Some(auth_method);
        credential.oauth_client_id_param_name = client_id_param_name;
        credential.oauth_request_options = options;
    }
    let credential = config
        .credentials
        .get_mut(service)
        .filter(|c| c.oauth_managed)
        .ok_or_else(invalid)?;
    let header = Zeroizing::new(format!(
        "{} {}",
        token.token_type.as_deref().unwrap_or("Bearer"),
        token.access_token.as_str()
    ));
    credential.header_value_encrypted =
        writer.store_field(field(CredentialPart::Header), &header)?;
    credential.oauth_access_token_encrypted =
        writer.store_field(field(CredentialPart::OAuthAccess), &token.access_token)?;
    if let Some(refresh) = &token.refresh_token {
        credential.oauth_refresh_token_encrypted =
            writer.store_field(field(CredentialPart::OAuthRefresh), refresh)?;
    }
    credential.oauth_token_expires_at = token
        .expires_in
        .map(|seconds| {
            let seconds = i64::try_from(seconds).map_err(|_| invalid())?;
            received_at
                .checked_add_signed(chrono::Duration::try_seconds(seconds).ok_or_else(invalid)?)
                .map(|date| date.to_rfc3339())
                .ok_or_else(invalid)
        })
        .transpose()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::encryption::LocalEncryption;
    use super::super::super::secret_fields::{CredentialPart, SecretField};
    use super::*;

    fn token() -> TokenResponse {
        serde_json::from_value(serde_json::json!({
            "access_token": "returned-access-SENTINEL", "refresh_token": "returned-refresh-SENTINEL", "expires_in": 3600
        })).unwrap()
    }
    fn fixture() -> (tempfile::TempDir, PathBuf, KeychainBackend) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let file = LocalEncryption::initialize(dir.path()).unwrap();
        let mut config =
            NodeConfig::new("wss://example.invalid".into(), "node".into(), "file".into());
        config.set_auth_token("synthetic-auth", &file).unwrap();
        config
            .add_header_credential("service", "Authorization", "Bearer original", &file)
            .unwrap();
        let credential = config.credentials.get_mut("service").unwrap();
        credential.oauth_managed = true;
        credential.oauth_access_token_encrypted = Some(file.encrypt("original-access").unwrap());
        credential.oauth_refresh_token_encrypted = Some(file.encrypt("original-refresh").unwrap());
        credential.oauth_client_id_encrypted = Some(file.encrypt("synthetic-client").unwrap());
        credential.oauth_token_url = Some("https://provider.invalid/token".into());
        credential.oauth_token_expires_at = Some("2000-01-01T00:00:00Z".into());
        config.save(&path).unwrap();
        // Proves the file path is usable with NO key-service implementation.
        let store = KeychainBackend::new_failing_mock("node", "key-service-unavailable-SENTINEL");
        (dir, path, store)
    }
    fn reader(path: &std::path::Path, store: &KeychainBackend) -> SecretBackend {
        SecretBackend::reader_with_keychain(
            &NodeConfig::load(path).unwrap(),
            path.parent().unwrap(),
            store.clone(),
        )
        .unwrap()
    }
    fn assert_tokens(path: &std::path::Path, store: &KeychainBackend) {
        let config = NodeConfig::load(path).unwrap();
        let reader = reader(path, store);
        for (part, expected) in [
            (CredentialPart::OAuthAccess, "returned-access-SENTINEL"),
            (CredentialPart::OAuthRefresh, "returned-refresh-SENTINEL"),
            (CredentialPart::Header, "Bearer returned-access-SENTINEL"),
        ] {
            assert!(
                reader
                    .require_field(
                        &config,
                        &SecretField::Credential {
                            slug: "service".into(),
                            part
                        }
                    )
                    .unwrap()
                    .as_str()
                    == expected
            );
        }
    }
    fn no_plaintext(dir: &std::path::Path) {
        for file in std::fs::read_dir(dir).unwrap() {
            let bytes = std::fs::read(file.unwrap().path()).unwrap();
            for secret in [
                "returned-access-SENTINEL",
                "returned-refresh-SENTINEL",
                "client-secret-SENTINEL",
            ] {
                assert!(
                    !bytes
                        .windows(secret.len())
                        .any(|window| window == secret.as_bytes())
                );
            }
        }
    }

    #[test]
    fn returned_tokens_survive_restart_and_recover_without_key_service() {
        let (dir, path, store) = fixture();
        let backend = reader(&path, &store);
        let mut rotation = Rotation::open(&backend, "service").unwrap();
        rotation.dispatch().unwrap();
        rotation.retain(Plan::Refresh, token()).unwrap();
        assert!(Rotation::open(&backend, "service").is_err());
        no_plaintext(dir.path());
        drop(rotation);
        recover_pending(&reader(&path, &store)).unwrap();
        assert_tokens(&path, &store);
        recover_pending(&reader(&path, &store)).unwrap();
        no_plaintext(dir.path());
    }

    #[test]
    fn recovery_uses_current_generation_through_an_older_runtime_reader() {
        let (_dir, path, store) = fixture();
        let runtime = reader(&path, &store);
        let mut config = NodeConfig::load(&path).unwrap();
        config.server.proxy_max_body_size += 1;
        config.save(&path).unwrap();
        let (_, current) = runtime.current_reader().unwrap();
        let mut rotation = Rotation::open(&current, "service").unwrap();
        rotation.dispatch().unwrap();
        rotation.retain(Plan::Refresh, token()).unwrap();
        drop(rotation);
        recover_pending(&runtime).unwrap();
        assert_tokens(&path, &store);
    }

    #[test]
    fn response_journal_and_publication_keep_the_original_directory_after_path_retarget() {
        let (dir, path, store) = fixture();
        let backend = reader(&path, &store);
        let mut rotation = Rotation::open(&backend, "service").unwrap();
        rotation.dispatch().unwrap();
        let moved = dir.path().with_extension("moved");
        std::fs::rename(dir.path(), &moved).unwrap();
        Directory::create_private(dir.path()).unwrap();
        rotation.retain(Plan::Refresh, token()).unwrap();
        rotation.recover().unwrap();
        assert!(
            Directory::open(dir.path())
                .unwrap()
                .names()
                .unwrap()
                .is_empty()
        );
        assert_tokens(&moved.join("config.toml"), &store);
        no_plaintext(&moved);
        drop(rotation);
        std::fs::remove_dir(dir.path()).unwrap();
        std::fs::rename(&moved, dir.path()).unwrap();
    }

    #[test]
    fn keychain_returned_rotation_is_retained_despite_live_object_loss_and_key_service_outage() {
        use super::super::super::secret_snapshot::SecretSnapshot;
        use super::super::super::secret_transaction::{Attempt, ObjectRef, ObjectStore};
        let (dir, path, _) = fixture();
        let source = NodeConfig::load(&path).unwrap();
        let directory = Directory::open(dir.path()).unwrap();
        let snapshot =
            SecretSnapshot::read_in(&directory, &source, &path, None, None, None).unwrap();
        let store = KeychainBackend::new_mock("node");
        let attempt =
            Attempt::begin_in(&directory, &path, source.publication.snapshot(), "node").unwrap();
        let modern = snapshot
            .project_keychain(&source, &attempt, &store)
            .unwrap();
        modern.save(&path).unwrap();
        drop(attempt);
        let backend = reader(&path, &store);
        let mut rotation = Rotation::open(&backend, "service").unwrap();
        rotation.dispatch().unwrap();
        let envelope_name = Rotation::key_name(rotation.record.as_ref().unwrap());
        assert!(envelope_name.starts_with("object/v1/oauth-recovery/"));
        let modern = NodeConfig::load(&path).unwrap();
        let reference = ObjectRef::parse(&modern.node.auth_token_encrypted)
            .unwrap()
            .unwrap();
        let original = ObjectStore::get(&store, reference).unwrap().unwrap();
        ObjectStore::delete(&store, reference).unwrap();
        // Even if key-service reads now fail, the reserved in-memory envelope
        // key can durably retain the returned response in the pinned directory.
        rotation.keychain =
            KeychainBackend::new_failing_mock("node", "key-service-outage-SENTINEL");
        rotation.retain(Plan::Refresh, token()).unwrap();
        assert!(rotation.recover().is_err());
        assert!(rotation.dispatch().is_err());
        no_plaintext(dir.path());
        drop(rotation);
        ObjectStore::put(&store, reference, &original).unwrap();
        recover_pending(&reader(&path, &store)).unwrap();
        assert_tokens(&path, &store);
        assert!(store.get_optional(&envelope_name).unwrap().is_none());
    }

    #[test]
    fn omitted_rotated_refresh_token_preserves_the_existing_token() {
        let (_dir, path, store) = fixture();
        let backend = reader(&path, &store);
        let mut rotation = Rotation::open(&backend, "service").unwrap();
        rotation.dispatch().unwrap();
        let mut returned = token();
        returned.refresh_token = None;
        rotation.retain(Plan::Refresh, returned).unwrap();
        rotation.recover().unwrap();
        let config = NodeConfig::load(&path).unwrap();
        let reader = reader(&path, &store);
        assert!(
            reader
                .require_field(
                    &config,
                    &SecretField::Credential {
                        slug: "service".into(),
                        part: CredentialPart::OAuthRefresh
                    }
                )
                .unwrap()
                .as_str()
                == "original-refresh"
        );
    }

    #[test]
    fn stale_publication_and_selected_key_loss_retain_rotation_and_refuse_replay() {
        for loss in [false, true] {
            let (dir, path, store) = fixture();
            let before = std::fs::read(&path).unwrap();
            let key = Zeroizing::new(std::fs::read(dir.path().join(".keyfile")).unwrap());
            let backend = reader(&path, &store);
            let mut rotation = Rotation::open(&backend, "service").unwrap();
            rotation.dispatch().unwrap();
            if loss {
                std::fs::remove_file(dir.path().join(".keyfile")).unwrap();
            } else {
                let mut changed = NodeConfig::load(&path).unwrap();
                changed.server.proxy_max_body_size += 1;
                changed.save(&path).unwrap();
            }
            // Retention succeeds despite lost publication permission/custody.
            rotation.retain(Plan::Refresh, token()).unwrap();
            assert!(rotation.recover().is_err());
            assert!(rotation.dispatch().is_err());
            assert!(rotation.recover_for_login().is_err());
            assert!(rotation.record.as_ref().unwrap().phase == Phase::Returned);
            no_plaintext(dir.path());
            drop(rotation);
            if loss {
                Directory::open(dir.path())
                    .unwrap()
                    .publish(OsStr::new(".keyfile"), key.as_slice(), false)
                    .unwrap();
            } else {
                std::fs::write(&path, before).unwrap();
            }
            recover_pending(&reader(&path, &store)).unwrap();
            assert_tokens(&path, &store);
        }
    }

    #[test]
    fn unknown_dispatch_never_retries_refresh_but_explicit_login_can_reauthorize() {
        let (_dir, path, store) = fixture();
        let backend = reader(&path, &store);
        let mut rotation = Rotation::open(&backend, "service").unwrap();
        rotation.dispatch().unwrap();
        drop(rotation);
        let mut resumed = Rotation::open(&backend, "service").unwrap();
        assert!(
            resumed
                .recover()
                .unwrap_err()
                .to_string()
                .contains("oauth_recovery_pending")
        );
        assert!(resumed.dispatch().is_err());
        assert!(!resumed.recover_for_login().unwrap());
        resumed.dispatch().unwrap();
        assert!(resumed.record.as_ref().unwrap().phase == Phase::Dispatched);
    }

    #[test]
    fn rotation_open_does_not_adopt_a_new_generation_for_old_provider_inputs() {
        let (_dir, path, store) = fixture();
        let backend = reader(&path, &store);
        let mut changed = NodeConfig::load(&path).unwrap();
        changed.server.proxy_max_body_size += 1;
        changed.save(&path).unwrap();
        assert!(Rotation::open(&backend, "service").is_err());
    }

    #[tokio::test]
    async fn provider_rotation_during_key_loss_is_retained_then_recovered_without_recontact() {
        use wiremock::matchers::{method, path as match_path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let (dir, path, store) = fixture();
        let key_path = dir.path().join(".keyfile");
        let key = Zeroizing::new(std::fs::read(&key_path).unwrap());
        let mut config = NodeConfig::load(&path).unwrap();
        config
            .credentials
            .get_mut("service")
            .unwrap()
            .oauth_token_url = Some(format!("{}/token", server.uri()));
        config.save(&path).unwrap();
        let remove_key = key_path.clone();
        Mock::given(method("POST")).and(match_path("/token"))
            .respond_with(move |_: &wiremock::Request| {
                std::fs::remove_file(&remove_key).unwrap();
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "access_token": "returned-access-SENTINEL", "refresh_token": "returned-refresh-SENTINEL", "expires_in": 3600
                }))
            }).expect(1).mount(&server).await;
        let backend = reader(&path, &store);
        assert!(
            super::super::super::agent::oauth_refresh_once(&backend)
                .await
                .is_err()
        );
        assert!(!key_path.exists());
        assert!(
            super::super::super::agent::oauth_refresh_once(&backend)
                .await
                .is_err()
        );
        no_plaintext(dir.path());
        Directory::open(dir.path())
            .unwrap()
            .publish(OsStr::new(".keyfile"), key.as_slice(), false)
            .unwrap();
        super::super::super::agent::oauth_refresh_once(&backend)
            .await
            .unwrap();
        assert_tokens(&path, &store);
        server.verify().await;
    }

    #[test]
    fn returned_storage_failure_propagates_and_cannot_acknowledge_or_repeat_dispatch() {
        let (dir, path, store) = fixture();
        let backend = reader(&path, &store);
        let mut rotation = Rotation::open(&backend, "service").unwrap();
        rotation.dispatch().unwrap();
        let record_path = dir.path().join(&rotation.name);
        std::fs::remove_file(&record_path).unwrap();
        std::fs::create_dir(&record_path).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(rotation.retain(Plan::Refresh, token()).is_err());
        assert!(rotation.dispatch().is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        // The live attempt still owns the encrypted returned response and can
        // retry persistence; it must never retry the provider to repair disk.
        std::fs::remove_dir(&record_path).unwrap();
        rotation.persist().unwrap();
        rotation.recover().unwrap();
        assert_tokens(&path, &store);
    }

    #[tokio::test]
    async fn durable_retention_waits_for_storage_repair_without_losing_response_or_lease() {
        let (dir, path, store) = fixture();
        let backend = reader(&path, &store);
        let mut rotation = Rotation::open(&backend, "service").unwrap();
        rotation.dispatch().unwrap();
        let record_path = dir.path().join(&rotation.name);
        std::fs::remove_file(&record_path).unwrap();
        std::fs::create_dir(&record_path).unwrap();
        let before = std::fs::read(&path).unwrap();
        {
            let retention = rotation.retain_durable(Plan::Refresh, token());
            tokio::pin!(retention);
            tokio::select! {
                result = &mut retention => panic!("unpersisted response must stay held: {result:?}"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
            }
            assert!(Rotation::open(&backend, "service").is_err());
            assert_eq!(std::fs::read(&path).unwrap(), before);
            std::fs::remove_dir(&record_path).unwrap();
            retention.await.unwrap();
        }
        rotation.recover().unwrap();
        assert_tokens(&path, &store);
    }

    #[test]
    fn retained_state_corruption_or_missing_envelope_key_blocks_publication_and_replay() {
        for corruption in ["ciphertext", "key", "owner-binding"] {
            let (dir, path, store) = fixture();
            let backend = reader(&path, &store);
            let mut rotation = Rotation::open(&backend, "service").unwrap();
            rotation.dispatch().unwrap();
            rotation.retain(Plan::Refresh, token()).unwrap();
            let before = std::fs::read(&path).unwrap();
            if corruption == "key" {
                std::fs::remove_file(
                    dir.path()
                        .join(Rotation::key_name(rotation.record.as_ref().unwrap())),
                )
                .unwrap();
            } else {
                let record = rotation.record.as_mut().unwrap();
                if corruption == "ciphertext" {
                    record.ciphertext = Some("invalid-ciphertext".into());
                } else {
                    record.binding = "other-node".into();
                }
                rotation.persist().unwrap();
            }
            drop(rotation);
            assert!(recover_pending(&backend).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
    }

    #[test]
    fn recovery_never_recreates_missing_ownership_lease() {
        let (dir, path, store) = fixture();
        let backend = reader(&path, &store);
        let mut rotation = Rotation::open(&backend, "service").unwrap();
        rotation.dispatch().unwrap();
        rotation.retain(Plan::Refresh, token()).unwrap();
        let lock = dir
            .path()
            .join(format!(".nyxid-storage-oauth-{}.lock", rotation.binding));
        drop(rotation);
        std::fs::remove_file(&lock).unwrap();
        let before = Directory::open(dir.path()).unwrap().names().unwrap();
        assert!(recover_pending(&backend).is_err());
        assert!(!lock.exists());
        assert_eq!(
            Directory::open(dir.path()).unwrap().names().unwrap(),
            before
        );
    }

    #[test]
    fn oauth_crash_child() {
        let Some(path) = std::env::var_os("NYXID_TEST_OAUTH_CRASH_CONFIG") else {
            return;
        };
        let path = PathBuf::from(path);
        let store = KeychainBackend::new_failing_mock("node", "no-key-service");
        let backend = reader(&path, &store);
        let mut rotation = Rotation::open(&backend, "service").unwrap();
        rotation.dispatch().unwrap();
        rotation.retain(Plan::Refresh, token()).unwrap();
        if std::env::var_os("NYXID_TEST_OAUTH_EXIT_AFTER_PUBLICATION").is_some() {
            rotation.recover().unwrap();
            panic!("test exit hook did not run");
        }
        // Deliberate process exit: no Rust Drop/cleanup runs.
        std::process::exit(0);
    }

    #[test]
    fn process_exit_after_response_or_install_resumes_without_another_provider_request() {
        for installed in [false, true] {
            let (_dir, path, store) = fixture();
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "node::oauth::storage::tests::oauth_crash_child",
                    "--nocapture",
                ])
                .env("NYXID_TEST_OAUTH_CRASH_CONFIG", &path);
            if installed {
                child.env("NYXID_TEST_OAUTH_EXIT_AFTER_PUBLICATION", "1");
            }
            assert!(child.status().unwrap().success());
            let before = std::fs::read(&path).unwrap();
            recover_pending(&reader(&path, &store)).unwrap();
            if installed {
                assert_eq!(std::fs::read(&path).unwrap(), before);
            }
            assert_tokens(&path, &store);
        }
    }
}
