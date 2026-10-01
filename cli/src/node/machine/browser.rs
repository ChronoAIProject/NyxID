//! Supervisor-owned policies, signed filler package and native messaging. No CDP.
use super::process::Identity;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
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
    json!({"DeveloperToolsAvailability":2,"RemoteDebuggingAllowed":false,"URLBlocklist":["javascript:*"],"PasswordManagerEnabled":false,"AutofillAddressEnabled":false,"AutofillCreditCardEnabled":false,"BrowserSignin":0,"SyncDisabled":true,"ExtensionInstallForcelist":[format!("{id};{update_url}")],"ExtensionSettings":{"*":{"installation_mode":"blocked"},id:{"installation_mode":"force_installed","update_url":update_url,"override_update_url":true}},"NativeMessagingBlocklist":["*"],"NativeMessagingAllowlist":[NATIVE_HOST],"NativeMessagingUserLevelHosts":false})
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

fn runtime_directory(path: &Path, uid: u32, gid: u32, mode: u32) -> Result<()> {
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

fn protected_runtime_parent(directory: &Path) -> Result<()> {
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

pub struct Browser {
    connection: Arc<Mutex<Option<UnixStream>>>,
    ready: Arc<Notify>,
    child: Mutex<Option<tokio::process::Child>>,
    accept: tokio::task::JoinHandle<()>,
    updates: tokio::task::JoinHandle<()>,
    socket: PathBuf,
}

impl Browser {
    pub async fn launch(
        directory: &Path,
        identity: &Identity,
        binary: &Path,
        port: u16,
        container: bool,
    ) -> Result<Self> {
        protected_runtime_parent(directory)?;
        let run = directory.join("browser-run");
        runtime_directory(&run, unsafe { libc::geteuid() }, identity.gid, 0o750)?;
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
        let tcp = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
        let actual_port = tcp.local_addr()?.port();
        let id = pin()["extension_id"]
            .as_str()
            .context("invalid extension pin")?
            .to_owned();
        let xml = format!(
            "<?xml version=\"1.0\"?><gupdate xmlns=\"http://www.google.com/update2/response\" protocol=\"2.0\"><app appid=\"{id}\"><updatecheck codebase=\"http://127.0.0.1:{actual_port}/filler.crx\" version=\"1.0.0\"/></app></gupdate>"
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
        let profile = directory.join("browser-profile");
        runtime_directory(&profile, identity.uid, identity.gid, 0o700)?;
        let mut command = Command::new(binary);
        identity.prepare(&mut command)?;
        for key in ["DISPLAY", "XAUTHORITY", "DBUS_SESSION_BUS_ADDRESS"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .env("NYXID_BROWSER_SOCKET", &socket)
            .arg(format!("--user-data-dir={}", profile.display()))
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-sync",
                "--disable-breakpad",
                "--disable-crash-reporter",
                "--password-store=basic",
                "--window-size=1280,800",
                "about:blank",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if container {
            if !cfg!(target_os = "linux") || identity.uid == 0 {
                bail!("container browser requires the isolated non-root browser user");
            }
            command.arg("--disable-setuid-sandbox");
        }
        command.kill_on_drop(true);
        let child = command.spawn().context("managed browser unavailable")?;
        let accepted = ready.clone();
        let shared = connection.clone();
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
                    *shared.lock().await = Some(stream);
                    accepted.notify_waiters();
                }
            }
        });
        let updates = tokio::spawn(async move {
            let _ = axum::serve(tcp, router).await;
        });
        Ok(Self {
            connection,
            ready,
            child: Mutex::new(Some(child)),
            accept,
            updates,
            socket,
        })
    }

    pub async fn ready(&self) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
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
        let mut connection = self.connection.lock().await;
        let stream = connection
            .as_mut()
            .context("managed browser extension unavailable; check the installed policies")?;
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
        let response = tokio::time::timeout(Duration::from_secs(15), async {
            write_native(stream, &bytes).await?;
            read_native(stream).await
        })
        .await;
        let raw = match response {
            Ok(Ok(bytes)) => bytes,
            _ => {
                *connection = None;
                bail!("managed browser did not acknowledge filling; never retry automatically");
            }
        };
        let response: Value = serde_json::from_slice(&raw).context("invalid browser response")?;
        if response["nonce"] != nonce {
            *connection = None;
            bail!("managed browser nonce mismatch");
        }
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
        self.updates.abort();
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
        assert_eq!(value["URLBlocklist"], json!(["javascript:*"]));
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
