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
        Ok(Self { roots, excluded })
    }

    pub fn cwd(&self, path: Option<&str>) -> Result<Vec<File>> {
        let supplied = path
            .map(PathBuf::from)
            .or_else(|| self.roots.first().cloned())
            .context("no workspace root")?;
        let resolved = self.resolve(&supplied, false)?;
        // Keep every ancestor for the child to check after dropping privileges.
        // A root supervisor must not bypass a protected parent's permissions.
        let mut directories = vec![File::from(open_at(
            libc::AT_FDCWD,
            std::ffi::OsStr::new("/"),
            libc::O_RDONLY | libc::O_DIRECTORY,
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
                    libc::O_RDONLY | libc::O_DIRECTORY,
                    0,
                )?));
            }
        }
        Ok(directories)
    }

    fn resolve(&self, path: &Path, create: bool) -> Result<PathBuf> {
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

    fn parent(&self, resolved: &Path) -> Result<(OwnedFd, CString)> {
        let root = self
            .roots
            .iter()
            .filter(|root| resolved.starts_with(root))
            .max_by_key(|root| root.components().count())
            .context(super::MachineError::PathOutsideRoots)?;
        let mut directory = open_at(
            libc::AT_FDCWD,
            std::ffi::OsStr::new("/"),
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )?;
        for component in root.components() {
            if let Component::Normal(name) = component {
                directory = open_at(
                    directory.as_raw_fd(),
                    name,
                    libc::O_RDONLY | libc::O_DIRECTORY,
                    0,
                )?;
            }
        }
        let components: Vec<_> = resolved.strip_prefix(root)?.components().collect();
        if components.is_empty() {
            return Ok((directory, CString::new(".")?));
        }
        for component in &components[..components.len() - 1] {
            let Component::Normal(name) = component else {
                return Err(super::MachineError::PathOutsideRoots.into());
            };
            directory = open_at(
                directory.as_raw_fd(),
                name,
                libc::O_RDONLY | libc::O_DIRECTORY,
                0,
            )?;
        }
        let Component::Normal(name) = components[components.len() - 1] else {
            return Err(super::MachineError::PathOutsideRoots.into());
        };
        Ok((directory, cstring(name)?))
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
        let bytes = vec![b'X'; 2 * 1024 * 1024];
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
