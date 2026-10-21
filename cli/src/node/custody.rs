//! Observe credential custody using real opens and key-service retrievals under
//! the worker's OS identity. Missing services/objects never establish denial.
use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWriteExt, process::Command};

use super::{config::NodeConfig, machine::process::Identity};

const CADENCE: Duration = Duration::from_secs(60);
const BOUND: Duration = Duration::from_secs(5);
const REQUEST_LIMIT: u64 = 64 * 1024;
const WARNING: &str = "Node credential custody is not verified: agent commands or browsers may access node tokens, signing secrets and stored credentials. Use a separated Linux supervisor with private node storage; install the node executable where every worker identity can execute it (e.g. /usr/local/bin); shared_legacy retains the owner's OS access.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Observation {
    Denied,
    Accessible,
    Unknown,
}

impl Observation {
    fn exit_code(self) -> i32 {
        match self {
            Self::Denied => 40,
            Self::Accessible => 41,
            Self::Unknown => 42,
        }
    }
    fn from_exit(code: Option<i32>) -> Self {
        match code {
            Some(40) => Self::Denied,
            Some(41) => Self::Accessible,
            _ => Self::Unknown,
        }
    }
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Self::Accessible, _) | (_, Self::Accessible) => Self::Accessible,
            (Self::Denied, Self::Denied) => Self::Denied,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Status {
    pub(crate) command: Observation,
    pub(crate) browser: Observation,
    pub(crate) developer: Observation,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            command: Observation::Unknown,
            browser: Observation::Unknown,
            developer: Observation::Unknown,
        }
    }
}

type IdentityStamp = Vec<Option<(u32, u32, Vec<u32>)>>;

#[derive(Debug, thiserror::Error)]
#[error("node_custody_unverified")]
pub(crate) struct Unverified;

struct Cached {
    status: Status,
    at: Instant,
    identities: IdentityStamp,
}

pub(crate) struct Custody {
    directory: PathBuf,
    cached: RwLock<Option<Cached>>,
    refresh: tokio::sync::Mutex<()>,
    monitoring: std::sync::atomic::AtomicBool,
}

impl Custody {
    pub(crate) fn new(directory: &Path) -> Self {
        // Retain the spelling used by later configuration readers. Resolving it
        // here would hide alias parents from observation. The storage walk
        // checks every supplied/expanded parent, so only root or the supervisor
        // UID can redirect an accepted alias (outside this custody boundary).
        Self {
            directory: directory.into(),
            cached: RwLock::new(None),
            refresh: tokio::sync::Mutex::new(()),
            monitoring: std::sync::atomic::AtomicBool::new(false),
        }
    }
    /// Contexts share the supervisor's credential location, never its identity
    /// observations. Their actual provisioned users must be observed separately.
    #[cfg(target_os = "linux")]
    pub(crate) fn for_context(&self) -> Self {
        Self::new(&self.directory)
    }
    pub(crate) fn status(&self) -> Status {
        self.cached
            .read()
            .ok()
            .and_then(|cache| {
                cache
                    .as_ref()
                    .filter(|c| c.at.elapsed() <= CADENCE + BOUND * 8)
                    .map(|c| c.status)
            })
            .unwrap_or_default()
    }
    /// No I/O on execution: separated contexts consume the cached observation.
    /// Ordinary/shared-legacy jobs keep their existing owner-risk behavior.
    pub(crate) fn require(&self, separated: bool) -> Result<()> {
        if separated && !self.status().all_denied() {
            return Err(Unverified.into());
        }
        Ok(())
    }
    pub(crate) async fn observe(&self, identities: &[Option<Identity>; 3]) {
        let _refresh = self.refresh.lock().await;
        let stamp = identities
            .iter()
            .map(|identity| {
                identity
                    .as_ref()
                    .map(|identity| (identity.uid, identity.gid, identity.policy_groups.clone()))
            })
            .collect::<Vec<_>>();
        if self.cached.read().ok().is_some_and(|c| {
            c.as_ref()
                .is_some_and(|v| v.at.elapsed() < CADENCE && v.identities == stamp)
        }) {
            return;
        }
        // An identity change invalidates prior authority before any OS I/O.
        let started = Instant::now();
        if let Ok(mut cached) = self.cached.write()
            && cached
                .as_ref()
                .is_some_and(|prior| prior.identities != stamp)
        {
            *cached = None;
        }
        let status = self.observe_uncached(identities).await.unwrap_or_default();
        if !status.all_denied() {
            tracing::warn!("{WARNING}");
        }
        if let Ok(mut cached) = self.cached.write() {
            *cached = Some(Cached {
                status,
                at: started,
                identities: stamp,
            });
        }
    }
    async fn observe_uncached(&self, identities: &[Option<Identity>; 3]) -> Result<Status> {
        let directory = super::storage::Directory::open(&self.directory)?;
        let config = NodeConfig::load_in(&directory, &self.directory.join("config.toml"))?;
        let paths = targets(&self.directory)?;
        let mut outcomes = [Observation::Unknown; 3];
        for (index, identity) in identities.iter().enumerate() {
            let Some(identity) = identity else { continue };
            if !Identity::resolve(Some(&identity.name))
                .is_ok_and(|current| current.uid == identity.uid && current.gid == identity.gid)
            {
                continue;
            }
            if identity.uid == 0 || identity.uid == unsafe { libc::geteuid() } {
                // Same-UID data cannot be protected by a per-user keychain.
                outcomes[index] = Observation::Unknown;
                continue;
            }
            let mut request = Request {
                paths: paths.clone(),
                keychain: None,
            };
            if config.storage_backend == "keychain" {
                // Synthetic, non-secret entry; never retrieve an actual node
                // credential for a probe or send secret bytes to the child.
                let key = KeyProbe {
                    owner: config.node.id.clone(),
                    key: format!("object/v1/custody/{}", uuid::Uuid::new_v4()),
                    action: Action::Seed,
                };
                let seeded = helper(
                    None,
                    &Request {
                        paths: vec![],
                        keychain: Some(key.clone()),
                    },
                )
                .await;
                if seeded != Observation::Denied {
                    // Even failed/uncertain seeding attempts must try retirement.
                    let mut retirement = key;
                    retirement.action = Action::Delete;
                    let _ = helper(
                        None,
                        &Request {
                            paths: vec![],
                            keychain: Some(retirement),
                        },
                    )
                    .await;
                    continue;
                }
                let mut read = key.clone();
                read.action = Action::Read;
                request.keychain = Some(read);
                outcomes[index] = helper(Some(identity), &request).await;
                let mut retirement = key;
                retirement.action = Action::Delete;
                if helper(
                    None,
                    &Request {
                        paths: vec![],
                        keychain: Some(retirement),
                    },
                )
                .await
                    != Observation::Denied
                {
                    outcomes[index] = Observation::Unknown;
                }
            } else if config.storage_backend == "file" {
                outcomes[index] = helper(Some(identity), &request).await;
            }
        }
        directory.check_path(&self.directory)?;
        Ok(Status {
            command: outcomes[0],
            browser: outcomes[1],
            developer: outcomes[2],
        })
    }
    /// A weak reference keeps shutdown finite. No inherited key-service session
    /// or parent environment is supplied to agent/browser jobs by this monitor.
    pub(crate) fn monitor(self: &Arc<Self>, identities: [Option<Identity>; 3]) {
        if self
            .monitoring
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut cadence = tokio::time::interval(CADENCE);
            cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            cadence.tick().await; // The construction observation already ran.
            loop {
                cadence.tick().await;
                let Some(custody) = weak.upgrade() else { break };
                custody.observe(&identities).await;
            }
        });
    }
}

impl Status {
    pub(crate) fn all_denied(self) -> bool {
        [self.command, self.browser, self.developer]
            .iter()
            .all(|v| *v == Observation::Denied)
    }
}

/// Enumerate the selected profile and supported sibling layouts. Validate
/// actual config/key files before probing: a missing path is never a denial.
fn targets(selected: &Path) -> Result<Vec<PathBuf>> {
    let mut directories = vec![selected.to_owned()];
    if let Some(home) = dirs::home_dir() {
        let default = home.join(".nyxid-node");
        if default.exists() {
            directories.push(default.clone());
            add_profiles(&default.join("profiles"), false, &mut directories)?;
        }
    }
    if selected
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "profiles")
    {
        add_profiles(
            selected.parent().context("node_custody_targets_unknown")?,
            false,
            &mut directories,
        )?;
    }
    if selected.file_name().is_some_and(|name| name == "node") {
        let root = selected
            .parent()
            .and_then(Path::parent)
            .context("node_custody_targets_unknown")?;
        add_profiles(root, true, &mut directories)?;
    }
    directories.sort();
    directories.dedup();
    ensure!(directories.len() <= 128, "node_custody_targets_unknown");
    let mut paths = vec![];
    for directory in directories {
        let mut found = vec![];
        for name in ["config.toml", ".keyfile"] {
            let path = directory.join(name);
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    ensure!(metadata.is_file(), "node_custody_targets_unknown");
                    found.push(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        if !found.is_empty() {
            super::storage::Directory::open(&directory)?;
            paths.extend(found);
        }
    }
    ensure!(!paths.is_empty(), "node_custody_targets_unknown");
    Ok(paths)
}

fn add_profiles(root: &Path, nested_node: bool, directories: &mut Vec<PathBuf>) -> Result<()> {
    match std::fs::read_dir(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => anyhow::bail!("node_custody_targets_unknown"),
        Ok(entries) => {
            for (index, entry) in entries.enumerate() {
                ensure!(index < 128, "node_custody_targets_unknown");
                let entry = entry?;
                let kind = entry.file_type()?;
                ensure!(!kind.is_symlink(), "node_custody_targets_unknown");
                if kind.is_dir() {
                    directories.push(if nested_node {
                        entry.path().join("node")
                    } else {
                        entry.path()
                    });
                }
            }
            ensure!(directories.len() <= 128, "node_custody_targets_unknown");
            Ok(())
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    paths: Vec<PathBuf>,
    keychain: Option<KeyProbe>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyProbe {
    owner: String,
    key: String,
    action: Action,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
enum Action {
    Seed,
    Read,
    Delete,
}

async fn helper(identity: Option<&Identity>, request: &Request) -> Observation {
    async fn run(identity: Option<&Identity>, request: &Request) -> Result<Observation> {
        let mut command = Command::new(std::env::current_exe()?);
        #[cfg(not(test))]
        command.args(["node", "custody-probe"]);
        #[cfg(test)]
        command.args([
            "--exact",
            &format!(
                "{}::tests::probe_child",
                module_path!().split_once("::").expect("crate module").1
            ),
            "--nocapture",
        ]);
        if let Some(identity) = identity {
            let identity = identity.clone();
            #[cfg(target_os = "linux")]
            let mut identity = identity;
            // Probe the OS identity's permissions, independently of a sandbox
            // which may hide a real resource and return a misleading ENOENT.
            #[cfg(target_os = "linux")]
            {
                identity.sandbox = None;
            }
            identity.prepare(&mut command)?;
            // Deliberately challenge the supervisor's real bus, rather than an
            // empty worker-local keychain. Never inherited by real worker jobs.
            if let Some(bus) = std::env::var_os("DBUS_SESSION_BUS_ADDRESS") {
                command.env("DBUS_SESSION_BUS_ADDRESS", bus);
            } else if cfg!(target_os = "linux") && request.keychain.is_some() {
                let runtime = std::env::var_os("XDG_RUNTIME_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() }))
                    });
                command.env(
                    "DBUS_SESSION_BUS_ADDRESS",
                    format!("unix:path={}", runtime.join("bus").display()),
                );
            }
        }
        #[cfg(test)]
        command.env("NYXID_CUSTODY_TEST_CHILD", "1");
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let bytes = serde_json::to_vec(request)?;
        ensure!(
            bytes.len() as u64 <= REQUEST_LIMIT,
            "node_custody_request_limit"
        );
        let mut child = command.spawn()?;
        let result = tokio::time::timeout(BOUND, async {
            let mut stdin = child.stdin.take().context("node_custody_stdin_unknown")?;
            stdin.write_all(&bytes).await?;
            drop(stdin);
            Ok::<_, anyhow::Error>(Observation::from_exit(child.wait().await?.code()))
        })
        .await;
        match result {
            Ok(value) => value,
            Err(_) => {
                child.start_kill()?;
                child.wait().await?;
                Ok(Observation::Unknown)
            }
        }
    }
    run(identity, request).await.unwrap_or(Observation::Unknown)
}

/// Internal shell-free helper. It never reads file contents, prints a path,
/// secret, key identifier, provider message or untrusted input.
pub(crate) fn worker() -> ! {
    use std::io::Read;
    let mut bytes = vec![];
    let result = std::io::stdin()
        .take(REQUEST_LIMIT + 1)
        .read_to_end(&mut bytes);
    let observation = if result.is_err() || bytes.len() as u64 > REQUEST_LIMIT {
        Observation::Unknown
    } else {
        serde_json::from_slice::<Request>(&bytes)
            .map(observe_request)
            .unwrap_or(Observation::Unknown)
    };
    std::process::exit(observation.exit_code());
}

fn observe_request(request: Request) -> Observation {
    let no_paths = request.paths.is_empty();
    let mut outcome = if no_paths {
        Observation::Unknown
    } else {
        Observation::Denied
    };
    for path in request.paths {
        use std::os::unix::fs::OpenOptionsExt;
        let observed = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        {
            Ok(_) => Observation::Accessible,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                Observation::Denied
            }
            Err(_) => Observation::Unknown,
        };
        outcome = outcome.combine(observed);
    }
    if let Some(key) = request.keychain {
        let observed = observe_key(&key);
        outcome = if no_paths {
            observed
        } else {
            outcome.combine(observed)
        };
    }
    outcome
}

fn observe_key(key: &KeyProbe) -> Observation {
    // Helper protocol never permits reading a genuine credential reference.
    let Some(id) = key.key.strip_prefix("object/v1/custody/") else {
        return Observation::Unknown;
    };
    if uuid::Uuid::parse_str(id).is_err() || key.owner.len() > 256 {
        return Observation::Unknown;
    }
    // Probe only local Unix Secret Service transports, never TCP/fallbacks.
    if cfg!(target_os = "linux")
        && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|address| {
            let Some(address) = address.to_str() else {
                return true;
            };
            !address.starts_with("unix:") || address.contains(';')
        })
    {
        return Observation::Unknown;
    }
    let Ok(entry) = keyring::Entry::new("nyxid-node", &format!("{}/{}", key.owner, key.key)) else {
        return Observation::Unknown;
    };
    match key.action {
        Action::Seed => match entry
            .set_password("nyxid-public-custody-sentinel")
            .and_then(|()| entry.get_password())
        {
            Ok(value) if value == "nyxid-public-custody-sentinel" => Observation::Denied, // setup success, not an isolation claim
            _ => Observation::Unknown,
        },
        Action::Delete => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Observation::Denied,
            _ => Observation::Unknown,
        },
        Action::Read => match entry.get_password() {
            Ok(value) => {
                drop(zeroize::Zeroizing::new(value));
                Observation::Accessible
            }
            Err(keyring::Error::BadEncoding(bytes)) => {
                drop(zeroize::Zeroizing::new(bytes));
                Observation::Accessible
            }
            Err(error) => key_error(&error),
        },
    }
}

fn key_error(error: &(dyn std::error::Error + 'static)) -> Observation {
    // Never parse Display/Debug strings: they may contain provider data. zbus
    // wraps its OS error in Arc; preserve the typed permission classification.
    if error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
        || error
            .downcast_ref::<Arc<std::io::Error>>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
    {
        return Observation::Denied;
    }
    error
        .source()
        .map(key_error)
        .unwrap_or(Observation::Unknown)
}

pub(crate) async fn files_denied(identity: &Identity, directories: &[PathBuf]) -> bool {
    if identity.uid == 0 || identity.uid == unsafe { libc::geteuid() } || directories.is_empty() {
        return false;
    }
    let mut paths = vec![];
    for directory in directories {
        match targets(directory) {
            Ok(found) => paths.extend(found),
            Err(_) => return false,
        }
    }
    helper(
        Some(identity),
        &Request {
            paths,
            keychain: None,
        },
    )
    .await
        == Observation::Denied
}

/// Explicit pre-registration capability probe: disposable public sentinels,
/// never a production encryption key or an absent credential lookup repair.
pub(crate) async fn preflight_files(identities: &[Option<Identity>; 3]) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Ok(temporary) = tempfile::tempdir() else {
        return false;
    };
    let profiles = temporary.path().join("profiles");
    for name in ["selected", "sibling"] {
        let path = profiles.join(name);
        if std::fs::create_dir_all(&path).is_err() {
            return false;
        }
        for file in ["config.toml", ".keyfile"] {
            let path = path.join(file);
            if std::fs::write(&path, b"public custody probe; not an encryption key").is_err()
                || std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).is_err()
            {
                return false;
            }
        }
    }
    for identity in identities {
        let Some(identity) = identity else {
            return false;
        };
        if !files_denied(identity, &[profiles.join("selected")]).await {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-local private profile setup; never a substitute for registration.
    #[cfg(target_os = "linux")]
    fn mk_private_dir(path: &std::path::Path) {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .unwrap();
    }

    #[tokio::test]
    async fn own_uid_is_never_reported_as_command_isolated() {
        let directory = tempfile::tempdir().unwrap();
        let identity = Identity::resolve(None).unwrap();
        assert!(!files_denied(&identity, &[directory.path().to_owned()]).await);
        assert!(!files_denied(&identity, &[]).await);
    }

    #[test]
    fn probe_child() {
        if std::env::var_os("NYXID_CUSTODY_TEST_CHILD").is_some() {
            worker()
        }
    }

    #[test]
    fn production_helper_dispatch_precedes_network_and_configuration_hooks() {
        // Both binary and node-proxy library tests bind the real CLI entrypoint.
        // Native acceptance also checks an empty HOME and a telemetry trap.
        let main = include_str!("../main.rs");
        let body = main.split_once("async fn run_main()").unwrap().1;
        let dispatch = body.find("node::custody::worker();").unwrap();
        for hook in [
            "telemetry::TelemetryClient::init(",
            "skill_self_heal::maybe_self_heal(",
            "update_check::start_update_notice(",
        ] {
            assert!(body.find(hook).unwrap() > dispatch, "{hook}");
        }
    }

    #[test]
    fn only_expected_permission_errors_establish_denial() {
        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::ConnectionRefused,
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::InvalidData,
        ] {
            assert_eq!(
                key_error(&keyring::Error::PlatformFailure(Box::new(
                    std::io::Error::from(kind)
                ))),
                Observation::Unknown
            );
        }
        assert_eq!(key_error(&keyring::Error::NoEntry), Observation::Unknown);
        assert_eq!(
            key_error(&keyring::Error::PlatformFailure(Box::new(Arc::new(
                std::io::Error::from(std::io::ErrorKind::PermissionDenied)
            )))),
            Observation::Denied
        );
    }

    #[test]
    fn same_uid_and_unknown_cache_never_block_shared_legacy() {
        let custody = Custody::new(Path::new("/unused"));
        custody.require(false).unwrap();
        assert_eq!(
            custody.require(true).unwrap_err().to_string(),
            "node_custody_unverified"
        );
        assert!(!custody.status().all_denied());
        *custody.cached.write().unwrap() = Some(Cached {
            status: Status {
                command: Observation::Accessible,
                browser: Observation::Denied,
                developer: Observation::Denied,
            },
            at: Instant::now(),
            identities: vec![],
        });
        custody.require(false).unwrap();
        assert!(custody.require(true).is_err());
    }

    #[tokio::test]
    async fn custom_directory_alias_with_writable_parent_cannot_verify_custody() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let mk_private_dir = |path: &Path| {
            std::fs::create_dir(path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        };
        let fixture = tempfile::tempdir().unwrap();
        let selected = fixture.path().join("selected");
        mk_private_dir(&selected);
        NodeConfig::new("wss://example.test".into(), "node".into(), "file".into())
            .save(&selected.join("config.toml"))
            .unwrap();
        let mutable = fixture.path().join("mutable");
        mk_private_dir(&mutable);
        symlink(fixture.path(), mutable.join("alias")).unwrap();
        let custody = Custody::new(&mutable.join("alias/selected"));
        let identities = [None, None, None];
        custody.observe_uncached(&identities).await.unwrap();
        std::fs::set_permissions(&mutable, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(custody.observe_uncached(&identities).await.is_err());
        custody.observe(&identities).await;
        assert_eq!(custody.status(), Status::default());
        assert!(custody.require(true).is_err());
    }

    #[tokio::test]
    async fn actual_open_success_and_missing_file_are_distinct() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sentinel");
        std::fs::write(&path, "public sentinel").unwrap();
        for (paths, expected) in [
            (vec![path.clone()], Observation::Accessible),
            (vec![path.with_extension("missing")], Observation::Unknown),
            (vec![], Observation::Unknown),
        ] {
            assert_eq!(
                helper(
                    None,
                    &Request {
                        paths,
                        keychain: None
                    }
                )
                .await,
                expected
            );
        }
        let symlink = directory.path().join("link");
        std::os::unix::fs::symlink(path, &symlink).unwrap();
        assert_eq!(
            helper(
                None,
                &Request {
                    paths: vec![symlink],
                    keychain: None
                }
            )
            .await,
            Observation::Unknown
        );
    }

    #[test]
    fn cached_execution_does_not_probe_and_expires_closed() {
        let custody = Custody::new(Path::new("/no-such-node-custody"));
        let denied = Status {
            command: Observation::Denied,
            browser: Observation::Denied,
            developer: Observation::Denied,
        };
        *custody.cached.write().unwrap() = Some(Cached {
            status: denied,
            at: Instant::now(),
            identities: vec![],
        });
        for _ in 0..1000 {
            custody.require(true).unwrap();
        }
        assert_eq!(custody.status(), denied);
        custody.cached.write().unwrap().as_mut().unwrap().at = Instant::now() - CADENCE - BOUND * 9;
        assert!(custody.require(true).is_err());
        custody.require(false).unwrap();
    }

    #[test]
    fn helper_refuses_real_credential_identifiers() {
        for key in [
            "auth_token",
            "vault",
            "cred/example",
            "object/v1/other",
            "object/v1/custody/not-uuid",
        ] {
            assert_eq!(
                observe_key(&KeyProbe {
                    owner: "test".into(),
                    key: key.into(),
                    action: Action::Read
                }),
                Observation::Unknown
            );
        }
    }

    #[test]
    fn sibling_keys_remain_probe_targets_without_config_and_links_are_unknown() {
        let root = tempfile::tempdir().unwrap();
        let selected = root.path().join("profiles/selected");
        let sibling = root.path().join("profiles/sibling");
        std::fs::create_dir_all(&selected).unwrap();
        std::fs::create_dir(&sibling).unwrap();
        std::fs::write(selected.join("config.toml"), b"public fixture").unwrap();
        let key = sibling.join(".keyfile");
        std::fs::write(&key, b"public key sentinel").unwrap();
        assert!(targets(&selected).unwrap().contains(&key));
        std::fs::remove_file(&key).unwrap();
        std::os::unix::fs::symlink(sibling.join("absent"), key).unwrap();
        assert!(
            targets(&selected).is_err(),
            "broken links cannot hide a target"
        );
    }

    /// Explicit disposable native acceptance, inert during ordinary discovery.
    /// Run as root with a private D-Bus/Secret Service and existing worker UIDs.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn native_linux_custody() {
        use std::os::unix::fs::PermissionsExt;
        let Some(root) = std::env::var_os("NYXID_TEST_CUSTODY_DIRECTORY") else {
            return;
        };
        assert_eq!(unsafe { libc::geteuid() }, 0);
        let root = PathBuf::from(root);
        assert!(
            std::fs::read_dir(&root).unwrap().next().is_none(),
            "fresh empty fixture required"
        );
        let names = std::env::var("NYXID_TEST_CUSTODY_IDENTITIES").unwrap();
        let identities = names
            .split(',')
            .map(|name| Some(Identity::resolve(Some(name)).unwrap()))
            .collect::<Vec<_>>();
        let identities: [Option<Identity>; 3] = identities.try_into().ok().unwrap();
        assert!(
            identities
                .iter()
                .all(|identity| identity.as_ref().unwrap().uid != 0)
        );
        assert_eq!(
            identities
                .iter()
                .map(|identity| identity.as_ref().unwrap().uid)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3,
            "three distinct worker identities required"
        );
        assert!(preflight_files(&identities).await);
        let selected = root.join("profiles/selected");
        let sibling = root.join("profiles/sibling");
        for directory in [&selected, &sibling] {
            mk_private_dir(directory);
            let config = NodeConfig::new(
                "wss://example.test".into(),
                uuid::Uuid::new_v4().to_string(),
                "file".into(),
            );
            config.save(&directory.join("config.toml")).unwrap();
            std::fs::write(directory.join(".keyfile"), [7; 32]).unwrap();
            std::fs::set_permissions(
                directory.join(".keyfile"),
                std::fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        let custody = Custody::new(&selected);
        assert!(
            custody
                .observe_uncached(&identities)
                .await
                .unwrap()
                .all_denied()
        );
        // A custom alias must not inherit the private destination's successful
        // claim when a different identity can replace its alias parent entry.
        let mutable = root.join("mutable");
        mk_private_dir(&mutable);
        std::os::unix::fs::symlink(root.join("profiles"), mutable.join("alias")).unwrap();
        std::fs::set_permissions(&mutable, std::fs::Permissions::from_mode(0o777)).unwrap();
        let alias_custody = Custody::new(&mutable.join("alias/selected"));
        alias_custody.observe(&identities).await;
        assert_eq!(alias_custody.status(), Status::default());
        assert!(alias_custody.require(true).is_err());
        let files = targets(&selected).unwrap();
        assert!(
            files.contains(&selected.join(".keyfile")) && files.contains(&sibling.join(".keyfile"))
        );
        assert_eq!(
            helper(
                None,
                &Request {
                    paths: files.clone(),
                    keychain: None
                }
            )
            .await,
            Observation::Accessible
        );
        // Positive control: real successful opens under each dropped UID.
        let public = root.join("public");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(&public, b"non-secret permission control").unwrap();
        std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o644)).unwrap();
        for identity in identities.iter().flatten() {
            assert_eq!(
                helper(
                    Some(identity),
                    &Request {
                        paths: vec![public.clone()],
                        keychain: None
                    }
                )
                .await,
                Observation::Accessible
            );
        }
        let mut key = KeyProbe {
            owner: uuid::Uuid::new_v4().to_string(),
            key: format!("object/v1/custody/{}", uuid::Uuid::new_v4()),
            action: Action::Seed,
        };
        assert_eq!(
            helper(
                None,
                &Request {
                    paths: vec![],
                    keychain: Some(key.clone())
                }
            )
            .await,
            Observation::Denied
        );
        key.action = Action::Read;
        assert_eq!(
            helper(
                None,
                &Request {
                    paths: vec![],
                    keychain: Some(key.clone())
                }
            )
            .await,
            Observation::Accessible
        );
        for identity in identities.iter().flatten() {
            assert_eq!(
                helper(
                    Some(identity),
                    &Request {
                        paths: vec![],
                        keychain: Some(key.clone())
                    }
                )
                .await,
                Observation::Denied,
                "real key-service permission denial required"
            );
        }
        key.action = Action::Delete;
        assert_eq!(
            helper(
                None,
                &Request {
                    paths: vec![],
                    keychain: Some(key)
                }
            )
            .await,
            Observation::Denied
        );
        let mut config = NodeConfig::load(&selected.join("config.toml")).unwrap();
        config.storage_backend = "keychain".into();
        config.save(&selected.join("config.toml")).unwrap();
        assert!(
            custody
                .observe_uncached(&identities)
                .await
                .unwrap()
                .all_denied()
        );
        // A replaced OS account cannot inherit an earlier identity observation.
        let mut changed = identities;
        changed[0].as_mut().unwrap().uid = 0;
        assert!(
            !custody
                .observe_uncached(&changed)
                .await
                .unwrap()
                .all_denied()
        );
    }
}
