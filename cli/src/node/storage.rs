//! Read-side subset of the private node storage layer, used by credential
//! custody. File custody assumes a distinct trusted supervisor OS identity; it
//! does not isolate processes sharing that identity. Nothing here creates,
//! repairs, publishes or migrates node files; the writer, lease and publication
//! API extends this module with the secret-storage migration work.
use std::fs::File;
use std::path::Path;

use super::error::{Error, Result};

#[cfg(unix)]
use crate::trusted_directory;

pub(crate) const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

fn invalid() -> Error {
    Error::Config(
        "Node storage requires private regular files and trusted directory custody".into(),
    )
}

/// Every operation after opening is relative to this pinned directory inode.
/// Ancestors must belong to root or this identity and disallow other writers,
/// except trusted-owned sticky directories (for example /tmp). Same-UID and
/// privileged changes are outside this custody boundary.
pub(crate) struct Directory(File);

impl Directory {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        Self::open_with_policy(path, true)
    }

    fn open_with_policy(path: &Path, final_component: bool) -> Result<Self> {
        #[cfg(unix)]
        {
            let absolute = if path.is_absolute() {
                path.to_owned()
            } else {
                std::env::current_dir()?.join(path)
            };
            let directory = trusted_directory::open(&absolute, false).map_err(|error| {
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    invalid()
                } else {
                    error.into()
                }
            })?;
            validate_directory(&directory, final_component)?;
            Ok(Self(directory))
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(Error::Config(
                "Node configuration custody currently requires a supported Unix supervisor".into(),
            ))
        }
    }

    /// Confirms the pathname still selects this pinned directory inode.
    pub(crate) fn check_path(&self, path: &Path) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let selected = Self::open(path)?;
            let original = self.0.metadata()?;
            let current = selected.0.metadata()?;
            if original.dev() != current.dev() || original.ino() != current.ino() {
                return Err(invalid());
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(invalid())
        }
    }

    /// Strict bounded read; never creates, repairs, or follows a symlink.
    pub(crate) fn read(&self, name: &std::ffi::OsStr, limit: u64) -> Result<Vec<u8>> {
        #[cfg(unix)]
        {
            use std::io::Read;
            let file = open_at(&self.0, name, libc::O_RDONLY | libc::O_NONBLOCK, 0)?;
            validate(&file)?;
            if file.metadata()?.len() > limit {
                return Err(invalid());
            }
            // Protect key material even if a partial read fails or crosses the
            // bound. Successful callers take ownership without copying bytes.
            let mut bytes = zeroize::Zeroizing::new(Vec::new());
            file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > limit {
                return Err(invalid());
            }
            Ok(std::mem::take(&mut *bytes))
        }
        #[cfg(not(unix))]
        {
            let _ = (name, limit);
            Err(invalid())
        }
    }
}

#[cfg(unix)]
fn filename(name: &std::ffi::OsStr) -> Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        return Err(invalid());
    }
    std::ffi::CString::new(bytes).map_err(|_| invalid())
}

#[cfg(unix)]
fn open_at(
    directory: &File,
    name: &std::ffi::OsStr,
    flags: libc::c_int,
    mode: libc::mode_t,
) -> Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let name = filename(name)?;
    // SAFETY: borrowed valid fd, owned NUL-terminated name, mode when creating.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: openat returned a new owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn validate_directory(directory: &File, final_component: bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta = directory.metadata()?;
    let uid = unsafe { libc::geteuid() };
    let trusted_owner = meta.uid() == uid || (!final_component && meta.uid() == 0);
    let sticky_ancestor =
        !final_component && u64::from(meta.mode()) & u64::from(libc::S_ISVTX) != 0;
    if !meta.is_dir() || !trusted_owner || (meta.mode() & 0o022 != 0 && !sticky_ancestor) {
        return Err(invalid());
    }
    Ok(())
}

fn validate(file: &File) -> Result<()> {
    let meta = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !meta.is_file()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o077 != 0
            || meta.nlink() != 1
        {
            return Err(invalid());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        Err(invalid())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};

    fn private_file(path: &Path, bytes: &[u8]) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        file.write_all(bytes).unwrap();
    }

    fn mk_private_dir(path: &Path) {
        std::fs::create_dir(path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn mutable_nonsticky_ancestor_and_final_directory_symlink_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("mutable");
        let child = parent.join("private");
        std::fs::create_dir(&parent).unwrap();
        mk_private_dir(&child);
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(Directory::open(&child).is_err());
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(Directory::open(&child).is_ok());
        let alias = dir.path().join("alias");
        symlink(&child, &alias).unwrap();
        assert!(Directory::open(&alias).is_err());
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(Directory::open(&child).is_err());
    }

    #[test]
    fn supplied_alias_traversal_requires_trusted_parents() {
        let fixture = tempfile::tempdir().unwrap();
        let trusted = fixture.path().join("trusted");
        let node = trusted.join("node");
        mk_private_dir(&trusted);
        mk_private_dir(&node);
        let parent = fixture.path().join("alias-parent");
        mk_private_dir(&parent);
        symlink(&trusted, parent.join("alias")).unwrap();
        let supplied = parent.join("alias/node");
        let pinned = Directory::open(&supplied).unwrap();
        pinned.check_path(&supplied).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(Directory::open(&supplied).is_err());
        assert!(pinned.check_path(&supplied).is_err());
        // Also inspect traversal inside a symlink target, before a second link
        // or '..' could erase the untrusted directory from the canonical path.
        symlink(parent.join("alias"), trusted.join("nested")).unwrap();
        assert!(Directory::open(&trusted.join("nested/node")).is_err());
        symlink(parent.join("../trusted"), trusted.join("dotdot")).unwrap();
        assert!(Directory::open(&trusted.join("dotdot/node")).is_err());
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o1777)).unwrap();
        assert!(Directory::open(&supplied).is_ok());
    }

    #[test]
    fn reads_reject_symlinks_hardlinks_unprotected_and_oversized_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("node");
        mk_private_dir(&root);
        let file = root.join("private");
        private_file(&file, b"test-only");
        let directory = Directory::open(&root).unwrap();
        assert_eq!(
            directory.read(file.file_name().unwrap(), 32).unwrap(),
            b"test-only"
        );
        assert!(directory.read(file.file_name().unwrap(), 2).is_err());
        for name in ["", ".", "..", "a/b"] {
            assert!(directory.read(std::ffi::OsStr::new(name), 32).is_err());
        }
        let alias = root.join("alias");
        symlink(&file, &alias).unwrap();
        assert!(directory.read(alias.file_name().unwrap(), 32).is_err());
        std::fs::remove_file(&alias).unwrap();
        std::fs::hard_link(&file, &alias).unwrap();
        assert!(directory.read(file.file_name().unwrap(), 32).is_err());
        std::fs::remove_file(&alias).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(directory.read(file.file_name().unwrap(), 32).is_err());
    }

    #[test]
    fn check_path_refuses_a_swapped_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("node");
        mk_private_dir(&root);
        let directory = Directory::open(&root).unwrap();
        directory.check_path(&root).unwrap();
        std::fs::rename(&root, dir.path().join("moved")).unwrap();
        mk_private_dir(&root);
        assert!(directory.check_path(&root).is_err());
    }

    #[test]
    fn config_load_in_reads_only_private_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("node");
        mk_private_dir(&root);
        let path = root.join("config.toml");
        let config = super::super::config::NodeConfig::new(
            "wss://example.test".into(),
            "node".into(),
            "keychain".into(),
        );
        config.save(&path).unwrap();
        let directory = Directory::open(&root).unwrap();
        let loaded = super::super::config::NodeConfig::load_in(&directory, &path).unwrap();
        assert_eq!(loaded.storage_backend, "keychain");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(super::super::config::NodeConfig::load_in(&directory, &path).is_err());
    }
}
