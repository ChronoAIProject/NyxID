//! File access is anchored to open directory descriptors. Every component is
//! opened without following links after canonicalization and exclusion checks.
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const MAX_EDIT_BYTES: u64 = 16 * 1024 * 1024;
const PAGE_BYTES: usize = 4096;

pub struct Roots {
    roots: Vec<PathBuf>,
    excluded: Vec<PathBuf>,
    workspace: Option<File>,
}

fn cstring(value: &std::ffi::OsStr) -> Result<CString> {
    CString::new(value.as_bytes()).context("invalid path")
}

fn open_at(parent: i32, name: &std::ffi::OsStr, flags: i32, mode: u32) -> Result<OwnedFd> {
    let name = cstring(name)?;
    // SAFETY: name is a live NUL-terminated string; a successful fd has one owner.
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

// /dev/fd directory traversal is not portable to macOS. fdopendir keeps
// enumeration anchored to the verified descriptor on both supported platforms.
fn directory_entries(directory: &File) -> Result<Vec<(std::ffi::OsString, libc::mode_t)>> {
    use std::os::fd::IntoRawFd;
    use std::os::unix::ffi::OsStringExt;
    let fd = open_at(
        directory.as_raw_fd(),
        std::ffi::OsStr::new("."),
        libc::O_RDONLY | libc::O_DIRECTORY,
        0,
    )?
    .into_raw_fd();
    let stream = unsafe { libc::fdopendir(fd) };
    if stream.is_null() {
        let error = std::io::Error::last_os_error();
        unsafe {
            libc::close(fd);
        }
        return Err(error.into());
    }
    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
    let stream = Directory(stream);
    let mut entries = Vec::new();
    loop {
        #[cfg(target_os = "macos")]
        let errno = unsafe { libc::__error() };
        #[cfg(target_os = "linux")]
        let errno = unsafe { libc::__errno_location() };
        unsafe {
            *errno = 0;
        }
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            if unsafe { *errno } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            break;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                name.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            // An external editor may delete an entry during enumeration.
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                continue;
            }
            return Err(error.into());
        }
        let mode = unsafe { metadata.assume_init().st_mode };
        entries.push((std::ffi::OsString::from_vec(name.to_bytes().to_vec()), mode));
        if entries.len() > 10000 {
            bail!("directory listing limit exceeded");
        }
    }
    Ok(entries)
}

impl Roots {
    /// Compute the actual filesystem intersection before opening a descriptor.
    /// Grant roots never replace the local allowlist or protected exclusions.
    pub fn restricted(&self, granted: &[String]) -> Result<Self> {
        let mut roots = Vec::new();
        for value in granted {
            let granted = std::fs::canonicalize(value)?;
            for local in &self.roots {
                let candidate = if granted.starts_with(local) {
                    &granted
                } else if local.starts_with(&granted) {
                    local
                } else {
                    continue;
                };
                if !self
                    .excluded
                    .iter()
                    .any(|denied| candidate.starts_with(denied))
                    && !roots.contains(candidate)
                {
                    roots.push(candidate.clone());
                }
            }
        }
        if roots.is_empty() {
            return Err(super::MachineError::PathOutsideRoots.into());
        }
        Self::new(&roots, &self.excluded)
    }
    pub fn paths(&self) -> &[PathBuf] {
        &self.roots
    }
    pub fn exclusions(&self) -> &[PathBuf] {
        &self.excluded
    }
    pub fn new(roots: &[PathBuf], excluded: &[PathBuf]) -> Result<Self> {
        let roots = roots
            .iter()
            .map(std::fs::canonicalize)
            .collect::<std::io::Result<Vec<_>>>()?;
        if roots.iter().any(|p| !p.is_dir()) {
            bail!("workspace root is not a directory");
        }
        let excluded = excluded
            .iter()
            .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
            .collect();
        Ok(Self {
            roots,
            excluded,
            workspace: None,
        })
    }

    /// The separated child inherits the supervisor-pinned workspace as cwd.
    /// Its private ancestors are deliberately not searchable by the child.
    #[cfg(target_os = "linux")]
    pub fn from_pinned_workspace(
        roots: &[PathBuf],
        excluded: &[PathBuf],
        workspace: &Path,
    ) -> Result<Self> {
        if roots.len() != 1 || roots[0] != workspace || !workspace.is_absolute() {
            return Err(super::MachineError::PathOutsideRoots.into());
        }
        Ok(Self {
            roots: roots.to_vec(),
            excluded: excluded.to_vec(),
            workspace: Some(File::from(open_at(
                libc::AT_FDCWD,
                std::ffi::OsStr::new("."),
                libc::O_PATH | libc::O_DIRECTORY,
                0,
            )?)),
        })
    }

    pub fn cwd(&self, path: Option<&str>) -> Result<Vec<File>> {
        let supplied = path
            .map(PathBuf::from)
            .or_else(|| self.roots.first().cloned())
            .context("no workspace root")?;
        let resolved = self.resolve(&supplied, false)?;
        // Keep every ancestor for the child to check after dropping privileges.
        // A root supervisor must not bypass a protected parent's permissions.
        // Linux O_PATH also pins a worker-owned 0700 cwd without requiring the
        // supervisor to read it. The child still checks X_OK on every ancestor.
        #[cfg(target_os = "linux")]
        const TRAVERSE: i32 = libc::O_PATH;
        #[cfg(not(target_os = "linux"))]
        const TRAVERSE: i32 = libc::O_RDONLY;
        let mut directories = vec![File::from(open_at(
            libc::AT_FDCWD,
            std::ffi::OsStr::new("/"),
            TRAVERSE | libc::O_DIRECTORY,
            0,
        )?)];
        for component in resolved.components() {
            if let Component::Normal(name) = component {
                let parent = directories
                    .last()
                    .context("working directory unavailable")?;
                directories.push(File::from(open_at(
                    parent.as_raw_fd(),
                    name,
                    TRAVERSE | libc::O_DIRECTORY,
                    0,
                )?));
            }
        }
        Ok(directories)
    }

    fn resolve(&self, path: &Path, create: bool) -> Result<PathBuf> {
        #[cfg(target_os = "linux")]
        if let Some(workspace) = &self.workspace {
            return self.resolve_in_workspace(workspace, path, create);
        }
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            self.roots.first().context("no workspace root")?.join(path)
        };
        let resolved = match std::fs::canonicalize(&path) {
            Ok(path) => path,
            Err(e) if create && e.kind() == std::io::ErrorKind::NotFound => {
                let parent = path.parent().context("invalid path")?.canonicalize()?;
                parent.join(path.file_name().context("invalid path")?)
            }
            Err(e) => return Err(e.into()),
        };
        if !self.roots.iter().any(|root| resolved.starts_with(root))
            || self
                .excluded
                .iter()
                .any(|denied| resolved.starts_with(denied))
        {
            return Err(super::MachineError::PathOutsideRoots.into());
        }
        Ok(resolved)
    }

    #[cfg(target_os = "linux")]
    fn resolve_in_workspace(&self, workspace: &File, path: &Path, create: bool) -> Result<PathBuf> {
        use std::collections::VecDeque;
        use std::os::unix::ffi::OsStringExt;

        let root = &self.roots[0];
        let relative = if path.is_absolute() {
            path.strip_prefix(root)
                .map_err(|_| super::MachineError::PathOutsideRoots)?
        } else {
            path
        };
        let directory_syntax = |path: &Path| {
            let bytes = path.as_os_str().as_bytes();
            bytes.ends_with(b"/") || bytes == b"." || bytes.ends_with(b"/.")
        };
        let parts = |path: &Path, directory: bool| -> Result<VecDeque<std::ffi::OsString>> {
            let mut parts: VecDeque<_> = path
                .components()
                .filter(|part| *part != Component::CurDir)
                .map(|part| match part {
                    Component::Normal(name) => Ok(name.to_owned()),
                    Component::ParentDir => Ok("..".into()),
                    _ => Err(super::MachineError::PathOutsideRoots.into()),
                })
                .collect::<Result<_>>()?;
            if directory {
                // Preserve canonicalize's directory requirement for `file/`
                // and `file/.`, including inside a symbolic-link target.
                parts.push_back(".".into());
            }
            Ok(parts)
        };
        let anchor = || {
            open_at(
                workspace.as_raw_fd(),
                std::ffi::OsStr::new("."),
                libc::O_PATH | libc::O_DIRECTORY,
                0,
            )
        };
        let mut pending = parts(relative, directory_syntax(path))?;
        let mut directories = vec![anchor()?];
        let mut resolved = PathBuf::new();
        let mut links = 0;
        let mut final_link = false;
        while let Some(name) = pending.pop_front() {
            if name == "." {
                continue;
            }
            if name == ".." {
                if !resolved.pop() {
                    return Err(super::MachineError::PathOutsideRoots.into());
                }
                directories.pop();
                continue;
            }
            let parent = directories.last().context("workspace unavailable")?;
            let entry = match open_at(parent.as_raw_fd(), &name, libc::O_PATH, 0) {
                Ok(entry) => entry,
                Err(error)
                    if create
                        && !final_link
                        && pending.is_empty()
                        && error
                            .downcast_ref::<std::io::Error>()
                            .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
                {
                    resolved.push(name);
                    break;
                }
                Err(error) => return Err(error),
            };
            let entry = File::from(entry);
            let metadata = entry.metadata()?;
            if metadata.file_type().is_symlink() {
                final_link |= pending.is_empty();
                links += 1;
                if links > 40 {
                    bail!("too many symbolic links");
                }
                // Read the pinned link itself; a concurrent replacement cannot
                // redirect this lookup. Subsequent opens still use NOFOLLOW.
                let mut target = vec![0u8; 4096];
                let length = unsafe {
                    libc::readlinkat(
                        entry.as_raw_fd(),
                        c"".as_ptr(),
                        target.as_mut_ptr().cast(),
                        target.len(),
                    )
                };
                if length < 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                if length as usize == target.len() {
                    bail!("symbolic link target too long");
                }
                target.truncate(length as usize);
                let target = PathBuf::from(std::ffi::OsString::from_vec(target));
                let directory = directory_syntax(&target);
                let target = if target.is_absolute() {
                    directories = vec![anchor()?];
                    resolved.clear();
                    target
                        .strip_prefix(root)
                        .map_err(|_| super::MachineError::PathOutsideRoots)?
                } else {
                    &target
                };
                let mut target = parts(target, directory)?;
                target.append(&mut pending);
                pending = target;
                continue;
            }
            resolved.push(name);
            if !pending.is_empty() {
                if !metadata.is_dir() {
                    bail!("not a directory");
                }
                directories.push(entry.into());
            }
        }
        let resolved = root.join(resolved);
        if self
            .excluded
            .iter()
            .any(|denied| resolved.starts_with(denied))
        {
            return Err(super::MachineError::PathOutsideRoots.into());
        }
        Ok(resolved)
    }

    fn parent(&self, resolved: &Path) -> Result<(OwnedFd, CString)> {
        // Traversing an ancestor does not require listing it. In a Landlock
        // context, granting read-directory on / would expose the entire tree.
        #[cfg(target_os = "linux")]
        const TRAVERSE: i32 = libc::O_PATH;
        #[cfg(not(target_os = "linux"))]
        const TRAVERSE: i32 = libc::O_RDONLY;
        let readable = |directory: &OwnedFd| {
            open_at(
                directory.as_raw_fd(),
                std::ffi::OsStr::new("."),
                libc::O_RDONLY | libc::O_DIRECTORY,
                0,
            )
        };
        let root = self
            .roots
            .iter()
            .filter(|root| resolved.starts_with(root))
            .max_by_key(|root| root.components().count())
            .context(super::MachineError::PathOutsideRoots)?;
        let mut directory = if let Some(workspace) = &self.workspace {
            open_at(
                workspace.as_raw_fd(),
                std::ffi::OsStr::new("."),
                TRAVERSE | libc::O_DIRECTORY,
                0,
            )?
        } else {
            let mut directory = open_at(
                libc::AT_FDCWD,
                std::ffi::OsStr::new("/"),
                TRAVERSE | libc::O_DIRECTORY,
                0,
            )?;
            for component in root.components() {
                if let Component::Normal(name) = component {
                    directory =
                        open_at(directory.as_raw_fd(), name, TRAVERSE | libc::O_DIRECTORY, 0)?;
                }
            }
            directory
        };
        let components: Vec<_> = resolved.strip_prefix(root)?.components().collect();
        if components.is_empty() {
            return Ok((readable(&directory)?, CString::new(".")?));
        }
        for component in &components[..components.len() - 1] {
            let Component::Normal(name) = component else {
                return Err(super::MachineError::PathOutsideRoots.into());
            };
            directory = open_at(directory.as_raw_fd(), name, TRAVERSE | libc::O_DIRECTORY, 0)?;
        }
        let Component::Normal(name) = components[components.len() - 1] else {
            return Err(super::MachineError::PathOutsideRoots.into());
        };
        Ok((readable(&directory)?, cstring(name)?))
    }

    fn open(&self, resolved: &Path, flags: i32) -> Result<File> {
        let (parent, name) = self.parent(resolved)?;
        Ok(File::from(open_at(
            parent.as_raw_fd(),
            std::ffi::OsStr::from_bytes(name.as_bytes()),
            flags,
            0,
        )?))
    }

    #[cfg(test)]
    pub fn read(&self, path: &str, offset: u64, limit: usize, encoding: &str) -> Result<Value> {
        if !matches!(encoding, "text" | "base64") {
            bail!("encoding must be text or base64");
        }
        let resolved = self.resolve(Path::new(path), false)?;
        let mut file = self.open(&resolved, libc::O_RDONLY | libc::O_NONBLOCK)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            bail!("not a regular file");
        }
        file.seek(SeekFrom::Start(offset.min(meta.len())))?;
        let mut bytes = Vec::with_capacity(limit.min(PAGE_BYTES));
        file.take(limit.min(PAGE_BYTES) as u64)
            .read_to_end(&mut bytes)?;
        let content = if encoding == "base64" {
            STANDARD.encode(&bytes)
        } else {
            std::str::from_utf8(&bytes)
                .context("binary file: request encoding base64")?
                .to_owned()
        };
        let next = offset.min(meta.len()) + bytes.len() as u64;
        Ok(
            json!({"content":content,"encoding":encoding,"offset":next,"size":meta.len(),"has_more":next<meta.len()}),
        )
    }

    pub fn stream_read(&self, path: &str, output: &mut dyn Write, limit: u64) -> Result<()> {
        let resolved = self.resolve(Path::new(path), false)?;
        let file = self.open(&resolved, libc::O_RDONLY | libc::O_NONBLOCK)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > limit {
            bail!("file transfer limit exceeded");
        }
        let copied = std::io::copy(&mut file.take(limit.saturating_add(1)), output)?;
        if copied > limit {
            bail!("file transfer limit exceeded");
        }
        Ok(())
    }

    pub fn read_context(
        &self,
        path: &str,
        offset: u64,
        limit: usize,
        padding: usize,
    ) -> Result<Value> {
        let resolved = self.resolve(Path::new(path), false)?;
        let mut file = self.open(&resolved, libc::O_RDONLY | libc::O_NONBLOCK)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            bail!("not a regular file");
        }
        let offset = offset.min(meta.len());
        let start = offset.saturating_sub(padding.min(65536) as u64);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.take((offset - start) + limit.min(PAGE_BYTES) as u64 + padding.min(65536) as u64)
            .read_to_end(&mut bytes)?;
        let skip = (offset - start) as usize;
        let count = bytes.len().saturating_sub(skip).min(limit.min(PAGE_BYTES));
        Ok(
            json!({"context":STANDARD.encode(bytes),"skip":skip,"count":count,"offset":offset+count as u64,"size":meta.len(),"has_more":offset+(count as u64)<meta.len()}),
        )
    }

    pub fn write(
        &self,
        path: &str,
        bytes: &[u8],
        mode: &str,
        expected: Option<&str>,
    ) -> Result<String> {
        self.write_stream(
            path,
            &mut std::io::Cursor::new(bytes),
            bytes.len() as u64,
            mode,
            expected,
            None,
        )
    }

    pub fn write_stream(
        &self,
        path: &str,
        source: &mut dyn Read,
        length: u64,
        mode: &str,
        expected: Option<&str>,
        content_hash: Option<&str>,
    ) -> Result<String> {
        if !matches!(mode, "create" | "overwrite" | "append") {
            bail!("invalid write mode");
        }
        let resolved = self.resolve(Path::new(path), true)?;
        let (parent, name) = self.parent(&resolved)?;
        // Serialize NyxID writers even when isolated file workers are separate
        // processes. Editors outside NyxID are checked again before rename.
        if unsafe { libc::flock(parent.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let existing = match self.open(&resolved, libc::O_RDONLY | libc::O_NONBLOCK) {
            Ok(file) => Some(file),
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                None
            }
            Err(e) => return Err(e),
        };
        if mode == "create" && existing.is_some() {
            bail!("file already exists");
        }
        if let Some(file) = &existing
            && !file.metadata()?.is_file()
        {
            bail!("not a regular file");
        }
        if let Some(expected) = expected {
            let mut file = existing.as_ref().context("sha256 mismatch")?.try_clone()?;
            if digest(&mut file)? != expected {
                bail!("sha256 mismatch");
            }
        }
        let permissions = existing
            .as_ref()
            .map(|f| f.metadata().map(|m| m.permissions().mode() & 0o777))
            .transpose()?
            .unwrap_or(0o600);
        let temporary = CString::new(format!(".nyxid-{}", uuid::Uuid::new_v4()))?;
        let mut file = File::from(open_at(
            parent.as_raw_fd(),
            std::ffi::OsStr::from_bytes(temporary.as_bytes()),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )?);
        let result = (|| -> Result<String> {
            if mode == "append"
                && let Some(mut old) = existing
            {
                old.seek(SeekFrom::Start(0))?;
                std::io::copy(&mut old, &mut file)?;
            }
            let copied = std::io::copy(&mut source.take(length.saturating_add(1)), &mut file)?;
            if copied != length {
                bail!("file transfer length mismatch");
            }
            file.set_permissions(std::fs::Permissions::from_mode(permissions))?;
            file.sync_all()?;
            file.seek(SeekFrom::Start(0))?;
            let written_hash = digest(&mut file)?;
            if content_hash.is_some_and(|hash| hash != written_hash) {
                bail!("file transfer checksum mismatch");
            }
            if let Some(expected) = expected {
                let mut current = File::from(open_at(
                    parent.as_raw_fd(),
                    std::ffi::OsStr::from_bytes(name.as_bytes()),
                    libc::O_RDONLY | libc::O_NONBLOCK,
                    0,
                )?);
                if !current.metadata()?.is_file() || digest(&mut current)? != expected {
                    bail!("sha256 mismatch");
                }
            }
            // A create must not replace a concurrent creator. linkat is atomic
            // and fails with EEXIST; overwrite/append use atomic renameat.
            let status = unsafe {
                if mode == "create" {
                    libc::linkat(
                        parent.as_raw_fd(),
                        temporary.as_ptr(),
                        parent.as_raw_fd(),
                        name.as_ptr(),
                        0,
                    )
                } else {
                    libc::renameat(
                        parent.as_raw_fd(),
                        temporary.as_ptr(),
                        parent.as_raw_fd(),
                        name.as_ptr(),
                    )
                }
            };
            if status != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(written_hash)
        })();
        // SAFETY: both strings and the parent descriptor remain alive.
        unsafe {
            libc::unlinkat(parent.as_raw_fd(), temporary.as_ptr(), 0);
        }
        result
    }

    pub fn edit(
        &self,
        path: &str,
        old: &str,
        new: &str,
        all: bool,
        expected: Option<&str>,
    ) -> Result<String> {
        if old.is_empty() {
            bail!("old_string must not be empty");
        }
        let resolved = self.resolve(Path::new(path), false)?;
        let file = self.open(&resolved, libc::O_RDONLY | libc::O_NONBLOCK)?;
        if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_EDIT_BYTES {
            bail!("edit size limit exceeded");
        }
        let mut bytes = Vec::new();
        file.take(MAX_EDIT_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_EDIT_BYTES {
            bail!("edit size limit exceeded");
        }
        let hash = hex::encode(Sha256::digest(&bytes));
        if expected.is_some_and(|expected| expected != hash) {
            bail!("sha256 mismatch");
        }
        let text = std::str::from_utf8(&bytes).context("edit requires a UTF-8 file")?;
        let count = text.matches(old).count();
        if count == 0 || (!all && count != 1) {
            bail!("old_string has no unique match; use replace_all for multiple matches");
        }
        let result = text.replace(old, new);
        if result.len() as u64 > MAX_EDIT_BYTES {
            bail!("edit size limit exceeded");
        }
        self.write(path, result.as_bytes(), "overwrite", Some(&hash))
    }

    pub fn list(
        &self,
        path: &str,
        depth: usize,
        offset: usize,
        glob: Option<&str>,
    ) -> Result<Value> {
        if depth > 8 || offset > 10000 || glob.is_some_and(|g| g.len() > 256) {
            bail!("directory listing limit exceeded");
        }
        let matcher = glob
            .map(|pattern| {
                globset::GlobBuilder::new(pattern)
                    .literal_separator(true)
                    .build()
                    .map(|g| g.compile_matcher())
            })
            .transpose()?;
        let resolved = self.resolve(Path::new(path), false)?;
        let mut pending = vec![(resolved.clone(), 0)];
        let mut entries = Vec::new();
        let mut seen = 0usize;
        let mut visited = 0usize;
        let mut output_bytes = 0usize;
        let mut more = false;
        while let Some((path, level)) = pending.pop() {
            let directory = self.open(&path, libc::O_RDONLY | libc::O_DIRECTORY)?;
            let mut children = directory_entries(&directory)?;
            children.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, mode) in children {
                visited += 1;
                if visited > 10000 {
                    bail!("directory listing limit exceeded; narrow the path");
                }
                let child = path.join(name);
                if self.excluded.iter().any(|denied| child.starts_with(denied)) {
                    continue;
                }
                let is_dir = mode & libc::S_IFMT == libc::S_IFDIR;
                let is_symlink = mode & libc::S_IFMT == libc::S_IFLNK;
                if is_dir && level < depth {
                    pending.push((child.clone(), level + 1));
                }
                if matcher
                    .as_ref()
                    .is_some_and(|m| !m.is_match(child.strip_prefix(&resolved).unwrap_or(&child)))
                {
                    continue;
                }
                if seen >= offset {
                    let value = json!({"name":child.strip_prefix(&self.roots[0]).unwrap_or(&child).to_string_lossy(),"kind":if is_dir{"directory"}else if is_symlink{"symlink"}else{"file"}});
                    let bytes = value.to_string().len();
                    if entries.len() >= 50 || (!entries.is_empty() && output_bytes + bytes > 6000) {
                        more = true;
                        break;
                    }
                    output_bytes += bytes;
                    entries.push(value);
                }
                seen += 1;
                if seen > 10000 {
                    bail!("listing offset limit exceeded");
                }
            }
            if more {
                break;
            }
        }
        Ok(json!({"entries":entries,"offset":seen,"has_more":more}))
    }
}

fn digest(file: &mut File) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn pinned_resolution_preserves_links_directory_syntax_and_exclusions() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("nested")).unwrap();
        std::fs::create_dir(root.join("private")).unwrap();
        std::fs::write(root.join("marker"), "allowed").unwrap();
        std::fs::write(root.join("private/key"), "denied").unwrap();
        for (target, link) in [
            ("../marker", "nested/link"),
            ("marker/", "not-directory"),
            ("loop", "loop"),
            ("private/key", "secret-link"),
            ("missing", "dangling"),
        ] {
            std::os::unix::fs::symlink(target, root.join(link)).unwrap();
        }
        std::os::unix::fs::symlink(root.join("marker"), root.join("absolute-link")).unwrap();
        std::os::unix::fs::symlink(root.join("marker/"), root.join("absolute-not-directory"))
            .unwrap();
        let roots = Roots {
            workspace: Some(File::open(&root).unwrap()),
            excluded: vec![root.join("private")],
            roots: vec![root.clone()],
        };
        for path in ["nested/link", "absolute-link", "nested/../marker"] {
            assert_eq!(
                roots.read(path, 0, 100, "text").unwrap()["content"],
                "allowed"
            );
        }
        for path in [
            "marker/",
            "marker/.",
            "not-directory",
            "loop",
            "secret-link",
            "../marker",
            "dangling",
            "absolute-not-directory",
        ] {
            assert!(roots.read(path, 0, 100, "text").is_err(), "{path}");
            assert!(
                roots.write(path, b"changed", "overwrite", None).is_err(),
                "{path}"
            );
        }
        assert_eq!(std::fs::read(root.join("marker")).unwrap(), b"allowed");
        for path in [root.join("marker/"), root.join("marker/.")] {
            assert!(roots.read(path.to_str().unwrap(), 0, 100, "text").is_err());
            assert!(
                roots
                    .write(path.to_str().unwrap(), b"changed", "overwrite", None)
                    .is_err()
            );
        }
        assert!(!root.join("missing").exists());
        assert!(roots.list("nested/..", 0, 0, None).is_ok());
    }

    #[test]
    fn shared_roots_keep_absolute_access_and_internal_links() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        std::fs::write(second.path().join("marker"), "second").unwrap();
        std::os::unix::fs::symlink(second.path().join("marker"), first.path().join("link"))
            .unwrap();
        let roots = Roots::new(&[first.path().into(), second.path().into()], &[]).unwrap();
        for path in [first.path().join("link"), second.path().join("marker")] {
            assert_eq!(
                roots.read(path.to_str().unwrap(), 0, 100, "text").unwrap()["content"],
                "second"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "requires a disposable hosted image, umask 077 and the fixed four capabilities"]
    async fn hosted_worker_paths_support_private_umask_without_dac_override() {
        use std::os::unix::fs::MetadataExt;
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        assert_eq!(unsafe { libc::geteuid() }, 0);
        for field in ["CapEff", "CapPrm", "CapBnd"] {
            let actual = status
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{field}:")))
                .unwrap();
            assert_eq!(u64::from_str_radix(actual.trim(), 16).unwrap(), 0xe1);
        }
        assert!(status.lines().any(|line| line == "NoNewPrivs:\t1"));
        assert!(status.lines().any(|line| line == "Umask:\t0077"));
        let root = tempfile::tempdir_in("/var/lib/nyxid-hosted-tests").unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o711)).unwrap();
        let data = root.path().join("contexts");
        super::super::hosted_context::directory(&data, true).unwrap();
        for uid in [20000, 20003] {
            let gate = data.join(uid.to_string());
            std::fs::create_dir(&gate).unwrap();
            std::fs::set_permissions(&gate, std::fs::Permissions::from_mode(0o710)).unwrap();
            std::os::unix::fs::chown(&gate, Some(0), Some(uid)).unwrap();
            let workspace = gate.join("workspace");
            std::fs::create_dir(&workspace).unwrap();
            std::fs::write(workspace.join("private"), uid.to_string()).unwrap();
            std::os::unix::fs::chown(&workspace.join("private"), Some(uid), Some(uid)).unwrap();
            std::fs::set_permissions(&workspace, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::os::unix::fs::chown(&workspace, Some(uid), Some(uid)).unwrap();
        }
        let workspace = data.join("20000/workspace");
        assert_eq!(
            File::open(&workspace).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        let identity = super::super::process::Identity::resolve(Some("nyxc20000")).unwrap();
        let mut command = tokio::process::Command::new("/bin/sh");
        identity.prepare_agent(&mut command).unwrap();
        let access = command
            .args(["-ec", "printf own > \"$1/marker\"; test \"$(cat \"$1/private\")\" = 20000; test ! -r \"$2/private\"; test ! -w \"$2\"; test ! -r \"$3\"; test ! -w \"$3\"", "sh"])
            .arg(&workspace)
            .arg(data.join("20003/workspace"))
            .arg(&data)
            .output()
            .await
            .unwrap();
        let roots = Roots::new(&[workspace.clone()], &[]).unwrap();
        let pinned = roots.cwd(None);
        println!(
            "contexts_mode={:o} worker_path_success={} supervisor_pin_success={} worker_stderr={}",
            std::fs::metadata(&data).unwrap().mode() & 0o777,
            access.status.success(),
            pinned.is_ok(),
            String::from_utf8_lossy(&access.stderr)
        );
        assert!(
            access.status.success(),
            "worker must access its own retained path while sibling access stays denied"
        );
        let mut command = tokio::process::Command::new("/bin/cat");
        identity.prepare_agent(&mut command).unwrap();
        super::super::process::pin_cwd(&mut command, pinned.unwrap());
        let output = command.arg("marker").output().await.unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"own");
        assert_eq!(std::fs::metadata(&workspace).unwrap().mode() & 0o777, 0o700);
        let pinned = roots.cwd(None).unwrap();
        std::fs::set_permissions(data.join("20000"), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let mut command = tokio::process::Command::new("/bin/true");
        identity.prepare_agent(&mut command).unwrap();
        super::super::process::pin_cwd(&mut command, pinned);
        assert_eq!(
            command.status().await.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        // Existing private metadata directories must never be broadened.
        let private = root.path().join("metadata");
        std::fs::create_dir(&private).unwrap();
        super::super::hosted_context::directory(&private, true).unwrap();
        assert_eq!(std::fs::metadata(private).unwrap().mode() & 0o777, 0o700);
    }

    #[tokio::test]
    async fn command_directory_stays_pinned_after_parent_symlink_swap() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("parent/work")).unwrap();
        std::fs::create_dir(outside.path().join("work")).unwrap();
        std::fs::write(root.path().join("parent/work/marker"), b"allowed").unwrap();
        std::fs::write(outside.path().join("work/marker"), b"outside").unwrap();
        let roots = Roots::new(&[root.path().into()], &[]).unwrap();
        let pinned = roots.cwd(Some("parent/work")).unwrap();
        std::fs::rename(root.path().join("parent"), root.path().join("moved")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("parent")).unwrap();
        let mut command = tokio::process::Command::new("/bin/cat");
        super::super::process::pin_cwd(&mut command, pinned);
        let output = command.arg("marker").output().await.unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"allowed");
        assert!(roots.cwd(Some("parent/work")).is_err());
    }
    #[test]
    fn streamed_writes_validate_size_and_hash_before_atomic_commit() {
        let temp = tempfile::tempdir().unwrap();
        let roots = Roots::new(&[temp.path().into()], &[]).unwrap();
        let bytes = vec![b'X'; 20 * 1024 * 1024];
        let expected = hex::encode(Sha256::digest(&bytes));
        let mut reader = std::io::Cursor::new(&bytes);
        assert_eq!(
            roots
                .write_stream(
                    "complete",
                    &mut reader,
                    bytes.len() as u64,
                    "create",
                    None,
                    Some(&expected)
                )
                .unwrap(),
            expected
        );
        for (name, length, hash) in [
            ("short", bytes.len() as u64 + 1, expected.as_str()),
            ("long", bytes.len() as u64 - 1, expected.as_str()),
            ("corrupt", bytes.len() as u64, "wrong"),
        ] {
            assert!(
                roots
                    .write_stream(
                        name,
                        &mut std::io::Cursor::new(&bytes),
                        length,
                        "create",
                        None,
                        Some(hash)
                    )
                    .is_err()
            );
            assert!(!temp.path().join(name).exists());
        }
        let mut output = Vec::new();
        roots
            .stream_read("complete", &mut output, bytes.len() as u64)
            .unwrap();
        assert_eq!(output, bytes);
        assert!(roots.stream_read("complete", &mut Vec::new(), 100).is_err());
        assert_eq!(
            std::fs::read_dir(temp.path()).unwrap().count(),
            1,
            "failed writes leave no temporary files"
        );
    }

    #[test]
    fn roots_reject_links_escapes_and_node_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let secret = root.join("node");
        std::fs::create_dir(&secret).unwrap();
        std::fs::write(secret.join("key"), "hidden").unwrap();
        std::fs::write(temp.path().join("outside"), "outside").unwrap();
        std::os::unix::fs::symlink(temp.path().join("outside"), root.join("link")).unwrap();
        let roots = Roots::new(std::slice::from_ref(&root), &[secret]).unwrap();
        for path in ["../outside", "link", "node/key"] {
            assert!(roots.read(path, 0, 100, "text").is_err());
        }
        assert!(roots.write("link", b"changed", "overwrite", None).is_err());
        assert_eq!(
            std::fs::read_to_string(temp.path().join("outside")).unwrap(),
            "outside"
        );
    }
    #[test]
    fn atomic_writes_preserve_mode_and_reject_stale_edits() {
        let temp = tempfile::tempdir().unwrap();
        let roots = Roots::new(&[temp.path().into()], &[]).unwrap();
        let hash = roots.write("file", b"one one", "create", None).unwrap();
        assert!(roots.write("file", b"oops", "create", None).is_err());
        assert!(roots.edit("file", "one", "two", false, None).is_err());
        std::fs::set_permissions(
            temp.path().join("file"),
            std::fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        roots.edit("file", "one", "two", true, Some(&hash)).unwrap();
        assert!(
            roots
                .write("file", b"stale", "overwrite", Some(&hash))
                .is_err()
        );
        assert_eq!(roots.read("file", 4, 3, "text").unwrap()["content"], "two");
        assert_eq!(
            std::fs::metadata(temp.path().join("file"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
    }

    #[test]
    fn listings_filter_recursive_globs_and_paginate_with_bounded_results() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("source")).unwrap();
        for index in 0..75 {
            std::fs::write(temp.path().join(format!("source/{index:03}.rs")), "").unwrap();
        }
        std::fs::write(temp.path().join("source/ignored.txt"), "").unwrap();
        let roots = Roots::new(&[temp.path().into()], &[]).unwrap();
        let first = roots.list(".", 1, 0, Some("**/*.rs")).unwrap();
        assert_eq!(first["entries"].as_array().unwrap().len(), 50);
        assert_eq!(first["has_more"], true);
        let second = roots
            .list(
                ".",
                1,
                first["offset"].as_u64().unwrap() as usize,
                Some("**/*.rs"),
            )
            .unwrap();
        assert_eq!(second["entries"].as_array().unwrap().len(), 25);
        assert_eq!(second["has_more"], false);
        assert!(roots.list(".", 9, 0, None).is_err());
    }
}
