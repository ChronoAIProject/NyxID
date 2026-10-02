//! Narrow Docker controller. The mailbox chooses a version, never an image or command.
use anyhow::{Context, Result, bail, ensure};
use nyxid_machine::update::{self, Connected, Phase, Progress};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub mod docker;

/// Only these fixed classifications leave the controller. Never format an
/// anyhow chain: Docker errors may contain environment or credential metadata.
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("machine_update failed at {stage}: {reason}")]
pub struct Failure {
    stage: &'static str,
    reason: &'static str,
}
impl Failure {
    fn new(stage: &'static str, reason: &'static str) -> Self {
        Self { stage, reason }
    }
    pub fn classify(error: &anyhow::Error, stage: &'static str) -> Self {
        error.downcast_ref::<Self>().copied().unwrap_or_else(|| {
            let reason = error
                .downcast_ref::<docker::CheckFailure>()
                .map(|failure| failure.code())
                .unwrap_or("operation_failed");
            Self::new(stage, reason)
        })
    }
    fn code(self) -> String {
        format!("{}:{}", self.stage, self.reason)
    }
}

/// Reuse tough's expiry/rollback datastore under the controller lock. Open the
/// directory without following a symlink and verify its owner and mode before
/// giving tough the path. Neither bootstrap nor watch needs a system temp dir.
fn trust_datastore(root: &Path) -> Result<PathBuf> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    private_directory(root)?;
    let path = root.join("tuf");
    private_directory(&path)?;
    let dir = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(&path)?;
    let metadata = dir.metadata()?;
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
        "Trust datastore is not private"
    );
    Ok(path)
}

async fn verified_image(
    api: &docker::Docker,
    root: &Path,
    image: &str,
    version: &str,
) -> Result<String> {
    use crate::update_attestation::ImageVerificationError;
    let stage = if image == update::UPDATER_IMAGE {
        "verify_updater_image"
    } else {
        "verify_machine_image"
    };
    let datastore =
        trust_datastore(root).map_err(|_| Failure::new(stage, "trust_store_unavailable"))?;
    api.verified_digest(image, version, Some(&datastore))
        .await
        .map_err(|error| {
            let reason = match error.downcast_ref::<ImageVerificationError>() {
                Some(ImageVerificationError::TrustRootUnavailable) => "trust_root_unavailable",
                Some(ImageVerificationError::AttestationUnavailable) => "attestation_unavailable",
                Some(ImageVerificationError::AttestationInvalid) => "attestation_invalid",
                None => error
                    .downcast_ref::<docker::CheckFailure>()
                    .map(|failure| failure.code())
                    .unwrap_or("image_resolution_failed"),
            };
            Failure::new(stage, reason).into()
        })
}

fn report_failure(
    root: &Path,
    target: &str,
    error: &anyhow::Error,
    stage: &'static str,
) -> Result<Failure> {
    let failure = Failure::classify(error, stage);
    // Preserve a rollback's terminal phase and use its reason in both channels.
    if stage == "bootstrap"
        && let Some(mut progress) = read(root, "progress.json", 4096)
            .ok()
            .and_then(|data| serde_json::from_slice::<Progress>(&data).ok())
            .filter(|p| p.target == target && p.phase == Phase::RolledBack)
    {
        let failure = Failure::new(stage, "replacement_did_not_reconnect");
        progress.code = Some(failure.code());
        report(root, &progress)?;
        return Ok(failure);
    }
    report(
        root,
        &Progress {
            target: target.into(),
            phase: Phase::Failed,
            started_at_ms: now_ms(),
            code: Some(failure.code()),
        },
    )?;
    Ok(failure)
}

/// Read-only production preflight, also useful to diagnose registry/provenance
/// access without replacing a machine. Uses exactly the bootstrap/watch store.
#[allow(dead_code)]
pub async fn verify(root: PathBuf, version: String) -> Result<()> {
    let _lock = lock(&root)?;
    update::version(&version).map_err(anyhow::Error::msg)?;
    let api = docker::Docker::new()?;
    let result = async {
        verified_image(&api, &root, update::UPDATER_IMAGE, &version).await?;
        verified_image(&api, &root, update::MACHINE_IMAGE, &version).await?;
        Ok(())
    }
    .await;
    if let Err(error) = &result {
        return Err(report_failure(&root, &version, error, "verify_images")?.into());
    } else {
        println!("machine_update verified updater and machine image provenance");
    }
    result
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn private_directory(root: &Path) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if !root.exists() {
        std::fs::create_dir_all(root)?;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
    }
    let meta = std::fs::symlink_metadata(root)?;
    if meta.is_dir()
        && !meta.file_type().is_symlink()
        && meta.uid() == unsafe { libc::geteuid() }
        && meta.mode() & 0o022 == 0
    {
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
    }
    let meta = std::fs::symlink_metadata(root)?;
    ensure!(
        meta.is_dir()
            && !meta.file_type().is_symlink()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0,
        "Updater mailbox is not private"
    );
    Ok(())
}

pub(crate) fn lock(root: &Path) -> Result<std::fs::File> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    private_directory(root)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join("controller.lock"))?;
    ensure!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "Another updater owns this mailbox"
    );
    Ok(file)
}

pub fn write(root: &Path, name: &str, value: &[u8]) -> Result<()> {
    use std::io::Write;
    private_directory(root)?;
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    file.write_all(value)?;
    file.as_file().sync_all()?;
    file.persist(root.join(name))?;
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}

pub fn read(root: &Path, name: &str, limit: u64) -> Result<Vec<u8>> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    private_directory(root)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(root.join(name))?;
    ensure!(file.metadata()?.is_file(), "Updater input is not a file");
    let mut value = Vec::new();
    file.take(limit + 1).read_to_end(&mut value)?;
    ensure!(value.len() as u64 <= limit, "Updater input exceeds limit");
    Ok(value)
}

pub fn report(root: &Path, progress: &Progress) -> Result<()> {
    write(root, "progress.json", &serde_json::to_vec(progress)?)
}

pub fn heartbeat(root: &Path) -> Result<()> {
    write(root, "heartbeat", b"")
}

pub fn request(root: &Path) -> Result<Option<(String, bool)>> {
    for (name, rollback) in [("request", false), ("rollback-request", true)] {
        if root.join(name).exists() {
            let value = read(root, name, 80)?;
            let target = String::from_utf8(value)?;
            update::version(&target).map_err(anyhow::Error::msg)?;
            return Ok(Some((target, rollback)));
        }
    }
    Ok(None)
}

pub fn connected(root: &Path, target: &str, since: u64) -> bool {
    read(root, "connected.json", 4096)
        .ok()
        .and_then(|v| serde_json::from_slice::<Connected>(&v).ok())
        .is_some_and(|v| v.version == target && v.at_ms > since)
}

#[derive(Clone, Serialize, Deserialize)]
struct Journal {
    original_id: String,
    original_name: String,
    new_id: Option<String>,
    networks: Value,
    progress: Progress,
}

fn save(root: &Path, journal: &Journal) -> Result<()> {
    write(root, "journal.json", &serde_json::to_vec(journal)?)
}

pub(crate) fn finish_request(root: &Path, journal_name: &str) -> Result<()> {
    for name in ["request", "rollback-request", journal_name] {
        match std::fs::remove_file(root.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        // Requests must be durably gone before deleting the recovery journal.
        std::fs::File::open(root)?.sync_all()?;
    }
    Ok(())
}

async fn finish_committed(api: &docker::Docker, root: &Path, journal: &Journal) -> Result<()> {
    report(root, &journal.progress)?;
    api.remove(&journal.original_id).await?;
    finish_request(root, "journal.json")
}

async fn recover(api: &docker::Docker, root: &Path, journal: &Journal) -> Result<()> {
    match journal.progress.phase {
        Phase::Connected => finish_committed(api, root, journal).await,
        Phase::RolledBack => {
            report(root, &journal.progress)?;
            finish_request(root, "journal.json")
        }
        _ => rollback(api, root, journal).await,
    }
}

async fn recover_pending(api: &docker::Docker, root: &Path, name: &str) -> Result<()> {
    if root.join("journal.json").exists() {
        let journal: Journal = serde_json::from_slice(&read(root, "journal.json", 16384)?)?;
        ensure!(
            journal.original_name == name,
            "Updater journal belongs to another machine"
        );
        recover(api, root, &journal).await?;
    }
    Ok(())
}

async fn rollback(api: &docker::Docker, root: &Path, journal: &Journal) -> Result<()> {
    // Look up by name too: a crash may occur after create but before journaling its id.
    if let Ok(current) = api.inspect(&journal.original_name).await {
        let id = current["Id"]
            .as_str()
            .context("Missing container identity")?;
        if id != journal.original_id {
            api.remove(id).await?;
        }
    }
    let original = api.inspect(&journal.original_id).await?;
    if original["Name"] != format!("/{}", journal.original_name) {
        api.rename(&journal.original_id, &journal.original_name)
            .await?;
    }
    api.restore_networks(&journal.original_id, &journal.networks)
        .await?;
    api.start(&journal.original_id).await?;
    let mut journal = journal.clone();
    journal.progress.phase = Phase::RolledBack;
    journal.progress.code = Some("replacement_did_not_reconnect".into());
    save(root, &journal)?;
    report(root, &journal.progress)?;
    finish_request(root, "journal.json")
}

/// A replacement is committed only after both Docker health and NyxID's
/// authenticated WebSocket reconnection marker agree on the requested version.
pub async fn replace(
    api: &docker::Docker,
    root: &Path,
    name: &str,
    target: &str,
    owner_rollback: bool,
    migration: bool,
) -> Result<()> {
    let inspect = api.inspect(name).await?;
    docker::validate_source(&inspect, name, migration)?;
    let current = docker::source_version(&inspect)?;
    docker::validate_target(current, target, owner_rollback)?;
    let mut progress = Progress {
        target: target.into(),
        phase: Phase::Verifying,
        started_at_ms: now_ms(),
        code: None,
    };
    report(root, &progress)?;
    let digest = verified_image(api, root, update::MACHINE_IMAGE, target).await?;
    progress.phase = Phase::Downloading;
    report(root, &progress)?;
    api.pull(&format!("{}@{digest}", update::MACHINE_IMAGE))
        .await
        .map_err(|error| Failure::classify(&error, "download_image"))?;
    let config = docker::replacement_config(&inspect, &digest, name, target, migration)?;
    let mut journal = Journal {
        original_id: inspect["Id"]
            .as_str()
            .context("Missing container id")?
            .into(),
        original_name: name.into(),
        new_id: None,
        networks: config["NetworkingConfig"]["EndpointsConfig"].clone(),
        progress: progress.clone(),
    };
    save(root, &journal)?;
    let result: Result<()> = async {
        api.stop(&journal.original_id).await?;
        api.disconnect_networks(&journal.original_id, &journal.networks)
            .await?;
        api.rename(
            &journal.original_id,
            &format!("{name}-nyxid-rollback-{}", progress.started_at_ms),
        )
        .await?;
        let id = api.create(name, &config).await?;
        journal.new_id = Some(id.clone());
        journal.progress.phase = Phase::Restarting;
        save(root, &journal)?;
        report(root, &journal.progress)?;
        api.start(&id).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(update::RECONNECT_SECONDS);
        loop {
            heartbeat(root)?;
            let state = api.inspect(&id).await?;
            if state["State"]["Running"] != true {
                bail!("Replacement exited");
            }
            if connected(root, target, progress.started_at_ms)
                && state["State"]["Health"]["Status"] == "healthy"
            {
                break;
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "Replacement did not reconnect"
            );
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        #[cfg(test)]
        eprintln!("Replacement test diagnostic: {error:#}");
        #[cfg(not(test))]
        let _ = error;
        rollback(api, root, &journal).await?;
        return Ok(());
    }
    // Commit before destroying the rollback copy. Recovery finishes cleanup and
    // never restores an old image after authenticated health has been committed.
    journal.progress.phase = Phase::Connected;
    save(root, &journal)?;
    finish_committed(api, root, &journal).await
}

pub(crate) struct Heartbeat(tokio::task::JoinHandle<()>);
impl Drop for Heartbeat {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub(crate) fn heartbeat_task(root: &Path) -> Heartbeat {
    let root = root.to_owned();
    Heartbeat(tokio::spawn(async move {
        loop {
            let _ = heartbeat(&root);
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    }))
}

pub async fn run(root: PathBuf, name: String) -> Result<()> {
    private_directory(&root)?;
    let _lock = lock(&root)?;
    let _heartbeat = heartbeat_task(&root);
    docker::valid_name(&name)?;
    let api = docker::Docker::new()?;
    recover_pending(&api, &root, &name).await?;
    loop {
        heartbeat(&root)?;
        match request(&root) {
            Ok(Some((target, rollback))) => {
                if let Err(error) = replace(&api, &root, &name, &target, rollback, false).await {
                    ensure!(
                        !root.join("journal.json").exists(),
                        "Update recovery pending"
                    );
                    eprintln!("{}", report_failure(&root, &target, &error, "watch")?);
                }
                for file in ["request", "rollback-request"] {
                    let _ = std::fs::remove_file(root.join(file));
                }
            }
            Err(_) => {
                for file in ["request", "rollback-request"] {
                    let _ = std::fs::remove_file(root.join(file));
                }
                eprintln!("machine_update invalid_request");
            }
            Ok(None) => {}
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Explicit owner-run migration. Existing identity/workspace/config are inspected
/// through Docker and passed back to Docker, never printed or returned to an agent.
#[allow(dead_code)] // Used by the companion binary; native CLI shares this module.
pub async fn bootstrap(root: PathBuf, name: String, version: String) -> Result<()> {
    bootstrap_with_api(root, name, version, docker::Docker::new()?)
        .await
        .map_err(|error| Failure::classify(&error, "bootstrap").into())
}

async fn bootstrap_with_api(
    root: PathBuf,
    name: String,
    version: String,
    api: docker::Docker,
) -> Result<()> {
    private_directory(&root)?;
    let _lock = lock(&root)?;
    let _heartbeat = heartbeat_task(&root);
    // Never persist arbitrary command-line input as a release identifier.
    let reported_target = if update::version(&version).is_ok() {
        version.as_str()
    } else {
        ""
    };
    report(
        &root,
        &Progress {
            target: reported_target.into(),
            phase: Phase::Verifying,
            started_at_ms: now_ms(),
            code: None,
        },
    )?;
    let result = async {
        docker::valid_name(&name)?;
        update::version(&version).map_err(|_| docker::CheckFailure::InvalidTargetVersion)?;
        bootstrap_locked(&root, &name, &version, &api).await
    }
    .await;
    if let Err(error) = &result {
        return Err(report_failure(&root, reported_target, error, "bootstrap")?.into());
    }
    // Release the controller lock before starting the watch process.
    drop(_lock);
    if let Some(companion) = result?
        && let Err(error) = api.start(&companion).await
    {
        return Err(report_failure(&root, &version, &error, "install_companion")?.into());
    }
    println!("Machine updated; identity and workspace retained; automatic updater installed.");
    Ok(())
}

async fn bootstrap_locked(
    root: &Path,
    name: &str,
    version: &str,
    api: &docker::Docker,
) -> Result<Option<String>> {
    recover_pending(api, root, name).await?;
    let inspect = api.inspect(name).await?;
    docker::validate_source(&inspect, name, true)?;
    let current = docker::source_version(&inspect)?;
    docker::validate_target(current, version, false)?;
    let companion = format!("{name}-updater");
    let existing_companion = match api.inspect(&companion).await {
        Ok(existing) => Some(existing),
        Err(error)
            if matches!(
                error.downcast_ref::<docker::CheckFailure>(),
                Some(docker::CheckFailure::ContainerNotFound)
            ) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    if let Some(existing) = &existing_companion {
        ensure!(
            existing["Config"]["Labels"]["dev.nyxid.machine.updater"] == name,
            docker::CheckFailure::CompanionNameTaken
        );
    }
    let mounted = inspect["Mounts"].as_array().and_then(|mounts| {
        mounts
            .iter()
            .find(|m| m["Destination"] == update::UPDATE_VOLUME)
    });
    if let Some(mount) = mounted {
        ensure!(
            mount["Type"] == "volume"
                && mount["Name"] == format!("{name}-nyxid-update")
                && inspect["Config"]["Labels"][update::CONTAINER_LABEL] == name,
            docker::CheckFailure::UpdateVolumeMismatch
        );
    }
    let updater_digest = verified_image(api, root, update::UPDATER_IMAGE, version).await?;
    api.pull(&format!("{}@{updater_digest}", update::UPDATER_IMAGE))
        .await
        .map_err(|error| Failure::classify(&error, "download_image"))?;
    if mounted.is_none() || current != version {
        replace(api, root, name, version, false, mounted.is_none()).await?;
        let progress: Progress = serde_json::from_slice(&read(root, "progress.json", 4096)?)?;
        ensure!(progress.phase == Phase::Connected, "Migration rolled back");
    }
    if existing_companion.is_some() {
        return Ok(Some(companion));
    }
    let config = json!({
        "Image":format!("{}@{updater_digest}",update::UPDATER_IMAGE),
        "Cmd":["watch",name],
        "Labels":{"dev.nyxid.machine.updater":name},
        "HostConfig":{
            "ReadonlyRootfs":true,"RestartPolicy":{"Name":"unless-stopped"},
            "CapDrop":["ALL"],"SecurityOpt":["no-new-privileges:true"],
            "Binds":["/var/run/docker.sock:/var/run/docker.sock",format!("{name}-nyxid-update:{}",update::UPDATE_VOLUME)],
            "Tmpfs":{"/tmp":"rw,noexec,nosuid,size=16m"}
        }
    });
    let id = api.create(&companion, &config).await?;
    Ok(Some(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[test]
    fn trust_store_is_private_reusable_and_cannot_follow_a_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let guard = lock(root.path()).unwrap();
        let path = trust_datastore(root.path()).unwrap();
        assert_eq!(path, root.path().join("tuf"));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::write(path.join("marker"), "retained").unwrap();
        assert_eq!(trust_datastore(root.path()).unwrap(), path);
        assert!(path.join("marker").exists());
        assert!(
            lock(root.path()).is_err(),
            "bootstrap and watch must serialize"
        );
        drop(guard);
        let _next = lock(root.path()).unwrap();
        std::fs::remove_dir_all(&path).unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), &path).unwrap();
        assert!(trust_datastore(root.path()).is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[test]
    fn failure_output_and_progress_contain_only_fixed_diagnostics() {
        let root = tempfile::tempdir().unwrap();
        for (error, expected) in [
            (
                anyhow::anyhow!("SECRET_DOCKER_ENV_AND_CREDENTIAL"),
                "bootstrap:operation_failed",
            ),
            (
                anyhow::anyhow!("SECRET_UPSTREAM_BODY").context(Failure::new(
                    "verify_updater_image",
                    "trust_root_unavailable",
                )),
                "verify_updater_image:trust_root_unavailable",
            ),
        ] {
            let reported = report_failure(root.path(), "0.41.0", &error, "bootstrap").unwrap();
            let bytes = read(root.path(), "progress.json", 4096).unwrap();
            let progress: Progress = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(progress.phase, Phase::Failed);
            assert_eq!(progress.code.as_deref(), Some(expected));
            // Main classifies with startup, but must retain exactly what was
            // persisted. report_failure does not print: the caller prints once.
            assert_eq!(
                Failure::classify(&reported.into(), "startup").code(),
                expected
            );
            assert!(update::failure_guidance(expected).is_some());
            assert!(!String::from_utf8(bytes).unwrap().contains("SECRET"));
            assert!(
                !Failure::classify(&error, "bootstrap")
                    .to_string()
                    .contains("SECRET")
            );
        }
        report(
            root.path(),
            &Progress {
                target: "0.41.0".into(),
                phase: Phase::RolledBack,
                started_at_ms: 1,
                code: Some("replacement_did_not_reconnect".into()),
            },
        )
        .unwrap();
        let failure = report_failure(
            root.path(),
            "0.41.0",
            &anyhow::anyhow!("Migration rolled back"),
            "bootstrap",
        )
        .unwrap();
        let progress: Progress =
            serde_json::from_slice(&read(root.path(), "progress.json", 4096).unwrap()).unwrap();
        assert_eq!(progress.phase, Phase::RolledBack);
        assert_eq!(progress.code.as_deref(), Some(failure.code().as_str()));
        assert_eq!(failure.reason, "replacement_did_not_reconnect");
    }

    #[tokio::test]
    async fn bootstrap_checks_persist_actionable_codes_before_any_mutation() {
        for reason in [
            "invalid_container_name",
            "container_not_found",
            "source_not_official_image",
            "source_auto_remove",
            "source_name_changed",
            "source_version_unknown",
            "companion_name_taken",
            "update_volume_mismatch",
            "downgrade_refused",
            "invalid_target_version",
            "operation_failed",
        ] {
            let root = tempfile::tempdir().unwrap();
            let server = MockServer::start().await;
            let mut source = original();
            let mut name = "test-machine";
            let mut target = "0.41.1";
            let mut status = 200;
            match reason {
                "invalid_container_name" => name = "SECRET/invalid",
                "container_not_found" => status = 404,
                "source_not_official_image" => source["Config"]["Image"] = json!("SECRET/image"),
                "source_auto_remove" => source["HostConfig"]["AutoRemove"] = json!(true),
                "source_name_changed" => source["Name"] = json!("/SECRET-renamed"),
                "source_version_unknown" => {
                    source["Config"]["Image"] = json!(format!(
                        "{}@sha256:{}",
                        update::MACHINE_IMAGE,
                        "a".repeat(64)
                    ))
                }
                "update_volume_mismatch" => {
                    source["Mounts"] = json!([{
                        "Destination": update::UPDATE_VOLUME,
                        "Type": "volume",
                        "Name": "SECRET-other",
                    }])
                }
                "downgrade_refused" => target = "0.39.0",
                "invalid_target_version" => target = "not-a-version",
                _ => {}
            }
            Mock::given(method("GET"))
                .and(path("/v1.45/containers/test-machine/json"))
                .respond_with(ResponseTemplate::new(status).set_body_json(source))
                .mount(&server)
                .await;
            let companion_status = match reason {
                "companion_name_taken" => 200,
                "operation_failed" => 500,
                _ => 404,
            };
            Mock::given(method("GET"))
                .and(path("/v1.45/containers/test-machine-updater/json"))
                .respond_with(
                    ResponseTemplate::new(companion_status).set_body_json(json!({
                        "Config": {
                            "Labels": {"dev.nyxid.machine.updater": "SECRET-other"},
                        },
                    })),
                )
                .mount(&server)
                .await;
            let api = docker::Docker::fixture(server.uri(), Err("must not verify or mutate"));
            let error = bootstrap_with_api(root.path().into(), name.into(), target.into(), api)
                .await
                .unwrap_err();
            let failure = Failure::classify(&error, "startup");
            assert_eq!(failure.code(), format!("bootstrap:{reason}"));
            assert!(!failure.to_string().contains("SECRET"));
            let progress: Progress =
                serde_json::from_slice(&read(root.path(), "progress.json", 4096).unwrap()).unwrap();
            assert_eq!(progress.code.as_deref(), Some(failure.code().as_str()));
            assert!(update::failure_guidance(&failure.code()).is_some());
            assert!(
                server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .all(|r| r.method == "GET")
            );
        }
    }

    #[tokio::test]
    async fn docker_connection_failure_is_not_a_missing_container_or_raw_error() {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        drop(socket);
        let root = tempfile::tempdir().unwrap();
        let api = docker::Docker::fixture(format!("http://{address}"), Err("must not verify"));
        let error = bootstrap_with_api(root.path().into(), "machine".into(), "0.41.1".into(), api)
            .await
            .unwrap_err();
        assert_eq!(
            Failure::classify(&error, "startup").code(),
            "bootstrap:docker_socket_unavailable"
        );
        let progress: Progress =
            serde_json::from_slice(&read(root.path(), "progress.json", 4096).unwrap()).unwrap();
        assert_eq!(
            progress.code.as_deref(),
            Some("bootstrap:docker_socket_unavailable")
        );
    }

    #[tokio::test]
    async fn bootstrap_preflight_failure_is_reported_before_replacement() {
        let root = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1.45/containers/test-machine/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(original()))
            .mount(&server)
            .await;
        let api = docker::Docker::fixture(server.uri(), Err("SECRET_VERIFICATION_DETAIL"));
        let error = bootstrap_with_api(
            root.path().into(),
            "test-machine".into(),
            "0.41.0".into(),
            api,
        )
        .await
        .unwrap_err();
        assert_eq!(
            Failure::classify(&error, "bootstrap").code(),
            "verify_updater_image:image_resolution_failed"
        );
        let progress: Progress =
            serde_json::from_slice(&read(root.path(), "progress.json", 4096).unwrap()).unwrap();
        assert_eq!(
            progress.code.as_deref(),
            Some("verify_updater_image:image_resolution_failed")
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
    }

    fn original() -> Value {
        json!({"Id":"old","Name":"/test-machine","Config":{"Image":format!("{}:0.40.0",update::MACHINE_IMAGE),"Env":["NYXID_NODE_URL=wss://example.test","SECRET=kept-not-logged"],"Labels":{"dev.nyxid.machine":"test-machine"}},"HostConfig":{"Binds":["identity:/identity"],"ShmSize":1073741824},"Mounts":[],"NetworkSettings":{"Networks":{}}})
    }
    async fn fixtures(server: &MockServer, root: &Path, healthy: bool) {
        let calls = Arc::new(AtomicUsize::new(0));
        Mock::given(method("GET"))
            .and(path("/v1.45/containers/test-machine/json"))
            .respond_with(move |_: &wiremock::Request| {
                ResponseTemplate::new(200).set_body_json(
                    if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                        original()
                    } else {
                        json!({"Id":"new"})
                    },
                )
            })
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1.45/containers/old/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(original()))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1.45/images/create"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"status\":\"done\"}\n"))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1.45/containers/create"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"Id":"new"})))
            .expect(1)
            .mount(server)
            .await;
        for (method_name, endpoint) in [
            ("POST", "/v1.45/containers/old/stop"),
            ("POST", "/v1.45/containers/old/rename"),
            ("POST", "/v1.45/containers/new/start"),
            ("POST", "/v1.45/containers/old/start"),
            ("DELETE", "/v1.45/containers/new"),
            ("DELETE", "/v1.45/containers/old"),
        ] {
            Mock::given(method(method_name))
                .and(path(endpoint))
                .respond_with(ResponseTemplate::new(204))
                .mount(server)
                .await;
        }
        let root = root.to_owned();
        Mock::given(method("GET")).and(path("/v1.45/containers/new/json")).respond_with(move |_:&wiremock::Request| {
            if healthy {write(&root,"connected.json",&serde_json::to_vec(&Connected{version:"0.41.0".into(),at_ms:now_ms()+100}).unwrap()).unwrap();}
            ResponseTemplate::new(200).set_body_json(json!({"State":{"Running":healthy,"Health":{"Status":if healthy{"healthy"}else{"unhealthy"}}}}))
        }).mount(server).await;
    }
    #[tokio::test]
    async fn replacement_waits_for_health_and_reconnect_and_preserves_identity_configuration() {
        let root = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        fixtures(&server, root.path(), true).await;
        let api = docker::Docker::fixture(server.uri(), Ok(format!("sha256:{}", "a".repeat(64))));
        replace(&api, root.path(), "test-machine", "0.41.0", false, false)
            .await
            .unwrap();
        let progress: Progress =
            serde_json::from_slice(&read(root.path(), "progress.json", 4096).unwrap()).unwrap();
        assert_eq!(progress.phase, Phase::Connected);
        assert!(!root.path().join("journal.json").exists());
        let requests = server.received_requests().await.unwrap();
        let create = requests
            .iter()
            .find(|r| r.url.path() == "/v1.45/containers/create")
            .unwrap();
        let config: Value = serde_json::from_slice(&create.body).unwrap();
        assert_eq!(config["Env"], original()["Config"]["Env"]);
        assert_eq!(config["HostConfig"], original()["HostConfig"]);
        assert!(requests.iter().any(|r| r.method == "DELETE"
            && r.url.path() == "/v1.45/containers/old"
            && r.url.query().unwrap().contains("v=false")));
        assert!(!serde_json::to_string(&progress).unwrap().contains("SECRET"));
        server.verify().await;
    }
    #[tokio::test]
    async fn failed_replacement_restores_retained_container_without_removing_volumes() {
        let root = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        fixtures(&server, root.path(), false).await;
        let api = docker::Docker::fixture(server.uri(), Ok(format!("sha256:{}", "a".repeat(64))));
        replace(&api, root.path(), "test-machine", "0.41.0", false, false)
            .await
            .unwrap();
        let progress: Progress =
            serde_json::from_slice(&read(root.path(), "progress.json", 4096).unwrap()).unwrap();
        assert_eq!(progress.phase, Phase::RolledBack);
        let requests = server.received_requests().await.unwrap();
        assert!(
            requests
                .iter()
                .any(|r| r.url.path() == "/v1.45/containers/old/start")
        );
        assert!(
            !requests
                .iter()
                .any(|r| r.method == "DELETE" && r.url.path() == "/v1.45/containers/old")
        );
        server.verify().await;
    }
    #[tokio::test]
    async fn failed_attestation_or_downgrade_never_stops_the_running_container() {
        for (target, verification) in [
            ("0.41.0", Err("invalid attestation")),
            ("0.39.0", Ok(format!("sha256:{}", "a".repeat(64)))),
        ] {
            let root = tempfile::tempdir().unwrap();
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1.45/containers/test-machine/json"))
                .respond_with(ResponseTemplate::new(200).set_body_json(original()))
                .expect(1)
                .mount(&server)
                .await;
            let api = docker::Docker::fixture(server.uri(), verification);
            assert!(
                replace(&api, root.path(), "test-machine", target, false, false)
                    .await
                    .is_err()
            );
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn committed_crash_recovery_never_rolls_back_or_replays_the_request() {
        for old_exists in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let server = MockServer::start().await;
            Mock::given(method("DELETE"))
                .and(path("/v1.45/containers/old"))
                .respond_with(ResponseTemplate::new(if old_exists { 204 } else { 404 }))
                .expect(1)
                .mount(&server)
                .await;
            let journal = Journal {
                original_id: "old".into(),
                original_name: "test-machine".into(),
                new_id: Some("new".into()),
                networks: json!({}),
                progress: Progress {
                    target: "0.41.0".into(),
                    phase: Phase::Connected,
                    started_at_ms: 1,
                    code: None,
                },
            };
            save(root.path(), &journal).unwrap();
            write(root.path(), "request", b"0.41.0").unwrap();
            let api = docker::Docker::fixture(server.uri(), Err("must not verify again"));
            recover_pending(&api, root.path(), "test-machine")
                .await
                .unwrap();
            assert!(request(root.path()).unwrap().is_none());
            assert!(!root.path().join("journal.json").exists());
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
            let progress: Progress =
                serde_json::from_slice(&read(root.path(), "progress.json", 4096).unwrap()).unwrap();
            assert_eq!(progress.phase, Phase::Connected);
        }
    }

    #[tokio::test]
    async fn bootstrap_installs_a_restricted_companion_for_the_verified_machine() {
        let root = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let mut source = original();
        source["Config"]["Image"] = json!(format!("{}:0.41.0", update::MACHINE_IMAGE));
        source["Mounts"] = json!([{"Type":"volume", "Name":"test-machine-nyxid-update", "Destination": update::UPDATE_VOLUME}]);
        Mock::given(method("GET"))
            .and(path("/v1.45/containers/test-machine/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(source))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1.45/images/create"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"status\":\"done\"}\n"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1.45/containers/create"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"Id":"helper"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1.45/containers/helper/start"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let digest = format!("sha256:{}", "a".repeat(64));
        let api = docker::Docker::fixture(server.uri(), Ok(digest.clone()));
        bootstrap_with_api(
            root.path().into(),
            "test-machine".into(),
            "0.41.0".into(),
            api,
        )
        .await
        .unwrap();
        let requests = server.received_requests().await.unwrap();
        let create = requests
            .iter()
            .find(|r| r.url.path() == "/v1.45/containers/create")
            .unwrap();
        let config: Value = serde_json::from_slice(&create.body).unwrap();
        assert_eq!(
            config["Image"],
            format!("{}@{digest}", update::UPDATER_IMAGE)
        );
        assert_eq!(config["Cmd"], json!(["watch", "test-machine"]));
        assert_eq!(config["HostConfig"]["ReadonlyRootfs"], true);
        assert_eq!(config["HostConfig"]["CapDrop"], json!(["ALL"]));
        assert_eq!(
            config["HostConfig"]["SecurityOpt"],
            json!(["no-new-privileges:true"])
        );
        assert_eq!(
            config["HostConfig"]["Binds"],
            json!([
                "/var/run/docker.sock:/var/run/docker.sock",
                format!("test-machine-nyxid-update:{}", update::UPDATE_VOLUME)
            ])
        );
        // Bootstrap releases the lease before starting the long-running helper.
        assert!(lock(root.path()).is_ok());
        server.verify().await;
    }
}

#[cfg(test)]
mod container_e2e {
    use super::*;
    use reqwest::Method;

    async fn status(client: &reqwest::Client) -> Result<Value> {
        Ok(client
            .get("http://127.0.0.1:33443/health")
            .send()
            .await?
            .json()
            .await?)
    }
    async fn call(client: &reqwest::Client, op: &str, params: Value) -> Result<Value> {
        Ok(client
            .post("http://127.0.0.1:33443/call")
            .json(&json!({"operation":op,"parameters":params}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    async fn ready(client: &reqwest::Client, connections: u64) -> Result<Value> {
        let end = tokio::time::Instant::now() + Duration::from_secs(100);
        loop {
            let s = status(client).await?;
            if s["connections"].as_u64().unwrap_or(0) >= connections
                && s["machine"]["computer_ready"] == true
            {
                return Ok(s);
            }
            ensure!(
                tokio::time::Instant::now() < end,
                "Fixture machine did not become ready"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    async fn exec_fixture(api: &docker::Docker, name: &str, command: Vec<&str>) -> Result<()> {
        let exec = api
            .call(
                Method::POST,
                &format!("/containers/{name}/exec"),
                Some(&json!({"Cmd":command,"AttachStdout":false,"AttachStderr":false})),
            )
            .await?;
        let id = exec["Id"].as_str().context("fixture exec id")?;
        api.call(
            Method::POST,
            &format!("/exec/{id}/start"),
            Some(&json!({"Detach":true})),
        )
        .await?;
        for _ in 0..100 {
            let state = api
                .call(Method::GET, &format!("/exec/{id}/json"), None)
                .await?;
            if state["Running"] == false {
                ensure!(
                    state["ExitCode"] == 0,
                    "fixture command failed (exit {})",
                    state["ExitCode"]
                );
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        bail!("fixture command timed out")
    }

    async fn secure_health(client: &reqwest::Client) -> Result<()> {
        let before = status(client).await?["loginInputs"].as_u64().unwrap_or(0);
        let page = call(
            client,
            "browser",
            json!({"action":"navigate","url":"https://127.0.0.1:33444/login"}),
        )
        .await?;
        ensure!(
            page["status"] == "ok" && page.to_string().contains("Saved login recovery"),
            "Secure browser unavailable: {page}"
        );
        for field in ["username", "password"] {
            let snapshot = call(client, "browser", json!({"action":"snapshot"})).await?;
            let label = if field == "username" {
                "Username"
            } else {
                "Password"
            };
            let target = snapshot["snapshot"]["elements"]
                .as_array()
                .context("login elements")?
                .iter()
                .find(|element| element["label"] == label)
                .context("login field ref")?;
            let clicked = call(
                client,
                "browser",
                json!({"action":"click","ref":target["ref"]}),
            )
            .await?;
            ensure!(
                clicked["status"] == "ok",
                "Login field focus failed: {clicked}"
            );
            let value = format!("fixture-{}-{field}", now_ms());
            let filled = call(
                client,
                "fill_login",
                json!({"field":field,"allowed_origins":["https://127.0.0.1:33444"],"value":value}),
            )
            .await?;
            ensure!(
                filled["status"] == "filled",
                "Saved login {field} unavailable: {filled}"
            );
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        ensure!(
            status(client).await?["loginInputs"].as_u64().unwrap_or(0) >= before + 2,
            "Trusted saved-login events missing"
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "real Docker migration; cli/tests/machine_updater_e2e.sh"]
    async fn real_container_migration_signed_upgrade_and_rollback() -> Result<()> {
        let name = std::env::var("NYXID_TEST_MACHINE")?;
        let driver = std::env::var("NYXID_TEST_DRIVER")?;
        let image = std::env::var("NYXID_TEST_IMAGE")?;
        let target = std::env::var("NYXID_TEST_TARGET_VERSION")?;
        update::version(&target).map_err(anyhow::Error::msg)?;
        ensure!(
            name.starts_with("nyxid-update-e2e-"),
            "Test container namespace required"
        );
        let root = Path::new(update::UPDATE_VOLUME);
        let mut api = docker::Docker::new()?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(50))
            .build()?;
        let seccomp = std::fs::read_to_string("/test/seccomp.json")?;
        let config = json!({
            "Image":format!("{}:0.40.0",update::MACHINE_IMAGE),
            "Env":["NYXID_NODE_URL=ws://127.0.0.1:33443", "NYXID_NODE_TOKEN=fixture-one-use", "CUSTOM=preserved"],
            "Cmd":["--machine","--computer"],"Labels":{"test":"preserved"},
            "HostConfig":{"NetworkMode":format!("container:{driver}"),"ShmSize":268435456,
                "RestartPolicy":{"Name":"unless-stopped"},"SecurityOpt":[format!("seccomp={seccomp}")],
                "Binds":[format!("{name}-identity:/var/lib/nyxid-machine"),format!("{name}-workspace:/workspace"),format!("{name}-browser-trust:/home/browser/.pki")]}
        });
        let original = api.create(&name, &config).await?;
        api.start(&original).await?;
        let initial = ready(&client, 1).await?;
        // Populate the actual old Chromium profile and force-installed extension.
        for (tool, args) in [
            ("hotkey", json!({"keys":["CTRL","L"]})),
            ("type_text", json!({"text":"http://127.0.0.1:33443/page"})),
            ("press_key", json!({"key":"ENTER"})),
        ] {
            let mut args = args;
            args["target"] = json!({"kind":"desktop","display_id":"primary"});
            let r = call(&client, "computer", json!({"tool":tool,"arguments":args})).await?;
            ensure!(r.get("error").is_none(), "Old computer fixture unavailable");
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
        ensure!(
            status(&client).await?["pageVisits"].as_u64().unwrap_or(0) > 0,
            "Old browser did not load the fixture"
        );
        // Chromium batches its persistent cookie-store writes. Model an existing
        // signed-in profile, not a just-created cookie still only in process RAM.
        tokio::time::sleep(Duration::from_secs(35)).await;
        let before = api.inspect(&name).await?;
        api = docker::Docker::local_fixture(&image)?;
        replace(&api, root, &name, &target, false, true).await?;
        let progress: Progress = serde_json::from_slice(&read(root, "progress.json", 4096)?)?;
        ensure!(progress.phase == Phase::Connected, "Migration rolled back");
        let migrated = ready(&client, 2).await?;
        ensure!(
            migrated["id"] == initial["id"] && migrated["registrations"] == 1,
            "Node identity changed"
        );
        let after = api.inspect(&name).await?;
        for k in ["Env", "Cmd", "Entrypoint", "WorkingDir", "User"] {
            ensure!(
                after["Config"][k] == before["Config"][k],
                "Config changed: {k}"
            );
        }
        for k in ["SecurityOpt", "ShmSize", "RestartPolicy", "NetworkMode"] {
            ensure!(
                after["HostConfig"][k] == before["HostConfig"][k],
                "Host config changed: {k}"
            );
        }
        for mount in before["Mounts"].as_array().context("mounts")? {
            ensure!(after["Mounts"].as_array().context("mounts")?.iter().any(|m|m["Name"]==mount["Name"]&&m["Destination"]==mount["Destination"]),"Volume changed");
        }
        let snapshot = call(
            &client,
            "browser",
            json!({"action":"navigate","url":"http://127.0.0.1:33443/page"}),
        )
        .await?;
        ensure!(
            snapshot.get("error").is_none()
                && snapshot
                    .to_string()
                    .contains("Identity and profile retained"),
            "Old extension did not refresh: {snapshot}"
        );
        ensure!(
            status(&client).await?["cookieVisits"].as_u64().unwrap_or(0) > 0,
            "Browser cookie did not survive migration"
        );
        exec_fixture(
            &api,
            &name,
            vec!["chown", "-R", "browser:browser", "/home/browser/.pki"],
        )
        .await?;
        println!("Importing local test CA into migrated browser NSS store");
        exec_fixture(
            &api,
            &name,
            vec![
                "curl",
                "-fsS",
                "http://127.0.0.1:33443/ca",
                "-o",
                "/tmp/nyxid-fixture-ca.pem",
            ],
        )
        .await?;
        exec_fixture(&api, &name, vec!["runuser", "-u", "browser", "--", "sh", "-c",
            "mkdir -p /home/browser/.pki/nssdb; certutil -N --empty-password -d sql:/home/browser/.pki/nssdb; certutil -A -d sql:/home/browser/.pki/nssdb -n fixture -t C,, -i /tmp/nyxid-fixture-ca.pem"]).await?;
        // A real container restart (including Chromium/NSS) on the migrated profile.
        let connections = status(&client).await?["connections"].as_u64().unwrap();
        api.stop(&name).await?;
        api.start(&name).await?;
        ready(&client, connections + 1).await?;
        secure_health(&client).await?;
        println!("Persisted secure browser and saved logins after container restart: passed");
        for mode in ["plain", "hash", "missing"] {
            let script = format!(
                r#"
import os,signal,json,shutil
base='/var/lib/nyxid-machine/desktop'
for pid in os.listdir('/proc'):
 if not pid.isdigit(): continue
 try: args=open('/proc/'+pid+'/cmdline','rb').read().split(b'\0')
 except OSError: continue
 if ('--user-data-dir='+base+'/browser-profile').encode() in args and not any(a.startswith(b'--type=') for a in args): os.kill(int(pid),signal.SIGKILL)
mode='{mode}'
if mode=='hash': open(base+'/browser-run/extension-package-sha256','w').write('previous-package')
if mode=='missing':
 policy=json.load(open('/etc/chromium/policies/managed/nyxid.json'))
 extension=policy['ExtensionInstallForcelist'][0].split(';')[0]
 shutil.rmtree(base+'/browser-profile/Default/Extensions/'+extension,ignore_errors=True)
"#
            );
            exec_fixture(&api, &name, vec!["python3", "-c", &script]).await?;
            secure_health(&client).await?;
            println!("Migrated profile {mode} relaunch: secure action and saved login passed");
        }
        exec_fixture(&api, &name, vec!["python3", "-c", r#"
import os,subprocess
p='/var/lib/nyxid-machine/desktop/browser-run/filler.sock'
before=os.stat(p).st_ino
subprocess.run(['nyxid','node','machine','--config','/var/lib/nyxid-machine/node','status'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
assert os.stat(p).st_ino==before
"#]).await?;
        secure_health(&client).await?;
        let dev = call(
            &client,
            "browser",
            json!({"browser":"dev","action":"navigate","url":"http://127.0.0.1:33443/page"}),
        )
        .await?;
        ensure!(
            dev["status"] == "ok" && dev.to_string().contains("Identity and profile retained"),
            "Developer browser unavailable after migration: {dev}"
        );
        let live = status(&client).await?;
        let denied=call(&client,"exec",json!({"runtime_id":live["machine"]["runtime_id"],"job_id":"2ad5f1c8-3105-4985-88eb-2c7711be40d5","conversation_id":"fixture","command":"test ! -w /var/lib/nyxid-machine-update","cwd":"/workspace","services":[],"timeout_secs":10})).await?;
        ensure!(
            denied["exit_code"] == 0,
            "Agent update-volume boundary failed: {denied}"
        );
        heartbeat(root)?;
        let upgrade = call(
            &client,
            "upgrade",
            json!({"version":target,"automatic":false,"owner_rollback":false}),
        )
        .await?;
        ensure!(
            upgrade["accepted"] == true,
            "Signed upgrade refused: {upgrade}"
        );
        ensure!(
            read(root, "request", 80)? == target.as_bytes(),
            "Mailbox contains more than version"
        );
        let connections = status(&client).await?["connections"].as_u64().unwrap();
        replace(&api, root, &name, &target, false, false).await?;
        ready(&client, connections + 1).await?;
        secure_health(&client).await?;
        ensure!(
            request(root)?.is_none(),
            "Committed request must not replay"
        );
        let known = api.inspect(&name).await?["Id"].clone();
        let connections = status(&client).await?["connections"].as_u64().unwrap();
        api.test_failed_replacement = true;
        replace(&api, root, &name, &target, false, false).await?;
        let rolled: Progress = serde_json::from_slice(&read(root, "progress.json", 4096)?)?;
        ensure!(
            rolled.phase == Phase::RolledBack,
            "Failed replacement was not rolled back"
        );
        ensure!(
            api.inspect(&name).await?["Id"] == known,
            "Rollback lost old container"
        );
        ready(&client, connections + 1).await?;
        secure_health(&client).await?;
        ensure!(
            status(&client).await?["registrations"] == 1,
            "Rollback re-paired node"
        );
        // Cleanup is duplicated in the shell's EXIT trap for assertion failures.
        api.remove(&name).await?;
        for volume in [
            format!("{name}-identity"),
            format!("{name}-workspace"),
            format!("{name}-browser-trust"),
        ] {
            api.call(Method::DELETE, &format!("/volumes/{volume}"), None)
                .await?;
        }
        println!(
            "Real Docker: old profile refreshed; identity/cookies/config preserved; signed upgrade; agent mailbox refusal; rollback passed"
        );
        Ok(())
    }
}
