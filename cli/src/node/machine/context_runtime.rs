//! Supervisor-owned separated context provisioning. No caller paths or UIDs.
//! The allocation journal is fsynced before creating users; old UID/generation
//! mappings are retained forever. This module is not a UID-only fallback.
use super::process::{Identity, context_sandbox::Sandbox};
use anyhow::{Context as _, Result, ensure};
use nyxid_machine::{
    authority::Authority,
    context::{Allocation, Registry},
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        },
    },
    path::{Component, Path, PathBuf},
    sync::Arc,
};
const STORE_LIMIT: u64 = 2 * 1024 * 1024;

pub struct Store {
    path: PathBuf,
    root: PathBuf,
    node: String,
    _lock: File,
    registry: Registry,
}
pub struct Context {
    pub allocation: Allocation,
    pub command: Identity,
    pub secure: Identity,
    pub dev: Identity,
    pub workspace: PathBuf,
    pub browser_root: PathBuf,
}

/// Open a root-owned ancestor chain without following symlinks. Components may
/// be created, but must never be mutable by an agent, including the legacy UID.
fn protected(path: &Path) -> Result<OwnedFd> {
    ensure!(path.is_absolute(), "context_path_invalid");
    let root = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_DIRECTORY | libc::O_RDONLY | libc::O_CLOEXEC,
        )
    };
    ensure!(root >= 0, "context_root_unavailable");
    let mut parent = unsafe { OwnedFd::from_raw_fd(root) };
    for part in path.components() {
        let Component::Normal(part) = part else {
            ensure!(part == Component::RootDir, "context_path_invalid");
            continue;
        };
        let name = std::ffi::CString::new(part.as_bytes())?;
        let created = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o711) } == 0;
        if !created {
            ensure!(
                std::io::Error::last_os_error().raw_os_error() == Some(libc::EEXIST),
                "context_directory_unavailable"
            );
        }
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_DIRECTORY | libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        ensure!(fd >= 0, "context_directory_unavailable");
        parent = unsafe { OwnedFd::from_raw_fd(fd) };
        let meta = File::from(parent.try_clone()?).metadata()?;
        ensure!(
            meta.uid() == 0 && meta.mode() & 0o022 == 0,
            "context_parent_not_private"
        );
        if created {
            // The daemon inherits umask 077. Managed ancestors need traversal;
            // role gates and private children supply the actual DAC boundary.
            own(&parent, 0, 0, 0o711)?;
        }
    }
    Ok(parent)
}
fn own(fd: &OwnedFd, uid: u32, gid: u32, mode: u32) -> Result<()> {
    ensure!(
        unsafe { libc::fchown(fd.as_raw_fd(), uid, gid) } == 0,
        "context_ownership_failed"
    );
    ensure!(
        unsafe { libc::fchmod(fd.as_raw_fd(), mode) } == 0,
        "context_permissions_failed"
    );
    Ok(())
}
fn child(parent: &OwnedFd, name: &str, uid: u32, gid: u32, mode: u32) -> Result<()> {
    let name = std::ffi::CString::new(name)?;
    let created = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), mode) } == 0;
    if !created {
        ensure!(
            std::io::Error::last_os_error().raw_os_error() == Some(libc::EEXIST),
            "context_directory_unavailable"
        );
    }
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_DIRECTORY | libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    ensure!(fd >= 0, "context_directory_unavailable");
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let meta = File::from(fd.try_clone()?).metadata()?;
    // A root-owned directory can be a crash between mkdir and chown. No
    // unprivileged user could traverse its protected parent in that interval.
    ensure!(
        meta.uid() == uid || (meta.uid() == 0 && meta.mode() & 0o077 == 0),
        "context_directory_owner_changed"
    );
    own(&fd, uid, gid, mode)
}
fn role(root: &Path, uid: u32) -> Result<()> {
    let directory = protected(root)?;
    // A user may chmod its home/workspace, but cannot expose it across this gate.
    own(&directory, 0, uid, 0o710)?;
    for name in ["workspace", "home", "tmp"] {
        child(&directory, name, uid, uid, 0o700)?;
    }
    Ok(())
}
fn occupied(uid: u32) -> bool {
    // Reentrant lookups: simultaneous profiles must never race libc's static
    // passwd/group buffers. Lookup failure burns the candidate, not authority.
    let mut buffer = vec![0u8; 65536];
    let mut user = unsafe { std::mem::zeroed() };
    let mut user_result = std::ptr::null_mut();
    let mut group = unsafe { std::mem::zeroed() };
    let mut group_result = std::ptr::null_mut();
    unsafe {
        if libc::getpwuid_r(
            uid,
            &mut user,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut user_result,
        ) != 0
            || !user_result.is_null()
        {
            return true;
        }
        if libc::getgrgid_r(
            uid,
            &mut group,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut group_result,
        ) != 0
            || !group_result.is_null()
        {
            return true;
        }
    }
    Identity::resolve(Some(&format!("nyxc{uid}"))).is_ok()
}

fn account(uid: u32, home: &Path, node: &str) -> Result<Identity> {
    let name = format!("nyxc{uid}");
    // passwd GECOS fields cannot contain ':' (useradd rejects it).
    let comment = format!("nyxid-context-{node}");
    if Identity::resolve(Some(&name)).is_err() {
        let status = std::process::Command::new("/usr/sbin/useradd")
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .args([
                "--no-log-init",
                "--no-create-home",
                "--user-group",
                "--shell",
                "/usr/sbin/nologin",
                "--uid",
                &uid.to_string(),
                "--comment",
                &comment,
                "--home-dir",
            ])
            .arg(home)
            .arg(&name)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        ensure!(status.success(), "context_user_creation_failed");
    }
    let identity = Identity::resolve(Some(&name))?;
    ensure!(
        identity.uid == uid && identity.gid == uid && identity.home == home,
        "context_user_collision"
    );
    // Read-only account metadata; do not log names, paths or account records.
    let passwd = std::fs::read_to_string("/etc/passwd")?;
    ensure!(
        passwd.lines().any(|line| {
            let f: Vec<_> = line.split(':').collect();
            f.len() == 7 && f[0] == name && f[4] == comment
        }),
        "context_user_collision"
    );
    Ok(identity)
}
impl Store {
    pub fn open(directory: &Path, node: &str) -> Result<Self> {
        ensure!(
            unsafe { libc::geteuid() } == 0 && uuid::Uuid::parse_str(node).is_ok(),
            "context_supervisor_required"
        );
        protected(directory)?;
        let root = directory
            .parent()
            .context("context_state_parent_required")?
            .join("contexts");
        own(&protected(&root)?, 0, 0, 0o711)?;
        let path = directory.join("machine-contexts.json");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(directory.join("machine-contexts.lock"))?;
        ensure!(
            lock.metadata()?.uid() == 0 && lock.metadata()?.mode() & 0o077 == 0,
            "context_store_invalid"
        );
        ensure!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "context_supervisor_already_running"
        );
        let registry = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
        {
            Ok(file) => {
                let meta = file.metadata()?;
                ensure!(
                    meta.is_file()
                        && meta.uid() == 0
                        && meta.mode() & 0o077 == 0
                        && meta.len() <= STORE_LIMIT,
                    "context_store_invalid"
                );
                let mut bytes = Vec::new();
                file.take(STORE_LIMIT + 1).read_to_end(&mut bytes)?;
                ensure!(bytes.len() as u64 <= STORE_LIMIT, "context_store_invalid");
                serde_json::from_slice::<Registry>(&bytes)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Registry::default(),
            Err(e) => return Err(e.into()),
        };
        registry.validate().map_err(anyhow::Error::msg)?;
        Ok(Self {
            path,
            root,
            node: node.into(),
            _lock: lock,
            registry,
        })
    }
    fn persist(&self, registry: &Registry) -> Result<()> {
        registry.validate().map_err(anyhow::Error::msg)?;
        let parent = self
            .path
            .parent()
            .context("context_state_parent_required")?;
        let bytes = serde_json::to_vec(registry)?;
        ensure!(bytes.len() as u64 <= STORE_LIMIT, "context_store_capacity");
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(&self.path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    }
    pub fn quarantine(&mut self, agent: &str, before_revision: i64) -> Result<Vec<u32>> {
        let mut next = self.registry.clone();
        let users = next.quarantine(agent, before_revision);
        self.persist(&next)?;
        self.registry = next;
        Ok(users)
    }
    /// Caller authenticates v2 and obtains owner consent before this mutation.
    pub fn provision(&mut self, authority: &Authority) -> Result<Context> {
        let mut next = self.registry.clone();
        let allocation = next
            .allocate(authority, occupied)
            .map_err(anyhow::Error::msg)?;
        self.persist(&next)?;
        self.registry = next;
        // Chromium puts SingletonSocket below TMPDIR. Using the journaled UID
        // keeps those Unix socket paths short; caller IDs never select a path.
        let root = self.root.join(allocation.command_uid.to_string());
        let command_root = root.join("command");
        let browser_root = root.join(format!("b{}", allocation.generation));
        // Repair a crash after mkdir but before chmod, without touching existing
        // config/credential ancestors or any user-owned directory.
        own(&protected(&root)?, 0, 0, 0o711)?;
        own(&protected(&browser_root)?, 0, 0, 0o711)?;
        let secure_root = browser_root.join("s");
        let dev_root = browser_root.join("d");
        let users = &allocation.browsers[&allocation.generation];
        role(&command_root, allocation.command_uid)?;
        role(&secure_root, users.secure_uid)?;
        role(&dev_root, users.dev_uid)?;
        let mut command = account(
            allocation.command_uid,
            &command_root.join("home"),
            &self.node,
        )?;
        let secure = account(users.secure_uid, &secure_root.join("home"), &self.node)?;
        let dev = account(users.dev_uid, &dev_root.join("home"), &self.node)?;
        command.sandbox = Some(Arc::new(Sandbox::open(&command_root, command.uid)?));
        Ok(Context {
            allocation,
            command,
            secure,
            dev,
            workspace: command_root.join("workspace"),
            browser_root,
        })
    }
}

/// Owner opt-in only. Root ownership makes chmod by the legacy user ineffective;
/// group access preserves that user's existing reads/writes inside the workspace.
pub fn protect_legacy_roots(roots: &[PathBuf], legacy: &Identity) -> Result<()> {
    ensure!(
        legacy.uid != 0 && legacy.gid != 0 && !roots.is_empty(),
        "context_legacy_identity_required"
    );
    let mut pinned = Vec::new();
    for root in roots.iter().filter(|root| {
        // Native setup puts workspace below the legacy home. Protect the outer
        // gate; the legacy user may still manage descendants behind that gate.
        !roots
            .iter()
            .any(|other| other != *root && root.starts_with(other))
    }) {
        let parent = root.parent().context("context_legacy_root_unsafe")?;
        let parent = protected(parent)?;
        let name = std::ffi::CString::new(
            root.file_name()
                .context("context_legacy_root_unsafe")?
                .as_bytes(),
        )?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_DIRECTORY | libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        ensure!(fd >= 0, "context_legacy_root_unsafe");
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let meta = File::from(fd.try_clone()?).metadata()?;
        ensure!(
            meta.uid() == legacy.uid || (meta.uid() == 0 && meta.gid() == legacy.gid),
            "context_legacy_root_unsafe"
        );
        pinned.push(fd);
    }
    for fd in pinned {
        own(&fd, 0, legacy.gid, 0o770)?;
    }
    Ok(())
}

pub struct BrowserResources {
    _secure_display: super::dev_display::Server,
    _dev_display: super::dev_display::Server,
    _buses: Vec<tokio::process::Child>,
}
impl Context {
    /// Called only after a successful workspace probe; neither browser inherits
    /// the legacy user's display, bus, socket or profile. Policies remain root
    /// owned and readable only by the appropriate secure/developer policy group.
    pub async fn browsers(
        &mut self,
        secure_policy_gid: u32,
        dev_policy_gid: u32,
    ) -> Result<BrowserResources> {
        for (identity, group) in [
            (&mut self.secure, secure_policy_gid),
            (&mut self.dev, dev_policy_gid),
        ] {
            let role_root = identity.home.parent().context("context_role_missing")?;
            identity.sandbox = Some(Arc::new(Sandbox::browser(role_root, identity.uid)?));
            identity.policy_groups = vec![group];
        }
        let secure_display = super::dev_display::Server::launch(&self.secure).await?;
        let dev_display = super::dev_display::Server::launch(&self.dev).await?;
        let mut buses = Vec::new();
        for (identity, display) in [
            (&mut self.secure, &secure_display),
            (&mut self.dev, &dev_display),
        ] {
            let role_root = identity.home.parent().context("context_role_missing")?;
            let runtime = role_root.join("tmp");
            let bus = format!("unix:path={}", runtime.join("bus").display());
            identity.desktop = Some(super::process::DesktopEnvironment {
                display: display.name.clone(),
                authority: display.authority.clone(),
                bus: bus.clone(),
                runtime,
            });
            let mut command = tokio::process::Command::new("dbus-daemon");
            identity.prepare(&mut command)?;
            identity.desktop_env(&mut command);
            command
                .args(["--session", "--nofork", "--nopidfile"])
                .arg(format!("--address={bus}"))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let child = command.spawn().context("context_dbus_unavailable")?;
            buses.push(child);
            // Chromium decides whether to export its accessibility tree during
            // startup. Activate the private AT-SPI bus first, as the legacy
            // desktop session does; a later Cua connection is too late.
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                let mut activate = tokio::process::Command::new("dbus-send");
                identity.prepare(&mut activate)?;
                identity.desktop_env(&mut activate);
                activate
                    .args([
                        "--session",
                        "--print-reply",
                        "--reply-timeout=5000",
                        "--dest=org.a11y.Bus",
                        "/org/a11y/bus",
                        "org.a11y.Bus.GetAddress",
                    ])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                if tokio::time::timeout_at(deadline, activate.status())
                    .await
                    .context("context_accessibility_unavailable")??
                    .success()
                {
                    break;
                }
                ensure!(
                    tokio::time::Instant::now() < deadline,
                    "context_accessibility_unavailable"
                );
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            // A fresh user's AT-SPI status defaults to disabled. Cua normally
            // enables it, but that may happen after the first browser action.
            // Set it on this private bus before Chromium chooses its bridge.
            for property in ["IsEnabled", "ScreenReaderEnabled"] {
                let mut enable = tokio::process::Command::new("dbus-send");
                identity.prepare(&mut enable)?;
                identity.desktop_env(&mut enable);
                enable
                    .args([
                        "--session",
                        "--print-reply",
                        "--reply-timeout=5000",
                        "--dest=org.a11y.Bus",
                        "/org/a11y/bus",
                        "org.freedesktop.DBus.Properties.Set",
                        "string:org.a11y.Status",
                    ])
                    .arg(format!("string:{property}"))
                    .arg("variant:boolean:true")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                ensure!(
                    tokio::time::timeout_at(deadline, enable.status())
                        .await
                        .context("context_accessibility_unavailable")??
                        .success(),
                    "context_accessibility_unavailable"
                );
            }
        }
        Ok(BrowserResources {
            _secure_display: secure_display,
            _dev_display: dev_display,
            _buses: buses,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root_fixture() -> Option<tempfile::TempDir> {
        if unsafe { libc::geteuid() } != 0 {
            return None;
        }
        let root = tempfile::tempdir_in("/var/lib").unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o711)).unwrap();
        Some(root)
    }
    #[test]
    fn managed_ancestors_and_role_gates_replay_without_widening_private_children() {
        let Some(root) = root_fixture() else { return };
        let role_root = root.path().join("context/b1/s");
        for _ in 0..2 {
            role(&role_root, 65534).unwrap();
            let gate = std::fs::metadata(&role_root).unwrap();
            assert_eq!(
                (gate.uid(), gate.gid(), gate.mode() & 0o777),
                (0, 65534, 0o710)
            );
            for name in ["workspace", "home", "tmp"] {
                let child = std::fs::metadata(role_root.join(name)).unwrap();
                assert_eq!((child.uid(), child.mode() & 0o777), (65534, 0o700));
            }
            assert_eq!(
                std::fs::metadata(role_root.parent().unwrap())
                    .unwrap()
                    .mode()
                    & 0o777,
                0o711
            );
        }
        let link = root.path().join("substitute");
        std::os::unix::fs::symlink(&role_root, &link).unwrap();
        assert!(role(&link, 65534).is_err());
    }
    #[test]
    fn legacy_native_home_is_the_outer_gate_for_nested_workspace() {
        let Some(root) = root_fixture() else { return };
        let home = root.path().join("legacy");
        let workspace = home.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        super::super::browser::chown(&home, 65534, 65534).unwrap();
        super::super::browser::chown(&workspace, 65534, 65534).unwrap();
        let mut identity = Identity::resolve(None).unwrap();
        identity.uid = 65534;
        identity.gid = 65534;
        identity.home = home.clone();
        for _ in 0..2 {
            protect_legacy_roots(&[workspace.clone(), home.clone()], &identity).unwrap();
        }
        let gate = std::fs::metadata(&home).unwrap();
        assert_eq!(
            (gate.uid(), gate.gid(), gate.mode() & 0o777),
            (0, 65534, 0o770)
        );
        assert_eq!(std::fs::metadata(workspace).unwrap().uid(), 65534);
    }
    #[test]
    fn allocation_store_rejects_parallel_supervisors_symlinks_and_corruption() {
        let Some(root) = root_fixture() else { return };
        let node = uuid::Uuid::new_v4().to_string();
        let path = root.path().join("node");
        let store = Store::open(&path, &node).unwrap();
        assert!(Store::open(&path, &node).is_err());
        store.persist(&Registry::default()).unwrap();
        drop(store);
        let store = Store::open(&path, &node).unwrap();
        drop(store);
        std::fs::write(path.join("machine-contexts.json"), b"malformed").unwrap();
        assert!(Store::open(&path, &node).is_err());
        std::fs::remove_file(path.join("machine-contexts.json")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", path.join("machine-contexts.json")).unwrap();
        assert!(Store::open(&path, &node).is_err());
    }
}
