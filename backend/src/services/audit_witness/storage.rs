//! A directory journal: fsync private record, publish without replacement using
//! link(2), then fsync the directory before acknowledgment. Published records are
//! never edited, replaced or removed. Incomplete unpublished files are ignored.
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use rand::RngCore;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{
    History, MAX_FRAME_BYTES, PROTOCOL, WitnessError, WitnessResult, WitnessSigningKey, ZERO_HASH,
    now_ms, sign_receipt,
};
use crate::models::audit_witness::*;

fn storage<T>(value: std::io::Result<T>) -> WitnessResult<T> {
    value.map_err(|_| WitnessError::Storage)
}
fn euid() -> u32 {
    // SAFETY: geteuid takes no pointers and has no preconditions.
    unsafe { libc::geteuid() }
}
fn sync_dir(path: &Path) -> WitnessResult<()> {
    storage(File::open(path))?
        .sync_all()
        .map_err(|_| WitnessError::Storage)
}

fn nofollow() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    options
}

fn check_file(path: &Path, owner: u32, mode: u32) -> WitnessResult<()> {
    let m = fs::symlink_metadata(path).map_err(|_| WitnessError::Permissions)?;
    if !m.is_file() || m.uid() != owner || m.mode() & 0o7777 != mode || m.nlink() != 1 {
        return Err(WitnessError::Permissions);
    }
    Ok(())
}

fn check_dir(path: &Path, owner: u32, mode: u32) -> WitnessResult<()> {
    let m = fs::symlink_metadata(path).map_err(|_| WitnessError::Permissions)?;
    if !m.is_dir() || m.uid() != owner || m.mode() & 0o7777 != mode {
        return Err(WitnessError::Permissions);
    }
    // Reject writable ancestors and symlink substitution. Root-owned sticky
    // /tmp is allowed; the private child itself has already been checked.
    for parent in path.ancestors().skip(1) {
        let m = fs::symlink_metadata(parent).map_err(|_| WitnessError::Permissions)?;
        if !m.is_dir()
            || (m.uid() != 0 && m.uid() != owner)
            || (m.mode() & 0o022 != 0 && !(m.uid() == 0 && m.mode() & 0o1000 != 0))
        {
            return Err(WitnessError::Permissions);
        }
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: usize) -> WitnessResult<Zeroizing<Vec<u8>>> {
    let file = storage(nofollow().read(true).open(path))?;
    // This helper also reads signing material: clear partial/oversized reads
    // on every error path, before any caller can construct its key wrapper.
    let mut bytes = Zeroizing::new(Vec::new());
    storage(file.take(limit as u64 + 1).read_to_end(&mut bytes))?;
    if bytes.len() > limit {
        return Err(WitnessError::Corrupt);
    }
    Ok(bytes)
}

fn create_file(path: &Path, bytes: &[u8], mode: u32) -> WitnessResult<()> {
    let mut file = storage(
        nofollow()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(path),
    )?;
    storage(file.write_all(bytes))?;
    storage(file.set_permissions(fs::Permissions::from_mode(mode)))?;
    storage(file.sync_all())
}

fn chown(path: &Path, uid: u32, gid: u32) -> WitnessResult<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let encoded_path =
        CString::new(path.as_os_str().as_bytes()).map_err(|_| WitnessError::Invalid)?;
    // SAFETY: encoded_path is a NUL-terminated CString alive for this call.
    if unsafe { libc::chown(encoded_path.as_ptr(), uid, gid) } != 0 {
        return Err(WitnessError::Permissions);
    }
    storage(File::open(path))?
        .sync_all()
        .map_err(|_| WitnessError::Storage)
}

/// Offline, root-only provisioning on the independent sink host. Existing
/// paths are errors. No reset/replace mode exists. Transfer the generated token
/// file to the application through a separate secret-provisioning channel.
pub fn enroll(
    admin_dir: &Path,
    journal_dir: &Path,
    token_output: &Path,
    sink_uid: u32,
    sink_gid: u32,
    chain_id: Uuid,
    head: AuditLink,
) -> WitnessResult<()> {
    if euid() != 0
        || sink_uid == 0
        || sink_gid == 0
        || !admin_dir.is_absolute()
        || !journal_dir.is_absolute()
        || !token_output.is_absolute()
    {
        return Err(WitnessError::Permissions);
    }
    let mut secret = Zeroizing::new([0_u8; 32]);
    rand::rngs::OsRng.fill_bytes(secret.as_mut());
    let key = WitnessSigningKey::new(secret);
    let mut token_bytes = Zeroizing::new([0_u8; 32]);
    rand::rngs::OsRng.fill_bytes(token_bytes.as_mut());
    let token = Zeroizing::new(hex::encode(token_bytes.as_ref()));
    let enrollment = Enrollment {
        protocol: PROTOCOL,
        chain_id,
        initial_head: head.clone(),
        verification_key: hex::encode(key.verifying_key().to_bytes()),
        append_token_sha256: hex::encode(Sha256::digest(token.as_bytes())),
    };
    let _ = History::new(enrollment.clone())?;
    // create_dir fails closed after an interrupted/previous enrollment. An
    // administrator must inspect it; the append client cannot re-enroll.
    storage(fs::create_dir(admin_dir))?;
    storage(fs::set_permissions(
        admin_dir,
        fs::Permissions::from_mode(0o750),
    ))?;
    chown(admin_dir, 0, sink_gid)?;
    check_dir(admin_dir, 0, 0o750)?;
    storage(fs::create_dir(journal_dir))?;
    storage(fs::set_permissions(
        journal_dir,
        fs::Permissions::from_mode(0o700),
    ))?;
    chown(journal_dir, sink_uid, sink_gid)?;
    check_dir(journal_dir, sink_uid, 0o700)?;
    create_file(&admin_dir.join("signing.key"), key.seed.as_ref(), 0o440)?;
    create_file(
        &admin_dir.join("enrollment.json"),
        &serde_json::to_vec(&enrollment).map_err(|_| WitnessError::Storage)?,
        0o440,
    )?;
    for name in ["signing.key", "enrollment.json"] {
        chown(&admin_dir.join(name), 0, sink_gid)?;
    }
    create_file(token_output, token.as_bytes(), 0o600)?;
    let receipt = sign_receipt(
        ReceiptBody {
            protocol: PROTOCOL,
            chain_id,
            receipt_seq: 0,
            previous_receipt: ZERO_HASH.into(),
            witness_time_ms: now_ms()?,
            head: head.clone(),
            event: WitnessEvent::Enrollment { head },
        },
        &key,
    );
    let journal = Journal::create(journal_dir)?;
    journal.append(&receipt)?;
    chown(&journal_dir.join("lock"), sink_uid, sink_gid)?;
    chown(&journal_dir.join(record_name(0)), sink_uid, sink_gid)?;
    sync_dir(admin_dir)?;
    sync_dir(admin_dir.parent().ok_or(WitnessError::Invalid)?)?;
    sync_dir(journal_dir.parent().ok_or(WitnessError::Invalid)?)?;
    sync_dir(token_output.parent().ok_or(WitnessError::Invalid)?)
}

pub(super) fn open(
    admin_dir: &Path,
    journal_dir: &Path,
) -> WitnessResult<(Enrollment, WitnessSigningKey, Journal)> {
    open_impl(admin_dir, journal_dir, true)
}

fn open_impl(
    admin_dir: &Path,
    journal_dir: &Path,
    separate_admin: bool,
) -> WitnessResult<(Enrollment, WitnessSigningKey, Journal)> {
    if !admin_dir.is_absolute() || !journal_dir.is_absolute() {
        return Err(WitnessError::Permissions);
    }
    let admin_uid = fs::symlink_metadata(admin_dir)
        .map_err(|_| WitnessError::Permissions)?
        .uid();
    if separate_admin && (euid() == 0 || admin_uid == euid()) {
        return Err(WitnessError::Permissions);
    }
    check_dir(admin_dir, admin_uid, 0o750)?;
    check_dir(journal_dir, euid(), 0o700)?;
    check_file(&admin_dir.join("enrollment.json"), admin_uid, 0o440)?;
    check_file(&admin_dir.join("signing.key"), admin_uid, 0o440)?;
    if separate_admin {
        // A dedicated sink group must hold the only group access. Do not rely
        // on an incidental supplementary group to read signing authority.
        // SAFETY: getegid takes no pointers and has no preconditions.
        let gid = unsafe { libc::getegid() };
        for path in [
            admin_dir.to_path_buf(),
            admin_dir.join("enrollment.json"),
            admin_dir.join("signing.key"),
            journal_dir.to_path_buf(),
        ] {
            if fs::symlink_metadata(path)
                .map_err(|_| WitnessError::Permissions)?
                .gid()
                != gid
            {
                return Err(WitnessError::Permissions);
            }
        }
    }
    let enrollment: Enrollment =
        serde_json::from_slice(&read_bounded(&admin_dir.join("enrollment.json"), 4096)?)
            .map_err(|_| WitnessError::Corrupt)?;
    let bytes = read_bounded(&admin_dir.join("signing.key"), 32)?;
    let secret =
        Zeroizing::new(<[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| WitnessError::Corrupt)?);
    let key = WitnessSigningKey::new(secret);
    let journal = Journal::create(journal_dir)?;
    Ok((enrollment, key, journal))
}

fn record_name(seq: u64) -> String {
    format!("{seq:020}.json")
}

pub(super) struct Journal {
    dir: PathBuf,
    _lock: File,
}

impl Journal {
    fn create(dir: &Path) -> WitnessResult<Self> {
        let lock_path = dir.join("lock");
        let lock = storage(
            nofollow()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(&lock_path),
        )?;
        check_file(&lock_path, euid(), 0o600)?;
        lock.try_lock().map_err(|_| WitnessError::Unavailable)?;
        sync_dir(dir)?;
        Ok(Self {
            dir: dir.into(),
            _lock: lock,
        })
    }

    pub(super) fn read_all(&self) -> WitnessResult<Vec<SignedReceipt>> {
        let mut names = Vec::new();
        for entry in storage(fs::read_dir(&self.dir))? {
            let name = storage(entry)?
                .file_name()
                .into_string()
                .map_err(|_| WitnessError::Corrupt)?;
            if name == "lock" {
                continue;
            }
            if let Some(id) = name.strip_prefix(".pending-")
                && Uuid::parse_str(id).is_ok()
            {
                continue;
            }
            if name.len() != 25
                || !name.ends_with(".json")
                || !name.as_bytes()[..20].iter().all(u8::is_ascii_digit)
            {
                return Err(WitnessError::Corrupt);
            }
            names.push(name);
        }
        names.sort();
        let mut records = Vec::new();
        for (seq, name) in names.iter().enumerate() {
            if name != &record_name(seq as u64) {
                return Err(WitnessError::Corrupt);
            }
            let path = self.dir.join(name);
            // A crash after publication can leave the private temporary hardlink.
            // Published contents are immutable; nlink==2 is harmless in this case.
            let m = fs::symlink_metadata(&path).map_err(|_| WitnessError::Corrupt)?;
            if !m.is_file() || m.mode() & 0o7777 != 0o400 || m.uid() != euid() {
                return Err(WitnessError::Permissions);
            }
            records.push(
                serde_json::from_slice(&read_bounded(&path, MAX_FRAME_BYTES)?)
                    .map_err(|_| WitnessError::Corrupt)?,
            );
        }
        Ok(records)
    }

    pub(super) fn append(&self, receipt: &SignedReceipt) -> WitnessResult<()> {
        let bytes = serde_json::to_vec(receipt).map_err(|_| WitnessError::Storage)?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(WitnessError::Invalid);
        }
        let pending = self.dir.join(format!(".pending-{}", Uuid::new_v4()));
        let mut file = storage(
            nofollow()
                .write(true)
                .create_new(true)
                .mode(0o400)
                .open(&pending),
        )?;
        storage(file.write_all(&bytes))?;
        crash_boundary("written");
        storage(file.sync_all())?;
        crash_boundary("file_synced");
        // hard_link refuses an existing destination; rename would replace it.
        storage(fs::hard_link(
            &pending,
            self.dir.join(record_name(receipt.body.receipt_seq)),
        ))?;
        crash_boundary("published");
        sync_dir(&self.dir)?;
        crash_boundary("directory_synced");
        storage(fs::remove_file(&pending))?;
        sync_dir(&self.dir)?;
        Ok(())
    }
}

#[cfg(not(test))]
fn crash_boundary(_: &str) {}

#[cfg(test)]
fn crash_boundary(boundary: &str) {
    if std::env::var("NYXID_WITNESS_TEST_CRASH").as_deref() == Ok(boundary) {
        std::process::abort();
    }
}

#[cfg(test)]
pub(super) fn fixture_open(
    admin: &Path,
    journal: &Path,
) -> WitnessResult<(Enrollment, WitnessSigningKey, Journal)> {
    open_impl(admin, journal, false)
}

#[cfg(test)]
pub(super) fn fixture_create(
    admin: &Path,
    journal_dir: &Path,
    chain_id: Uuid,
    head: AuditLink,
) -> WitnessResult<Enrollment> {
    storage(fs::create_dir(admin))?;
    storage(fs::set_permissions(
        admin,
        fs::Permissions::from_mode(0o750),
    ))?;
    storage(fs::create_dir(journal_dir))?;
    storage(fs::set_permissions(
        journal_dir,
        fs::Permissions::from_mode(0o700),
    ))?;
    let secret = Zeroizing::new([37_u8; 32]);
    let key = WitnessSigningKey::new(secret);
    let enrollment = Enrollment {
        protocol: PROTOCOL,
        chain_id,
        initial_head: head.clone(),
        verification_key: hex::encode(key.verifying_key().to_bytes()),
        append_token_sha256: hex::encode(Sha256::digest(b"a".repeat(64))),
    };
    create_file(&admin.join("signing.key"), key.seed.as_ref(), 0o440)?;
    create_file(
        &admin.join("enrollment.json"),
        &serde_json::to_vec(&enrollment).unwrap(),
        0o440,
    )?;
    let journal = Journal::create(journal_dir)?;
    journal.append(&sign_receipt(
        ReceiptBody {
            protocol: PROTOCOL,
            chain_id,
            receipt_seq: 0,
            previous_receipt: ZERO_HASH.into(),
            witness_time_ms: now_ms()?,
            head: head.clone(),
            event: WitnessEvent::Enrollment { head },
        },
        &key,
    ))?;
    Ok(enrollment)
}
