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
    update::validate_target(current, target, owner_rollback).map_err(anyhow::Error::msg)?;
    let mut progress = Progress {
        target: target.into(),
        phase: Phase::Verifying,
        started_at_ms: now_ms(),
        code: None,
    };
    report(root, &progress)?;
    let digest = api.verified_digest(update::MACHINE_IMAGE, target).await?;
    progress.phase = Phase::Downloading;
    report(root, &progress)?;
    api.pull(&format!("{}@{digest}", update::MACHINE_IMAGE))
        .await?;
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
                if replace(&api, &root, &name, &target, rollback, false)
                    .await
                    .is_err()
                {
                    ensure!(
                        !root.join("journal.json").exists(),
                        "Update recovery pending"
                    );
                    report(
                        &root,
                        &Progress {
                            target,
                            phase: Phase::Failed,
                            started_at_ms: now_ms(),
                            code: Some("verification_or_update_failed".into()),
                        },
                    )?;
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
    bootstrap_with_api(root, name, version, docker::Docker::new()?).await
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
    docker::valid_name(&name)?;
    update::version(&version).map_err(anyhow::Error::msg)?;
    recover_pending(&api, &root, &name).await?;
    let inspect = api.inspect(&name).await?;
    docker::validate_source(&inspect, &name, true)?;
    let companion = format!("{name}-updater");
    let existing_companion = api.inspect(&companion).await.ok();
    if let Some(existing) = &existing_companion {
        ensure!(
            existing["Config"]["Labels"]["dev.nyxid.machine.updater"] == name,
            "Companion name belongs to another container"
        );
    }
    let updater_digest = api.verified_digest(update::UPDATER_IMAGE, &version).await?;
    api.pull(&format!("{}@{updater_digest}", update::UPDATER_IMAGE))
        .await?;
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
            "Update volume does not belong to this labelled machine"
        );
    }
    if mounted.is_none() || docker::source_version(&inspect)? != version {
        replace(&api, &root, &name, &version, false, mounted.is_none()).await?;
        let progress: Progress = serde_json::from_slice(&read(&root, "progress.json", 4096)?)?;
        ensure!(progress.phase == Phase::Connected, "Migration rolled back");
    }
    if existing_companion.is_some() {
        drop(_lock);
        api.start(&companion).await?;
        return Ok(());
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
    drop(_lock);
    api.start(&id).await?;
    println!("Machine updated; identity and workspace retained; automatic updater installed.");
    Ok(())
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
    #[tokio::test]
    #[ignore = "real Docker migration; cli/tests/machine_updater_e2e.sh"]
    async fn real_container_migration_signed_upgrade_and_rollback() -> Result<()> {
        let name = std::env::var("NYXID_TEST_MACHINE")?;
        let driver = std::env::var("NYXID_TEST_DRIVER")?;
        let image = std::env::var("NYXID_TEST_IMAGE")?;
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
                "Binds":[format!("{name}-identity:/var/lib/nyxid-machine"),format!("{name}-workspace:/workspace")]}
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
        replace(&api, root, &name, "0.40.0", false, true).await?;
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
        let denied=call(&client,"exec",json!({"runtime_id":migrated["machine"]["runtime_id"],"job_id":"2ad5f1c8-3105-4985-88eb-2c7711be40d5","conversation_id":"fixture","command":"test ! -w /var/lib/nyxid-machine-update","cwd":"/workspace","services":[],"timeout_secs":10})).await?;
        ensure!(
            denied["exit_code"] == 0,
            "Agent update-volume boundary failed: {denied}"
        );
        heartbeat(root)?;
        let upgrade = call(
            &client,
            "upgrade",
            json!({"version":"0.40.0","automatic":false,"owner_rollback":false}),
        )
        .await?;
        ensure!(
            upgrade["accepted"] == true,
            "Signed upgrade refused: {upgrade}"
        );
        ensure!(
            read(root, "request", 80)? == b"0.40.0",
            "Mailbox contains more than version"
        );
        replace(&api, root, &name, "0.40.0", false, false).await?;
        ready(&client, 3).await?;
        ensure!(
            request(root)?.is_none(),
            "Committed request must not replay"
        );
        let known = api.inspect(&name).await?["Id"].clone();
        api.test_failed_replacement = true;
        replace(&api, root, &name, "0.40.0", false, false).await?;
        let rolled: Progress = serde_json::from_slice(&read(root, "progress.json", 4096)?)?;
        ensure!(
            rolled.phase == Phase::RolledBack,
            "Failed replacement was not rolled back"
        );
        ensure!(
            api.inspect(&name).await?["Id"] == known,
            "Rollback lost old container"
        );
        ready(&client, 4).await?;
        ensure!(
            status(&client).await?["registrations"] == 1,
            "Rollback re-paired node"
        );
        // Cleanup is duplicated in the shell's EXIT trap for assertion failures.
        api.remove(&name).await?;
        for volume in [format!("{name}-identity"), format!("{name}-workspace")] {
            api.call(Method::DELETE, &format!("/volumes/{volume}"), None)
                .await?;
        }
        println!(
            "Real Docker: old profile refreshed; identity/cookies/config preserved; signed upgrade; agent mailbox refusal; rollback passed"
        );
        Ok(())
    }
}
