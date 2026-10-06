use std::{collections::BTreeMap, ffi::CString, path::PathBuf};

use anyhow::{Result, bail};
use tokio::process::Command;

#[cfg(target_os = "linux")]
#[path = "context_sandbox.rs"]
pub mod context_sandbox;

#[derive(Clone)]
pub struct Identity {
    pub uid: u32,
    pub gid: u32,
    pub name: String,
    pub home: PathBuf,
    /// Supervisor-selected policy groups only; command contexts leave this empty.
    pub policy_groups: Vec<u32>,
    pub desktop: Option<DesktopEnvironment>,
    #[cfg(target_os = "linux")]
    pub sandbox: Option<std::sync::Arc<context_sandbox::Sandbox>>,
}

#[derive(Clone)]
pub struct DesktopEnvironment {
    pub display: String,
    pub authority: PathBuf,
    pub bus: String,
    pub runtime: PathBuf,
}

impl Identity {
    /// Probe using the same UID/GID and supplementary-group policy as commands.
    /// A configured username alone says nothing about filesystem access.
    pub async fn commands_isolated(&self, directories: &[PathBuf]) -> bool {
        if self.uid == unsafe { libc::geteuid() } || self.uid == 0 || directories.is_empty() {
            return false;
        }
        let mut paths = Vec::new();
        for (index, directory) in directories.iter().enumerate() {
            if !directory.is_dir() {
                if index == 0 {
                    return false;
                }
                continue;
            }
            use std::os::unix::ffi::OsStrExt;
            let Ok(path) = CString::new(directory.as_os_str().as_bytes()) else {
                return false;
            };
            paths.push(path);
        }
        let mut probe = Command::new("/bin/true");
        if self.prepare_agent(&mut probe).is_err() {
            return false;
        }
        unsafe {
            probe.pre_exec(move || {
                for path in &paths {
                    for mode in [libc::R_OK, libc::X_OK] {
                        if libc::faccessat(libc::AT_FDCWD, path.as_ptr(), mode, libc::AT_EACCESS)
                            == 0
                        {
                            return Err(std::io::Error::from_raw_os_error(libc::EACCES));
                        }
                        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EACCES) {
                            return Err(std::io::Error::last_os_error());
                        }
                    }
                }
                Ok(())
            });
        }
        matches!(probe.status().await, Ok(status) if status.success())
    }

    /// Agent children inherit a namespace-denying filter. Browser/cua children
    /// use `prepare` instead, so Chromium can establish its own sandbox.
    pub fn prepare_agent(&self, command: &mut Command) -> Result<()> {
        self.prepare(command)?;
        #[cfg(target_os = "linux")]
        unsafe {
            // Runs after prepare's NO_NEW_PRIVS hook; only stack data/syscalls.
            command.pre_exec(deny_agent_namespaces);
        }
        Ok(())
    }

    pub fn resolve(name: Option<&str>) -> Result<Self> {
        let name = name.map(CString::new).transpose()?;
        let mut record: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result = std::ptr::null_mut();
        let mut buffer = vec![0u8; 65536];
        // SAFETY: buffer and record outlive the reentrant lookup and string copies.
        let status = unsafe {
            match &name {
                Some(name) => libc::getpwnam_r(
                    name.as_ptr(),
                    &mut record,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut result,
                ),
                None => libc::getpwuid_r(
                    libc::geteuid(),
                    &mut record,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut result,
                ),
            }
        };
        if status != 0 || result.is_null() {
            bail!("machine OS user not found");
        }
        let string = |pointer| unsafe {
            std::ffi::CStr::from_ptr(pointer)
                .to_string_lossy()
                .into_owned()
        };
        Ok(Self {
            uid: record.pw_uid,
            gid: record.pw_gid,
            name: string(record.pw_name),
            home: PathBuf::from(string(record.pw_dir)),
            policy_groups: Vec::new(),
            desktop: None,
            #[cfg(target_os = "linux")]
            sandbox: None,
        })
    }

    /// Desktop-only environment. Commands/file workers never call this method.
    pub fn desktop_env(&self, command: &mut Command) {
        if let Some(desktop) = &self.desktop {
            command
                .env("DISPLAY", &desktop.display)
                .env("XAUTHORITY", &desktop.authority)
                .env("DBUS_SESSION_BUS_ADDRESS", &desktop.bus)
                .env("XDG_RUNTIME_DIR", &desktop.runtime);
            return;
        }
        for key in [
            "DISPLAY",
            "XAUTHORITY",
            "WAYLAND_DISPLAY",
            "XDG_RUNTIME_DIR",
            "DBUS_SESSION_BUS_ADDRESS",
            "AT_SPI_BUS_ADDRESS",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        #[cfg(target_os = "linux")]
        if let Ok(address) =
            std::fs::read_to_string(self.home.join(".nyxid-desktop/session-bus.address"))
        {
            let address = address.trim();
            if address.starts_with("unix:") && !address.contains(['\n', '\r', '\0']) {
                command.env("DBUS_SESSION_BUS_ADDRESS", address);
            }
        }
    }

    pub fn prepare(&self, command: &mut Command) -> Result<()> {
        let uid = self.uid;
        let gid = self.gid;
        let supervisor = unsafe { libc::geteuid() };
        if supervisor != 0 && supervisor != uid {
            bail!("OS user separation requires the installed supervisor service");
        }
        command.env_clear();
        for (key, value) in std::env::vars_os() {
            let text = key.to_string_lossy();
            if matches!(text.as_ref(), "PATH" | "LANG" | "TMPDIR") || text.starts_with("LC_") {
                command.env(key, value);
            }
        }
        command
            .env(
                "PATH",
                std::env::var_os("PATH").unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into()),
            )
            .env("HOME", &self.home)
            .env("USER", &self.name)
            .env("LOGNAME", &self.name)
            .env("SHELL", "/bin/sh")
            .env("TERM", "dumb");
        if supervisor == 0 && uid != 0 {
            let groups = self.policy_groups.clone();
            // SAFETY: only async-signal-safe syscalls in the post-fork child.
            unsafe {
                command.pre_exec(move || {
                    if libc::setgroups(groups.len() as _, groups.as_ptr()) != 0
                        || libc::setgid(gid) != 0
                        || libc::setuid(uid) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        // Inherited by every descendant: setuid executables and file capabilities
        // cannot restore privileges after the supervisor drops to the worker user.
        #[cfg(target_os = "linux")]
        unsafe {
            command.pre_exec(|| {
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        #[cfg(target_os = "linux")]
        if let Some(sandbox) = &self.sandbox {
            sandbox.prepare(command);
        }
        command.process_group(0).kill_on_drop(true);
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn deny_agent_namespaces() -> std::io::Result<()> {
    // seccomp_data: nr at 0, audit arch at 4, args[0] at 16. Supported Linux
    // release architectures are little-endian x86_64 and aarch64. Checking arch
    // also prevents a child switching syscall ABIs to bypass this filter.
    #[cfg(target_arch = "x86_64")]
    const ARCH: u32 = 0xc000003e;
    #[cfg(target_arch = "aarch64")]
    const ARCH: u32 = 0xc00000b7;
    const NEW_NAMESPACES: u32 = 0x7e020080; // All CLONE_NEW*, including NEWTIME.
    const fn ins(code: u16, jt: u8, jf: u8, k: u32) -> libc::sock_filter {
        libc::sock_filter { code, jt, jf, k }
    }
    const LOAD: u16 = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
    const EQ: u16 = (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16;
    const RET: u16 = (libc::BPF_RET | libc::BPF_K) as u16;
    const DENY: u32 = libc::SECCOMP_RET_ERRNO | libc::EPERM as u32;
    let mut filter = [
        ins(LOAD, 0, 0, 4),
        ins(EQ, 1, 0, ARCH),
        ins(RET, 0, 0, DENY),
        ins(LOAD, 0, 0, 0),
        // Deny x32 syscall numbers as well as unsupported high-number ABIs.
        ins(
            (libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K) as u16,
            0,
            1,
            0x40000000,
        ),
        ins(RET, 0, 0, DENY),
        ins(EQ, 0, 1, libc::SYS_mount as u32),
        ins(RET, 0, 0, DENY),
        ins(EQ, 0, 1, libc::SYS_umount2 as u32),
        ins(RET, 0, 0, DENY),
        ins(EQ, 0, 1, libc::SYS_pivot_root as u32),
        ins(RET, 0, 0, DENY),
        ins(EQ, 0, 1, libc::SYS_unshare as u32),
        ins(RET, 0, 0, DENY),
        ins(EQ, 0, 1, libc::SYS_setns as u32),
        ins(RET, 0, 0, DENY),
        ins(EQ, 0, 1, libc::SYS_clone3 as u32),
        ins(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32),
        ins(EQ, 0, 3, libc::SYS_clone as u32),
        ins(LOAD, 0, 0, 16),
        ins(
            (libc::BPF_JMP | libc::BPF_JSET | libc::BPF_K) as u16,
            0,
            1,
            NEW_NAMESPACES,
        ),
        ins(RET, 0, 0, DENY),
        ins(RET, 0, 0, libc::SECCOMP_RET_ALLOW),
    ];
    let program = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_mut_ptr(),
    };
    // SAFETY: kernel copies the stack-owned BPF program during this syscall.
    if unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::SECCOMP_MODE_FILTER,
            &program,
            0,
            0,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

pub fn request_env(command: &mut Command, values: &BTreeMap<String, String>) -> Result<()> {
    if values.len() > 64 || values.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>() > 16384 {
        bail!("environment limit exceeded");
    }
    for (key, value) in values {
        if key.is_empty()
            || !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            || key.starts_with("NYXID_")
            || key.starts_with("GIT_CONFIG_")
            || value.contains('\0')
        {
            bail!("environment key is reserved or invalid");
        }
        command.env(key, value);
    }
    Ok(())
}

pub fn pin_cwd(command: &mut Command, directories: Vec<std::fs::File>) {
    use std::os::fd::AsRawFd;
    // Pin the directory before fork. A later path swap cannot redirect the child.
    unsafe {
        command.pre_exec(move || {
            for directory in &directories {
                if libc::faccessat(
                    directory.as_raw_fd(),
                    c".".as_ptr(),
                    libc::X_OK,
                    libc::AT_EACCESS,
                ) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
            }
            let directory = directories
                .last()
                .ok_or_else(|| std::io::Error::from_raw_os_error(libc::ENOENT))?;
            if libc::fchdir(directory.as_raw_fd()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn own_uid_is_never_reported_as_command_isolated() {
        let directory = tempfile::tempdir().unwrap();
        let identity = Identity::resolve(None).unwrap();
        assert!(
            !identity
                .commands_isolated(&[directory.path().to_owned()])
                .await
        );
        assert!(!identity.commands_isolated(&[]).await);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn agent_namespace_filter_is_inherited_and_allows_ordinary_children() {
        let identity = Identity::resolve(None).unwrap();
        let mut command = Command::new("/bin/sh");
        identity.prepare_agent(&mut command).unwrap();
        // Verify errno in the filtered child, without relying on the host's
        // user-namespace setting. Invalid setns/clone3 normally return EBADF/EINVAL.
        unsafe {
            command.pre_exec(|| {
                for (number, arg, expected) in [
                    (libc::SYS_unshare, 0, libc::EPERM),
                    (libc::SYS_setns, -1, libc::EPERM),
                    (libc::SYS_clone3, 0, libc::ENOSYS),
                    (
                        libc::SYS_clone,
                        libc::CLONE_NEWUSER as libc::c_long,
                        libc::EPERM,
                    ),
                ] {
                    if libc::syscall(number, arg, 0, 0, 0, 0) != -1
                        || std::io::Error::last_os_error().raw_os_error() != Some(expected)
                    {
                        return Err(std::io::Error::from_raw_os_error(libc::EINVAL));
                    }
                }
                Ok(())
            });
        }
        let output = command
            .args(["-c", "/bin/sh -c 'printf child-ok'"])
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"child-ok");
    }

    #[tokio::test]
    async fn children_do_not_inherit_node_environment_and_reject_reserved_input() {
        let identity = Identity::resolve(None).unwrap();
        let mut command = Command::new("/usr/bin/env");
        identity.prepare(&mut command).unwrap();
        assert!(
            request_env(
                &mut command,
                &BTreeMap::from([("NYXID_TOKEN".into(), "forbidden".into())])
            )
            .is_err()
        );
        request_env(
            &mut command,
            &BTreeMap::from([("EXAMPLE".into(), "visible".into())]),
        )
        .unwrap();
        let output = command.output().await.unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("EXAMPLE=visible"));
        assert!(!text.contains("NYXID_"));
        assert!(!text.contains("CODEX_"));
    }
}
