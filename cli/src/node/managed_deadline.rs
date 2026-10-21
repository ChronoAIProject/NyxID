//! Durable protected local execution cutoff; supplemental to physical runtime enforcement.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
};
fn prepare_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path)?;
    ensure!(
        !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
        "managed deadline directory invalid"
    );
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}
fn read_private_file(path: &Path) -> Result<Vec<u8>> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0
            && meta.len() <= 16 * 1024,
        "managed deadline file invalid"
    );
    let mut bytes = Vec::new();
    (&mut file).take(16 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 16 * 1024, "managed deadline file too large");
    Ok(bytes)
}
fn atomic_private_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let directory = path
        .parent()
        .context("managed deadline directory missing")?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
}
/// Supervisor-owned local tombstone. This is supplemental process admission,
/// never physical Pod stop or storage exclusion evidence.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeadlineMarker {
    binding: nyxid_machine::managed::Binding,
    revision: u64,
    cutoff_ms: i64,
    boot_id: String,
    monotonic_cutoff_ms: u64,
    last_utc_ms: i64,
    closed: bool,
}
impl DeadlineMarker {
    pub(crate) fn live(&mut self) -> bool {
        let Ok(now) = nyxid_machine::managed::host_clock_sample() else {
            self.closed = true;
            return false;
        };
        self.live_at(&now)
    }
    fn live_at(&mut self, now: &nyxid_machine::managed::HostClockSample) -> bool {
        self.closed |= now.boot_id != self.boot_id
            || now.utc_ms < self.last_utc_ms
            || now.utc_ms >= self.cutoff_ms
            || now.monotonic_ms >= self.monotonic_cutoff_ms;
        self.last_utc_ms = self.last_utc_ms.max(now.utc_ms);
        !self.closed
    }
}
fn monotonic_cutoff(
    prior: Option<&DeadlineMarker>,
    now: &nyxid_machine::managed::HostClockSample,
    cutoff_ms: i64,
) -> Result<u64> {
    ensure!(cutoff_ms > now.utc_ms, "managed deadline expired");
    let remaining = cutoff_ms
        .checked_sub(now.utc_ms)
        .context("managed deadline overflow")?;
    let sampled = now
        .monotonic_ms
        .checked_add(u64::try_from(remaining)?)
        .context("managed deadline overflow")?;
    let Some(prior) = prior else {
        return Ok(sampled);
    };
    // A new revision must not resample away elapsed time if wall time stalls.
    // Preserve the same target's original mapping and add only the explicit
    // absolute cutoff change. A forward wall adjustment can tighten it further.
    let translated =
        i128::from(prior.monotonic_cutoff_ms) + i128::from(cutoff_ms) - i128::from(prior.cutoff_ms);
    let translated = u64::try_from(translated).context("managed deadline overflow")?;
    Ok(sampled.min(translated))
}
fn deadline_marker_lock(directory: &Path) -> Result<std::fs::File> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.join("managed-deadline.lock"))?;
    ensure!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0,
        "managed deadline lock unavailable"
    );
    Ok(file)
}
pub(crate) fn install_deadline_marker(
    directory: &Path,
    window: &nyxid_machine::managed::RuntimeWindow,
) -> Result<DeadlineMarker> {
    prepare_directory(directory)?;
    let _lock = deadline_marker_lock(directory)?;
    let path = directory.join("managed-deadline-v1.json");
    let previous: Option<DeadlineMarker> = match read_private_file(&path) {
        Ok(bytes) => Some(serde_json::from_slice(&bytes)?),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    let now = nyxid_machine::managed::host_clock_sample().map_err(anyhow::Error::msg)?;
    let mut prior_elapsed = None;
    if let Some(mut prior) = previous {
        if prior.binding == window.binding {
            if !prior.live_at(&now) {
                atomic_private_json(&path, &prior)?;
                anyhow::bail!("managed deadline is terminal");
            }
            ensure!(
                window.control_revision >= prior.revision,
                "stale managed deadline revision"
            );
            if window.control_revision == prior.revision {
                ensure!(
                    window.run_not_after_ms == prior.cutoff_ms,
                    "managed deadline revision mismatch"
                );
                atomic_private_json(&path, &prior)?;
                return Ok(prior);
            }
            prior_elapsed = Some(prior);
        } else {
            ensure!(
                prior.binding.node_id == window.binding.node_id
                    && (window.binding.runtime_generation > prior.binding.runtime_generation
                        || (window.binding.runtime_generation == prior.binding.runtime_generation
                            && window.binding.credential_epoch > prior.binding.credential_epoch)),
                "managed deadline binding must advance"
            );
        }
    }
    let monotonic_cutoff_ms =
        monotonic_cutoff(prior_elapsed.as_ref(), &now, window.run_not_after_ms)?;
    let marker = DeadlineMarker {
        binding: window.binding.clone(),
        revision: window.control_revision,
        cutoff_ms: window.run_not_after_ms,
        boot_id: now.boot_id,
        monotonic_cutoff_ms,
        last_utc_ms: now.utc_ms,
        closed: false,
    };
    atomic_private_json(&path, &marker)?;
    Ok(marker)
}
pub(crate) fn close_deadline_marker(directory: &Path, marker: &DeadlineMarker) -> Result<()> {
    let _lock = deadline_marker_lock(directory)?;
    let path = directory.join("managed-deadline-v1.json");
    let mut current: DeadlineMarker = serde_json::from_slice(&read_private_file(&path)?)?;
    if current.binding == marker.binding && current.revision == marker.revision {
        current.closed = true;
        current.last_utc_ms = current.last_utc_ms.max(marker.last_utc_ms);
        atomic_private_json(&path, &current)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> nyxid_machine::managed::Binding {
        let id = || uuid::Uuid::new_v4().to_string();
        nyxid_machine::managed::Binding {
            node_id: id(),
            sandbox_uid: id(),
            pod_uid: id(),
            runtime_generation: 1,
            credential_epoch: 1,
            writer_fence: id(),
            operation_id: id(),
            delivery_id: id(),
        }
    }
    #[test]
    fn renewed_cutoff_keeps_elapsed_mapping_when_wall_time_stalls() {
        let mut prior = DeadlineMarker {
            binding: binding(),
            revision: 1,
            cutoff_ms: 61_000,
            boot_id: "test-boot".into(),
            monotonic_cutoff_ms: 62_000,
            last_utc_ms: 1_000,
            closed: false,
        };
        let mut now = nyxid_machine::managed::HostClockSample {
            utc_ms: 1_000,
            monotonic_ms: 12_000,
            boot_id: "test-boot".into(),
        };
        assert!(prior.live_at(&now));
        assert_eq!(
            monotonic_cutoff(Some(&prior), &now, 61_000).unwrap(),
            62_000
        );
        assert_eq!(
            monotonic_cutoff(Some(&prior), &now, 81_000).unwrap(),
            82_000
        );
        assert_eq!(
            monotonic_cutoff(Some(&prior), &now, 41_000).unwrap(),
            42_000
        );
        // Faster wall time can shorten the retained bound, never lengthen it.
        now.utc_ms = 21_000;
        assert_eq!(
            monotonic_cutoff(Some(&prior), &now, 61_000).unwrap(),
            52_000
        );
        now.utc_ms = 1_000;
        now.monotonic_ms = 62_000;
        assert!(!prior.live_at(&now));
    }
    #[test]
    fn renewed_cutoff_overflow_is_rejected() {
        let now = nyxid_machine::managed::HostClockSample {
            utc_ms: -1,
            monotonic_ms: 0,
            boot_id: "test-boot".into(),
        };
        assert!(monotonic_cutoff(None, &now, i64::MAX).is_err());
    }
    #[test]
    fn durable_cutoff_rejects_restart_revival_and_allows_new_generation() {
        let directory = tempfile::tempdir().unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let mut window = nyxid_machine::managed::RuntimeWindow {
            binding: binding(),
            control_revision: 1,
            run_not_after_ms: now + 60_000,
            issued_at_ms: now,
        };
        let original = install_deadline_marker(directory.path(), &window).unwrap();
        close_deadline_marker(directory.path(), &original).unwrap();
        window.control_revision += 1;
        assert!(install_deadline_marker(directory.path(), &window).is_err());
        window.binding.runtime_generation += 1;
        window.binding.credential_epoch += 1;
        window.binding.pod_uid = uuid::Uuid::new_v4().to_string();
        assert!(install_deadline_marker(directory.path(), &window).is_ok());
    }
    #[test]
    fn durable_cutoff_rejects_boot_change_and_retains_original_elapsed_deadline() {
        let directory = tempfile::tempdir().unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let window = nyxid_machine::managed::RuntimeWindow {
            binding: binding(),
            control_revision: 1,
            run_not_after_ms: now + 60_000,
            issued_at_ms: now,
        };
        let marker = install_deadline_marker(directory.path(), &window).unwrap();
        assert_eq!(
            marker.monotonic_cutoff_ms,
            install_deadline_marker(directory.path(), &window)
                .unwrap()
                .monotonic_cutoff_ms
        );
        let mut changed = marker;
        changed.boot_id = uuid::Uuid::new_v4().to_string();
        atomic_private_json(&directory.path().join("managed-deadline-v1.json"), &changed).unwrap();
        assert!(install_deadline_marker(directory.path(), &window).is_err());
    }
}
