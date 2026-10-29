//! Supervisor-owned separated context provisioning. No caller paths or UIDs.
//! The allocation journal is fsynced before creating users; old UID/generation
//! mappings are retained forever. This module is not a UID-only fallback.
pub use super::hosted_context::VolumeBinding;
use super::hosted_context::{AccountPool, HostedStore, RETAINED_ROOT};
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
    _lock: Option<File>,
    registry: Registry,
    hosted: Option<HostedStore>,
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
    // Minimal hosted supervisors have CHOWN, never FOWNER. Take ownership
    // before chmod and relinquish it last; changing an already worker-owned
    // directory's mode directly would fail under the formal Pod capabilities.
    ensure!(
        unsafe { libc::fchown(fd.as_raw_fd(), 0, 0) } == 0,
        "context_ownership_failed"
    );
    ensure!(
        unsafe { libc::fchmod(fd.as_raw_fd(), mode) } == 0,
        "context_permissions_failed"
    );
    ensure!(
        unsafe { libc::fchown(fd.as_raw_fd(), uid, gid) } == 0,
        "context_ownership_failed"
    );
    Ok(())
}

/// The hosted supervisor has CHOWN, not DAC_OVERRIDE/DAC_READ_SEARCH or
/// FOWNER. After validating an O_PATH-pinned private directory, take ownership
/// through that descriptor before requesting read access. Reopen the pinned
/// inode itself, never its pathname: the no-follow and protected-parent checks
/// made by the caller must continue to cover the ownership/mode transition.
fn readable_as_supervisor(pinned: &OwnedFd) -> Result<OwnedFd> {
    ensure!(
        unsafe { libc::fchownat(pinned.as_raw_fd(), c"".as_ptr(), 0, 0, libc::AT_EMPTY_PATH) } == 0,
        "context_ownership_failed"
    );
    let fd = unsafe {
        libc::openat(
            pinned.as_raw_fd(),
            c".".as_ptr(),
            libc::O_DIRECTORY | libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    ensure!(fd >= 0, "context_directory_unavailable");
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

#[cfg(test)]
mod legacy_pinning_tests {
    use super::*;

    #[test]
    #[ignore = "requires disposable hardened Linux supervisor"]
    fn private_home_transition_uses_the_pinned_inode_after_name_replacement() {
        let root = tempfile::tempdir_in("/var/lib/nyxid-hosted-tests").unwrap();
        let home = root.path().join("home");
        let retired = root.path().join("retired");
        std::fs::create_dir(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::chown(&home, Some(1000), Some(1000)).unwrap();
        let pinned = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&home)
            .unwrap();
        let original = pinned.metadata().unwrap();
        assert_eq!(original.uid(), 1000);
        assert!(File::open(&home).is_err(), "no DAC override may be used");
        // A trusted test actor changes the pathname after pinning. Production's
        // protected parent denies this to workers; even this swap must never
        // redirect the subsequent ownership/read-open to the replacement.
        std::fs::rename(&home, &retired).unwrap();
        std::fs::create_dir(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::chown(&home, Some(1002), Some(1002)).unwrap();
        let fd: OwnedFd = pinned.into();
        let readable = readable_as_supervisor(&fd).unwrap();
        own(&readable, 0, 1000, 0o770).unwrap();
        let transitioned = File::from(readable).metadata().unwrap();
        assert_eq!(
            (transitioned.dev(), transitioned.ino()),
            (original.dev(), original.ino())
        );
        assert_eq!(
            (
                transitioned.uid(),
                transitioned.gid(),
                transitioned.mode() & 0o777
            ),
            (0, 1000, 0o770)
        );
        let replacement = std::fs::metadata(&home).unwrap();
        assert_eq!(
            (
                replacement.uid(),
                replacement.gid(),
                replacement.mode() & 0o777
            ),
            (1002, 1002, 0o700)
        );
    }
}

/// Registration keeps its state private, including newly created ancestors.
/// Only the standard native profile parent is shared with context/display
/// roles. Give it traversal after pinning the node directory behind a private
/// gate; never widen an owner-selected config directory or an arbitrary home.
pub(super) fn prepare_native_profile(directory: &Path) -> Result<()> {
    let profile = directory
        .parent()
        .context("context_state_parent_required")?;
    ensure!(
        directory.file_name() == Some(std::ffi::OsStr::new("node"))
            && profile.parent() == Some(Path::new("/var/lib/nyxid-machine")),
        "context_native_profile_path_invalid"
    );
    own(&protected(directory)?, 0, 0, 0o700)?;
    own(&protected(profile)?, 0, 0, 0o711)
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
            libc::O_DIRECTORY | libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW,
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
    // On restart an existing HOME/tmp belongs to the command UID with 0700.
    // The hosted parent has CHOWN, not DAC_OVERRIDE/FOWNER: pin it without
    // reading, verify ownership, then take ownership through that descriptor.
    // Its supervisor-owned parent prevents the role from replacing this inode.
    ensure!(
        unsafe { libc::fchownat(fd.as_raw_fd(), c"".as_ptr(), 0, 0, libc::AT_EMPTY_PATH) } == 0,
        "context_ownership_failed"
    );
    let readable = unsafe {
        libc::openat(
            fd.as_raw_fd(),
            c".".as_ptr(),
            libc::O_DIRECTORY | libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    ensure!(readable >= 0, "context_directory_unavailable");
    let fd = unsafe { OwnedFd::from_raw_fd(readable) };
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
                // Context roles map only their own UID in Chromium's sandbox;
                // they never need subordinate identities. Lima and other hosts
                // may reserve the default subordinate range for an owner.
                "--key",
                "SUB_UID_COUNT=0",
                "--key",
                "SUB_GID_COUNT=0",
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
            std::env::var("NYXID_MANAGED_KUBERNETES").as_deref() != Ok("1")
                && std::env::var("NYXID_MACHINE_MANAGED").as_deref() != Ok("1"),
            "hosted_context_binding_required"
        );
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
            _lock: Some(lock),
            registry,
            hosted: None,
        })
    }
    /// Only the managed bootstrap integration may call this with authenticated
    /// original PVC/current runtime identities. Native storage is never a fallback.
    pub fn open_hosted(
        directory: &Path,
        binding: VolumeBinding,
        runtime_id: &str,
        initialize: bool,
    ) -> Result<Self> {
        protected(directory)?;
        let root = directory
            .parent()
            .context("context_state_parent_required")?
            .join("contexts");
        own(&protected(&root)?, 0, 0, 0o711)?;
        let hosted = HostedStore::open(
            Path::new(RETAINED_ROOT),
            binding.clone(),
            runtime_id,
            initialize,
            AccountPool::system()?,
        )?;
        Ok(Self {
            path: directory.join("machine-contexts.json"),
            root,
            node: binding.node_id,
            _lock: None,
            registry: Registry::default(),
            hosted: Some(hosted),
        })
    }
    pub(super) fn is_hosted(&self) -> bool {
        self.hosted.is_some()
    }
    pub(super) fn hosted_volume_matches(&self, binding: &VolumeBinding) -> bool {
        self.hosted
            .as_ref()
            .is_some_and(|hosted| hosted.volume_binding() == binding)
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
        if let Some(hosted) = &mut self.hosted {
            return hosted.quarantine(agent, before_revision);
        }
        let mut next = self.registry.clone();
        let users = next.quarantine(agent, before_revision);
        self.persist(&next)?;
        self.registry = next;
        Ok(users)
    }
    /// Caller authenticates v2 and obtains owner consent before this mutation.
    pub fn provision(&mut self, authority: &Authority) -> Result<Context> {
        let (allocation, retained) = if let Some(hosted) = &mut self.hosted {
            let (allocation, workspace) = hosted.provision(authority)?;
            (allocation, Some(workspace))
        } else {
            let mut next = self.registry.clone();
            let allocation = next
                .allocate(authority, occupied)
                .map_err(anyhow::Error::msg)?;
            self.persist(&next)?;
            self.registry = next;
            (allocation, None)
        };
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
        let account = |uid, home: &Path| -> Result<Identity> {
            if self.hosted.is_some() {
                let mut identity = Identity::resolve(Some(&format!("nyxc{uid}")))?;
                ensure!(
                    identity.uid == uid
                        && identity.gid == uid
                        && identity.home == Path::new("/nonexistent"),
                    "context_account_collision"
                );
                identity.home = home.to_owned();
                Ok(identity)
            } else {
                account(uid, home, &self.node)
            }
        };
        let mut command = account(allocation.command_uid, &command_root.join("home"))?;
        let secure = account(users.secure_uid, &secure_root.join("home"))?;
        let dev = account(users.dev_uid, &dev_root.join("home"))?;
        let workspace = retained.unwrap_or_else(|| command_root.join("workspace"));
        command.sandbox = Some(Arc::new(Sandbox::open_with_workspace(
            &command_root,
            command.uid,
            &workspace,
        )?));
        Ok(Context {
            allocation,
            command,
            secure,
            dev,
            workspace,
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
                libc::O_DIRECTORY | libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
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
        own(&readable_as_supervisor(&fd)?, 0, legacy.gid, 0o770)?;
    }
    Ok(())
}

pub fn protect_legacy_browser_home(legacy: &Identity) -> Result<()> {
    ensure!(
        legacy.uid != 0 && legacy.gid != 0,
        "context_legacy_identity_required"
    );
    let parent = protected(legacy.home.parent().context("context_legacy_home_unsafe")?)?;
    let name = std::ffi::CString::new(
        legacy
            .home
            .file_name()
            .context("context_legacy_home_unsafe")?
            .as_bytes(),
    )?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_DIRECTORY | libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    ensure!(fd >= 0, "context_legacy_home_unsafe");
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let meta = File::from(fd.try_clone()?).metadata()?;
    ensure!(
        meta.uid() == legacy.uid || (meta.uid() == 0 && meta.gid() == 0),
        "context_legacy_home_unsafe"
    );
    let fd = readable_as_supervisor(&fd)?;
    own(&fd, 0, 0, 0o700)?;
    super::context_policy::legacy_home(&File::from(fd), legacy.uid)
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
    fn legacy_browser_home_denies_policy_group_but_preserves_legacy_uid_access() {
        let Some(root) = root_fixture() else { return };
        use std::os::unix::process::CommandExt;
        let home = root.path().join("legacy-browser");
        std::fs::create_dir(&home).unwrap();
        super::super::browser::chown(&home, 65534, 65534).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o750)).unwrap();
        let mut identity = Identity::resolve(None).unwrap();
        identity.uid = 65534;
        identity.gid = 65534;
        identity.home = home.clone();
        for _ in 0..2 {
            protect_legacy_browser_home(&identity).unwrap();
            for (uid, script) in [
                (
                    65534,
                    "test -r \"$1\" && touch \"$1/ours\" && ! chmod 0755 \"$1\" 2>/dev/null",
                ),
                (
                    65533,
                    "test ! -r \"$1\" && test ! -w \"$1\" && test ! -x \"$1\"",
                ),
            ] {
                let mut command = std::process::Command::new("/bin/sh");
                command.args(["-c", script, "fixture"]).arg(&home);
                unsafe {
                    command.pre_exec(move || {
                        if libc::setgroups(1, [65534].as_ptr()) != 0
                            || libc::setgid(uid) != 0
                            || libc::setuid(uid) != 0
                        {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
                assert!(
                    command.status().unwrap().success(),
                    "fixture identity command failed"
                );
            }
        }
        let link = root.path().join("substitute");
        std::os::unix::fs::symlink(&home, &link).unwrap();
        identity.home = link;
        assert!(protect_legacy_browser_home(&identity).is_err());
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

    #[test]
    fn native_profile_parent_allows_role_traversal_but_keeps_node_state_private() {
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        use std::os::unix::process::CommandExt;
        let managed = Path::new("/var/lib/nyxid-machine");
        protected(managed).unwrap();
        let profile = tempfile::tempdir_in(managed).unwrap();
        let node = profile.path().join("node");
        std::fs::create_dir(&node).unwrap();
        std::fs::write(node.join("secret"), b"private fixture").unwrap();
        let role = profile.path().join("role");
        std::fs::create_dir(&role).unwrap();
        std::fs::set_permissions(&role, std::fs::Permissions::from_mode(0o755)).unwrap();
        for _ in 0..2 {
            prepare_native_profile(&node).unwrap();
            let mut command = std::process::Command::new("/bin/sh");
            command
                .args([
                    "-c",
                    "test -x \"$1\" && test ! -r \"$2/secret\" && test ! -x \"$2\"",
                    "fixture",
                ])
                .arg(&role)
                .arg(&node);
            unsafe {
                command.pre_exec(|| {
                    if libc::setgroups(0, std::ptr::null()) != 0
                        || libc::setgid(65534) != 0
                        || libc::setuid(65534) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            assert!(command.status().unwrap().success());
            assert_eq!(
                std::fs::metadata(profile.path()).unwrap().mode() & 0o777,
                0o711
            );
            assert_eq!(std::fs::metadata(&node).unwrap().mode() & 0o777, 0o700);
        }
        assert!(prepare_native_profile(&profile.path().join("custom")).is_err());
        assert!(prepare_native_profile(Path::new("/root/node")).is_err());
    }

    #[test]
    fn validation_paths_are_safe_without_a_hardened_host_fixture() {
        // These checks deliberately exercise the rejection gates that run
        // before any supervisor-owned mutation. They keep coverage useful on
        // ordinary developer machines where the hosted image is unavailable.
        assert!(protected(Path::new("relative"),).is_err());
        assert!(protected(Path::new("/tmp"),).is_err());
        assert!(prepare_native_profile(Path::new("relative")).is_err());
        assert!(prepare_native_profile(Path::new("/tmp/node")).is_err());

        let root = tempfile::tempdir().unwrap();
        assert!(role(root.path(), unsafe { libc::geteuid() }).is_err());

        let descriptor = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_PATH | libc::O_CLOEXEC)
            .open(root.path())
            .unwrap();
        let descriptor: OwnedFd = descriptor.into();
        let _ = own(
            &descriptor,
            unsafe { libc::geteuid() },
            unsafe { libc::getegid() },
            0o700,
        );
        let _ = readable_as_supervisor(&descriptor);

        assert!(occupied(0));
        assert!(!occupied(u32::MAX));
        assert!(account(u32::MAX, &root.path().join("home"), "coverage").is_err());

        let child = child(
            &descriptor,
            "child",
            unsafe { libc::geteuid() },
            unsafe { libc::getegid() },
            0o700,
        );
        if child.is_ok() {
            assert!(root.path().join("child").is_dir());
        }

        let mut identity = Identity::resolve(None).unwrap();
        identity.uid = 0;
        identity.gid = 0;
        assert!(protect_legacy_roots(&[], &identity).is_err());
        assert!(protect_legacy_browser_home(&identity).is_err());
        identity.uid = unsafe { libc::geteuid() }.saturating_add(1);
        identity.gid = unsafe { libc::getegid() }.saturating_add(1);
        identity.home = PathBuf::from("relative-home");
        assert!(protect_legacy_roots(&[PathBuf::from("relative-root")], &identity).is_err());
        assert!(protect_legacy_browser_home(&identity).is_err());

        assert!(Store::open(Path::new("/tmp/nyxid-invalid-store"), "invalid-node").is_err());
    }
}
