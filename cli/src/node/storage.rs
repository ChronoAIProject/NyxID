//! Private local storage primitives. File custody assumes a distinct trusted
//! supervisor OS identity; it does not isolate processes sharing that identity.
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use super::error::{Error, Result};

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
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
            // O_NOFOLLOW rejects a final-component symlink, even if its target
            // would otherwise pass custody checks. Opening has no write effect.
            let initial = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
                .open(path)?;
            validate_directory(&initial, true)?;
            // Resolve platform aliases such as macOS /var, then walk without
            // following any further symlinks. Compare identities so a path swap
            // during validation cannot silently select a different directory.
            let canonical = std::fs::canonicalize(path)?;
            let mut directory = File::open("/")?;
            validate_directory(&directory, false)?;
            for component in canonical.components() {
                if let std::path::Component::Normal(name) = component {
                    directory = open_at(&directory, name, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
                    validate_directory(&directory, false)?;
                }
            }
            let first = initial.metadata()?;
            let last = directory.metadata()?;
            if first.dev() != last.dev() || first.ino() != last.ino() {
                return Err(invalid());
            }
            validate_directory(&directory, true)?;
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

    /// Strict bounded read; never creates, repairs, or follows a symlink.
    pub(crate) fn read(&self, name: &std::ffi::OsStr, limit: u64) -> Result<Vec<u8>> {
        #[cfg(unix)]
        {
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

    /// Fixed lock inode; never unlink it on release. All reads and publication
    /// in the transaction must use this same Directory, not reopen a pathname.
    pub(crate) fn writer_lock(&self) -> Result<File> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let file = open_at(
                &self.0,
                std::ffi::OsStr::new(".storage-writer.lock"),
                libc::O_RDWR | libc::O_CREAT | libc::O_NONBLOCK,
                0o600,
            )?;
            validate(&file)?;
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(file)
        }
        #[cfg(not(unix))]
        {
            Err(invalid())
        }
    }

    /// Before installation failures leave the destination unchanged. Once
    /// installed, any cleanup/sync failure is StorageDurabilityUncertain: the
    /// caller must retain referenced credentials and reconcile, never rollback.
    pub(crate) fn publish(
        &self,
        name: &std::ffi::OsStr,
        bytes: &[u8],
        replace: bool,
    ) -> Result<()> {
        self.publish_with_sync(name, bytes, replace, |directory| directory.sync_all())
    }

    fn publish_with_sync(
        &self,
        name: &std::ffi::OsStr,
        bytes: &[u8],
        replace: bool,
        sync: impl FnOnce(&File) -> std::io::Result<()>,
    ) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let destination = filename(name)?;
            let temporary_name = format!(".nyxid-storage-{}", uuid::Uuid::new_v4());
            let mut file = open_at(
                &self.0,
                std::ffi::OsStr::new(&temporary_name),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
                0o600,
            )?;
            // Only own cleanup after successful exclusive creation. A name
            // collision must never unlink another attempt's file.
            let temporary = Temporary {
                directory: &self.0,
                name: filename(std::ffi::OsStr::new(&temporary_name))?,
            };
            validate(&file)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            let fd = self.0.as_raw_fd();
            if replace {
                // SAFETY: owned directory fd and NUL-terminated single names.
                if unsafe { libc::renameat(fd, temporary.name.as_ptr(), fd, destination.as_ptr()) }
                    != 0
                {
                    return Err(std::io::Error::last_os_error().into());
                }
            } else {
                exclusive_rename(&self.0, &temporary.name, &destination)?;
            }
            // Both installation modes atomically consume the temporary. There
            // is no double-link state that could strand a strict reader after
            // process loss. From here a sync error means installed/uncertain.
            sync(&self.0).map_err(Error::StorageDurabilityUncertain)?;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = (name, bytes, replace, sync);
            Err(invalid())
        }
    }
}

/// No link/unlink emulation: interruption between those operations strands
/// the destination with two links, violating strict reopen custody. Unsupported
/// kernels/filesystems fail before installation rather than weakening custody.
#[cfg(unix)]
fn exclusive_rename(
    directory: &File,
    temporary: &std::ffi::CStr,
    destination: &std::ffi::CStr,
) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let fd = directory.as_raw_fd();
    #[cfg(target_os = "macos")]
    let status = unsafe {
        libc::renameatx_np(
            fd,
            temporary.as_ptr(),
            fd,
            destination.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(target_os = "linux")]
    let status = unsafe {
        libc::renameat2(
            fd,
            temporary.as_ptr(),
            fd,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (fd, temporary, destination);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Node storage requires atomic exclusive rename support",
        ));
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
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
    let sticky_ancestor = !final_component && u64::from(meta.mode()) & u64::from(libc::S_ISVTX) != 0;
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

#[cfg(unix)]
struct Temporary<'a> {
    directory: &'a File,
    name: std::ffi::CString,
}

#[cfg(unix)]
impl Temporary<'_> {
    fn remove(&mut self) -> std::io::Result<()> {
        use std::os::fd::AsRawFd;
        if unsafe { libc::unlinkat(self.directory.as_raw_fd(), self.name.as_ptr(), 0) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for Temporary<'_> {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

pub(crate) fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let directory = Directory::open(path.parent().unwrap_or(Path::new(".")))?;
    directory.read(path.file_name().ok_or_else(invalid)?, limit)
}

/// Each clone retains its own observed snapshot, so an old clone cannot adopt
/// a newer writer's expectation. No serialized fingerprint or secret is logged.
#[derive(Default)]
pub struct Publication(Mutex<Option<[u8; 32]>>);

impl Clone for Publication {
    fn clone(&self) -> Self {
        Self(Mutex::new(
            *self.0.lock().expect("publication lock poisoned"),
        ))
    }
}

impl std::fmt::Debug for Publication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Publication { snapshot: [REDACTED] }")
    }
}

impl Publication {
    pub(crate) fn observed(bytes: &[u8]) -> Self {
        Self(Mutex::new(Some(Sha256::digest(bytes).into())))
    }

    pub(crate) fn save(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.save_with_sync(path, bytes, File::sync_all)
    }

    fn save_with_sync(
        &self,
        path: &Path,
        bytes: &[u8],
        sync: impl FnOnce(&File) -> std::io::Result<()>,
    ) -> Result<()> {
        // Enforce the reader's bound before even creating a writer-lock file.
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(Error::Config(
                "Node configuration exceeds its size limit".into(),
            ));
        }
        let directory = Directory::open(path.parent().unwrap_or(Path::new(".")))?;
        let name = path.file_name().ok_or_else(invalid)?;
        let _writer = directory.writer_lock()?;
        let mut expected = self.0.lock().expect("publication lock poisoned");
        let current = match directory.read(name, MAX_CONFIG_BYTES) {
            Ok(bytes) => Some(<[u8; 32]>::from(Sha256::digest(&bytes))),
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if current != *expected {
            return Err(Error::Config(
                "Node configuration changed; reload before retrying this operation".into(),
            ));
        }
        let result = directory.publish_with_sync(name, bytes, current.is_some(), sync);
        if result.is_ok() || matches!(result, Err(Error::StorageDurabilityUncertain(_))) {
            // Installation happened. Preserve the new observation even when its
            // crash durability is unknown. Never treat this as an absent commit.
            *expected = Some(Sha256::digest(bytes).into());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for_test_file(path: &Path) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !path.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "child readiness timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn start_child(directory: &Path, role: &str) -> std::process::Child {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "node::storage::tests::publication_process_child",
                "--ignored",
                "--test-threads=1",
                "--nocapture",
            ])
            .env("NYXID_STORAGE_TEST_DIRECTORY", directory)
            .env("NYXID_STORAGE_TEST_ROLE", role)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    }

    fn finish_child(mut child: std::process::Child) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if child.try_wait().unwrap().is_some() {
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "child failed: {} {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                );
                return;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("storage child exceeded its bounded runtime");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore = "invoked by cross-process publication parents with disposable profile"]
    fn publication_process_child() {
        let directory = std::path::PathBuf::from(
            std::env::var_os("NYXID_STORAGE_TEST_DIRECTORY").expect("parent directory"),
        );
        let role = std::env::var("NYXID_STORAGE_TEST_ROLE").expect("parent role");
        let path = directory.join("config.toml");
        if role == "exit-after-install"
            || role == "exit-after-create"
            || role == "exit-after-key-create"
        {
            let directory = Directory::open(&directory).unwrap();
            let _writer = directory.writer_lock().unwrap();
            let keyfile = role == "exit-after-key-create";
            let name = if keyfile { ".keyfile" } else { "config.toml" };
            let bytes: &[u8] = if keyfile {
                &[7u8; 32]
            } else {
                b"installed-before-exit"
            };
            let _ = directory.publish_with_sync(
                std::ffi::OsStr::new(name),
                bytes,
                role == "exit-after-install",
                |_| std::process::exit(0),
            );
            panic!("post-install exit callback was not reached");
        }
        assert!(role == "first" || role == "second");
        let publication = Publication::observed(&read(&path, MAX_CONFIG_BYTES).unwrap());
        std::fs::write(directory.join(format!("ready-{role}")), []).unwrap();
        wait_for_test_file(&directory.join("go"));
        let result = publication.save(&path, role.as_bytes());
        let outcome = match result {
            Ok(()) => "committed",
            Err(Error::Config(message)) if message.contains("changed; reload") => "refused",
            Err(_) => panic!("unexpected publication failure"),
        };
        std::fs::write(directory.join(format!("result-{role}")), outcome).unwrap();
    }

    #[test]
    fn concurrent_processes_from_same_snapshot_have_exactly_one_winning_publication() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        Publication::default().save(&path, b"initial").unwrap();
        let first = start_child(directory.path(), "first");
        let second = start_child(directory.path(), "second");
        wait_for_test_file(&directory.path().join("ready-first"));
        wait_for_test_file(&directory.path().join("ready-second"));
        std::fs::write(directory.path().join("go"), []).unwrap();
        finish_child(first);
        finish_child(second);
        let first = std::fs::read_to_string(directory.path().join("result-first")).unwrap();
        let second = std::fs::read_to_string(directory.path().join("result-second")).unwrap();
        assert!(
            (first == "committed" && second == "refused")
                || (first == "refused" && second == "committed")
        );
        let expected = if first == "committed" {
            "first"
        } else {
            "second"
        };
        assert_eq!(read(&path, MAX_CONFIG_BYTES).unwrap(), expected.as_bytes());
    }

    #[test]
    fn process_exit_after_install_reopens_and_releases_writer_lock() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let previous = Publication::default();
        previous.save(&path, b"initial").unwrap();
        finish_child(start_child(directory.path(), "exit-after-install"));
        let installed = read(&path, MAX_CONFIG_BYTES).unwrap();
        assert_eq!(installed, b"installed-before-exit");
        assert!(previous.save(&path, b"stale").is_err());
        // The child exited without Rust destructors. The OS released its flock
        // and a fresh observation can publish after restart. This exercises
        // process loss, not hardware power-loss durability.
        Publication::observed(&installed)
            .save(&path, b"reopened")
            .unwrap();
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn process_exit_after_exclusive_creation_keeps_single_link_config_and_key_reopenable() {
        use std::os::unix::fs::MetadataExt;
        for role in ["exit-after-create", "exit-after-key-create"] {
            let directory = tempfile::tempdir().unwrap();
            finish_child(start_child(directory.path(), role));
            let name = if role == "exit-after-key-create" {
                ".keyfile"
            } else {
                "config.toml"
            };
            let path = directory.path().join(name);
            assert_eq!(std::fs::metadata(&path).unwrap().nlink(), 1);
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
            if name == ".keyfile" {
                let existing =
                    crate::node::encryption::LocalEncryption::open_existing(directory.path())
                        .unwrap();
                let ciphertext = existing.encrypt("disposable-reopen-value").unwrap();
                let initialized =
                    crate::node::encryption::LocalEncryption::initialize(directory.path()).unwrap();
                assert_eq!(
                    initialized.decrypt(&ciphertext).unwrap(),
                    "disposable-reopen-value"
                );
                assert_eq!(std::fs::read(&path).unwrap(), [7u8; 32]);
            } else {
                let installed = read(&path, MAX_CONFIG_BYTES).unwrap();
                assert_eq!(installed, b"installed-before-exit");
                Publication::observed(&installed)
                    .save(&path, b"reopened")
                    .unwrap();
            }
        }
    }

    #[test]
    fn maximum_is_enforced_before_any_side_effect_and_preserves_valid_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let publication = Publication::default();
        let too_large = vec![b'x'; MAX_CONFIG_BYTES as usize + 1];
        assert!(publication.save(&path, &too_large).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let maximum = vec![b'v'; MAX_CONFIG_BYTES as usize];
        publication.save(&path, &maximum).unwrap();
        assert!(read(&path, MAX_CONFIG_BYTES).unwrap() == maximum);
        assert!(publication.save(&path, &too_large).is_err());
        assert!(read(&path, MAX_CONFIG_BYTES).unwrap() == maximum);
    }

    #[test]
    fn installed_but_sync_uncertain_retains_destination_and_updates_observation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let publication = Publication::default();
        publication.save(&path, b"previous-reference").unwrap();
        let stale = publication.clone();
        let result = publication.save_with_sync(&path, b"installed-reference", |_| {
            Err(std::io::Error::from_raw_os_error(libc::EIO))
        });
        assert!(matches!(result, Err(Error::StorageDurabilityUncertain(_))));
        assert_eq!(
            read(&path, MAX_CONFIG_BYTES).unwrap(),
            b"installed-reference"
        );
        assert!(stale.save(&path, b"stale-reference").is_err());
        // The installed snapshot is retained. This retry can sync/publish it;
        // a transaction caller must reconcile refs rather than roll them back.
        publication.save(&path, b"installed-reference").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn mutable_nonsticky_ancestor_and_final_directory_symlink_are_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("mutable");
        let child = parent.join("private");
        std::fs::create_dir(&parent).unwrap();
        std::fs::create_dir(&child).unwrap();
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(Directory::open(&child).is_err());
        assert!(
            Publication::default()
                .save(&child.join("config.toml"), b"value")
                .is_err()
        );
        assert_eq!(std::fs::read_dir(&child).unwrap().count(), 0);
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(Directory::open(&child).is_ok());
        let alias = dir.path().join("alias");
        symlink(&child, &alias).unwrap();
        assert!(Directory::open(&alias).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn read_lock_publication_and_temporary_cleanup_remain_on_pinned_directory() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("profile");
        let moved = dir.path().join("moved");
        std::fs::create_dir(&original).unwrap();
        let directory = Directory::open(&original).unwrap();
        directory
            .publish(std::ffi::OsStr::new("config.toml"), b"original", false)
            .unwrap();
        std::fs::rename(&original, &moved).unwrap();
        std::fs::create_dir(&original).unwrap();
        let replacement = Directory::open(&original).unwrap();
        replacement
            .publish(std::ffi::OsStr::new("config.toml"), b"replacement", false)
            .unwrap();
        let _writer = directory.writer_lock().unwrap();
        assert_eq!(
            directory
                .read(std::ffi::OsStr::new("config.toml"), 32)
                .unwrap(),
            b"original"
        );
        directory
            .publish(std::ffi::OsStr::new("config.toml"), b"pinned", true)
            .unwrap();
        assert!(
            directory
                .publish(std::ffi::OsStr::new("config.toml"), b"refused", false)
                .is_err()
        );
        assert_eq!(std::fs::read(moved.join("config.toml")).unwrap(), b"pinned");
        assert_eq!(
            std::fs::read(original.join("config.toml")).unwrap(),
            b"replacement"
        );
        assert!(moved.join(".storage-writer.lock").exists());
        assert!(!original.join(".storage-writer.lock").exists());
        assert_eq!(std::fs::read_dir(&moved).unwrap().count(), 2);
        assert_eq!(std::fs::read_dir(&original).unwrap().count(), 1);
    }

    #[test]
    fn stale_snapshot_cannot_overwrite_another_writer_or_deleted_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let first = Publication::default();
        first.save(&path, b"initial").unwrap();
        let stale = first.clone();
        first.save(&path, b"newer").unwrap();
        assert!(stale.save(&path, b"stale").is_err());
        assert_eq!(read(&path, 32).unwrap(), b"newer");
        std::fs::remove_file(&path).unwrap();
        assert!(first.save(&path, b"resurrected").is_err());
        assert!(!path.exists());
    }

    #[test]
    fn old_fixed_temporary_is_never_opened_and_failed_publish_cleans_own_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join(".config.toml.tmp");
        std::fs::write(&old, b"unrelated").unwrap();
        let path = dir.path().join("config.toml");
        Publication::default().save(&path, b"private").unwrap();
        assert_eq!(std::fs::read(&old).unwrap(), b"unrelated");
        assert!(
            Directory::open(dir.path())
                .unwrap()
                .publish(path.file_name().unwrap(), b"must not overwrite", false)
                .is_err()
        );
        assert_eq!(read(&path, 32).unwrap(), b"private");
        assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".nyxid-storage-")
        }));
    }

    #[cfg(unix)]
    #[test]
    fn reads_and_locks_reject_symlinks_hardlinks_and_unprotected_files() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("private");
        Directory::open(dir.path())
            .unwrap()
            .publish(file.file_name().unwrap(), b"test-only", false)
            .unwrap();
        let alias = dir.path().join("alias");
        symlink(&file, &alias).unwrap();
        assert!(read(&alias, 32).is_err());
        std::fs::remove_file(&alias).unwrap();
        std::fs::hard_link(&file, &alias).unwrap();
        assert!(read(&file, 32).is_err());
        std::fs::remove_file(&alias).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read(&file, 32).is_err());
        assert!(read(&file, 2).is_err());
        symlink(&file, dir.path().join(".storage-writer.lock")).unwrap();
        assert!(Directory::open(dir.path()).unwrap().writer_lock().is_err());
    }
}
