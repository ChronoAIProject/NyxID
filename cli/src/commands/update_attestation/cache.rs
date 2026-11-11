//! Durable TUF state. tough's datastore is a scratch workspace: it truncates
//! files and ignores malformed cached metadata, and does not bootstrap from its
//! cached root. Keep authenticated state in one atomic, locked generation.
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tough::{
    Repository, RepositoryLoader, Transport, TransportError,
    schema::{Root, Signed, Snapshot, Timestamp},
};

#[path = "storage.rs"]
mod storage;
use storage::Directory;

const MAX_STATE: u64 = 16 * 1024 * 1024;
const STATE: &str = "trusted-state-v1.json";

#[cfg(test)]
pub(super) fn test_directory() -> tempfile::TempDir {
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    builder.tempdir().unwrap()
}

pub(super) fn native_path() -> Result<PathBuf> {
    resolve_native_path(
        std::env::var_os("NYXID_TUF_STATE_DIR"),
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }),
    )
}

fn resolve_native_path(
    configured: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Result<PathBuf> {
    let path = if let Some(path) = configured {
        ensure!(!path.is_empty(), "NYXID_TUF_STATE_DIR must not be empty");
        PathBuf::from(path)
    } else {
        let home = home.context("No home directory; configure a private NYXID_TUF_STATE_DIR")?;
        ensure!(!home.is_empty(), "No home directory for TUF state");
        PathBuf::from(home).join(".nyxid").join("tuf-public")
    };
    ensure!(
        path.is_absolute(),
        "TUF state requires an absolute persistent path"
    );
    Ok(path)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    anchor: Signed<Root>,
    // Each root is cross-signed by its predecessor and self-signed. Keeping the
    // lineage also detects corrupt/replaced cached trust anchors.
    roots: Vec<Signed<Root>>,
    timestamp: Option<Signed<Timestamp>>,
    snapshot: Option<Signed<Snapshot>>,
    latest_time: DateTime<Utc>,
}

impl State {
    fn validate(&self) -> Result<Signed<Root>> {
        ensure!(
            self.version == 1 && self.roots.len() <= 1024,
            "Unsupported TUF state"
        );
        let mut root = self.anchor.clone();
        root.signed.verify_role(&root)?;
        for next in &self.roots {
            ensure!(
                Some(next.signed.version.get()) == root.signed.version.get().checked_add(1),
                "TUF root lineage is not contiguous"
            );
            root.signed
                .verify_role(next)
                .context("TUF root lineage signature invalid")?;
            next.signed.verify_role(next)?;
            root = next.clone();
        }
        if let Some(timestamp) = &self.timestamp {
            root.signed
                .verify_role(timestamp)
                .context("Cached TUF timestamp signature invalid")?;
        }
        if let Some(snapshot) = &self.snapshot {
            root.signed
                .verify_role(snapshot)
                .context("Cached TUF snapshot signature invalid")?;
        }
        Ok(root)
    }
}

pub(super) fn prepare_datastore(path: &Path) -> Result<()> {
    Directory::open(path)?.open_private(".state.lock", true)?;
    Ok(())
}

fn read_state(directory: &Directory, lock: &fs::File) -> Result<Option<State>> {
    let mut bytes = Vec::new();
    match directory.open_private(STATE, false) {
        Ok(file) => {
            ensure!(
                file.metadata()?.len() <= MAX_STATE,
                "TUF state exceeds limit"
            );
            file.take(MAX_STATE + 1).read_to_end(&mut bytes)?;
            ensure!(bytes.len() as u64 <= MAX_STATE, "TUF state exceeds limit");
            Ok(Some(serde_json::from_slice(&bytes).context(
                "Corrupt TUF state; refusing to reset rollback protection",
            )?))
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            ensure!(
                lock.metadata()?.len() == 0,
                "TUF state is missing after initialization; refusing to reset rollback protection"
            );
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn persist(directory: &Directory, lock: &mut fs::File, state: &State) -> Result<()> {
    let bytes = serde_json::to_vec(state)?;
    ensure!(bytes.len() as u64 <= MAX_STATE, "TUF state exceeds limit");
    directory.publish(STATE, &bytes)?;
    if lock.metadata()?.len() == 0 {
        lock.write_all(b"initialized\n")?;
        lock.sync_all()?;
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct Recording<T> {
    inner: T,
    roots: Arc<Mutex<BTreeMap<u64, Vec<u8>>>>,
}

#[tough::async_trait]
impl<T: Transport + Clone + 'static> Transport for Recording<T> {
    async fn fetch(
        &self,
        url: url::Url,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<tough::Bytes, TransportError>>,
        TransportError,
    > {
        let root_version = url
            .path()
            .rsplit('/')
            .next()
            .and_then(|file| file.strip_suffix(".root.json"))
            .and_then(|n| n.parse::<u64>().ok());
        let stream = self.inner.fetch(url).await?;
        let Some(version) = root_version else {
            return Ok(stream);
        };
        let roots = self.roots.clone();
        // tough imposes max_root_size before collecting this stream. Retain no
        // more than its 1MiB root bound, even for a malicious transport.
        let mut collected = Vec::new();
        Ok(Box::pin(stream.map_ok(move |chunk| {
            if collected.len() + chunk.len() <= 1024 * 1024 {
                collected.extend_from_slice(&chunk);
                roots
                    .lock()
                    .expect("root recorder lock")
                    .insert(version, collected.clone());
            }
            chunk
        })))
    }
}

fn optional<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    optional_file(options.open(path).map_err(Into::into))
}

fn optional_in<T: serde::de::DeserializeOwned>(
    directory: &Directory,
    name: &str,
) -> Result<Option<T>> {
    optional_file(directory.open_metadata(name))
}

fn optional_file<T: serde::de::DeserializeOwned>(file: Result<fs::File>) -> Result<Option<T>> {
    match file {
        Ok(file) => {
            ensure!(
                file.metadata()?.is_file() && file.metadata()?.len() <= MAX_STATE,
                "Invalid or oversized TUF metadata file"
            );
            let mut bytes = Vec::new();
            file.take(MAX_STATE + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= MAX_STATE,
                "Oversized TUF metadata file"
            );
            Ok(Some(serde_json::from_slice(&bytes)?))
        }
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

pub(super) struct CachedRepository {
    repository: Repository,
    _scratch: tempfile::TempDir,
    _directory: Directory,
}

impl std::ops::Deref for CachedRepository {
    type Target = Repository;
    fn deref(&self) -> &Repository {
        &self.repository
    }
}

fn roles_changed(old: &Root, new: &Root) -> bool {
    use tough::schema::RoleType;
    [RoleType::Timestamp, RoleType::Snapshot]
        .into_iter()
        .any(|role| old.keys(role).ne(new.keys(role)))
}

async fn next_root<T: Transport>(
    transport: &T,
    metadata: &url::Url,
    old: &Signed<Root>,
    embedded: &Signed<Root>,
) -> Result<Signed<Root>> {
    let version = old
        .signed
        .version
        .get()
        .checked_add(1)
        .context("TUF root version overflow")?;
    let next =
        if version == embedded.signed.version.get() && old.signed.verify_role(embedded).is_ok() {
            embedded.clone()
        } else {
            let mut stream = transport
                .fetch(metadata.join(&format!("{version}.root.json"))?)
                .await?;
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.try_next().await? {
                ensure!(
                    chunk.len() <= (1024usize * 1024).saturating_sub(bytes.len()),
                    "Oversized root transition"
                );
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&bytes)?
        };
    ensure!(
        next.signed.version.get() == version,
        "TUF root transition version mismatch"
    );
    old.signed
        .verify_role(&next)
        .context("Incompatible embedded TUF bootstrap lineage")?;
    next.signed.verify_role(&next)?;
    if version == embedded.signed.version.get() {
        ensure!(
            next.signed == embedded.signed,
            "Incompatible embedded TUF bootstrap fork"
        );
    }
    Ok(next)
}

async fn reconcile_bootstrap<T: Transport>(
    state: &mut State,
    embedded: &Signed<Root>,
    transport: &T,
    metadata: &url::Url,
) -> Result<Signed<Root>> {
    let mut latest = state.validate()?;
    if embedded.signed.version < state.anchor.signed.version {
        // Older binaries may still use an earlier out-of-band bootstrap. Prove
        // continuity forward to the cache anchor; do not reset newer floors.
        let mut root = embedded.clone();
        let mut prefix = Vec::new();
        while root.signed.version < state.anchor.signed.version {
            ensure!(
                prefix.len() + state.roots.len() < 1024,
                "TUF root lineage exceeds limit"
            );
            root = next_root(transport, metadata, &root, &state.anchor).await?;
            prefix.push(root.clone());
        }
        ensure!(
            root.signed == state.anchor.signed,
            "Incompatible cached TUF anchor"
        );
        prefix.append(&mut state.roots);
        state.roots = prefix;
        state.anchor = embedded.clone();
    } else if embedded.signed.version <= latest.signed.version {
        let matched = std::iter::once(&state.anchor)
            .chain(&state.roots)
            .find(|root| root.signed.version == embedded.signed.version)
            .context("Embedded TUF bootstrap is absent from cached lineage")?;
        ensure!(
            matched.signed == embedded.signed,
            "Incompatible embedded TUF bootstrap fork"
        );
    } else {
        // A newer binary may carry a bootstrap ahead of this cache. Advance
        // through cross-signed roots and reset floors only for real key changes.
        while latest.signed.version < embedded.signed.version {
            ensure!(state.roots.len() < 1024, "TUF root lineage exceeds limit");
            let next = next_root(transport, metadata, &latest, embedded).await?;
            if roles_changed(&latest.signed, &next.signed) {
                state.timestamp = None;
                state.snapshot = None;
            }
            state.roots.push(next.clone());
            latest = next;
        }
        ensure!(
            latest.signed == embedded.signed,
            "Incompatible embedded TUF bootstrap fork"
        );
    }
    Ok(latest)
}

pub(super) async fn load<T: Transport + Clone + 'static>(
    bootstrap: &[u8],
    transport: T,
    metadata: url::Url,
    targets: url::Url,
    path: &Path,
) -> Result<CachedRepository> {
    let directory = Directory::open(path)?;
    let mut lock = directory.open_private(".state.lock", true)?;
    lock.try_lock()
        .context("Another TUF refresh is running; retry later")?;
    let prior = read_state(&directory, &lock)?;
    let had_state = prior.is_some();
    let legacy_root = directory.exists("root.json")?;
    ensure!(
        had_state
            || legacy_root
            || !["timestamp.json", "snapshot.json", "latest_known_time.json"]
                .into_iter()
                .map(|name| directory.exists(name))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .any(|exists| exists),
        "Incomplete legacy TUF state; refusing to reset rollback protection"
    );
    let embedded: Signed<Root> = serde_json::from_slice(bootstrap)?;
    embedded.signed.verify_role(&embedded)?;
    let mut state = prior.unwrap_or(State {
        version: 1,
        anchor: embedded.clone(),
        roots: Vec::new(),
        timestamp: None,
        snapshot: None,
        latest_time: Utc::now(),
    });
    let mut root = state.validate().context("Invalid persisted TUF state")?;
    let recording = Recording {
        inner: transport,
        roots: Arc::default(),
    };
    // Migrate the previous flat tough datastore without discarding its floors.
    if !had_state && legacy_root {
        let old: Signed<Root> =
            optional_in(&directory, "root.json")?.context("Missing legacy root")?;
        if old.signed.version < root.signed.version {
            old.signed.verify_role(&old)?;
            state.anchor = old.clone();
            root = old.clone();
        }
        while root.signed.version < old.signed.version {
            ensure!(state.roots.len() < 1024, "TUF root lineage exceeds limit");
            let version = root.signed.version.get() + 1;
            let mut stream = recording
                .fetch(metadata.join(&format!("{version}.root.json"))?)
                .await?;
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.try_next().await? {
                ensure!(
                    chunk.len() <= (1024usize * 1024).saturating_sub(bytes.len()),
                    "Oversized legacy root"
                );
                bytes.extend_from_slice(&chunk);
            }
            let next: Signed<Root> = serde_json::from_slice(&bytes)?;
            ensure!(
                next.signed.version.get() == version,
                "Legacy TUF root version mismatch"
            );
            root.signed.verify_role(&next)?;
            next.signed.verify_role(&next)?;
            state.roots.push(next.clone());
            root = next;
        }
        ensure!(
            root.signed == old.signed,
            "Legacy TUF root differs from authenticated lineage"
        );
        state.timestamp = optional_in(&directory, "timestamp.json")?;
        state.snapshot = optional_in(&directory, "snapshot.json")?;
        state.latest_time = optional_in(&directory, "latest_known_time.json")?
            .context("Missing legacy TUF time")?;
        state.validate()?;
    }
    root = reconcile_bootstrap(&mut state, &embedded, &recording, &metadata).await?;
    state.validate()?;
    persist(&directory, &mut lock, &state)?;
    let scratch = directory.scratch()?;
    fs::write(
        scratch.path().join("latest_known_time.json"),
        serde_json::to_vec(&state.latest_time)?,
    )?;
    if let Some(value) = &state.timestamp {
        fs::write(
            scratch.path().join("timestamp.json"),
            serde_json::to_vec(value)?,
        )?;
    }
    if let Some(value) = &state.snapshot {
        fs::write(
            scratch.path().join("snapshot.json"),
            serde_json::to_vec(value)?,
        )?;
    }
    let result = RepositoryLoader::new(&serde_json::to_vec(&root)?, metadata, targets)
        .transport(recording.clone())
        .datastore(scratch.path())
        .expiration_enforcement(tough::ExpirationEnforcement::Safe)
        .load()
        .await;
    // Persist authenticated progress even when a later role/target fails. Root
    // rotation legitimately resets timestamp/snapshot floors only once.
    if let Some(latest) = optional::<Signed<Root>>(&scratch.path().join("root.json"))? {
        let changed_roles = roles_changed(&root.signed, &latest.signed);
        let roots = recording.roots.lock().expect("root recorder lock");
        while root.signed.version < latest.signed.version {
            let version = root.signed.version.get() + 1;
            let next: Signed<Root> = serde_json::from_slice(
                roots
                    .get(&version)
                    .context("Missing authenticated root transition")?,
            )?;
            ensure!(
                next.signed.version.get() == version,
                "TUF root transition mismatch"
            );
            root.signed.verify_role(&next)?;
            next.signed.verify_role(&next)?;
            state.roots.push(next.clone());
            root = next;
        }
        ensure!(
            root == latest,
            "TUF scratch root differs from authenticated lineage"
        );
        state.timestamp = optional(&scratch.path().join("timestamp.json"))?;
        state.snapshot = optional(&scratch.path().join("snapshot.json"))?;
        // tough checks root expiry before clearing old-role metadata. If that
        // fails, retain the authenticated root advancement without old roles
        // signed by keys that this new root no longer authorizes.
        if changed_roles
            && matches!(
                &result,
                Err(tough::error::Error::ExpiredMetadata {
                    role: tough::schema::RoleType::Root,
                    ..
                })
            )
        {
            // Even overlapping key sets can still authenticate old floors.
            // Reset on the actual key change, not on a signature failure.
            state.timestamp = None;
            state.snapshot = None;
        }
    }
    state.latest_time = optional(&scratch.path().join("latest_known_time.json"))?
        .context("Missing TUF clock state")?;
    state.validate()?;
    persist(&directory, &mut lock, &state)?;
    let repository = result?;
    Ok(CachedRepository {
        repository,
        _scratch: scratch,
        _directory: directory,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use sigstore::crypto::{SigningScheme, signing_key::SigStoreSigner};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

    #[test]
    fn native_state_selection_rejects_missing_empty_and_relative_paths() {
        for (configured, home) in [
            (None, None),
            (None, Some("")),
            (None, Some("relative/home")),
            (Some(""), Some("/home/fixture")),
            (Some("relative-state"), Some("/home/fixture")),
        ] {
            assert!(resolve_native_path(configured.map(Into::into), home.map(Into::into)).is_err());
        }
        assert_eq!(
            resolve_native_path(None, Some("/home/fixture".into())).unwrap(),
            PathBuf::from("/home/fixture/.nyxid/tuf-public")
        );
        assert_eq!(
            resolve_native_path(Some("/private/tuf".into()), None).unwrap(),
            PathBuf::from("/private/tuf")
        );
    }

    #[tokio::test]
    async fn explicit_relative_datastore_fails_before_network_or_directory_creation() {
        let server = MockServer::start().await;
        let error = refresh(
            &root(1, &Key::new(), None),
            &server,
            Path::new("relative-tuf-state"),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("absolute persistent path"));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    // Small independently generated signed repository. Its signing keys exist
    // only in this test process. Sort JSON recursively as TUF canonical JSON.
    fn canonical(value: &Value) -> Vec<u8> {
        match value {
            Value::Object(map) => {
                let sorted: BTreeMap<_, _> = map.iter().collect();
                let mut bytes = vec![b'{'];
                for (i, (key, value)) in sorted.into_iter().enumerate() {
                    if i > 0 {
                        bytes.push(b',');
                    }
                    bytes.extend(serde_json::to_vec(key).unwrap());
                    bytes.push(b':');
                    bytes.extend(canonical(value));
                }
                bytes.push(b'}');
                bytes
            }
            Value::Array(values) => {
                let mut bytes = vec![b'['];
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        bytes.push(b',');
                    }
                    bytes.extend(canonical(value));
                }
                bytes.push(b']');
                bytes
            }
            _ => serde_json::to_vec(value).unwrap(),
        }
    }

    struct Key {
        signer: SigStoreSigner,
        public: Value,
        id: String,
    }
    impl Key {
        fn new() -> Self {
            let signer = SigningScheme::ED25519.create_signer().unwrap();
            let der = signer
                .to_sigstore_keypair()
                .unwrap()
                .public_key_to_der()
                .unwrap();
            let public = json!({"keytype":"ed25519","scheme":"ed25519","keyval":{"public":hex::encode(&der[der.len()-32..])}});
            let id = hex::encode(Sha256::digest(canonical(&public)));
            Self { signer, public, id }
        }
    }

    fn signed(value: Value, keys: &[&Key]) -> Value {
        let bytes = canonical(&value);
        json!({"signed":value,"signatures":keys.iter().map(|key| json!({"keyid":key.id,"sig":hex::encode(key.signer.sign(&bytes).unwrap())})).collect::<Vec<_>>()})
    }

    fn root(version: u64, key: &Key, previous: Option<&Key>) -> Vec<u8> {
        let mut keys = serde_json::Map::new();
        keys.insert(key.id.clone(), key.public.clone());
        let roles: serde_json::Map<_, _> = ["root", "timestamp", "snapshot", "targets"]
            .into_iter()
            .map(|role| (role.into(), json!({"keyids":[key.id],"threshold":1})))
            .collect();
        let value = json!({"_type":"root","spec_version":"1.0.31","version":version,"expires":"2100-01-01T00:00:00Z","consistent_snapshot":false,"keys":keys,"roles":roles});
        let signers = match previous {
            Some(old) => vec![old, key],
            None => vec![key],
        };
        serde_json::to_vec(&signed(value, &signers)).unwrap()
    }

    fn meta(bytes: &[u8], version: u64) -> Value {
        json!({"version":version,"length":bytes.len(),"hashes":{"sha256":hex::encode(Sha256::digest(bytes))}})
    }

    async fn publish(
        server: &MockServer,
        key: &Key,
        timestamp_version: u64,
        snapshot_version: u64,
        expires: &str,
    ) {
        server.reset().await;
        let targets = serde_json::to_vec(&signed(json!({"_type":"targets","spec_version":"1.0.31","version":snapshot_version,"expires":"2100-01-01T00:00:00Z","targets":{}}), &[key])).unwrap();
        let snapshot = serde_json::to_vec(&signed(json!({"_type":"snapshot","spec_version":"1.0.31","version":snapshot_version,"expires":"2100-01-01T00:00:00Z","meta":{"targets.json":meta(&targets,snapshot_version)}}), &[key])).unwrap();
        let timestamp = serde_json::to_vec(&signed(json!({"_type":"timestamp","spec_version":"1.0.31","version":timestamp_version,"expires":expires,"meta":{"snapshot.json":meta(&snapshot,snapshot_version)}}), &[key])).unwrap();
        for (name, bytes) in [
            ("targets.json", targets),
            ("snapshot.json", snapshot),
            ("timestamp.json", timestamp),
        ] {
            Mock::given(path(format!("/{name}")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
                .mount(server)
                .await;
        }
    }

    async fn refresh(root: &[u8], server: &MockServer, state: &Path) -> Result<()> {
        load(
            root,
            super::super::CliTransport(reqwest::Client::new()),
            format!("{}/", server.uri()).parse()?,
            format!("{}/targets/", server.uri()).parse()?,
            state,
        )
        .await?;
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn replacing_initialized_directory_through_writable_parent_cannot_rebootstrap() {
        use std::os::unix::fs::PermissionsExt;
        let server = MockServer::start().await;
        let key = Key::new();
        let bootstrap = root(1, &key, None);
        let parent = test_directory();
        let state = parent.path().join("state");
        publish(&server, &key, 7, 7, "2100-01-01T00:00:00Z").await;
        refresh(&bootstrap, &server, &state).await.unwrap();
        // Model the verifier's parent writer replacing the entire directory,
        // including the initialization marker and every retained version floor.
        fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o777)).unwrap();
        fs::rename(&state, parent.path().join("removed")).unwrap();
        publish(&server, &key, 1, 1, "2100-01-01T00:00:00Z").await;
        assert!(refresh(&bootstrap, &server, &state).await.is_err());
        assert!(
            !state.exists(),
            "must reject before recreating bootstrap state"
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(refresh(&bootstrap, &server, &state).await.is_err());
        assert!(!state.join(STATE).exists());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn writable_ancestor_and_alias_parent_rejected_before_first_bootstrap() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let server = MockServer::start().await;
        let key = Key::new();
        let bootstrap = root(1, &key, None);
        let fixture = test_directory();
        let trusted = fixture.path().join("trusted");
        fs::create_dir(&trusted).unwrap();
        let mutable = fixture.path().join("mutable");
        fs::create_dir(&mutable).unwrap();
        symlink(&trusted, mutable.join("alias")).unwrap();
        fs::set_permissions(&mutable, fs::Permissions::from_mode(0o777)).unwrap();
        for state in [mutable.join("direct/state"), mutable.join("alias/state")] {
            assert!(prepare_datastore(&state).is_err());
            assert!(refresh(&bootstrap, &server, &state).await.is_err());
            assert!(!state.exists());
        }
        assert!(server.received_requests().await.unwrap().is_empty());
        fs::set_permissions(&mutable, fs::Permissions::from_mode(0o700)).unwrap();
        publish(&server, &key, 2, 2, "2100-01-01T00:00:00Z").await;
        let state = mutable.join("alias/state");
        refresh(&bootstrap, &server, &state).await.unwrap();
        fs::set_permissions(&mutable, fs::Permissions::from_mode(0o1777)).unwrap();
        publish(&server, &key, 1, 1, "2100-01-01T00:00:00Z").await;
        assert!(
            format!(
                "{:#}",
                refresh(&bootstrap, &server, &state).await.unwrap_err()
            )
            .contains("previously fetched version 2")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pinned_state_lock_reads_and_publication_survive_whole_directory_replacement() {
        use std::os::unix::fs::PermissionsExt;
        let server = MockServer::start().await;
        let key = Key::new();
        let bootstrap = root(1, &key, None);
        let parent = test_directory();
        let path = parent.path().join("state");
        publish(&server, &key, 7, 7, "2100-01-01T00:00:00Z").await;
        refresh(&bootstrap, &server, &path).await.unwrap();
        let directory = Directory::open(&path).unwrap();
        let mut lock = directory.open_private(".state.lock", true).unwrap();
        lock.try_lock().unwrap();
        let moved = parent.path().join("moved");
        fs::rename(&path, &moved).unwrap();
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let state = read_state(&directory, &lock).unwrap().unwrap();
        assert_eq!(state.timestamp.as_ref().unwrap().signed.version.get(), 7);
        persist(&directory, &mut lock, &state).unwrap();
        assert!(!path.join(STATE).exists());
        assert!(!path.join(".state.lock").exists());
        let original = Directory::open(&moved).unwrap();
        assert!(
            original
                .open_private(".state.lock", true)
                .unwrap()
                .try_lock()
                .is_err()
        );
        assert_eq!(
            read_state(&original, &lock)
                .unwrap()
                .unwrap()
                .timestamp
                .unwrap()
                .signed
                .version
                .get(),
            7
        );
        fs::remove_file(moved.join(STATE)).unwrap();
        assert!(
            format!("{:#}", read_state(&directory, &lock).err().unwrap())
                .contains("missing after initialization")
        );
    }

    #[tokio::test]
    async fn signed_unexpired_timestamp_and_snapshot_rollbacks_fail_across_restarts() {
        let server = MockServer::start().await;
        let key = Key::new();
        let root = root(1, &key, None);
        let state = test_directory();
        publish(&server, &key, 2, 2, "2100-01-01T00:00:00Z").await;
        refresh(&root, &server, state.path()).await.unwrap();
        publish(&server, &key, 1, 1, "2100-01-01T00:00:00Z").await;
        let error = refresh(&root, &server, state.path()).await.unwrap_err();
        assert!(
            format!("{error:#}")
                .contains("timestamp metadata when we had previously fetched version 2"),
            "{error:#}"
        );
        publish(&server, &key, 3, 1, "2100-01-01T00:00:00Z").await;
        let error = refresh(&root, &server, state.path()).await.unwrap_err();
        assert!(
            format!("{error:#}").contains("Snapshot version 1 in timestamp 3 is less than 2"),
            "{error:#}"
        );
        publish(&server, &key, 3, 3, "2100-01-01T00:00:00Z").await;
        refresh(&root, &server, state.path()).await.unwrap();
        publish(&server, &key, 4, 4, "2000-01-01T00:00:00Z").await;
        let error = refresh(&root, &server, state.path()).await.unwrap_err();
        assert!(format!("{error:#}").contains("expired"), "{error:#}");
    }

    #[tokio::test]
    async fn persisted_rotated_root_prevents_repeated_floor_reset_and_corruption_fails_closed() {
        let server = MockServer::start().await;
        let old = Key::new();
        let key = Key::new();
        let bootstrap = root(1, &old, None);
        let state = test_directory();
        publish(&server, &old, 7, 7, "2100-01-01T00:00:00Z").await;
        refresh(&bootstrap, &server, state.path()).await.unwrap();
        publish(&server, &key, 2, 2, "2100-01-01T00:00:00Z").await;
        Mock::given(path("/2.root.json"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(root(2, &key, Some(&old))))
            .mount(&server)
            .await;
        refresh(&bootstrap, &server, state.path()).await.unwrap();
        publish(&server, &key, 1, 1, "2100-01-01T00:00:00Z").await;
        let error = refresh(&bootstrap, &server, state.path())
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}")
                .contains("timestamp metadata when we had previously fetched version 2"),
            "{error:#}"
        );
        assert!(
            !server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|r| r.url.path() == "/2.root.json")
        );
        let mut cached: Value =
            serde_json::from_slice(&fs::read(state.path().join(STATE)).unwrap()).unwrap();
        cached["timestamp"]["signed"]["version"] = 1.into();
        fs::write(
            state.path().join(STATE),
            serde_json::to_vec(&cached).unwrap(),
        )
        .unwrap();
        let before = server.received_requests().await.unwrap().len();
        let error = refresh(&bootstrap, &server, state.path())
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("Cached TUF timestamp signature invalid"),
            "{error:#}"
        );
        assert_eq!(before, server.received_requests().await.unwrap().len());
        fs::remove_file(state.path().join(STATE)).unwrap();
        assert!(
            format!(
                "{:#}",
                refresh(&bootstrap, &server, state.path())
                    .await
                    .unwrap_err()
            )
            .contains("missing after initialization")
        );
    }

    #[tokio::test]
    async fn embedded_bootstrap_advancement_and_older_binary_preserve_applicable_floors() {
        let server = MockServer::start().await;
        let key = Key::new();
        let root1 = root(1, &key, None);
        let root2 = root(2, &key, None);
        let root3 = root(3, &key, None);
        let state = test_directory();
        publish(&server, &key, 5, 5, "2100-01-01T00:00:00Z").await;
        Mock::given(path("/2.root.json"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(root2.clone()))
            .mount(&server)
            .await;
        Mock::given(path("/3.root.json"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(root3.clone()))
            .mount(&server)
            .await;
        refresh(&root1, &server, state.path()).await.unwrap();
        // New binary bootstrap is inside the previously accepted v1->v3 lineage.
        refresh(&root2, &server, state.path()).await.unwrap();
        refresh(&root3, &server, state.path()).await.unwrap();
        publish(&server, &key, 4, 4, "2100-01-01T00:00:00Z").await;
        for bootstrap in [&root1, &root2, &root3] {
            let error = refresh(bootstrap, &server, state.path()).await.unwrap_err();
            assert!(
                format!("{error:#}").contains("previously fetched version 5"),
                "{error:#}"
            );
        }
        let fork = root(2, &Key::new(), None);
        let before = server.received_requests().await.unwrap().len();
        assert!(
            format!(
                "{:#}",
                refresh(&fork, &server, state.path()).await.unwrap_err()
            )
            .contains("bootstrap fork")
        );
        assert_eq!(before, server.received_requests().await.unwrap().len());
        // Cache is behind a newer bootstrap with unchanged role keys: preserve
        // its signed timestamp/snapshot floors while advancing root authority.
        let behind = test_directory();
        publish(&server, &key, 5, 5, "2100-01-01T00:00:00Z").await;
        refresh(&root1, &server, behind.path()).await.unwrap();
        refresh(&root2, &server, behind.path()).await.unwrap();
        publish(&server, &key, 4, 4, "2100-01-01T00:00:00Z").await;
        assert!(
            format!(
                "{:#}",
                refresh(&root2, &server, behind.path()).await.unwrap_err()
            )
            .contains("previously fetched version 5")
        );
        // A cache initialized by a new binary is usable by an older binary only
        // after proving the intermediate roots forward to that cached anchor.
        let ahead = test_directory();
        publish(&server, &key, 5, 5, "2100-01-01T00:00:00Z").await;
        refresh(&root3, &server, ahead.path()).await.unwrap();
        Mock::given(path("/2.root.json"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(root2))
            .mount(&server)
            .await;
        refresh(&root1, &server, ahead.path()).await.unwrap();
    }

    #[tokio::test]
    async fn newer_embedded_bootstrap_resets_floors_only_after_authenticated_role_rotation() {
        let server = MockServer::start().await;
        let old = Key::new();
        let new = Key::new();
        let root1 = root(1, &old, None);
        let root2 = root(2, &new, Some(&old));
        let state = test_directory();
        publish(&server, &old, 9, 9, "2100-01-01T00:00:00Z").await;
        refresh(&root1, &server, state.path()).await.unwrap();
        publish(&server, &new, 2, 2, "2100-01-01T00:00:00Z").await;
        refresh(&root2, &server, state.path()).await.unwrap();
        publish(&server, &new, 1, 1, "2100-01-01T00:00:00Z").await;
        assert!(
            format!(
                "{:#}",
                refresh(&root2, &server, state.path()).await.unwrap_err()
            )
            .contains("previously fetched version 2")
        );
    }

    #[tokio::test]
    async fn self_signed_new_bootstrap_uses_published_cross_signed_transition() {
        let server = MockServer::start().await;
        let old = Key::new();
        let new = Key::new();
        let root1 = root(1, &old, None);
        let embedded = root(2, &new, None);
        let state = test_directory();
        publish(&server, &old, 9, 9, "2100-01-01T00:00:00Z").await;
        refresh(&root1, &server, state.path()).await.unwrap();
        publish(&server, &new, 2, 2, "2100-01-01T00:00:00Z").await;
        Mock::given(path("/2.root.json"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(root(2, &new, Some(&old))))
            .mount(&server)
            .await;
        refresh(&embedded, &server, state.path()).await.unwrap();
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|r| r.url.path() == "/2.root.json")
        );
        publish(&server, &new, 1, 1, "2100-01-01T00:00:00Z").await;
        let error = refresh(&embedded, &server, state.path()).await.unwrap_err();
        assert!(
            format!("{error:#}").contains("previously fetched version 2"),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn accepted_timestamp_survives_later_snapshot_failure_and_legacy_migration() {
        let server = MockServer::start().await;
        let key = Key::new();
        let bootstrap = root(1, &key, None);
        let state = test_directory();
        publish(&server, &key, 2, 2, "2100-01-01T00:00:00Z").await;
        refresh(&bootstrap, &server, state.path()).await.unwrap();
        // Build a real previous tough datastore from the authenticated state.
        let prior: State =
            serde_json::from_slice(&fs::read(state.path().join(STATE)).unwrap()).unwrap();
        let legacy = test_directory();
        for (name, bytes) in [
            ("root.json", bootstrap.clone()),
            (
                "timestamp.json",
                serde_json::to_vec(&prior.timestamp.unwrap()).unwrap(),
            ),
            (
                "snapshot.json",
                serde_json::to_vec(&prior.snapshot.unwrap()).unwrap(),
            ),
            (
                "latest_known_time.json",
                serde_json::to_vec(&prior.latest_time).unwrap(),
            ),
        ] {
            fs::write(legacy.path().join(name), bytes).unwrap();
        }
        publish(&server, &key, 1, 1, "2100-01-01T00:00:00Z").await;
        let error = refresh(&bootstrap, &server, legacy.path())
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("previously fetched version 2"),
            "{error:#}"
        );
        // A newer authenticated timestamp is a floor even if its snapshot fails.
        publish(&server, &key, 3, 3, "2100-01-01T00:00:00Z").await;
        Mock::given(path("/snapshot.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .with_priority(1)
            .mount(&server)
            .await;
        assert!(refresh(&bootstrap, &server, state.path()).await.is_err());
        publish(&server, &key, 2, 2, "2100-01-01T00:00:00Z").await;
        let error = refresh(&bootstrap, &server, state.path())
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("previously fetched version 3"),
            "{error:#}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn nonroot_readonly_install_uses_only_explicit_private_durable_volume() {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(
            unsafe { libc::geteuid() },
            0,
            "Run this boundary check as a nonroot service identity"
        );
        let server = MockServer::start().await;
        let key = Key::new();
        let bootstrap = root(1, &key, None);
        let install = test_directory();
        fs::set_permissions(install.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let rejected = refresh(&bootstrap, &server, &install.path().join("tuf")).await;
        fs::set_permissions(install.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(rejected.is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
        let volume = test_directory();
        publish(&server, &key, 2, 2, "2100-01-01T00:00:00Z").await;
        refresh(&bootstrap, &server, volume.path()).await.unwrap();
        publish(&server, &key, 1, 1, "2100-01-01T00:00:00Z").await;
        let error = refresh(&bootstrap, &server, volume.path())
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("previously fetched version 2"),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn concurrent_refresh_and_nonprivate_or_symlink_state_are_rejected() {
        if let Some(path) = std::env::var_os("NYXID_TEST_TUF_LOCK_DIR") {
            let lock = Directory::open(&PathBuf::from(path))
                .unwrap()
                .open_private(".state.lock", true)
                .unwrap();
            assert!(
                lock.try_lock().is_err(),
                "Child process acquired an already held trust-state lock"
            );
            return;
        }
        let state = test_directory();
        let lock = Directory::open(state.path())
            .unwrap()
            .open_private(".state.lock", true)
            .unwrap();
        lock.try_lock().unwrap();
        let full_name = concat!(
            module_path!(),
            "::concurrent_refresh_and_nonprivate_or_symlink_state_are_rejected"
        );
        let test_name = full_name.split_once("::").unwrap().1;
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .env("NYXID_TEST_TUF_LOCK_DIR", state.path())
            .output()
            .unwrap();
        assert!(
            child.status.success() && String::from_utf8_lossy(&child.stdout).contains("1 passed"),
            "Child lock check did not execute successfully: {} {}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        let server = MockServer::start().await;
        let root = root(1, &Key::new(), None);
        assert!(
            format!(
                "{:#}",
                refresh(&root, &server, state.path()).await.unwrap_err()
            )
            .contains("Another TUF refresh")
        );
        drop(lock);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            let target = state.path().join("target");
            fs::write(&target, b"{}").unwrap();
            symlink(&target, state.path().join(STATE)).unwrap();
            assert!(refresh(&root, &server, state.path()).await.is_err());
            fs::remove_file(state.path().join(STATE)).unwrap();
            use std::os::unix::ffi::OsStrExt;
            let fifo =
                std::ffi::CString::new(state.path().join(STATE).as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
            assert!(refresh(&root, &server, state.path()).await.is_err());
            fs::remove_file(state.path().join(STATE)).unwrap();
            fs::set_permissions(state.path(), fs::Permissions::from_mode(0o755)).unwrap();
            assert!(
                format!(
                    "{:#}",
                    refresh(&root, &server, state.path()).await.unwrap_err()
                )
                .contains("0700")
            );
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
