//! Unix directory traversal shared by node custody and durable release trust.
//! Validate the supplied traversal, including symlink targets, before any
//! canonicalization can erase an alias parent. Root and the current UID are
//! trusted; their sticky directories may admit other writers. Changes by those
//! trusted identities are outside this boundary.
use std::{
    collections::VecDeque,
    ffi::{CString, OsStr, OsString},
    fs::File,
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path},
};

fn untrusted() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Untrusted directory custody",
    )
}

fn validate(directory: &File) -> io::Result<()> {
    let metadata = directory.metadata()?;
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_dir()
        || (metadata.uid() != 0 && metadata.uid() != uid)
        || (metadata.mode() & 0o022 != 0
            && u64::from(metadata.mode()) & u64::from(libc::S_ISVTX) == 0)
    {
        return Err(untrusted());
    }
    Ok(())
}

fn components(path: &Path) -> VecDeque<OsString> {
    path.components()
        .map(|component| match component {
            Component::RootDir => OsString::from("/"),
            _ => component.as_os_str().to_owned(),
        })
        .collect()
}

/// Open/create directories relative to checked descriptors. Intermediate
/// aliases are expanded explicitly; a selected directory cannot be an alias.
pub(crate) fn open(path: &Path, create: bool) -> io::Result<File> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Absolute path required",
        ));
    }
    let mut directory = File::open("/")?;
    validate(&directory)?;
    let mut pending = components(path);
    let mut links = 0;
    while let Some(component) = pending.pop_front() {
        if component == OsStr::new("/") {
            directory = File::open("/")?;
            validate(&directory)?;
            continue;
        }
        let name = CString::new(component.as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let open_child = || {
            // SAFETY: valid borrowed descriptor and NUL-terminated component.
            let fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                Err(io::Error::last_os_error())
            } else {
                // SAFETY: openat returned a newly owned descriptor.
                Ok(unsafe { File::from_raw_fd(fd) })
            }
        };
        let child = match open_child() {
            Ok(child) => child,
            Err(error) => {
                // Inspect the component itself, never its resolved destination.
                let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
                // SAFETY: valid descriptor/name and writable stat storage.
                let result = unsafe {
                    libc::fstatat(
                        directory.as_raw_fd(),
                        name.as_ptr(),
                        metadata.as_mut_ptr(),
                        libc::AT_SYMLINK_NOFOLLOW,
                    )
                };
                if result == 0 {
                    // SAFETY: successful fstatat initialized the stat.
                    let metadata = unsafe { metadata.assume_init() };
                    if metadata.st_mode & libc::S_IFMT != libc::S_IFLNK {
                        return Err(error);
                    }
                    links += 1;
                    if links > 40 || pending.is_empty() {
                        return Err(io::Error::from_raw_os_error(libc::ELOOP));
                    }
                    if metadata.st_uid != 0 && metadata.st_uid != unsafe { libc::geteuid() } {
                        return Err(untrusted());
                    }
                    let mut target = vec![0u8; 64 * 1024];
                    // SAFETY: valid directory/name and writable bounded buffer.
                    let length = unsafe {
                        libc::readlinkat(
                            directory.as_raw_fd(),
                            name.as_ptr(),
                            target.as_mut_ptr().cast(),
                            target.len(),
                        )
                    };
                    if length < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    if length as usize == target.len() {
                        return Err(io::Error::from(io::ErrorKind::InvalidInput));
                    }
                    target.truncate(length as usize);
                    let mut expanded = components(Path::new(OsStr::from_bytes(&target)));
                    expanded.append(&mut pending);
                    pending = expanded;
                    continue;
                }
                if !create || error.kind() != io::ErrorKind::NotFound {
                    return Err(error);
                }
                // SAFETY: trusted pinned parent, new private directory component.
                if unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) } < 0
                    && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
                {
                    return Err(io::Error::last_os_error());
                }
                open_child()?
            }
        };
        validate(&child)?;
        directory = child;
    }
    Ok(directory)
}
