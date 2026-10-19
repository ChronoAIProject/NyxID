//! Native profiles share Chromium's system policy directory. Named-user ACLs
//! restrict only the new developer UID; never change another profile's groups
//! or make the secure policy unreadable to an existing secure browser.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
};
const ATTRIBUTE: &std::ffi::CStr = c"system.posix_acl_access";
const UNDEFINED: u32 = u32::MAX;
const MAX_ACL: usize = 32768;

fn entry(tag: u16, perm: u16, id: u32) -> [u8; 8] {
    let mut bytes = [0; 8];
    bytes[..2].copy_from_slice(&tag.to_le_bytes());
    bytes[2..4].copy_from_slice(&perm.to_le_bytes());
    bytes[4..].copy_from_slice(&id.to_le_bytes());
    bytes
}
fn named_access(file: &File, uid: u32, allow: u16) -> Result<()> {
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.uid() == 0 && meta.mode() & 0o022 == 0,
        "context_policy_owner_invalid"
    );
    let mut buffer = vec![0u8; MAX_ACL];
    let count = unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            ATTRIBUTE.as_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    let mut entries = if count < 0 {
        ensure!(
            std::io::Error::last_os_error().raw_os_error() == Some(libc::ENODATA),
            "context_policy_acl_unavailable"
        );
        vec![
            entry(1, ((meta.mode() >> 6) & 7) as u16, UNDEFINED),
            entry(4, ((meta.mode() >> 3) & 7) as u16, UNDEFINED),
            entry(32, (meta.mode() & 7) as u16, UNDEFINED),
        ]
    } else {
        let bytes = &buffer[..count as usize];
        ensure!(
            bytes.len() >= 4 && bytes[..4] == 2u32.to_le_bytes(),
            "context_policy_acl_invalid"
        );
        let (entries, tail) = bytes[4..].as_chunks::<8>();
        ensure!(tail.is_empty(), "context_policy_acl_invalid");
        entries.to_vec()
    };
    let tag = |e: &[u8; 8]| u16::from_le_bytes([e[0], e[1]]);
    let id = |e: &[u8; 8]| u32::from_le_bytes(e[4..].try_into().expect("ACL id"));
    entries.retain(|e| tag(e) != 2 || id(e) != uid);
    entries.push(entry(2, allow, uid));
    if let Some(mask) = entries.iter_mut().find(|e| tag(e) == 16) {
        let current = u16::from_le_bytes([mask[2], mask[3]]);
        *mask = entry(16, current | allow, UNDEFINED);
    } else {
        entries.push(entry(
            16,
            ((meta.mode() >> 3) & 7) as u16 | allow,
            UNDEFINED,
        ));
    }
    entries.sort_by_key(|e| (tag(e), id(e)));
    ensure!(
        entries.len() * 8 + 4 <= MAX_ACL,
        "context_policy_acl_capacity"
    );
    let mut bytes = 2u32.to_le_bytes().to_vec();
    for e in entries {
        bytes.extend_from_slice(&e);
    }
    ensure!(
        unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                ATTRIBUTE.as_ptr(),
                bytes.as_ptr().cast(),
                bytes.len(),
                0,
            )
        } == 0,
        "context_policy_acl_unavailable"
    );
    file.sync_all()?;
    Ok(())
}
fn open(path: &Path) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .context("context_policy_unavailable")
}
pub fn prepare(root: &Path, dev_uid: u32) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0 && dev_uid >= nyxid_machine::context::FIRST_UID,
        "context_policy_supervisor_required"
    );
    let resources = root.join("opt/nyxid/machine-browser");
    super::browser::protected_runtime_parent(&resources)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(resources.join("context-policies.lock"))?;
    ensure!(
        lock.metadata()?.uid() == 0 && lock.metadata()?.mode() & 0o077 == 0,
        "context_policy_lock_invalid"
    );
    ensure!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "context_policy_busy"
    );
    // Private dev policy is installed before the new UID can launch Chromium.
    // Existing secure users cannot read it; their system policy stays intact.
    let directory = root.join("etc/chromium/policies/managed");
    super::browser::protected_runtime_parent(&directory)?;
    let path = directory.join("nyxid-context-dev.json");
    let expected = serde_json::to_vec(&super::browser::dev_policy())?;
    let mut staged = tempfile::NamedTempFile::new_in(&directory)?;
    staged.write_all(&expected)?;
    staged.as_file().sync_all()?;
    match staged.persist_noclobber(&path) {
        Ok(_) => File::open(&directory)?.sync_all()?,
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.error.into()),
    }
    let policy = open(&path)?;
    ensure!(
        policy.metadata()?.len() == expected.len() as u64 && std::fs::read(&path)? == expected,
        "context_policy_refresh_required"
    );
    named_access(&policy, dev_uid, 4)?;
    for path in [
        "etc/chromium/policies/managed/nyxid.json",
        "etc/chromium/native-messaging-hosts/dev.nyxid.machine_filler.json",
        "opt/nyxid/machine-browser/native-host",
        "opt/nyxid/machine-browser/nyxid-native-host",
    ] {
        named_access(&open(&root.join(path))?, dev_uid, 0)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    #[test]
    fn native_context_policies_keep_existing_profiles_and_deny_only_developer_users() {
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let protected = [
            "etc/chromium/policies/managed/nyxid.json",
            "etc/chromium/native-messaging-hosts/dev.nyxid.machine_filler.json",
            "opt/nyxid/machine-browser/native-host",
            "opt/nyxid/machine-browser/nyxid-native-host",
        ];
        for name in protected {
            let path = root.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"policy fixture").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let readable = |uid, path: &Path| {
            let mut command = std::process::Command::new("/bin/cat");
            command.arg(path);
            unsafe {
                command.pre_exec(move || {
                    if libc::setgroups(0, std::ptr::null()) < 0
                        || libc::setgid(uid) < 0
                        || libc::setuid(uid) < 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            command.output().unwrap().status.success()
        };
        for uid in [65534, 65532, 65534] {
            prepare(root.path(), uid).unwrap();
        }
        for name in protected {
            let path = root.path().join(name);
            assert!(readable(65533, &path), "legacy/secure reader retained");
            assert!(!readable(65534, &path), "first developer denied");
            assert!(!readable(65532, &path), "second developer denied");
        }
        let dev = root
            .path()
            .join("etc/chromium/policies/managed/nyxid-context-dev.json");
        assert!(readable(65534, &dev));
        assert!(readable(65532, &dev));
        assert!(!readable(65533, &dev));
    }
}
