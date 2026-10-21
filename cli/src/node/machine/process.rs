use std::{collections::BTreeMap, ffi::CString, path::PathBuf};

#[cfg(any(target_os = "linux", test))]
use anyhow::Context;
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
        if let Some(address) =
            std::fs::read_to_string(self.home.join(".nyxid-desktop/session-bus.address"))
                .ok()
                .or_else(|| {
                    (self.name == "browser"
                        && std::env::var_os("NYXID_MACHINE_CONTAINER").is_some())
                    .then(|| {
                        protected_desktop_bus(std::path::Path::new(
                            "/var/lib/nyxid-machine/desktop/session-bus.address",
                        ))
                    })
                    .flatten()
                })
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

/// The full image copies only the browser's session address into this
/// supervisor-owned handoff. No DAC override is needed to traverse a browser
/// home, and the command user cannot replace the handoff or inherit its value.
#[cfg(any(target_os = "linux", test))]
fn protected_desktop_bus(path: &std::path::Path) -> Option<String> {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.len() > 4096
    {
        return None;
    }
    let mut address = String::new();
    (&mut file).take(4097).read_to_string(&mut address).ok()?;
    let address = address.trim();
    (address.len() <= 4096 && address.starts_with("unix:") && !address.contains(['\n', '\r', '\0']))
        .then(|| address.to_owned())
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

#[cfg(any(target_os = "linux", test))]
#[derive(Debug, thiserror::Error)]
#[error("separated_probe_timeout")]
pub(super) struct ProbeTimeout;

#[cfg(any(target_os = "linux", test))]
struct ProbeGroup(i32);

#[cfg(any(target_os = "linux", test))]
impl ProbeGroup {
    fn kill(&mut self) -> std::io::Result<()> {
        if self.0 == 0 {
            return Ok(());
        }
        if unsafe { libc::kill(-self.0, libc::SIGKILL) } == 0 {
            self.0 = 0;
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            self.0 = 0;
            Ok(())
        } else {
            Err(error)
        }
    }
}

#[cfg(any(target_os = "linux", test))]
impl Drop for ProbeGroup {
    fn drop(&mut self) {
        // Cancellation must also sweep helpers, rather than just the parent.
        let _ = self.kill();
    }
}

#[cfg(any(target_os = "linux", test))]
/// Own the child until it is reaped, including on timeout and output overflow.
/// Kill-on-drop and the group guard also cover cancellation of the caller.
pub(super) async fn probe_output(mut command: Command) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    const OUTPUT_LIMIT: u64 = 64 * 1024;
    command
        .kill_on_drop(true)
        .process_group(0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = command.spawn()?;
    // Keep the group ID even after wait() has consumed the parent's PID.
    let mut group = ProbeGroup(child.id().context("separated_probe_pid_missing")? as i32);
    let mut stdout = child
        .stdout
        .take()
        .context("separated_probe_stdout_missing")?
        .take(OUTPUT_LIMIT + 1);
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).await?;
        anyhow::ensure!(
            output.len() as u64 <= OUTPUT_LIMIT,
            "separated_probe_output_limit"
        );
        anyhow::ensure!(
            child.wait().await?.success(),
            "separated_mechanism_probe_failed"
        );
        Ok(output)
    })
    .await;
    // A successful Chromium parent can leave zygotes/helpers behind too.
    let swept = group.kill();
    if child.id().is_some() {
        let killed = child.start_kill();
        child.wait().await?;
        killed?;
    }
    swept?;
    match result {
        Ok(Ok(output)) => Ok(output),
        Ok(error) => error,
        Err(_) => Err(ProbeTimeout.into()),
    }
}

/// Observe a live disposable child without using a DevTools transport. Output
/// is bounded and private; every exit path sweeps helpers and reaps the parent.
#[cfg(any(target_os = "linux", test))]
pub(super) async fn probe_running(
    mut command: Command,
    bound: std::time::Duration,
    mut observe: impl FnMut(u32, &[u8]) -> Result<bool>,
) -> Result<()> {
    use tokio::io::AsyncReadExt;
    command
        .kill_on_drop(true)
        .process_group(0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn()?;
    let pid = child.id().context("separated_probe_pid_missing")?;
    let mut group = ProbeGroup(pid as i32);
    let mut stderr = child
        .stderr
        .take()
        .context("separated_probe_stderr_missing")?;
    let result = tokio::time::timeout(bound, async {
        let mut output = Vec::new();
        let mut buffer = [0u8; 4096];
        let mut eof = false;
        loop {
            anyhow::ensure!(child.try_wait()?.is_none(), "separated_mechanism_probe_exited");
            if observe(pid, &output)? {
                anyhow::ensure!(child.try_wait()?.is_none(), "separated_mechanism_probe_exited");
                return Ok(());
            }
            tokio::select! {
                bytes = stderr.read(&mut buffer), if !eof => {
                    let bytes = bytes?;
                    eof = bytes == 0;
                    anyhow::ensure!(output.len() + bytes <= 64 * 1024, "separated_probe_output_limit");
                    output.extend_from_slice(&buffer[..bytes]);
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
            }
        }
    }).await;
    let swept = group.kill();
    if child.id().is_some() {
        let killed = child.start_kill();
        child.wait().await?;
        killed?;
    }
    swept?;
    match result {
        Ok(result) => result,
        Err(_) => Err(ProbeTimeout.into()),
    }
}

#[cfg(target_os = "linux")]
async fn probe_status(command: Command) -> Result<()> {
    probe_output(command).await.map(|_| ())
}

#[cfg(target_os = "linux")]
pub(super) async fn probe_command_seccomp(identity: &Identity) -> Result<()> {
    let mut command = Command::new("/bin/true");
    identity.prepare_agent(&mut command)?;
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) != 1
                || libc::prctl(libc::PR_GET_SECCOMP, 0, 0, 0, 0)
                    != libc::SECCOMP_MODE_FILTER as libc::c_int
                || libc::syscall(libc::SYS_unshare, 0) != -1
                || std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM)
            {
                return Err(std::io::Error::from_raw_os_error(libc::EPERM));
            }
            Ok(())
        });
    }
    probe_status(command).await
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
    async fn live_probe_sweeps_on_observation_and_timeout() {
        for ready in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let pid_file = directory.path().join("pid");
            let mut command = Command::new("/bin/sh");
            command
                .args(["-c", "echo $$ > \"$1\"; exec /bin/sleep 30", "probe"])
                .arg(&pid_file);
            let result = probe_running(command, std::time::Duration::from_millis(200), |_, _| {
                Ok(ready && pid_file.exists())
            })
            .await;
            if ready {
                result.unwrap();
            } else {
                assert!(result.unwrap_err().is::<ProbeTimeout>());
            }
            let pid: i32 = std::fs::read_to_string(&pid_file)
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
            let mut status = 0;
            assert_eq!(
                unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
                -1
            );
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ECHILD)
            );
        }
    }

    #[tokio::test]
    async fn successful_probe_sweeps_surviving_grandchildren() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("grandchild");
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "/bin/sleep 30 </dev/null >/dev/null 2>&1 & echo $! > \"$1\"; printf success; exit 0",
                "probe",
            ])
            .arg(&pid_file);
        assert_eq!(probe_output(command).await.unwrap(), b"success");
        let pid: i32 = std::fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // Allow init to reap the killed orphan. The production probe itself
        // must have returned successfully, rather than timing out on stdout.
        for _ in 0..100 {
            if unsafe { libc::kill(pid, 0) } == -1 {
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::ESRCH)
                );
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("probe grandchild survived successful parent exit");
    }

    #[tokio::test]
    async fn timed_out_probe_is_killed_and_reaped() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("pid");
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "echo $$ > \"$1\"; exec /bin/sleep 30", "probe"])
            .arg(&pid_file);
        let error = probe_output(command).await.unwrap_err();
        assert_eq!(error.to_string(), "separated_probe_timeout");
        let pid: i32 = std::fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "probe child survived timeout"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }

    #[test]
    fn desktop_bus_handoff_requires_private_regular_supervisor_file() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bus");
        std::fs::write(&path, "unix:path=/tmp/fixture-bus\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            protected_desktop_bus(&path).as_deref(),
            Some("unix:path=/tmp/fixture-bus")
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(protected_desktop_bus(&path).is_none());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(protected_desktop_bus(&link).is_none());
        std::fs::write(&path, "tcp:host=untrusted").unwrap();
        assert!(protected_desktop_bus(&path).is_none());
        std::fs::write(&path, "unix:path=/tmp/first\nunix:path=/tmp/second").unwrap();
        assert!(protected_desktop_bus(&path).is_none());
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
