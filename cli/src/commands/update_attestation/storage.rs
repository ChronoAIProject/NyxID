//! Pinned persistent TUF I/O. Never re-resolve the caller's pathname after the
//! directory walk. Scratch metadata is disposable; only authenticated state is
//! published through this handle.
use anyhow::{Result, ensure};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use crate::trusted_directory;

pub(super) struct Directory {
    #[cfg(unix)]
    directory: fs::File,
    #[cfg(not(target_os = "linux"))]
    path: PathBuf,
}

impl Directory {
    pub(super) fn open(path: &Path) -> Result<Self> {
        ensure!(
            path.is_absolute(),
            "TUF state requires an absolute persistent path"
        );
        #[cfg(unix)]
        {
            use anyhow::Context;
            use std::os::unix::fs::MetadataExt;
            let directory = trusted_directory::open(path, true)
                .context("TUF state requires trusted directory ancestry")?;
            let metadata = directory.metadata()?;
            ensure!(
                metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
                "TUF state directory must be owned by this user and private (0700)"
            );
            // The resolved spelling is only for libraries requiring a pathname
            // on Unix platforms without Linux's descriptor filesystem. All
            // authoritative state operations below use directory-relative I/O.
            let path = fs::canonicalize(path)?;
            let selected = trusted_directory::open(&path, false)?.metadata()?;
            ensure!(
                metadata.dev() == selected.dev() && metadata.ino() == selected.ino(),
                "TUF state directory changed while opening"
            );
            Ok(Self {
                directory,
                #[cfg(not(target_os = "linux"))]
                path,
            })
        }
        #[cfg(not(unix))]
        {
            fs::create_dir_all(path)?;
            ensure!(
                fs::symlink_metadata(path)?.is_dir(),
                "TUF state directory is not a real directory"
            );
            Ok(Self { path: path.into() })
        }
    }

    pub(super) fn open_private(&self, name: &str, create: bool) -> Result<fs::File> {
        self.open_file(name, create, false, true)
    }

    pub(super) fn open_metadata(&self, name: &str) -> Result<fs::File> {
        // Flat legacy tough metadata is public signed material; previous tough
        // versions did not promise private file modes inside the private tree.
        self.open_file(name, false, false, false)
    }

    fn open_file(
        &self,
        name: &str,
        create: bool,
        exclusive: bool,
        private: bool,
    ) -> Result<fs::File> {
        ensure!(
            !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\']),
            "Invalid TUF state filename"
        );
        #[cfg(unix)]
        let file = {
            use std::os::fd::{AsRawFd, FromRawFd};
            let name = std::ffi::CString::new(name)?;
            let flags = if create {
                libc::O_RDWR | libc::O_CREAT
            } else {
                libc::O_RDONLY
            };
            // SAFETY: valid borrowed directory descriptor and component string.
            let fd = unsafe {
                libc::openat(
                    self.directory.as_raw_fd(),
                    name.as_ptr(),
                    flags
                        | if exclusive { libc::O_EXCL } else { 0 }
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC
                        | libc::O_NONBLOCK,
                    0o600 as libc::c_uint,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            // SAFETY: openat returned a newly owned descriptor.
            unsafe { fs::File::from_raw_fd(fd) }
        };
        #[cfg(not(unix))]
        let file = fs::OpenOptions::new()
            .read(true)
            .write(create)
            .create(create)
            .create_new(exclusive)
            .truncate(false)
            .open(self.path.join(name))?;
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "TUF state file is not private");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                !private
                    || (metadata.uid() == unsafe { libc::geteuid() }
                        && metadata.mode() & 0o077 == 0
                        && metadata.nlink() == 1),
                "TUF state file is not private"
            );
        }
        #[cfg(not(unix))]
        let _ = private;
        Ok(file)
    }

    pub(super) fn exists(&self, name: &str) -> Result<bool> {
        match self.open_metadata(name) {
            Ok(_) => Ok(true),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn publish(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let temporary = format!(".state-{}.tmp", uuid::Uuid::new_v4());
        let mut file = self.open_file(&temporary, true, true, true)?;
        let result = (|| -> Result<()> {
            file.write_all(bytes)?;
            file.sync_all()?;
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                let source = std::ffi::CString::new(temporary.as_str())?;
                let target = std::ffi::CString::new(name)?;
                // SAFETY: source and target are relative to the pinned inode.
                if unsafe {
                    libc::renameat(
                        self.directory.as_raw_fd(),
                        source.as_ptr(),
                        self.directory.as_raw_fd(),
                        target.as_ptr(),
                    )
                } < 0
                {
                    return Err(std::io::Error::last_os_error().into());
                }
                self.directory.sync_all()?;
            }
            #[cfg(not(unix))]
            fs::rename(self.path.join(&temporary), self.path.join(name))?;
            Ok(())
        })();
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let temporary = std::ffi::CString::new(temporary)?;
            // SAFETY: cleanup is relative to the same pinned directory. After
            // publication ENOENT is expected; never unlink through a pathname.
            unsafe {
                libc::unlinkat(self.directory.as_raw_fd(), temporary.as_ptr(), 0);
            }
        }
        #[cfg(not(unix))]
        let _ = fs::remove_file(self.path.join(&temporary));
        result
    }

    pub(super) fn scratch(&self) -> Result<tempfile::TempDir> {
        #[cfg(target_os = "linux")]
        let path = {
            use std::os::fd::AsRawFd;
            PathBuf::from(format!("/proc/self/fd/{}", self.directory.as_raw_fd()))
        };
        #[cfg(not(target_os = "linux"))]
        let path = self.path.clone();
        Ok(tempfile::Builder::new()
            .prefix(".refresh-")
            .tempdir_in(path)?)
    }
}
