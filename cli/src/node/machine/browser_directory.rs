//! Browser directory transitions for supervisors with CHOWN but no FOWNER or
//! DAC override. Callers hold their browser startup lock while repairing a
//! profile. Never change ownership or permissions through the supplied path.
use anyhow::{Context, Result, ensure};
use std::{
    ffi::CString,
    fs::{File, Metadata},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::Path,
};

#[derive(Clone, Copy, Debug, thiserror::Error)]
enum Stage {
    #[error("browser_directory_parent_untrusted")]
    Parent,
    #[error("browser_directory_path_invalid")]
    Path,
    #[error("browser_directory_create_failed")]
    Create,
    #[error("browser_directory_open_failed")]
    Open,
    #[error("browser_directory_metadata_failed")]
    Metadata,
    #[error("browser_directory_owner_untrusted")]
    Owner,
    #[error("browser_directory_take_ownership_failed")]
    TakeOwnership,
    #[error("browser_directory_permissions_failed")]
    Permissions,
    #[error("browser_directory_restore_ownership_failed")]
    RestoreOwnership,
    #[error("browser_directory_verification_failed")]
    Verify,
}

fn checked_parent(path: &Path, create: bool) -> Result<File> {
    // Shared custody traversal validates every ancestor before following a
    // supervisor-owned system alias (e.g. macOS /var). The selected parent
    // itself cannot be a symlink, or writable even through a sticky directory.
    let parent = crate::trusted_directory::open(path, create).context(Stage::Parent)?;
    let meta = parent.metadata().context(Stage::Metadata)?;
    ensure!(meta.mode() & 0o022 == 0, Stage::Parent);
    Ok(parent)
}

pub(super) fn protected_parent(path: &Path) -> Result<()> {
    checked_parent(path, true).map(|_| ())
}

fn syscall(result: libc::c_int, stage: Stage) -> Result<()> {
    if result != 0 {
        return Err(anyhow::Error::new(std::io::Error::last_os_error()).context(stage));
    }
    Ok(())
}

fn own(pinned: &File, uid: u32, gid: u32, stage: Stage) -> Result<()> {
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::fchownat(
            pinned.as_raw_fd(),
            c"".as_ptr(),
            uid,
            gid,
            libc::AT_EMPTY_PATH,
        )
    };
    #[cfg(not(target_os = "linux"))]
    let result = unsafe { libc::fchown(pinned.as_raw_fd(), uid, gid) };
    syscall(result, stage)
}

fn readable(pinned: &File) -> Result<File> {
    let open = || unsafe {
        libc::openat(
            pinned.as_raw_fd(),
            c".".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    let fd = open();
    #[cfg(target_os = "linux")]
    let fd = if fd < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EACCES) {
        // O_PATH pins even a mode-000 directory but cannot be fchmod'ed. The
        // supervisor now owns this inode: repair only that descriptor's kernel
        // procfs link, never the original (possibly replaced) pathname. This
        // also works on kernels predating fchmodat2(AT_EMPTY_PATH).
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            format!("/proc/self/fd/{}", pinned.as_raw_fd()),
            std::fs::Permissions::from_mode(0o700),
        )
        .context(Stage::Permissions)?;
        open()
    } else {
        fd
    };
    if fd < 0 {
        return Err(anyhow::Error::new(std::io::Error::last_os_error()).context(Stage::Open));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn matches(meta: &Metadata, uid: u32, gid: u32, mode: u32) -> bool {
    meta.uid() == uid && meta.gid() == gid && meta.mode() & 0o7777 == mode
}

fn repair(pinned: &File, uid: u32, gid: u32, mode: u32) -> Result<()> {
    let supervisor = unsafe { libc::geteuid() };
    if supervisor == 0 {
        own(
            pinned,
            supervisor,
            unsafe { libc::getegid() },
            Stage::TakeOwnership,
        )?;
    }
    let result: Result<()> = (|| {
        let directory = readable(pinned)?;
        syscall(
            unsafe { libc::fchmod(directory.as_raw_fd(), mode as libc::mode_t) },
            Stage::Permissions,
        )?;
        own(pinned, uid, gid, Stage::RestoreOwnership)?;
        ensure!(
            matches(&pinned.metadata().context(Stage::Metadata)?, uid, gid, mode),
            Stage::Verify
        );
        Ok(())
    })();
    if result.is_err() {
        // A failed chmod/read-open must not silently strand a live profile
        // under the temporary supervisor owner. If restoring ownership also
        // fails, startup still stops; a private supervisor-owned directory is
        // accepted on the next attempt as an interrupted initialization.
        own(pinned, uid, gid, Stage::RestoreOwnership)?;
    }
    result
}

pub(super) fn initialize(path: &Path, uid: u32, gid: u32, mode: u32) -> Result<()> {
    let mut observed: Option<Metadata> = None;
    let result: Result<()> = (|| {
        ensure!(mode & !0o777 == 0, Stage::Path);
        let parent = checked_parent(path.parent().context(Stage::Path)?, false)?;
        let name =
            CString::new(path.file_name().context(Stage::Path)?.as_bytes()).context(Stage::Path)?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0
            && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST)
        {
            return Err(anyhow::Error::new(std::io::Error::last_os_error()).context(Stage::Create));
        }
        #[cfg(target_os = "linux")]
        let access = libc::O_PATH;
        #[cfg(not(target_os = "linux"))]
        let access = libc::O_RDONLY;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                access | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(anyhow::Error::new(std::io::Error::last_os_error()).context(Stage::Open));
        }
        let pinned = unsafe { File::from_raw_fd(fd) };
        let meta = observed.insert(pinned.metadata().context(Stage::Metadata)?);
        let supervisor = unsafe { libc::geteuid() };
        ensure!(
            meta.is_dir() && (meta.uid() == uid || meta.uid() == supervisor),
            Stage::Owner
        );
        // Only a private supervisor-owned profile may be an interrupted mkdir
        // or repair. A browser-run directory intentionally stays supervisor-owned.
        ensure!(
            meta.uid() != supervisor || uid == supervisor || meta.mode() & 0o077 == 0,
            Stage::Owner
        );
        if matches(meta, uid, gid, mode) {
            return Ok(());
        }
        repair(&pinned, uid, gid, mode)
    })();
    if let Err(error) = &result {
        tracing::warn!(
            stage = "runtime_directory",
            reason = %error.downcast_ref::<Stage>().copied().unwrap_or(Stage::Metadata),
            errno = error.downcast_ref::<std::io::Error>().and_then(std::io::Error::raw_os_error),
            expected_uid = uid, expected_gid = gid, expected_mode = mode,
            actual_uid = observed.as_ref().map(MetadataExt::uid),
            actual_gid = observed.as_ref().map(MetadataExt::gid),
            actual_mode = observed.as_ref().map(|m| m.mode() & 0o7777),
            "browser_directory_initialization_failed"
        );
    }
    result
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    #[test]
    #[ignore = "requires disposable hardened Linux supervisor"]
    fn runtime_directory_repair_keeps_the_pinned_inode_after_name_replacement() {
        let root = tempfile::tempdir_in("/var/lib/nyxid-hosted-tests").unwrap();
        let path = root.path().join("profile");
        initialize(&path, 1001, 1001, 0o700).unwrap();
        let pinned = File::options()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .unwrap();
        let original = pinned.metadata().unwrap();
        let retired = root.path().join("retired");
        std::fs::rename(&path, &retired).unwrap();
        initialize(&path, 1002, 1002, 0o700).unwrap();
        let replacement = std::fs::symlink_metadata(&path).unwrap();
        // Even a trusted actor replacing the protected pathname cannot redirect
        // the ownership/mode transition to a different worker's directory.
        repair(&pinned, 1001, 1001, 0o700).unwrap();
        let after = pinned.metadata().unwrap();
        assert_eq!((after.dev(), after.ino()), (original.dev(), original.ino()));
        assert!(matches(&after, 1001, 1001, 0o700));
        let untouched = std::fs::symlink_metadata(&path).unwrap();
        assert_eq!(
            (
                untouched.uid(),
                untouched.gid(),
                untouched.mode(),
                untouched.ctime(),
                untouched.ctime_nsec()
            ),
            (
                replacement.uid(),
                replacement.gid(),
                replacement.mode(),
                replacement.ctime(),
                replacement.ctime_nsec()
            )
        );
        for path in [&path, &retired] {
            std::os::unix::fs::chown(path, Some(0), Some(0)).unwrap();
        }
    }

    #[test]
    #[ignore = "requires disposable hardened Linux supervisor"]
    fn runtime_directory_failed_chmod_restores_worker_ownership() {
        const CHILD: &str = "NYXID_TEST_BROWSER_DIRECTORY_CHMOD_FAILURE";
        if let Some(path) = std::env::var_os(CHILD) {
            // Real kernel failure injection, isolated to a new test process:
            // chown succeeds, fchmod returns EIO, restoration remains allowed.
            let mut filter = [
                libc::sock_filter {
                    code: 0x20,
                    jt: 0,
                    jf: 0,
                    k: 0,
                },
                libc::sock_filter {
                    code: 0x15,
                    jt: 0,
                    jf: 1,
                    k: libc::SYS_fchmod as u32,
                },
                libc::sock_filter {
                    code: 0x06,
                    jt: 0,
                    jf: 0,
                    k: 0x0005_0000 | libc::EIO as u32,
                },
                libc::sock_filter {
                    code: 0x06,
                    jt: 0,
                    jf: 0,
                    k: 0x7fff_0000,
                },
            ];
            let program = libc::sock_fprog {
                len: filter.len() as u16,
                filter: filter.as_mut_ptr(),
            };
            assert_eq!(unsafe { libc::prctl(libc::PR_SET_SECCOMP, 2, &program) }, 0);
            let error = initialize(Path::new(&path), 1001, 1001, 0o700).unwrap_err();
            assert_eq!(
                error
                    .downcast_ref::<std::io::Error>()
                    .unwrap()
                    .raw_os_error(),
                Some(libc::EIO)
            );
            return;
        }
        let root = tempfile::tempdir_in("/var/lib/nyxid-hosted-tests").unwrap();
        let path = root.path().join("profile");
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
        std::os::unix::fs::chown(&path, Some(1001), Some(1001)).unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!(
                    "{}::runtime_directory_failed_chmod_restores_worker_ownership",
                    module_path!().split_once("::").unwrap().1
                ),
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD, &path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "injected chmod failure regression failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("test result: ok. 1 passed"),
            "the injected failure subprocess must execute exactly one test"
        );
        assert!(matches(
            &std::fs::symlink_metadata(&path).unwrap(),
            1001,
            1001,
            0o500
        ));
        initialize(&path, 1001, 1001, 0o700).unwrap();
        std::os::unix::fs::chown(&path, Some(0), Some(0)).unwrap();
    }
}
