use super::*;
use nyxid_machine::managed::{Usage, UsageVolume};
use std::{
    ffi::{CStr, CString},
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::Component,
};

impl Runtime {
    pub fn managed_usage(&self) -> Result<Usage> {
        let binding = self
            .managed_binding
            .get()
            .context("managed binding unavailable")?
            .clone();
        let workspace = filesystem_usage(Path::new("/workspace"))?;
        let temporary = temporary_usage(&[Path::new("/tmp"), Path::new("/home/agent")]);
        let disk_full = workspace.available_bytes == Some(0);
        Ok(Usage {
            runtime_id: self.runtime_id.clone(),
            managed_runtime: binding,
            sampled_at_ms: chrono::Utc::now().timestamp_millis(),
            workspace,
            temporary,
            disk_full,
        })
    }
}

fn filesystem_usage(path: &Path) -> Result<UsageVolume> {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(name.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let stat = unsafe { stat.assume_init() };
    // libc's statvfs field widths differ between Linux and macOS.
    #[allow(clippy::unnecessary_cast)]
    let (unit, blocks, free, available) = (
        stat.f_frsize as u64,
        stat.f_blocks as u64,
        stat.f_bfree as u64,
        stat.f_bavail as u64,
    );
    Ok(UsageVolume {
        capacity_bytes: Some(blocks.saturating_mul(unit)),
        used_bytes: blocks.saturating_sub(free).saturating_mul(unit),
        available_bytes: Some(available.saturating_mul(unit)),
        measurement: "filesystem".into(),
        complete: true,
    })
}

fn temporary_usage(paths: &[&Path]) -> UsageVolume {
    temporary_usage_observed(paths, |_| {})
}

fn open_directory_at(parent: i32, name: &CStr) -> std::io::Result<File> {
    // Every lookup is relative to an already-open directory. O_NOFOLLOW rejects
    // a command-user symlink swap between metadata observation and openat.
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn open_usage_root(path: &Path) -> std::io::Result<File> {
    if !path.is_absolute() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let mut directory = open_directory_at(libc::AT_FDCWD, c"/")?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                let name = CString::new(name.as_bytes())
                    .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
                directory = open_directory_at(directory.as_raw_fd(), &name)?;
            }
            _ => return Err(std::io::ErrorKind::InvalidInput.into()),
        }
    }
    Ok(directory)
}

struct UsageDirectory(*mut libc::DIR);

impl Drop for UsageDirectory {
    fn drop(&mut self) {
        unsafe { libc::closedir(self.0) };
    }
}

// The no-op production observer optimizes away. Tests use the observer to swap
// an actual directory after fstatat and before openat, exercising that race.
fn temporary_usage_observed(
    paths: &[&Path],
    mut observed_directory: impl FnMut(&CStr),
) -> UsageVolume {
    let started = std::time::Instant::now();
    let mut pending: Vec<(File, u32)> = paths
        .iter()
        .take(64)
        .filter_map(|path| open_usage_root(path).ok().map(|file| (file, 0)))
        .collect();
    let mut seen = std::collections::HashSet::new();
    let mut bytes = 0u64;
    let mut visited = 0usize;
    'scan: while let Some((directory, depth)) = pending.pop() {
        if visited >= 10_000 || started.elapsed() > std::time::Duration::from_millis(100) {
            break;
        }
        visited += 1;
        let Ok(metadata) = directory.metadata() else {
            continue;
        };
        if !seen.insert((metadata.dev(), metadata.ino())) {
            continue;
        }
        bytes = bytes.saturating_add(metadata.blocks().saturating_mul(512));
        let fd = directory.into_raw_fd();
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            unsafe { libc::close(fd) };
            continue;
        }
        let stream = UsageDirectory(stream);
        loop {
            if visited >= 10_000 || started.elapsed() > std::time::Duration::from_millis(100) {
                break 'scan;
            }
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            visited += 1;
            let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe {
                libc::fstatat(
                    fd,
                    name.as_ptr(),
                    metadata.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                continue;
            }
            let metadata = unsafe { metadata.assume_init() };
            // Match MetadataExt's u64 identity across libc platform typedefs.
            #[allow(clippy::unnecessary_cast)]
            let observed_identity = (metadata.st_dev as u64, metadata.st_ino as u64);
            if metadata.st_mode & libc::S_IFMT == libc::S_IFDIR {
                // Descriptor/depth bounds keep a hostile tree from consuming
                // unbounded resources. Skips preserve lower-bound semantics.
                if depth < 64 && pending.len() < 64 {
                    observed_directory(name);
                    if let Ok(child) = open_directory_at(fd, name)
                        && let Ok(current) = child.metadata()
                        && (current.dev(), current.ino()) == observed_identity
                    {
                        pending.push((child, depth + 1));
                    }
                }
            } else if seen.insert(observed_identity) {
                bytes =
                    bytes.saturating_add((metadata.st_blocks.max(0) as u64).saturating_mul(512));
            }
        }
    }
    UsageVolume {
        capacity_bytes: None,
        used_bytes: bytes,
        available_bytes: None,
        measurement: "node_paths_lower_bound".into(),
        complete: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filesystem_snapshot_is_bounded_and_temporary_never_claims_quota_accounting() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("data"), vec![1; 8192]).unwrap();
        std::os::unix::fs::symlink("/", root.path().join("outside")).unwrap();
        let disk = filesystem_usage(root.path()).unwrap();
        assert!(disk.capacity_bytes.unwrap() > 0);
        assert!(disk.available_bytes.unwrap() <= disk.capacity_bytes.unwrap());
        let root_path = root.path().canonicalize().unwrap();
        let temporary = temporary_usage(&[&root_path]);
        assert!(!temporary.complete);
        assert_eq!(temporary.available_bytes, None);
        assert_eq!(temporary.capacity_bytes, None);
        assert!(temporary.used_bytes >= 8192 && temporary.used_bytes < 1024 * 1024);
    }

    #[test]
    fn temporary_scan_rejects_actual_directory_swap_after_metadata() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let target = root_path.join("swap");
        std::fs::create_dir(&target).unwrap();
        // A real allocated file makes accidental traversal observable without
        // exposing any real protected paths or depending on /proc contents.
        std::fs::write(
            outside.path().join("must-not-count"),
            vec![1; 4 * 1024 * 1024],
        )
        .unwrap();
        let mut swapped = false;
        let usage = temporary_usage_observed(&[&root_path], |name| {
            if name.to_bytes() == b"swap" && !swapped {
                std::fs::rename(&target, root_path.join("moved")).unwrap();
                std::os::unix::fs::symlink(outside.path(), &target).unwrap();
                swapped = true;
            }
        });
        assert!(swapped, "test must reach the metadata/open race point");
        assert!(usage.used_bytes < 1024 * 1024);
        assert!(!usage.complete);
        assert_eq!(usage.available_bytes, None);
    }

    #[test]
    fn temporary_scan_rejects_symlink_in_root_components() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        std::fs::create_dir(outside.path().join("nested")).unwrap();
        std::fs::write(outside.path().join("nested/data"), vec![1; 8192]).unwrap();
        std::os::unix::fs::symlink(outside.path(), root_path.join("alias")).unwrap();
        let usage = temporary_usage(&[&root_path.join("alias/nested")]);
        assert_eq!(usage.used_bytes, 0);
        assert!(!usage.complete);
    }
}
