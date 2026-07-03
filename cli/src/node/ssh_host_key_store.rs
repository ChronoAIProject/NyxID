use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use chrono::Utc;
use serde::{Deserialize, Serialize};

const STORE_FILE_NAME: &str = "ssh_cert_host_keys.toml";
const STORE_VERSION: u32 = 1;

pub type SharedCertHostKeyStore = std::sync::Arc<std::sync::Mutex<CertHostKeyStore>>;

#[derive(Debug, thiserror::Error)]
pub enum CertHostKeyStoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML serialization error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("TOML deserialization error: {0}")]
    TomlDeserialize(#[from] toml::de::Error),

    #[error("invalid SSH host-key fingerprint")]
    InvalidFingerprint,

    #[error("SSH host key mismatch: expected {expected}, got {observed}")]
    HostKeyMismatch { expected: String, observed: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostKeyDecision {
    MatchedExisting,
    PinnedNew,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CertHostKeySource {
    Tofu,
    Explicit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertHostKeyEntry {
    pub host: String,
    pub port: u16,
    pub host_key_sha256: String,
    pub source: CertHostKeySource,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default = "default_store_version")]
    version: u32,
    #[serde(default)]
    hosts: Vec<CertHostKeyEntry>,
}

impl Default for StoreFile {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            hosts: Vec::new(),
        }
    }
}

fn default_store_version() -> u32 {
    STORE_VERSION
}

pub struct CertHostKeyStore {
    path: PathBuf,
    file: StoreFile,
}

impl fmt::Debug for CertHostKeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CertHostKeyStore")
            .field("path", &self.path)
            .field("entries", &self.file.hosts.len())
            .finish()
    }
}

impl CertHostKeyStore {
    pub fn load(config_dir: &Path) -> Result<Self, CertHostKeyStoreError> {
        let path = store_path(config_dir);
        let file = load_store_file(&path)?;
        Ok(Self { path, file })
    }

    pub fn entries(&self) -> &[CertHostKeyEntry] {
        &self.file.hosts
    }

    fn reload(&mut self) -> Result<(), CertHostKeyStoreError> {
        self.file = load_store_file(&self.path)?;
        Ok(())
    }

    pub fn get(&self, host: &str, port: u16) -> Option<&CertHostKeyEntry> {
        let normalized_host = normalize_host(host);
        self.file
            .hosts
            .iter()
            .find(|entry| entry.host == normalized_host && entry.port == port)
    }

    pub fn ensure_matches_or_pin(
        &mut self,
        host: &str,
        port: u16,
        observed_sha256: &str,
    ) -> Result<HostKeyDecision, CertHostKeyStoreError> {
        self.reload()?;
        let normalized_host = normalize_host(host);
        let observed = canonical_sha256_fingerprint(observed_sha256)?;

        if let Some(entry) = self.entry_mut(&normalized_host, port) {
            if fingerprints_equal(&entry.host_key_sha256, &observed) {
                return Ok(HostKeyDecision::MatchedExisting);
            }

            return Err(CertHostKeyStoreError::HostKeyMismatch {
                expected: canonical_sha256_fingerprint(&entry.host_key_sha256)?,
                observed,
            });
        }

        let now = Utc::now().to_rfc3339();
        self.file.hosts.push(CertHostKeyEntry {
            host: normalized_host,
            port,
            host_key_sha256: observed,
            source: CertHostKeySource::Tofu,
            created_at: now.clone(),
            updated_at: now,
        });
        self.sort_entries();
        self.save()?;
        Ok(HostKeyDecision::PinnedNew)
    }

    pub fn set_explicit_pin(
        &mut self,
        host: &str,
        port: u16,
        fingerprint: &str,
    ) -> Result<(), CertHostKeyStoreError> {
        self.reload()?;
        let normalized_host = normalize_host(host);
        let fingerprint = canonical_sha256_fingerprint(fingerprint)?;
        let now = Utc::now().to_rfc3339();

        if let Some(entry) = self.entry_mut(&normalized_host, port) {
            entry.host_key_sha256 = fingerprint;
            entry.source = CertHostKeySource::Explicit;
            entry.updated_at = now;
        } else {
            self.file.hosts.push(CertHostKeyEntry {
                host: normalized_host,
                port,
                host_key_sha256: fingerprint,
                source: CertHostKeySource::Explicit,
                created_at: now.clone(),
                updated_at: now,
            });
            self.sort_entries();
        }

        self.save()
    }

    pub fn forget(
        &mut self,
        host: &str,
        port: u16,
    ) -> Result<Option<CertHostKeyEntry>, CertHostKeyStoreError> {
        self.reload()?;
        let normalized_host = normalize_host(host);
        let removed = self
            .file
            .hosts
            .iter()
            .position(|entry| entry.host == normalized_host && entry.port == port)
            .map(|index| self.file.hosts.remove(index));

        if removed.is_some() {
            self.save()?;
        }

        Ok(removed)
    }

    fn entry_mut(&mut self, host: &str, port: u16) -> Option<&mut CertHostKeyEntry> {
        self.file
            .hosts
            .iter_mut()
            .find(|entry| entry.host == host && entry.port == port)
    }

    fn sort_entries(&mut self) {
        self.file
            .hosts
            .sort_by(|a, b| (&a.host, a.port).cmp(&(&b.host, b.port)));
    }

    fn save(&self) -> Result<(), CertHostKeyStoreError> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let content = toml::to_string_pretty(&self.file)?;
        let tmp_path = parent.join(format!(
            ".{}.tmp",
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(STORE_FILE_NAME)
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
        }

        #[cfg(not(unix))]
        {
            std::fs::write(&tmp_path, content.as_bytes())?;
        }

        std::fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }
}

fn load_store_file(path: &Path) -> Result<StoreFile, CertHostKeyStoreError> {
    if !path.exists() {
        return Ok(StoreFile::default());
    }

    let content = std::fs::read_to_string(path)?;
    let mut file: StoreFile = toml::from_str(&content)?;
    file.hosts
        .sort_by(|a, b| (&a.host, a.port).cmp(&(&b.host, b.port)));
    Ok(file)
}

pub fn store_path(config_dir: &Path) -> PathBuf {
    config_dir.join(STORE_FILE_NAME)
}

fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

pub fn canonical_sha256_fingerprint(fingerprint: &str) -> Result<String, CertHostKeyStoreError> {
    let normalized = normalize_sha256_fingerprint(fingerprint);
    if normalized.len() != 43
        || !normalized
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    {
        return Err(CertHostKeyStoreError::InvalidFingerprint);
    }

    let decoded = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(&normalized)
        .map_err(|_| CertHostKeyStoreError::InvalidFingerprint)?;
    if decoded.len() != 32 {
        return Err(CertHostKeyStoreError::InvalidFingerprint);
    }

    Ok(format!("SHA256:{normalized}"))
}

fn normalize_sha256_fingerprint(fingerprint: &str) -> String {
    let trimmed = fingerprint.trim();
    let without_prefix = trimmed
        .strip_prefix("SHA256:")
        .or_else(|| trimmed.strip_prefix("sha256:"))
        .unwrap_or(trimmed);
    without_prefix.trim_end_matches('=').to_string()
}

fn fingerprints_equal(left: &str, right: &str) -> bool {
    normalize_sha256_fingerprint(left) == normalize_sha256_fingerprint(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const KEY_B: &str = "SHA256://////////////////////////////////////////8";

    #[test]
    fn tofu_first_connect_accepts_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = CertHostKeyStore::load(dir.path()).unwrap();

        let decision = store
            .ensure_matches_or_pin("SSH.Example.", 2222, KEY_A)
            .unwrap();

        assert_eq!(decision, HostKeyDecision::PinnedNew);
        let reloaded = CertHostKeyStore::load(dir.path()).unwrap();
        let entry = reloaded.get("ssh.example", 2222).unwrap();
        assert_eq!(entry.host, "ssh.example");
        assert_eq!(entry.host_key_sha256, KEY_A);
        assert_eq!(entry.source, CertHostKeySource::Tofu);
    }

    #[test]
    fn tofu_second_connect_matches_existing_pin() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = CertHostKeyStore::load(dir.path()).unwrap();
        store
            .ensure_matches_or_pin("ssh.example", 22, KEY_A)
            .unwrap();

        let decision = store
            .ensure_matches_or_pin(
                "ssh.example",
                22,
                "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
            )
            .unwrap();

        assert_eq!(decision, HostKeyDecision::MatchedExisting);
    }

    #[test]
    fn changed_key_rejects_existing_tofu_pin() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = CertHostKeyStore::load(dir.path()).unwrap();
        store
            .ensure_matches_or_pin("ssh.example", 22, KEY_A)
            .unwrap();

        let err = store
            .ensure_matches_or_pin("ssh.example", 22, KEY_B)
            .unwrap_err();

        assert!(matches!(
            err,
            CertHostKeyStoreError::HostKeyMismatch { expected, observed }
                if expected == KEY_A && observed == KEY_B
        ));
    }

    #[test]
    fn explicit_pin_match_is_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = CertHostKeyStore::load(dir.path()).unwrap();
        store
            .set_explicit_pin(
                "[SSH.Example]",
                22,
                "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
            )
            .unwrap();

        let decision = store
            .ensure_matches_or_pin("ssh.example.", 22, KEY_A)
            .unwrap();

        assert_eq!(decision, HostKeyDecision::MatchedExisting);
        let entry = store.get("ssh.example", 22).unwrap();
        assert_eq!(entry.source, CertHostKeySource::Explicit);
    }

    #[test]
    fn explicit_pin_mismatch_rejects_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = CertHostKeyStore::load(dir.path()).unwrap();
        store.set_explicit_pin("ssh.example", 22, KEY_A).unwrap();

        let err = store
            .ensure_matches_or_pin("ssh.example", 22, KEY_B)
            .unwrap_err();

        assert!(matches!(
            err,
            CertHostKeyStoreError::HostKeyMismatch { expected, observed }
                if expected == KEY_A && observed == KEY_B
        ));
        assert_eq!(store.get("ssh.example", 22).unwrap().host_key_sha256, KEY_A);
        assert_eq!(
            store.get("ssh.example", 22).unwrap().source,
            CertHostKeySource::Explicit
        );
    }

    #[test]
    fn running_store_reloads_external_explicit_pin_on_verify() {
        let dir = tempfile::tempdir().unwrap();
        let mut running_store = CertHostKeyStore::load(dir.path()).unwrap();
        running_store
            .ensure_matches_or_pin("ssh.example", 22, KEY_A)
            .unwrap();

        let mut cli_store = CertHostKeyStore::load(dir.path()).unwrap();
        cli_store
            .set_explicit_pin("ssh.example", 22, KEY_B)
            .unwrap();

        let err = running_store
            .ensure_matches_or_pin("ssh.example", 22, KEY_A)
            .unwrap_err();

        assert!(matches!(
            err,
            CertHostKeyStoreError::HostKeyMismatch { expected, observed }
                if expected == KEY_B && observed == KEY_A
        ));
        let reloaded = CertHostKeyStore::load(dir.path()).unwrap();
        assert_eq!(
            reloaded.get("ssh.example", 22).unwrap().source,
            CertHostKeySource::Explicit
        );
    }

    #[test]
    fn running_store_reloads_external_forget_and_retofus() {
        let dir = tempfile::tempdir().unwrap();
        let mut running_store = CertHostKeyStore::load(dir.path()).unwrap();
        running_store
            .ensure_matches_or_pin("ssh.example", 22, KEY_A)
            .unwrap();

        let mut cli_store = CertHostKeyStore::load(dir.path()).unwrap();
        let removed = cli_store.forget("ssh.example", 22).unwrap();
        assert!(removed.is_some());

        let decision = running_store
            .ensure_matches_or_pin("ssh.example", 22, KEY_B)
            .unwrap();

        assert_eq!(decision, HostKeyDecision::PinnedNew);
        let reloaded = CertHostKeyStore::load(dir.path()).unwrap();
        let entry = reloaded.get("ssh.example", 22).unwrap();
        assert_eq!(entry.host_key_sha256, KEY_B);
        assert_eq!(entry.source, CertHostKeySource::Tofu);
    }

    #[test]
    fn invalid_fingerprint_is_rejected() {
        for fingerprint in [
            "",
            "SHA256:",
            "SHA256:not-base64",
            "MD5:aa:bb:cc",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI...",
        ] {
            assert!(matches!(
                canonical_sha256_fingerprint(fingerprint),
                Err(CertHostKeyStoreError::InvalidFingerprint)
            ));
        }
    }
}
