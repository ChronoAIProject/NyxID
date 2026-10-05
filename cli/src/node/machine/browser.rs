//! Supervisor-owned policies, signed filler package and native messaging. No CDP.
use super::process::Identity;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    process::Command,
    sync::{Mutex, Notify},
};
use zeroize::Zeroizing;

const PACKAGE: &[u8] = include_bytes!("../../../resources/machine-browser/filler.crx");
const PIN: &str = include_str!("../../../resources/machine-browser/package.json");
const MAX_NATIVE: usize = 128 * 1024;
pub const NATIVE_HOST: &str = "dev.nyxid.machine_filler";

pub fn pin() -> Value {
    serde_json::from_str(PIN).expect("embedded extension pin")
}
pub fn policy(update_url: &str) -> Value {
    let id = pin()["extension_id"]
        .as_str()
        .expect("extension id")
        .to_owned();
    json!({"DeveloperToolsAvailability":2,"RemoteDebuggingAllowed":false,"URLBlocklist":["javascript:*", "file://*"],"PasswordManagerEnabled":false,"AutofillAddressEnabled":false,"AutofillCreditCardEnabled":false,"BrowserSignin":0,"SyncDisabled":true,"ExtensionInstallForcelist":[format!("{id};{update_url}")],"ExtensionSettings":{"*":{"installation_mode":"blocked"},id:{"installation_mode":"force_installed","update_url":update_url,"override_update_url":true}},"NativeMessagingBlocklist":["*"],"NativeMessagingAllowlist":[NATIVE_HOST],"NativeMessagingUserLevelHosts":false})
}

pub fn dev_policy() -> Value {
    json!({
        "DeveloperToolsAvailability": 1,
        "RemoteDebuggingAllowed": true,
        "URLBlocklist": ["file://*"],
        "PasswordManagerEnabled": false,
        "AutofillAddressEnabled": false,
        "AutofillCreditCardEnabled": false,
        "BrowserSignin": 0,
        "SyncDisabled": true,
        "ExtensionSettings": {"*": {"installation_mode": "blocked"}},
        "NativeMessagingBlocklist": ["*"],
        "NativeMessagingUserLevelHosts": false
    })
}

pub fn macos_policy(update_url: &str) -> Result<Vec<u8>> {
    let value: plist::Value =
        serde_json::from_value(policy(update_url)).context("could not encode browser policy")?;
    let mut bytes = Vec::new();
    value.to_writer_xml(&mut bytes)?;
    Ok(bytes)
}

pub fn manifest(launcher: &Path) -> Value {
    json!({"name":NATIVE_HOST,"description":"NyxID supervised saved-login filler","path":launcher,"type":"stdio","allowed_origins":[format!("chrome-extension://{}/",pin()["extension_id"].as_str().unwrap_or_default())]})
}

/// Refuse writable or symlinked ancestors before installing supervisor files.
/// `root` is the system root (or an isolated root used by installation tests).
fn owned_directory(root: &Path, relative: &Path) -> Result<PathBuf> {
    let mut path = root.canonicalize()?;
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            bail!("invalid managed browser directory");
        };
        path.push(component);
        match std::fs::create_dir(&path) {
            Ok(()) => std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o022 != 0
        {
            bail!(
                "managed browser directories must be supervisor-owned and not writable by other users"
            );
        }
    }
    Ok(path)
}

fn write_owned(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().context("invalid managed policy path")?;
    std::fs::create_dir_all(parent)?;
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("managed browser file must not be a symlink");
    }
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    file.write_all(bytes)?;
    file.as_file()
        .set_permissions(std::fs::Permissions::from_mode(mode))?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

/// Docker's AppArmor profile forbids bwrap policy mounts. Chromium reads only
/// its own group's policy file; both files remain supervisor-owned and immutable
/// to either browser. Native Linux continues using the private bwrap policy mount.
pub fn install_container_policies(root: &Path, secure_gid: u32, dev_gid: u32) -> Result<()> {
    anyhow::ensure!(
        secure_gid != dev_gid,
        "Browser policy groups must be distinct"
    );
    let policies = owned_directory(root, Path::new("etc/chromium/policies/managed"))?;
    write_owned(
        &policies.join("nyxid-dev.json"),
        &serde_json::to_vec(&dev_policy())?,
        0o640,
    )?;
    for (path, gid, mode) in [
        (policies.join("nyxid.json"), secure_gid, 0o640),
        (policies.join("nyxid-dev.json"), dev_gid, 0o640),
        (
            root.join("etc/chromium/native-messaging-hosts/dev.nyxid.machine_filler.json"),
            secure_gid,
            0o640,
        ),
        (
            root.join("opt/nyxid/machine-browser/native-host"),
            secure_gid,
            0o750,
        ),
        (
            root.join("opt/nyxid/machine-browser/nyxid-native-host"),
            secure_gid,
            0o750,
        ),
    ] {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        anyhow::ensure!(
            file.metadata()?.is_file(),
            "Managed policy must be a regular file"
        );
        // File descriptor ownership avoids following a replaced path.
        use std::os::fd::AsRawFd;
        if unsafe { libc::fchown(file.as_raw_fd(), libc::geteuid(), gid) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

pub(super) fn runtime_directory(path: &Path, uid: u32, gid: u32, mode: u32) -> Result<()> {
    match std::fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || ![uid, unsafe { libc::geteuid() }].contains(&metadata.uid()) {
        bail!("managed browser runtime directory has an unsafe owner or is a symlink");
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    if unsafe { libc::geteuid() } == 0 {
        chown(path, uid, gid)?;
    }
    Ok(())
}

pub(super) fn protected_runtime_parent(directory: &Path) -> Result<()> {
    let supervisor = unsafe { libc::geteuid() };
    std::fs::create_dir_all(directory)?;
    if std::fs::symlink_metadata(directory)?
        .file_type()
        .is_symlink()
    {
        bail!("managed browser data directory must not be a symlink");
    }
    let canonical = directory.canonicalize()?;
    for ancestor in canonical.ancestors() {
        let metadata = std::fs::metadata(ancestor)?;
        let sticky_root = metadata.uid() == 0 && metadata.mode() & 0o1000 != 0;
        if ![0, supervisor].contains(&metadata.uid())
            || (metadata.mode() & 0o022 != 0 && !sticky_root)
        {
            bail!("managed browser data directory must be protected from other OS users");
        }
    }
    Ok(())
}

/// Called only by the privileged setup helper. Installs no credentials.
pub fn install(system_root: &Path, binary: &Path, update_url: &str, macos: bool) -> Result<()> {
    if hex::encode(Sha256::digest(PACKAGE)) != pin()["sha256"].as_str().unwrap_or_default() {
        bail!("filler package checksum mismatch");
    }
    let resources = owned_directory(system_root, Path::new("opt/nyxid/machine-browser"))?;
    // Do not execute a user-writable CLI from a privileged native-host manifest.
    // Stage a private copy at setup; subsequent CLI updates require policy setup.
    let executable_path = resources.join("nyxid-native-host");
    let mut input = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(binary)?;
    if !input.metadata()?.is_file() {
        bail!("native-host executable must be a regular file");
    }
    let mut executable_file = tempfile::NamedTempFile::new_in(&resources)?;
    std::io::copy(&mut input, &mut executable_file)?;
    executable_file
        .as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o755))?;
    executable_file.as_file().sync_all()?;
    executable_file.persist(&executable_path)?;
    write_owned(&resources.join("filler.crx"), PACKAGE, 0o644)?;
    let dev_policies = owned_directory(
        system_root,
        Path::new("opt/nyxid/machine-browser/dev-policies"),
    )?;
    write_owned(
        &dev_policies.join("nyxid.json"),
        &serde_json::to_vec(&dev_policy())?,
        0o644,
    )?;
    let launcher = resources.join("native-host");
    let executable = shlex::try_quote(
        executable_path
            .to_str()
            .context("invalid executable path")?,
    )
    .map_err(|_| anyhow::anyhow!("invalid executable path"))?;
    write_owned(
        &launcher,
        format!("#!/bin/sh\nexec {executable} node machine-native-host \"$@\"\n").as_bytes(),
        0o755,
    )?;
    if macos {
        owned_directory(system_root, Path::new("Library/Managed Preferences"))?;
        owned_directory(
            system_root,
            Path::new("Library/Google/Chrome/NativeMessagingHosts"),
        )?;
        write_owned(
            &system_root.join("Library/Managed Preferences/com.google.Chrome.plist"),
            &macos_policy(update_url)?,
            0o644,
        )?;
        let dev: plist::Value = serde_json::from_value(dev_policy())?;
        let mut dev_bytes = Vec::new();
        dev.to_writer_xml(&mut dev_bytes)?;
        write_owned(
            &system_root.join("Library/Managed Preferences/com.google.Chrome.beta.plist"),
            &dev_bytes,
            0o644,
        )?;
        write_owned(
            &system_root
                .join("Library/Google/Chrome/NativeMessagingHosts/dev.nyxid.machine_filler.json"),
            &serde_json::to_vec(&manifest(&launcher))?,
            0o644,
        )?;
    } else {
        owned_directory(system_root, Path::new("etc/chromium/policies/managed"))?;
        owned_directory(
            system_root,
            Path::new("etc/chromium/native-messaging-hosts"),
        )?;
        write_owned(
            &system_root.join("etc/chromium/policies/managed/nyxid.json"),
            &serde_json::to_vec(&policy(update_url))?,
            0o644,
        )?;
        write_owned(
            &system_root.join("etc/chromium/native-messaging-hosts/dev.nyxid.machine_filler.json"),
            &serde_json::to_vec(&manifest(&launcher))?,
            0o644,
        )?;
    }
    Ok(())
}

/// Runs as the browser user, with Chromium stopped. Deleting the package alone
/// leaves Chromium's policy-install registration intact: it will not reinstall
/// the same version. Clear only this extension's registration and integrity MAC
/// in both preference stores, so force-install starts a fresh signed download.
/// Cookies, website storage and every unrelated preference remain untouched.
pub fn refresh_extension(profile: &Path, force: bool) -> Result<()> {
    if unsafe { libc::geteuid() } == 0 {
        bail!("Extension cache refresh must run as the browser user");
    }
    let id = pin()["extension_id"]
        .as_str()
        .context("extension id")?
        .to_owned();
    for entry in std::fs::read_dir(profile)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let directory = entry.path();
        let installed = directory.join("Extensions").join(&id);
        let present = std::fs::read_dir(&installed).ok().is_some_and(|entries| {
            entries.filter_map(Result::ok).any(|version| {
                version.file_type().is_ok_and(|kind| kind.is_dir())
                    && std::fs::read(version.path().join("background.js")).is_ok_and(|bytes| {
                        bytes == include_bytes!("../../../resources/machine-browser/background.js")
                    })
            })
        });
        if !force && present {
            continue;
        }
        for filename in ["Preferences", "Secure Preferences"] {
            let path = directory.join(filename);
            let mut input = match std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            anyhow::ensure!(
                input.metadata()?.len() <= 32 * 1024 * 1024,
                "Browser preferences exceed limit"
            );
            let mut bytes = Zeroizing::new(Vec::new());
            use std::io::Read;
            input.read_to_end(&mut bytes)?;
            let mut preferences: Value =
                serde_json::from_slice(&bytes).context("Invalid browser preferences")?;
            for pointer in [
                "/extensions/settings",
                "/protection/macs/extensions/settings",
            ] {
                if let Some(settings) = preferences
                    .pointer_mut(pointer)
                    .and_then(Value::as_object_mut)
                {
                    settings.remove(&id);
                }
            }
            let bytes = Zeroizing::new(serde_json::to_vec(&preferences)?);
            write_owned(&path, &bytes, 0o600)?;
        }
        match std::fs::remove_dir_all(&installed) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum Failure {
    #[error("secure_browser_extension_unavailable")]
    ExtensionUnavailable,
    #[error("secure_browser_already_running")]
    AlreadyRunning,
    #[error("secure_browser_transport_write")]
    TransportWrite,
    #[error("secure_browser_transport_read")]
    TransportRead,
    #[error("secure_browser_transport_timeout")]
    TransportTimeout,
    #[error("secure_browser_response_invalid")]
    ResponseInvalid,
}
impl Failure {
    pub fn message(self) -> &'static str {
        match self {
            Self::ExtensionUnavailable => {
                "machine_browser_unavailable: secure browser extension did not connect after automatic repair; retry once, then offer a machine update or reinstall managed browser policies"
            }
            Self::AlreadyRunning => {
                "machine_browser_unavailable: another supervisor owns this secure browser; use the running daemon instead of starting a second node"
            }
            Self::TransportWrite => {
                "machine_browser_unavailable: secure browser connection closed while sending; observe before retrying"
            }
            Self::TransportRead => {
                "machine_browser_unavailable: secure browser connection closed while receiving; observe before retrying"
            }
            Self::TransportTimeout => {
                "machine_browser_unavailable: secure browser response timed out; observe before retrying"
            }
            Self::ResponseInvalid => {
                "machine_browser_unavailable: secure browser response invalid; retry once, then update the machine"
            }
        }
    }

    fn record(self) -> Self {
        // Fixed classification only: never include native messages or page data.
        tracing::warn!(reason = %self, "secure browser exchange unavailable");
        self
    }
}

// Context browsers share the public signed-package endpoint, never profiles or
// native sockets. Its lifetime must not depend on the legacy browser being alive.
struct PackageServer(tokio::task::JoinHandle<()>);
impl Drop for PackageServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}
static PACKAGE_SERVERS: std::sync::LazyLock<
    Mutex<std::collections::HashMap<u16, std::sync::Weak<PackageServer>>>,
> = std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

impl PackageServer {
    async fn acquire(port: u16) -> Result<Arc<Self>> {
        let mut servers = PACKAGE_SERVERS.lock().await;
        servers.retain(|_, server| server.strong_count() > 0);
        if port != 0
            && let Some(server) = servers.get(&port).and_then(std::sync::Weak::upgrade)
        {
            return Ok(server);
        }
        anyhow::ensure!(servers.len() < 128, "browser_package_server_capacity");
        let tcp = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
        let actual_port = tcp.local_addr()?.port();
        let id = pin()["extension_id"]
            .as_str()
            .context("invalid extension pin")?
            .to_owned();
        let version = pin()["version"]
            .as_str()
            .context("extension version")?
            .to_owned();
        let xml = format!(
            "<?xml version=\"1.0\"?><gupdate xmlns=\"http://www.google.com/update2/response\" protocol=\"2.0\"><app appid=\"{id}\"><updatecheck codebase=\"http://127.0.0.1:{actual_port}/filler.crx\" version=\"{version}\"/></app></gupdate>"
        );
        let router = axum::Router::new()
            .route(
                "/update.xml",
                axum::routing::get(move || async move {
                    ([(axum::http::header::CONTENT_TYPE, "application/xml")], xml)
                }),
            )
            .route(
                "/filler.crx",
                axum::routing::get(|| async {
                    (
                        [(
                            axum::http::header::CONTENT_TYPE,
                            "application/x-chrome-extension",
                        )],
                        PACKAGE,
                    )
                }),
            );
        let server = Arc::new(Self(tokio::spawn(async move {
            let _ = axum::serve(tcp, router).await;
        })));
        servers.insert(actual_port, Arc::downgrade(&server));
        Ok(server)
    }
}

pub struct Browser {
    connection: Arc<Mutex<Option<UnixStream>>>,
    ready: Arc<Notify>,
    /// A separated context must prove one complete extension exchange before
    /// its first user action.  The extension hello only proves that the
    /// native host connected; it does not prove that the MV3 worker can yet
    /// service a request after a fresh profile launch.
    context_browser: bool,
    startup_verified: Arc<AtomicBool>,
    connection_generation: Arc<AtomicU64>,
    child: Mutex<Option<tokio::process::Child>>,
    accept: tokio::task::JoinHandle<()>,
    _updates: Arc<PackageServer>,
    socket: PathBuf,
    _lock: std::fs::File,
}

impl Browser {
    pub async fn launch(
        directory: &Path,
        identity: &Identity,
        binary: &Path,
        port: u16,
        container: bool,
        repair: bool,
    ) -> Result<Self> {
        protected_runtime_parent(directory)?;
        // Keep per-context Unix socket paths below sockaddr_un.sun_path even
        // for named VM profiles. Legacy paths remain unchanged.
        let run = directory.join(if identity.desktop.is_some() {
            "r"
        } else {
            "browser-run"
        });
        runtime_directory(&run, unsafe { libc::geteuid() }, identity.gid, 0o750)?;
        use std::os::fd::AsRawFd;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(run.join("supervisor.lock"))?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(Failure::AlreadyRunning.into());
        }
        let socket = run.join("filler.sock");
        if socket.exists() {
            std::fs::remove_file(&socket)?;
        }
        let listener = UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o660))?;
        if unsafe { libc::geteuid() } == 0 {
            chown(&socket, 0, identity.gid)?;
        }
        let connection = Arc::new(Mutex::new(None));
        let ready = Arc::new(Notify::new());
        let startup_verified = Arc::new(AtomicBool::new(false));
        let connection_generation = Arc::new(AtomicU64::new(0));
        let updates = PackageServer::acquire(port).await?;
        let profile = directory.join("browser-profile");
        runtime_directory(&profile, identity.uid, identity.gid, 0o700)?;
        let package_hash = hex::encode(Sha256::digest(PACKAGE));
        let marker = run.join("extension-package-sha256");
        let force =
            repair || std::fs::read_to_string(&marker).ok().as_deref() != Some(&package_hash);
        // Always check for a missing installed directory, even with a current
        // package marker (including profiles damaged by the old refresh path).
        let mut refresh = Command::new(std::env::current_exe()?);
        identity.prepare(&mut refresh)?;
        refresh
            .args(["node", "machine-browser-refresh"])
            .arg(&profile);
        if force {
            refresh.arg("--force");
        }
        let result = refresh
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await?;
        if !result.success() {
            bail!("Managed extension refresh failed");
        }
        write_owned(&marker, package_hash.as_bytes(), 0o600)?;
        let mut command = Command::new(binary);
        identity.prepare(&mut command)?;
        identity.desktop_env(&mut command);
        command
            .env("NYXID_BROWSER_SOCKET", &socket)
            .arg(format!("--user-data-dir={}", profile.display()))
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-sync",
                "--force-renderer-accessibility",
                "--disable-breakpad",
                "--disable-crash-reporter",
                "--password-store=basic",
                "--window-size=1280,800",
                "about:blank",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if container && (!cfg!(target_os = "linux") || identity.uid == 0) {
            bail!("container browser requires the isolated non-root browser user");
        }
        command.kill_on_drop(true);
        let child = command.spawn().context("managed browser unavailable")?;
        let accepted = ready.clone();
        let shared = connection.clone();
        let accepted_startup = startup_verified.clone();
        let accepted_generation = connection_generation.clone();
        let browser_uid = identity.uid;
        let accept = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                if !stream
                    .peer_cred()
                    .is_ok_and(|cred| cred.uid() == browser_uid)
                {
                    continue;
                }
                let hello =
                    tokio::time::timeout(Duration::from_secs(5), read_native(&mut stream)).await;
                if hello
                    .ok()
                    .and_then(Result::ok)
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .is_some_and(|v| {
                        v["type"] == "hello" && v["extension_id"] == pin()["extension_id"]
                    })
                {
                    accepted_startup.store(false, Ordering::Release);
                    accepted_generation.fetch_add(1, Ordering::AcqRel);
                    *shared.lock().await = Some(stream);
                    accepted.notify_waiters();
                }
            }
        });
        Ok(Self {
            connection,
            ready,
            context_browser: identity.desktop.is_some(),
            startup_verified,
            connection_generation,
            child: Mutex::new(Some(child)),
            accept,
            _updates: updates,
            socket,
            _lock: lock,
        })
    }

    pub async fn ready(&self) -> bool {
        self.wait_ready(Duration::from_secs(12)).await
    }

    pub async fn wait_ready(&self, grace: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + grace;
        loop {
            let ready = self.ready.notified();
            if self.connection.lock().await.is_some() {
                return true;
            }
            if tokio::time::timeout_at(deadline, ready).await.is_err() {
                return false;
            }
        }
    }

    pub fn is_context_browser(&self) -> bool {
        self.context_browser
    }

    /// Wait for a fresh separated-context browser to complete a harmless
    /// native exchange.  This is deliberately separate from the normal
    /// response timeout: Chromium/profile/extension startup gets a bounded
    /// grace, while each real operation retains its existing 20 s deadline.
    pub async fn wait_startup(&self, grace: Duration) -> bool {
        if !self.context_browser {
            return self.ready().await;
        }
        if self.startup_verified.load(Ordering::Acquire) {
            if self.connection.lock().await.is_some() {
                return true;
            }
            self.startup_verified.store(false, Ordering::Release);
        }
        let deadline = tokio::time::Instant::now() + grace;
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return false;
            }
            let remaining = deadline.saturating_duration_since(now);
            if !self.wait_ready(remaining.min(Duration::from_secs(3))).await {
                continue;
            }
            let nonce = uuid::Uuid::new_v4().to_string();
            let probe = json!({
                "operation": "browser",
                "action": "tabs",
                "nonce": nonce,
                "expires_at_ms": chrono::Utc::now().timestamp_millis() + 5000,
            });
            let bytes = match serde_json::to_vec(&probe) {
                Ok(bytes) => Zeroizing::new(bytes),
                Err(_) => return false,
            };
            let timeout = deadline
                .saturating_duration_since(tokio::time::Instant::now())
                .min(Duration::from_secs(4));
            if timeout.is_zero() {
                return false;
            }
            let generation = self.connection_generation.load(Ordering::Acquire);
            match self.exchange_with_timeout(&nonce, &bytes, timeout).await {
                Ok(response)
                    if response["status"] == "ok"
                        && self.connection_generation.load(Ordering::Acquire) == generation =>
                {
                    self.startup_verified.store(true, Ordering::Release);
                    return true;
                }
                Ok(_) | Err(_) => {
                    // A failed probe consumes its stream.  The extension will
                    // reconnect; retry within the startup grace instead of
                    // handing the first user action a stale pipe.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    pub async fn stop(&self) {
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }

    pub async fn alive(&self) -> bool {
        self.child
            .lock()
            .await
            .as_mut()
            .is_some_and(|child| child.try_wait().is_ok_and(|exit| exit.is_none()))
    }

    pub async fn action(&self, mut parameters: Value) -> Result<Value> {
        if !self.ready().await {
            return Err(Failure::ExtensionUnavailable.into());
        }
        let nonce = uuid::Uuid::new_v4().to_string();
        parameters["operation"] = json!("browser");
        parameters["nonce"] = json!(nonce);
        parameters["expires_at_ms"] = json!(chrono::Utc::now().timestamp_millis() + 20000);
        let bytes = zeroize::Zeroizing::new(serde_json::to_vec(&parameters)?);
        let mut response = self.exchange(&nonce, &bytes).await?;
        if let Some(object) = response.as_object_mut() {
            object.remove("nonce");
        }
        Ok(response)
    }

    async fn exchange(&self, nonce: &str, bytes: &[u8]) -> Result<Value> {
        self.exchange_with_timeout(nonce, bytes, Duration::from_secs(20))
            .await
    }

    async fn exchange_with_timeout(
        &self,
        nonce: &str,
        bytes: &[u8],
        timeout: Duration,
    ) -> Result<Value> {
        // Take ownership before I/O. Dropping a cancelled exchange closes the
        // stream, forcing a fresh handshake; a late reply cannot poison a turn.
        let generation = self.connection_generation.load(Ordering::Acquire);
        let mut stream = {
            let mut connection = self.connection.lock().await;
            connection.take().context(Failure::ExtensionUnavailable)?
        };
        let raw = tokio::time::timeout(timeout, async {
            write_native(&mut stream, bytes)
                .await
                .map_err(|_| Failure::TransportWrite.record())?;
            read_native(&mut stream)
                .await
                .map_err(|_| Failure::TransportRead.record())
        })
        .await
        .map_err(|_| Failure::TransportTimeout.record())??;
        let response: Value =
            serde_json::from_slice(&raw).map_err(|_| Failure::ResponseInvalid.record())?;
        if response["nonce"] != nonce {
            return Err(Failure::ResponseInvalid.record().into());
        }
        // Accept can install a replacement while this request is in flight.
        // Never let the old exchange overwrite the new stream, and never hold
        // the mutex across I/O (otherwise reconnect handshakes deadlock behind
        // the response timeout).
        if self.connection_generation.load(Ordering::Acquire) == generation {
            let mut connection = self.connection.lock().await;
            if connection.is_none() {
                *connection = Some(stream);
            }
        }
        Ok(response)
    }

    pub async fn fill(&self, field: &str, origins: &[String], value: &str) -> Result<Value> {
        if self
            .child
            .lock()
            .await
            .as_mut()
            .context("managed browser unavailable")?
            .try_wait()?
            .is_some()
        {
            bail!("managed browser unavailable");
        }
        if !self.ready().await {
            bail!("managed browser extension unavailable; check the installed policies");
        }
        let nonce = uuid::Uuid::new_v4().to_string();
        #[derive(serde::Serialize)]
        struct Fill<'a> {
            nonce: &'a str,
            expires_at_ms: i64,
            allowed_origins: &'a [String],
            field: &'a str,
            value: &'a str,
        }
        let request = Fill {
            nonce: &nonce,
            expires_at_ms: chrono::Utc::now().timestamp_millis() + 15000,
            allowed_origins: origins,
            field,
            value,
        };
        let bytes = Zeroizing::new(serde_json::to_vec(&request)?);
        let response = self.exchange(&nonce, &bytes).await?;
        if response["status"] == "filled"
            && response["field"] == field
            && origins.iter().any(|origin| response["origin"] == *origin)
        {
            return Ok(json!({"status":"filled","field":field,"origin":response["origin"]}));
        }
        let reason = match response["reason"].as_str() {
            Some("origin_mismatch") => "origin_mismatch",
            Some("wrong_field" | "no_suitable_focused_field") => "wrong_field",
            Some("focus_changed") => "focus_changed",
            _ => "input_refused",
        };
        Ok(json!({"status":"refused","reason":reason}))
    }
}
impl Drop for Browser {
    fn drop(&mut self) {
        self.accept.abort();
        let _ = std::fs::remove_file(&self.socket);
    }
}

pub(super) fn chown(path: &Path, uid: u32, gid: u32) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    if unsafe { libc::chown(path.as_ptr(), uid, gid) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
async fn read_native(reader: &mut (impl AsyncRead + Unpin)) -> Result<Zeroizing<Vec<u8>>> {
    let length = reader.read_u32_le().await? as usize;
    if length == 0 || length > MAX_NATIVE {
        bail!("native message limit exceeded");
    }
    let mut bytes = Zeroizing::new(vec![0u8; length]);
    reader.read_exact(&mut bytes).await?;
    Ok(bytes)
}
async fn write_native(writer: &mut (impl AsyncWrite + Unpin), bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_NATIVE {
        bail!("native message limit exceeded");
    }
    writer.write_u32_le(bytes.len() as u32).await?;
    writer.write_all(bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// Only the signed extension's native host origin is accepted. Messages contain
/// login values; errors and process output deliberately contain no payload.
pub async fn native_host(origin: &str) -> Result<()> {
    if origin
        != format!(
            "chrome-extension://{}/",
            pin()["extension_id"].as_str().unwrap_or_default()
        )
    {
        bail!("native host origin refused");
    }
    let socket =
        std::env::var_os("NYXID_BROWSER_SOCKET").context("managed browser socket unavailable")?;
    let stream = UnixStream::connect(PathBuf::from(socket)).await?;
    let (mut reader, mut writer) = stream.into_split();
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    tokio::select! {
        result=bridge_native(&mut stdin, &mut writer)=>result,
        result=bridge_native(&mut reader, &mut stdout)=>result,
    }
}

async fn bridge_native(
    reader: &mut (impl AsyncRead + Unpin),
    writer: &mut (impl AsyncWrite + Unpin),
) -> Result<()> {
    loop {
        let bytes = read_native(reader).await?;
        write_native(writer, &bytes).await?;
    }
}

/// The desktop cookie is accessible to the browser user, never the agent user.
pub fn create_xauthority(path: &Path, browser: &str, display: u16) -> Result<()> {
    let identity = Identity::resolve(Some(browser))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cookie = Zeroizing::new(hex::encode(rand::random::<[u8; 16]>()));
    let mut child = std::process::Command::new("xauth")
        .args([
            "-f",
            path.to_str().context("invalid display path")?,
            "source",
            "-",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    use std::io::Write;
    let command = Zeroizing::new(format!(
        "add :{display} MIT-MAGIC-COOKIE-1 {}\n",
        cookie.as_str()
    ));
    child
        .stdin
        .take()
        .context("xauth input unavailable")?
        .write_all(command.as_bytes())?;
    if !child.wait()?.success() {
        bail!("Could not initialize protected machine display");
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    chown(path, identity.uid, identity.gid)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_transport_failures_are_typed_and_discard_the_connection() {
        for case in ["write", "eof", "decode", "nonce"] {
            let temp = tempfile::tempdir().unwrap();
            let (stream, mut peer) = UnixStream::pair().unwrap();
            let task = if case == "write" {
                drop(peer);
                tokio::spawn(async {})
            } else {
                tokio::spawn(async move {
                    let _request = read_native(&mut peer).await.unwrap();
                    match case {
                        "decode" => write_native(&mut peer, b"invalid-private-content")
                            .await
                            .unwrap(),
                        "nonce" => write_native(&mut peer, br#"{"nonce":"wrong"}"#)
                            .await
                            .unwrap(),
                        _ => {}
                    }
                })
            };
            let browser = Browser {
                connection: Arc::new(Mutex::new(Some(stream))),
                ready: Arc::new(Notify::new()),
                context_browser: false,
                startup_verified: Arc::new(AtomicBool::new(false)),
                connection_generation: Arc::new(AtomicU64::new(0)),
                child: Mutex::new(None),
                accept: tokio::spawn(std::future::pending()),
                _updates: PackageServer::acquire(0).await.unwrap(),
                socket: temp.path().join("unused.sock"),
                _lock: std::fs::File::create(temp.path().join("lock")).unwrap(),
            };
            // Per-run nonce: the "nonce" case replies with a different value.
            let nonce = uuid::Uuid::new_v4().to_string();
            let error = browser.exchange(&nonce, b"{}").await.unwrap_err();
            let failure = error.downcast_ref::<Failure>().unwrap();
            assert!(match case {
                "write" => matches!(failure, Failure::TransportWrite),
                "eof" => matches!(failure, Failure::TransportRead),
                _ => matches!(failure, Failure::ResponseInvalid),
            });
            assert!(!failure.message().contains("private-content"));
            assert!(browser.connection.lock().await.is_none());
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn separated_startup_requires_and_caches_a_real_exchange() {
        let temp = tempfile::tempdir().unwrap();
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let task = tokio::spawn(async move {
            let request = read_native(&mut peer).await.unwrap();
            let request: Value = serde_json::from_slice(&request).unwrap();
            let response = json!({"nonce": request["nonce"], "status": "ok"});
            write_native(&mut peer, &serde_json::to_vec(&response).unwrap())
                .await
                .unwrap();
        });
        let browser = Browser {
            connection: Arc::new(Mutex::new(Some(stream))),
            ready: Arc::new(Notify::new()),
            context_browser: true,
            startup_verified: Arc::new(AtomicBool::new(false)),
            connection_generation: Arc::new(AtomicU64::new(0)),
            child: Mutex::new(None),
            accept: tokio::spawn(std::future::pending()),
            _updates: PackageServer::acquire(0).await.unwrap(),
            socket: temp.path().join("unused.sock"),
            _lock: std::fs::File::create(temp.path().join("lock")).unwrap(),
        };
        assert!(browser.wait_startup(Duration::from_secs(1)).await);
        task.await.unwrap();
        // A successful probe is cached; a later action does not repeat the
        // startup handshake or require another native peer.
        assert!(browser.wait_startup(Duration::from_millis(1)).await);
    }

    #[tokio::test]
    async fn exchange_does_not_block_a_replacement_native_connection() {
        let temp = tempfile::tempdir().unwrap();
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let browser = Arc::new(Browser {
            connection: Arc::new(Mutex::new(Some(stream))),
            ready: Arc::new(Notify::new()),
            context_browser: false,
            startup_verified: Arc::new(AtomicBool::new(false)),
            connection_generation: Arc::new(AtomicU64::new(0)),
            child: Mutex::new(None),
            accept: tokio::spawn(std::future::pending()),
            _updates: PackageServer::acquire(0).await.unwrap(),
            socket: temp.path().join("unused.sock"),
            _lock: std::fs::File::create(temp.path().join("lock")).unwrap(),
        });
        let nonce = uuid::Uuid::new_v4().to_string();
        let response_nonce = nonce.clone();
        let exchange = {
            let browser = browser.clone();
            tokio::spawn(async move { browser.exchange(&nonce, b"{}").await })
        };
        let _request = read_native(&mut peer).await.unwrap();
        // A reconnect handler must be able to install a fresh stream while a
        // stale response is still pending.
        let replacement =
            tokio::time::timeout(Duration::from_millis(100), browser.connection.lock())
                .await
                .expect("exchange held the connection mutex across I/O");
        assert!(replacement.is_none());
        drop(replacement);
        write_native(
            &mut peer,
            &serde_json::to_vec(&json!({"nonce": response_nonce, "status": "ok"})).unwrap(),
        )
        .await
        .unwrap();
        assert!(exchange.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn package_server_survives_legacy_browser_release() {
        let first = PackageServer::acquire(0).await.unwrap();
        let port = PACKAGE_SERVERS
            .lock()
            .await
            .iter()
            .find_map(|(port, server)| {
                server
                    .upgrade()
                    .filter(|server| Arc::ptr_eq(server, &first))
                    .map(|_| *port)
            })
            .unwrap();
        let context = PackageServer::acquire(port).await.unwrap();
        assert!(Arc::ptr_eq(&first, &context));
        drop(first);
        let response = reqwest::get(format!("http://127.0.0.1:{port}/filler.crx"))
            .await
            .unwrap();
        assert_eq!(response.bytes().await.unwrap().as_ref(), PACKAGE);
        assert!(Arc::ptr_eq(
            &PackageServer::acquire(port).await.unwrap(),
            &context
        ));
    }
    #[test]
    fn missing_package_and_forced_refresh_clear_only_managed_registration() {
        if unsafe { libc::geteuid() } == 0 {
            return;
        } // real non-root helper covered in container
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join("Default");
        std::fs::create_dir(&profile).unwrap();
        let id = pin()["extension_id"].as_str().unwrap().to_owned();
        for force in [false, true] {
            let prefs = json!({
                "extensions":{"settings":{&id:{"location":7},"unrelated":{"keep":true}}},
                "protection":{"macs":{"extensions":{"settings":{&id:"old-mac"}}}},
                "other":{"keep":"website preferences"},
            });
            for file in ["Preferences", "Secure Preferences"] {
                std::fs::write(profile.join(file), serde_json::to_vec(&prefs).unwrap()).unwrap();
            }
            let installed = profile.join("Extensions").join(&id).join("1.0.1_0");
            if force {
                std::fs::create_dir_all(&installed).unwrap();
                std::fs::write(
                    installed.join("background.js"),
                    include_bytes!("../../../resources/machine-browser/background.js"),
                )
                .unwrap();
            }
            refresh_extension(root.path(), force).unwrap();
            for file in ["Preferences", "Secure Preferences"] {
                let repaired: Value =
                    serde_json::from_slice(&std::fs::read(profile.join(file)).unwrap()).unwrap();
                assert!(repaired["extensions"]["settings"].get(&id).is_none());
                assert!(
                    repaired["protection"]["macs"]["extensions"]["settings"]
                        .get(&id)
                        .is_none()
                );
                assert_eq!(
                    repaired["extensions"]["settings"]["unrelated"],
                    prefs["extensions"]["settings"]["unrelated"]
                );
                assert_eq!(repaired["other"], prefs["other"]);
            }
            assert!(!installed.exists());
        }
    }

    #[test]
    fn runtime_directories_reject_symlinked_profiles_and_shared_parents() {
        let root = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let profile = root.path().join("profile");
        std::os::unix::fs::symlink(target.path(), &profile).unwrap();
        assert!(
            runtime_directory(
                &profile,
                unsafe { libc::geteuid() },
                unsafe { libc::getegid() },
                0o700
            )
            .is_err()
        );
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(protected_runtime_parent(root.path()).is_err());
    }
    #[test]
    fn linux_and_macos_policy_pin_extension_and_close_debugging() {
        let url = "http://127.0.0.1:47821/update.xml";
        let value = policy(url);
        assert_eq!(value["DeveloperToolsAvailability"], 2);
        assert_eq!(value["RemoteDebuggingAllowed"], false);
        assert_eq!(value["URLBlocklist"], json!(["javascript:*", "file://*"]));
        let id = pin()["extension_id"].as_str().unwrap().to_owned();
        assert_eq!(
            value["ExtensionSettings"][&id]["installation_mode"],
            "force_installed"
        );
        assert_eq!(value["PasswordManagerEnabled"], false);
        let plist = macos_policy(url).unwrap();
        let decoded: plist::Value = plist::from_bytes(&plist).unwrap();
        let dictionary = decoded.as_dictionary().unwrap();
        assert_eq!(
            dictionary["DeveloperToolsAvailability"].as_signed_integer(),
            Some(2)
        );
        assert_eq!(hex::encode(Sha256::digest(PACKAGE)), pin()["sha256"]);
    }
    #[test]
    fn installer_refuses_writable_and_symlinked_policy_directories() {
        let root = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(target.path(), root.path().join("opt")).unwrap();
        assert!(owned_directory(root.path(), Path::new("opt/nyxid")).is_err());
        std::fs::remove_file(root.path().join("opt")).unwrap();
        std::fs::create_dir(root.path().join("opt")).unwrap();
        std::fs::set_permissions(
            root.path().join("opt"),
            std::fs::Permissions::from_mode(0o777),
        )
        .unwrap();
        assert!(owned_directory(root.path(), Path::new("opt/nyxid")).is_err());
    }
    #[test]
    fn installed_policy_package_and_host_are_not_writable_by_agent() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("test-cli");
        std::fs::write(&executable, b"test executable").unwrap();
        install(
            root.path(),
            &executable,
            "http://127.0.0.1:47821/update.xml",
            false,
        )
        .unwrap();
        for file in [
            "opt/nyxid/machine-browser/filler.crx",
            "opt/nyxid/machine-browser/native-host",
            "opt/nyxid/machine-browser/nyxid-native-host",
            "etc/chromium/policies/managed/nyxid.json",
            "etc/chromium/native-messaging-hosts/dev.nyxid.machine_filler.json",
        ] {
            assert_eq!(
                std::fs::metadata(root.path().join(file))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o022,
                0
            );
        }
    }
}
