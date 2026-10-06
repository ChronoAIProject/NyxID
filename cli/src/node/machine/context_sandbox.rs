//! Linux defence in depth for separated workspaces. ABI 6 is mandatory:
//! filesystem restrictions plus signal and abstract-socket scopes. DAC on
//! supervisor-owned ancestors supplies the metadata/pathname-socket boundary
//! that Landlock does not implement. No fallback to UID-only execution.
use anyhow::{Context, Result, ensure};
use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use tokio::process::Command;

const MIN_ABI: i64 = 6;
const READ: u64 = (1 << 0) | (1 << 2) | (1 << 3);
const ALL_FS: u64 = (1 << 16) - 1; // ABI 5: includes TRUNCATE and IOCTL_DEV.
const FILE_RIGHTS: u64 = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 14) | (1 << 15);
const CONTEXT_RIGHTS: u64 = ALL_FS & !((1 << 6) | (1 << 11)); // no device creation
const SCOPED: u64 = 3; // ABI 6: abstract Unix sockets and signals.

#[repr(C)]
struct RulesetAttr {
    fs: u64,
    net: u64,
    scoped: u64,
}

#[repr(C, packed)]
struct PathAttr {
    access: u64,
    parent_fd: i32,
}

fn check(value: libc::c_long) -> std::io::Result<libc::c_long> {
    if value < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(value)
    }
}

pub fn abi() -> std::io::Result<i64> {
    check(unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, 0, 0, 1) })
}

fn open_path(path: &Path) -> Result<OwnedFd> {
    let name = CString::new(path.as_os_str().as_bytes())?;
    let raw = check(unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
        .into()
    })?;
    Ok(unsafe { OwnedFd::from_raw_fd(raw as i32) })
}

fn stat(fd: &OwnedFd) -> Result<libc::stat> {
    let mut info = unsafe { std::mem::zeroed() };
    check(unsafe { libc::fstat(fd.as_raw_fd(), &mut info).into() })?;
    Ok(info)
}

/// Pin every ancestor. Refuse symlinks, writable supervisor parents and wrong owners.
pub fn context_directories(root: &Path, uid: u32) -> Result<Vec<OwnedFd>> {
    ensure!(root.is_absolute(), "context_root_not_absolute");
    let mut parent = open_path(Path::new("/"))?;
    for component in root.components() {
        let Component::Normal(part) = component else {
            ensure!(
                component == Component::RootDir,
                "context_root_not_canonical"
            );
            continue;
        };
        let name = CString::new(part.as_bytes())?;
        let fd = check(unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
            .into()
        })?;
        parent = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        let info = stat(&parent)?;
        ensure!(
            info.st_uid == 0 && info.st_mode & 0o022 == 0,
            "context_parent_not_private"
        );
    }
    let mut result = Vec::new();
    for name in [c"workspace", c"home", c"tmp"] {
        let fd = check(unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
            .into()
        })?;
        let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        let info = stat(&fd)?;
        ensure!(
            info.st_uid == uid && info.st_mode & 0o777 == 0o700,
            "context_directory_not_private"
        );
        result.push(fd);
    }
    Ok(result)
}

fn add_rule(ruleset: &OwnedFd, fd: &OwnedFd, rights: u64) -> Result<()> {
    let info = stat(fd)?;
    ensure!(
        info.st_mode & libc::S_IFMT != libc::S_IFLNK,
        "runtime_symlink_not_resolved"
    );
    let attr = PathAttr {
        access: if info.st_mode & libc::S_IFMT == libc::S_IFDIR {
            rights
        } else {
            rights & FILE_RIGHTS
        },
        parent_fd: fd.as_raw_fd(),
    };
    check(unsafe {
        libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset.as_raw_fd(),
            1,
            &attr,
            0,
        )
    })?;
    Ok(())
}

pub fn ruleset(directories: &[OwnedFd]) -> Result<OwnedFd> {
    ensure!(abi()? >= MIN_ABI, "landlock_abi_6_required");
    let attr = RulesetAttr {
        fs: ALL_FS,
        net: 0,
        scoped: SCOPED,
    };
    let raw = check(unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &attr,
            size_of::<RulesetAttr>(),
            0,
        )
    })?;
    let fd = unsafe { OwnedFd::from_raw_fd(raw as i32) };
    for directory in directories {
        add_rule(&fd, directory, CONTEXT_RIGHTS)?;
    }
    // No /home, /var, /tmp, /workspace, broad /etc, /proc or /sys grants.
    for path in [
        "/usr/bin",
        "/usr/lib",
        "/usr/libexec",
        "/usr/include",
        "/usr/local/lib/python3.13",
        "/usr/share/git-core",
        "/usr/share/nodejs",
        "/usr/share/node_modules",
        "/usr/share/npm",
        "/usr/share/zoneinfo",
        "/usr/share/misc",
        "/usr/share/locale",
        "/usr/share/perl",
        "/usr/share/perl5",
        "/usr/share/python-wheels",
        "/usr/share/python3",
        "/usr/share/ca-certificates",
        "/etc/ssl/certs",
        "/etc/ssl/openssl.cnf",
        "/etc/ld.so.cache",
        "/etc/nsswitch.conf",
        "/etc/passwd",
        "/etc/group",
        "/etc/hosts",
        "/etc/resolv.conf",
        "/etc/gai.conf",
        "/etc/debian_version",
        "/etc/os-release",
        "/etc/lsb-release",
        "/etc/mime.types",
        "/etc/localtime",
        "/proc/cpuinfo",
        "/proc/meminfo",
        "/sys/devices/system/cpu/online",
        "/sys/devices/system/cpu/possible",
    ] {
        if Path::new(path).exists() {
            let resolved = std::fs::canonicalize(path)?;
            add_rule(&fd, &open_path(&resolved)?, READ)
                .with_context(|| format!("runtime_rule: {path}"))?;
        }
    }
    let executable = std::env::current_exe()?;
    add_rule(&fd, &open_path(&executable)?, READ)?;
    for path in ["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom"] {
        add_rule(&fd, &open_path(Path::new(path))?, (1 << 1) | (1 << 2))?;
    }
    // POSIX shm/semaphores (Python process pools) are a deliberately shared
    // namespace. Distinct UIDs and mode 0600 protect contents; names/capacity
    // are shared. Never describe this as full machine isolation.
    add_rule(&fd, &open_path(Path::new("/dev/shm"))?, CONTEXT_RIGHTS)?;
    Ok(fd)
}

/// Open and pin the allowlist before fork. No path resolution or allocation in
/// pre_exec, and no supervisor descriptors survive exec (including error paths).
/// Calling code still applies Identity::prepare_agent first: UID/GID, NNP, seccomp.
pub struct Sandbox {
    root: PathBuf,
    directories: Vec<OwnedFd>,
    ruleset: OwnedFd,
}
impl Sandbox {
    pub fn open(root: &Path, uid: u32) -> Result<Self> {
        let directories = context_directories(root, uid)?;
        let ruleset = ruleset(&directories)?;
        Ok(Self {
            root: root.to_owned(),
            directories,
            ruleset,
        })
    }
    pub fn browser(root: &Path, uid: u32) -> Result<Self> {
        let directories = context_directories(root, uid)?;
        ensure!(abi()? >= MIN_ABI, "landlock_abi_6_required");
        // Chromium must write its child namespace uid_map/gid_map/setgroups to
        // establish its own sandbox. A read-only /proc filesystem rule prevents
        // that; broad /proc writes would misrepresent the narrow file allowlist.
        // Browser boundaries are dedicated UIDs + protected DAC ancestors and
        // Chromium's sandbox. Landlock adds signal/abstract-socket scopes here;
        // command/file workers additionally receive the filesystem ruleset.
        let attr = RulesetAttr {
            fs: 0,
            net: 0,
            scoped: SCOPED,
        };
        let raw = check(unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &attr,
                size_of::<RulesetAttr>(),
                0,
            )
        })?;
        Ok(Self {
            root: root.to_owned(),
            directories,
            ruleset: unsafe { OwnedFd::from_raw_fd(raw as i32) },
        })
    }
    pub fn prepare(self: &std::sync::Arc<Self>, command: &mut Command) {
        command
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("HOME", self.root.join("home"))
            .env("TMPDIR", self.root.join("tmp"))
            .env("XDG_CACHE_HOME", self.root.join("home/.cache"))
            .env("XDG_CONFIG_HOME", self.root.join("home/.config"))
            .env("CARGO_HOME", self.root.join("home/.cargo"));
        let sandbox = self.clone();
        unsafe {
            command.pre_exec(move || {
                check(libc::fchdir(sandbox.directories[0].as_raw_fd()).into())?;
                libc::umask(0o077);
                check(libc::syscall(
                    libc::SYS_landlock_restrict_self,
                    sandbox.ruleset.as_raw_fd(),
                    0,
                ))?;
                // CLOEXEC keeps Rust's spawn-error pipe usable until exec.
                check(libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 4u32))?;
                Ok(())
            });
        }
    }
    /// Enforce the policy in an actual child, not just a kernel version check.
    pub async fn probe(self: &std::sync::Arc<Self>, identity: &super::Identity) -> Result<()> {
        let mut command = Command::new("/bin/true");
        identity.prepare_agent(&mut command)?;
        self.prepare(&mut command);
        ensure!(
            command.status().await?.success(),
            "context_sandbox_probe_failed"
        );
        Ok(())
    }
}

/// Live ABI/enforcement probe without provisioning a context or changing legacy
/// roots. A syscall/version advertisement alone is not capability evidence.
pub async fn probe(identity: &super::Identity) -> Result<()> {
    let ruleset = ruleset(&[])?;
    let mut command = Command::new("/bin/true");
    identity.prepare_agent(&mut command)?;
    unsafe {
        command.pre_exec(move || {
            check(libc::syscall(
                libc::SYS_landlock_restrict_self,
                ruleset.as_raw_fd(),
                0,
            ))?;
            check(libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 4u32))?;
            Ok(())
        });
    }
    ensure!(
        command.status().await?.success(),
        "context_landlock_probe_failed"
    );
    Ok(())
}
