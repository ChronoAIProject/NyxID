//! Kernel-backed inventory. Argument buffers may contain secrets: zeroize them
//! and return only a fixed refusal, never arguments, executable paths or PIDs.
use super::cutover::refuse;
use crate::node::error::Result;
use zeroize::Zeroizing;

pub(super) fn exists(pid: u32) -> Result<bool> {
    if pid == 0 || pid > i32::MAX as u32 {
        return Err(refuse("process_inventory_invalid"));
    }
    let result = unsafe { libc::kill(pid as i32, 0) };
    if result == 0 {
        return Ok(true);
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(refuse("process_inventory_unavailable")),
    }
}

fn is_runtime(executable: &[u8], arguments: &[&[u8]]) -> bool {
    fn basename(path: &[u8]) -> &[u8] {
        path.rsplit(|b| *b == b'/').next().unwrap_or(path)
    }
    // Historical standalone agents have no `node start` prefix. Other node
    // commands may also retain source values, so conservatively refuse them,
    // including renamed executables. The calling migration PID is excluded by
    // the inventory, not by argument classification. Node.js argv[0] is skipped.
    basename(executable) == b"nyxid-node"
        || arguments
            .first()
            .is_some_and(|v| basename(v) == b"nyxid-node")
        || arguments
            .get(1..)
            .unwrap_or_default()
            .windows(2)
            .any(|pair| pair[0] == b"node")
}

#[cfg(target_os = "macos")]
pub(super) fn require_no_runtimes() -> Result<()> {
    // A bounded oversized buffer avoids accepting a truncated PID inventory.
    // PROC_UID_ONLY asks the kernel for the effective supervisor UID. Querying
    // all PIDs first cannot establish this scope: macOS denies metadata for
    // protected processes under other UIDs. Privileged/other-UID runtimes are
    // explicitly covered by the operator's maintenance acknowledgement.
    const PROC_UID_ONLY: u32 = 4;
    let uid = unsafe { libc::geteuid() };
    let mut pids = vec![0i32; 65536];
    let capacity = std::mem::size_of_val(pids.as_slice());
    let bytes = unsafe {
        libc::proc_listpids(
            PROC_UID_ONLY,
            uid,
            pids.as_mut_ptr().cast(),
            capacity as i32,
        )
    };
    if bytes <= 0
        || bytes as usize >= capacity
        || !(bytes as usize).is_multiple_of(size_of::<i32>())
    {
        return Err(refuse("process_inventory_unavailable"));
    }
    // Keep one bounded zeroizing scratch buffer for this whole scan. Wiping a
    // fresh megabyte for every PID makes each phase's inventory unnecessarily
    // expensive in unoptimized builds. Only the kernel-returned prefix is ever
    // parsed, and the complete buffer is wiped on every exit, including errors.
    let mut arguments = Zeroizing::new(vec![0u8; 1024 * 1024]);
    for pid in pids
        .into_iter()
        .take(bytes as usize / size_of::<i32>())
        .filter(|p| *p > 0 && *p as u32 != std::process::id())
    {
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>();
        let read = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size as i32,
            )
        };
        if read as usize != size {
            if !exists(pid as u32)? {
                continue;
            }
            return Err(refuse("process_inventory_unavailable"));
        }
        let info = unsafe { info.assume_init() };
        // Zombies cannot retain credentials or execute. Other UIDs cannot use
        // this owner-only directory without privileges; privileged custody is
        // explicitly part of the operator's manual cutover boundary.
        if info.pbi_status == 5 || info.pbi_uid != uid {
            continue;
        }
        let mut length = arguments.len();
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
        let read = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                arguments.as_mut_ptr().cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        if read != 0 {
            if !exists(pid as u32)? {
                continue;
            }
            return Err(refuse("process_inventory_unavailable"));
        }
        let bytes = arguments
            .get(..length)
            .ok_or_else(|| refuse("process_inventory_invalid"))?;
        let (executable, args) = darwin_arguments(bytes)?;
        if is_runtime(executable, &args) {
            return Err(refuse("node_process_running"));
        }
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn darwin_arguments(bytes: &[u8]) -> Result<(&[u8], Vec<&[u8]>)> {
    let count = bytes
        .get(..4)
        .ok_or_else(|| refuse("process_inventory_invalid"))?;
    let count = i32::from_ne_bytes(count.try_into().unwrap());
    if !(1..=65536).contains(&count) {
        return Err(refuse("process_inventory_invalid"));
    }
    let rest = &bytes[4..];
    let end = rest
        .iter()
        .position(|v| *v == 0)
        .ok_or_else(|| refuse("process_inventory_invalid"))?;
    let executable = &rest[..end];
    let mut rest = &rest[end..];
    // Darwin pads between executable path and argv[0]. argv after that keeps
    // empty elements; environment bytes are deliberately not interpreted.
    while rest.first() == Some(&0) {
        rest = &rest[1..];
    }
    let mut args = Vec::new();
    for _ in 0..count {
        let end = rest
            .iter()
            .position(|v| *v == 0)
            .ok_or_else(|| refuse("process_inventory_invalid"))?;
        args.push(&rest[..end]);
        rest = &rest[end + 1..];
    }
    Ok((executable, args))
}

#[cfg(target_os = "linux")]
pub(super) fn require_no_runtimes() -> Result<()> {
    use std::io::Read;
    fn bounded(path: &std::path::Path, max: u64) -> std::io::Result<Zeroizing<Vec<u8>>> {
        let mut bytes = Zeroizing::new(Vec::new());
        std::fs::File::open(path)?
            .take(max + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > max {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        Ok(bytes)
    }
    let uid = unsafe { libc::geteuid() };
    let entries =
        std::fs::read_dir("/proc").map_err(|_| refuse("process_inventory_unavailable"))?;
    for (index, entry) in entries.enumerate() {
        if index >= 65536 {
            return Err(refuse("process_inventory_unavailable"));
        }
        let entry = entry.map_err(|_| refuse("process_inventory_unavailable"))?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|v| v.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        let result = (|| {
            let status = bounded(&entry.path().join("status"), 65536)?;
            let status =
                std::str::from_utf8(&status).map_err(|_| std::io::ErrorKind::InvalidData)?;
            let ids = status
                .lines()
                .find_map(|s| s.strip_prefix("Uid:"))
                .ok_or(std::io::ErrorKind::InvalidData)?;
            let ids = ids
                .split_whitespace()
                .map(str::parse::<u32>)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| std::io::ErrorKind::InvalidData)?;
            if ids.len() != 4 {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            // Kernel threads have no userspace argv or credential store. Only
            // the kernel's explicit flag permits this case; an empty argv for
            // any userspace process still refuses the inventory.
            let kernel_thread = status
                .lines()
                .any(|s| s.strip_prefix("Kthread:").is_some_and(|v| v.trim() == "1"));
            if !ids.contains(&uid)
                || kernel_thread
                || status.lines().any(|s| s.starts_with("State:\tZ"))
            {
                return Ok(false);
            }
            let args = bounded(&entry.path().join("cmdline"), 1024 * 1024)?;
            let args = args
                .strip_suffix(&[0])
                .ok_or(std::io::ErrorKind::InvalidData)?
                .split(|b| *b == 0)
                .collect::<Vec<_>>();
            let executable = std::fs::read_link(entry.path().join("exe"))?;
            use std::os::unix::ffi::OsStrExt;
            Ok::<_, std::io::Error>(is_runtime(executable.as_os_str().as_bytes(), &args))
        })();
        match result {
            Ok(true) => return Err(refuse("node_process_running")),
            Ok(false) => (),
            Err(_) if !exists(pid)? => (),
            Err(_) => return Err(refuse("process_inventory_unavailable")),
        }
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) fn require_no_runtimes() -> Result<()> {
    Err(refuse("platform_unsupported"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_renamed_modern_and_historical_agents_without_name_heuristics() {
        assert!(is_runtime(
            b"/renamed",
            &[b"renamed", b"node", b"start", b"--config", b"/some profile"]
        ));
        assert!(is_runtime(b"/old/nyxid-node", &[b"whatever"]));
        assert!(is_runtime(b"/renamed", &[b"nyxid-node", b"--config", b"x"]));
        for command in [b"managed-start".as_slice(), b"docker-entrypoint"] {
            assert!(is_runtime(b"/renamed", &[b"renamed", b"node", command]));
        }
        assert!(!is_runtime(
            b"/bin/sh",
            &[b"sh", b"-c", b"nyxid node start"]
        ));
        assert!(is_runtime(b"/nyxid", &[b"nyxid", b"node", b"migrate"]));
        assert!(!is_runtime(b"/usr/bin/node", &[b"node", b"start"]));
    }
    #[test]
    fn darwin_argv_keeps_empty_values_and_never_parses_the_environment() {
        let mut bytes = 4i32.to_ne_bytes().to_vec();
        bytes.extend_from_slice(
            b"/path with spaces/renamed\0\0renamed\0node\0start\0\0TOKEN=SENTINEL\0",
        );
        let (path, args) = darwin_arguments(&bytes).unwrap();
        assert_eq!(path, b"/path with spaces/renamed");
        assert!(is_runtime(path, &args));
        assert_eq!(args.last().unwrap(), &b"".as_slice());
        assert!(darwin_arguments(&bytes[..10]).is_err());
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::time::{Duration, Instant};
    #[test]
    fn inventory_process_child() {
        let Some(ready) = std::env::var_os("NYXID_TEST_NODE_PROCESS_READY") else {
            return;
        };
        std::fs::write(ready, b"ready").unwrap();
        loop {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[test]
    fn native_inventory_detects_an_old_agent_without_a_new_lock_or_pid_file() {
        let _serial = crate::test_support::env_lock().lock().unwrap();
        require_no_runtimes().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let ready = temp.path().join("ready");
        let mut child = Child(
            std::process::Command::new(std::env::current_exe().unwrap())
                .arg0("nyxid-node")
                .args([
                    "--exact",
                    "node::daemon::processes::native_tests::inventory_process_child",
                ])
                .env("NYXID_TEST_NODE_PROCESS_READY", &ready)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while !ready.exists() {
            assert!(child.0.try_wait().unwrap().is_none());
            assert!(Instant::now() < deadline, "probe child never became ready");
            std::thread::sleep(Duration::from_millis(10));
        }
        let error = require_no_runtimes().unwrap_err().to_string();
        assert!(error.contains("node_process_running"));
        assert!(exists(child.0.id()).unwrap());
        drop(child);
        require_no_runtimes().unwrap();
        assert!(!temp.path().join("nyxid-node.pid").exists());
        assert!(!temp.path().join(".nyxid-storage-runtime.lock").exists());
    }
}
