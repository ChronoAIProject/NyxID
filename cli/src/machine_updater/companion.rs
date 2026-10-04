//! Fenced two-process handoff. Only controller.lock admits mailbox work. The
//! predecessor remains a watchdog until a successor heartbeat is committed.
//! handoff.lock serializes commit versus rollback; never wait for controller.lock
//! while holding it. The journal is durable before each Docker side effect.
use super::*;
use nyxid_machine::update::CompanionStatus;
use std::fs::File;

const JOURNAL: &str = "companion-journal.json";
const HANDOFF_MS: u64 = 60_000;
const POLL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Step {
    Prepared,
    Released,
    Healthy,
    Committed,
    RollingBack,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Swap {
    epoch: u64,
    machine: String,
    old_id: String,
    old_status: CompanionStatus,
    name: String,
    temporary: String,
    retired: String,
    successor_id: Option<String>,
    digest: String,
    image_id: String,
    target: String,
    step: Step,
    deadline_ms: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pulse {
    epoch: u64,
    id: String,
    digest: String,
    at_ms: u64,
}

fn failure(reason: &'static str) -> anyhow::Error {
    Failure::new("update_companion", reason).into()
}
fn classified(error: &anyhow::Error) -> Failure {
    let reason = Failure::classify(error, "update_companion").reason;
    Failure::new("update_companion", reason)
}

pub(super) fn status(root: &Path) -> Option<CompanionStatus> {
    read(root, "updater.json", 2048)
        .ok()
        .and_then(|v| serde_json::from_slice::<CompanionStatus>(&v).ok())
        .filter(CompanionStatus::valid)
}
fn publish(root: &Path, status: &CompanionStatus) -> Result<()> {
    ensure!(status.valid(), "Invalid companion metadata");
    write(root, "updater.json", &serde_json::to_vec(status)?)?;
    if let Ok(bytes) = read(root, "progress.json", 4096)
        && let Ok(progress) = serde_json::from_slice::<Progress>(&bytes)
    {
        report(root, &progress)?;
    }
    Ok(())
}
fn failed(
    root: &Path,
    current: &CompanionStatus,
    target: &str,
    error: &anyhow::Error,
) -> Result<()> {
    let failure = classified(error);
    publish(
        root,
        &CompanionStatus {
            phase: "failed".into(),
            target_version: Some(target.into()),
            code: Some(failure.code()),
            ..current.clone()
        },
    )?;
    eprintln!("{failure}");
    Ok(())
}
fn load(root: &Path) -> Result<Option<Swap>> {
    if !root.join(JOURNAL).exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&read(root, JOURNAL, 8192)?)?))
}
fn save(root: &Path, swap: &Swap) -> Result<()> {
    write(root, JOURNAL, &serde_json::to_vec(swap)?)
}
fn remove_file(root: &Path, name: &str) -> Result<()> {
    match std::fs::remove_file(root.join(name)) {
        Ok(()) => File::open(root)?.sync_all()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
async fn fence(root: &Path) -> Result<File> {
    loop {
        if let Some(guard) = try_lock(root, "handoff.lock")? {
            return Ok(guard);
        }
        tokio::time::sleep(POLL).await;
    }
}

/// The successor inherits Hostname verbatim, so it is not a process identity.
/// These kernel mount roots identify the actual container even on cgroup v2.
fn mount_identity(mountinfo: &str) -> Option<String> {
    mountinfo.lines().find_map(|line| {
        let fields: Vec<_> = line.split_ascii_whitespace().take(5).collect();
        if fields.len() != 5
            || !matches!(
                fields[4],
                "/etc/hostname" | "/etc/hosts" | "/etc/resolv.conf"
            )
        {
            return None;
        }
        let mut parts = fields[3].split('/').rev();
        parts.next()?;
        let id = parts.next()?;
        (id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())).then(|| id.to_owned())
    })
}
fn own_id() -> Result<String> {
    mount_identity(&std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default())
        .ok_or_else(|| failure("companion_identity_unavailable"))
}

fn release_version() -> &'static str {
    // The standalone companion crate is not versioned as a cargo-dist binary.
    // Embed the CLI release manifest, also for its shared implementation here.
    include_str!("../../Cargo.toml")
        .lines()
        .find_map(|line| {
            line.strip_prefix("version = \"")
                .and_then(|s| s.strip_suffix('"'))
        })
        .expect("CLI release version")
}
fn current(inspect: &Value) -> CompanionStatus {
    let version = release_version();
    #[cfg(test)]
    let version = if inspect["Config"]["Image"]
        .as_str()
        .is_some_and(|s| s.ends_with("-old"))
    {
        "0.40.0"
    } else {
        version
    };
    CompanionStatus {
        version: version.into(),
        digest: inspect["Config"]["Image"]
            .as_str()
            .and_then(|s| s.strip_prefix(&format!("{}@", update::UPDATER_IMAGE)))
            .filter(|s| update::valid_digest(s))
            .map(str::to_owned),
        phase: "current".into(),
        ..Default::default()
    }
}

fn validate(inspect: &Value, machine: &str) -> Result<()> {
    let host = &inspect["HostConfig"];
    let mounts = inspect["Mounts"]
        .as_array()
        .ok_or_else(|| failure("companion_config_invalid"))?;
    ensure!(
        inspect["Config"]["Labels"]["dev.nyxid.machine.updater"] == machine
            && host["AutoRemove"] != true
            && host["ReadonlyRootfs"] == true
            && host["CapDrop"]
                .as_array()
                .is_some_and(|v| v.iter().any(|s| s == "ALL"))
            && host["SecurityOpt"]
                .as_array()
                .is_some_and(|v| v.iter().any(|s| s
                    .as_str()
                    .is_some_and(|s| matches!(s, "no-new-privileges" | "no-new-privileges:true"))))
            && host["Tmpfs"]["/tmp"].is_string()
            && mounts
                .iter()
                .any(|m| m["Destination"] == "/var/run/docker.sock"
                    && m["Source"] == "/var/run/docker.sock")
            && mounts
                .iter()
                .any(|m| m["Destination"] == update::UPDATE_VOLUME
                    && m["Type"] == "volume"
                    && m["Name"] == format!("{machine}-nyxid-update")),
        failure("companion_config_invalid")
    );
    Ok(())
}

fn wants_update(current: &str, target: &str) -> Result<bool> {
    Ok(update::version(target).map_err(anyhow::Error::msg)?
        > update::version(current).map_err(anyhow::Error::msg)?)
}

/// Called only while owning the controller. The journal precedes create, so
/// recovery can find a created successor even if saving its ID was interrupted.
async fn prepare(
    api: &docker::Docker,
    root: &Path,
    machine: &str,
    id: &str,
    target: &str,
) -> Result<bool> {
    let original = api.inspect(id).await?;
    let status = current(&original);
    if !wants_update(&status.version, target)? {
        return Ok(false);
    }
    publish(
        root,
        &CompanionStatus {
            phase: "pending".into(),
            target_version: Some(target.into()),
            ..status.clone()
        },
    )?;
    let result = async {
        validate(&original, machine)?;
        let digest = verified_image(api, root, update::UPDATER_IMAGE, target).await?;
        let image = format!("{}@{digest}", update::UPDATER_IMAGE);
        api.pull(&image).await?;
        let image_id = api.image_id(&image).await?;
        let mut config = docker::copy_config(&original)?;
        config["Image"] = json!(image);
        let epoch = now_ms();
        let name = format!("{machine}-updater");
        let mut swap = Swap {
            epoch,
            machine: machine.into(),
            old_id: id.into(),
            old_status: status.clone(),
            temporary: format!("{name}-next-{epoch}"),
            retired: format!("{name}-previous-{epoch}"),
            name,
            successor_id: None,
            digest,
            image_id,
            target: target.into(),
            step: Step::Prepared,
            deadline_ms: 0,
        };
        let _fence = fence(root).await?;
        ensure!(load(root)?.is_none(), "Companion recovery pending");
        save(root, &swap)?;
        let successor = api.create(&swap.temporary, &config).await?;
        swap.successor_id = Some(successor.clone());
        save(root, &swap)?;
        api.start(&successor).await?;
        swap.step = Step::Released;
        swap.deadline_ms = now_ms() + HANDOFF_MS;
        save(root, &swap)?;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        failed(root, &status, target, &error)?;
        // A partially created successor is cleaned by startup/monitor recovery.
        return Ok(load(root)?.is_some());
    }
    Ok(true)
}

fn pulse_matches(swap: &Swap, pulse: &Pulse, now: u64) -> bool {
    Some(&pulse.id) == swap.successor_id.as_ref()
        && pulse.epoch == swap.epoch
        && pulse.digest == swap.digest
        && pulse.at_ms >= swap.epoch
        && pulse.at_ms <= now
        && now - pulse.at_ms < 5_000
}

/// Call under the handoff fence. Neither controller can cross a durable
/// rollback/commit decision, even if Docker operations or a process crash.
async fn commit(api: &docker::Docker, root: &Path, swap: &mut Swap) -> Result<bool> {
    if swap.step != Step::Healthy {
        return Ok(false);
    }
    let pulse: Pulse = serde_json::from_slice(&read(root, "companion-heartbeat.json", 1024)?)?;
    if !pulse_matches(swap, &pulse, now_ms()) {
        return Ok(false);
    }
    let state = api.inspect(&pulse.id).await?;
    if state["State"]["Running"] != true || state["Image"] != swap.image_id {
        return Ok(false);
    }
    swap.step = Step::Committed;
    save(root, swap)?;
    Ok(true)
}

async fn finish(api: &docker::Docker, root: &Path, swap: &Swap) -> Result<()> {
    ensure!(swap.step == Step::Committed, "Uncommitted companion");
    let id = swap.successor_id.as_deref().context("Missing successor")?;
    let successor = api.inspect(id).await?;
    if successor["Name"] != format!("/{}", swap.name) {
        if let Some(old) = inspect_optional(api, &swap.old_id).await?
            && old["Name"] != format!("/{}", swap.retired)
        {
            api.rename(&swap.old_id, &swap.retired).await?;
        }
        api.rename(id, &swap.name).await?;
    }
    publish(
        root,
        &CompanionStatus {
            version: swap.target.clone(),
            digest: Some(swap.digest.clone()),
            phase: "current".into(),
            ..Default::default()
        },
    )?;
    // The successor executes this cleanup, never the process being removed.
    api.remove(&swap.old_id).await?;
    remove_file(root, "companion-heartbeat.json")?;
    remove_file(root, JOURNAL)
}
async fn inspect_optional(api: &docker::Docker, id: &str) -> Result<Option<Value>> {
    match api.inspect(id).await {
        Ok(v) => Ok(Some(v)),
        Err(e)
            if matches!(
                e.downcast_ref::<docker::CheckFailure>(),
                Some(docker::CheckFailure::ContainerNotFound)
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// The old process is the rollback monitor. Fence before removing the candidate;
/// a candidate holding controller.lock cannot do work before commit.
async fn restore(api: &docker::Docker, root: &Path, swap: &mut Swap) -> Result<()> {
    ensure!(
        swap.step != Step::Committed,
        "Committed successor cannot roll back"
    );
    swap.step = Step::RollingBack;
    save(root, swap)?;
    if let Some(candidate) = inspect_optional(api, &swap.temporary).await? {
        let id = candidate["Id"]
            .as_str()
            .context("Missing candidate identity")?;
        ensure!(
            id != swap.old_id
                && candidate["Config"]["Labels"]["dev.nyxid.machine.updater"] == swap.machine
                && candidate["Image"] == swap.image_id
                && swap
                    .successor_id
                    .as_deref()
                    .is_none_or(|expected| expected == id),
            failure("companion_identity_unavailable")
        );
        api.remove(id).await?;
    }
    if let Some(id) = &swap.successor_id {
        api.remove(id).await?;
    }
    failed(
        root,
        &swap.old_status,
        &swap.target,
        &failure("successor_unhealthy"),
    )?;
    remove_file(root, "companion-heartbeat.json")?;
    remove_file(root, JOURNAL)
}

/// On both startup and handoff, reacquire the controller only in the role allowed
/// by the persisted fence. The old process remains alive until new health commits.
async fn acquire(api: &docker::Docker, root: &Path, machine: &str, id: &str) -> Result<File> {
    loop {
        {
            let _fence = fence(root).await?;
            match load(root)? {
                None => {
                    if let Some(guard) = try_lock(root, "controller.lock")? {
                        return Ok(guard);
                    }
                }
                Some(mut swap) => {
                    ensure!(
                        swap.machine == machine,
                        "Companion journal belongs to another machine"
                    );
                    if swap.old_id == id {
                        if swap.step == Step::Healthy {
                            commit(api, root, &mut swap).await?;
                        }
                        match swap.step {
                            Step::Prepared | Step::RollingBack => {
                                restore(api, root, &mut swap).await?
                            }
                            Step::Released | Step::Healthy if now_ms() >= swap.deadline_ms => {
                                restore(api, root, &mut swap).await?
                            }
                            Step::Committed => {
                                // A crash after commit is cleanup, never rollback.
                                api.start(
                                    swap.successor_id.as_deref().context("Missing successor")?,
                                )
                                .await?;
                            }
                            _ => (),
                        }
                    } else if swap.successor_id.as_deref() == Some(id) {
                        if matches!(swap.step, Step::Released | Step::Healthy | Step::Committed)
                            && let Some(guard) = try_lock(root, "controller.lock")?
                        {
                            let inspect = api.inspect(id).await?;
                            verify_successor(&inspect, &swap)?;
                            validate(&inspect, machine)?;
                            if swap.step != Step::Committed {
                                ensure!(
                                    now_ms() < swap.deadline_ms,
                                    failure("successor_unhealthy")
                                );
                                heartbeat(root)?;
                                write(
                                    root,
                                    "companion-heartbeat.json",
                                    &serde_json::to_vec(&Pulse {
                                        epoch: swap.epoch,
                                        id: id.into(),
                                        digest: swap.digest.clone(),
                                        at_ms: now_ms(),
                                    })?,
                                )?;
                                swap.step = Step::Healthy;
                                save(root, &swap)?;
                                // The predecessor can commit this proof too. The
                                // successor completes it if the predecessor crashed.
                                commit(api, root, &mut swap).await?;
                            }
                            ensure!(swap.step == Step::Committed, failure("successor_unhealthy"));
                            finish(api, root, &swap).await?;
                            return Ok(guard);
                        }
                    } else {
                        return Err(failure("companion_identity_unavailable"));
                    }
                }
            }
        }
        tokio::time::sleep(POLL).await;
    }
}

fn verify_successor(inspect: &Value, swap: &Swap) -> Result<()> {
    let expected = format!("{}@{}", update::UPDATER_IMAGE, swap.digest);
    let configured = inspect["Config"]["Image"].as_str() == Some(&expected);
    #[cfg(test)]
    let configured = configured
        || inspect["Config"]["Image"]
            .as_str()
            .is_some_and(|s| s.starts_with("nyxid-self-update-e2e:"));
    ensure!(
        configured && inspect["Image"] == swap.image_id && current(inspect).version == swap.target,
        failure("successor_image_mismatch")
    );
    Ok(())
}

pub(super) async fn run(root: PathBuf, name: String, api: docker::Docker) -> Result<()> {
    private_directory(&root)?;
    docker::valid_name(&name)?;
    let id = own_id()?;
    let mut guard = Some(acquire(&api, &root, &name, &id).await?);
    let mut pulse = Some(heartbeat_task(&root));
    let installed = current(&api.inspect(&id).await?);
    // Retain a prior rollback/failure on restart until an explicit retry succeeds.
    if status(&root).is_none_or(|s| s.version != installed.version) {
        publish(&root, &installed)?;
    }
    recover_pending(&api, &root, &name).await?;
    let mut startup = true;
    loop {
        heartbeat(&root)?;
        let pending = request(&root);
        let mut target = None;
        match pending {
            Ok(Some((version, rollback))) => {
                let machine = api.inspect(&name).await?;
                let same_machine = machine["State"]["Health"]["Status"] == "healthy"
                    && connected(&root, &version, now_ms().saturating_sub(90_000));
                // An explicit retry on V repairs just the older companion, without
                // interrupting an already current machine. Other updates keep the
                // existing verification, replacement and rollback path.
                if same_machine
                    && wants_update(&current(&api.inspect(&id).await?).version, &version)?
                {
                    report(
                        &root,
                        &Progress {
                            target: version.clone(),
                            phase: Phase::Connected,
                            started_at_ms: now_ms(),
                            code: None,
                            updater: None,
                        },
                    )?;
                    finish_request(&root, "request")?;
                    target = Some(version);
                } else {
                    if let Err(error) = replace(&api, &root, &name, &version, rollback, false).await
                    {
                        ensure!(
                            !root.join("journal.json").exists(),
                            "Update recovery pending"
                        );
                        eprintln!("{}", report_failure(&root, &version, &error, "watch")?);
                    }
                    let connected = read(&root, "progress.json", 4096)
                        .ok()
                        .and_then(|v| serde_json::from_slice::<Progress>(&v).ok())
                        .is_some_and(|p| p.target == version && p.phase == Phase::Connected);
                    finish_request(&root, "request")?;
                    if connected {
                        target = Some(version);
                    }
                }
            }
            Err(_) => {
                finish_request(&root, "request")?;
                eprintln!("machine_update invalid_request");
            }
            Ok(None) if startup => {
                let machine = api.inspect(&name).await?;
                // Startup only follows a healthy, connected installed machine.
                let live: Option<Connected> = read(&root, "connected.json", 4096)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok());
                if let Some(live) = live.filter(|live| {
                    machine["State"]["Health"]["Status"] == "healthy"
                        && live.at_ms > now_ms().saturating_sub(90_000)
                        && update::version(&live.version).is_ok()
                }) {
                    target = Some(live.version);
                } else {
                    tokio::time::sleep(POLL).await;
                    continue;
                }
            }
            Ok(None) => (),
        }
        startup = false;
        if let Some(target) = target {
            let swapped = prepare(&api, &root, &name, &id, &target).await;
            match swapped {
                Ok(true) => {
                    drop(pulse.take());
                    drop(guard.take());
                    guard = Some(acquire(&api, &root, &name, &id).await?);
                    pulse = Some(heartbeat_task(&root));
                }
                Ok(false) => (),
                Err(error) => failed(&root, &installed, &target, &error)?,
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    fn inspect(id: &str, name: &str, image: &str) -> Value {
        json!({
            "Id": id, "Name": format!("/{name}"), "Image": format!("sha256:{}", "b".repeat(64)),
            "Config": {"Image": image, "Hostname": "inherited-hostname", "Env": ["PRIVATE=preserved"],
                "Cmd": ["watch", "machine"], "Labels": {"dev.nyxid.machine.updater": "machine", "custom": "preserved"}},
            "State": {"Running": true},
            "Mounts": [
                {"Type": "bind", "Source": "/var/run/docker.sock", "Destination": "/var/run/docker.sock"},
                {"Type": "volume", "Name": "machine-nyxid-update", "Destination": update::UPDATE_VOLUME, "RW": true}
            ],
            "HostConfig": {"ReadonlyRootfs": true, "CapDrop": ["ALL"],
                "Tmpfs": {"/tmp": "rw,noexec,nosuid,size=16m"}, "SecurityOpt": ["no-new-privileges:true"],
                "Binds": ["/var/run/docker.sock:/var/run/docker.sock", format!("machine-nyxid-update:{}", update::UPDATE_VOLUME)],
                "RestartPolicy": {"Name": "unless-stopped"}},
            "NetworkSettings": {"Networks": {"bridge": {"IPAMConfig": null, "Aliases": ["custom"]}}}
        })
    }
    fn swap(step: Step) -> Swap {
        Swap {
            epoch: 1,
            machine: "machine".into(),
            old_id: "old".into(),
            old_status: CompanionStatus {
                version: "0.40.0".into(),
                phase: "current".into(),
                ..Default::default()
            },
            name: "machine-updater".into(),
            temporary: "machine-updater-next-1".into(),
            retired: "machine-updater-previous-1".into(),
            successor_id: Some("new".into()),
            digest: format!("sha256:{}", "a".repeat(64)),
            image_id: format!("sha256:{}", "b".repeat(64)),
            target: release_version().into(),
            step,
            deadline_ms: 0,
        }
    }
    #[derive(Clone)]
    struct Engine(Arc<Mutex<Vec<Value>>>);
    impl Respond for Engine {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let mut records = self.0.lock().unwrap();
            let path = request.url.path().trim_start_matches("/v1.45/containers/");
            let id = path.split('/').next().unwrap();
            let index = records
                .iter()
                .position(|v| v["Id"] == id || v["Name"] == format!("/{id}"));
            let Some(index) = index else {
                return ResponseTemplate::new(404);
            };
            match (request.method.as_str(), path.rsplit('/').next().unwrap()) {
                ("GET", "json") => ResponseTemplate::new(200).set_body_json(records[index].clone()),
                ("POST", "rename") => {
                    let name = request
                        .url
                        .query_pairs()
                        .find(|(k, _)| k == "name")
                        .unwrap()
                        .1
                        .into_owned();
                    assert!(
                        !records
                            .iter()
                            .any(|v| v["Name"] == format!("/{name}") && v["Id"] != id)
                    );
                    records[index]["Name"] = json!(format!("/{name}"));
                    ResponseTemplate::new(204)
                }
                ("POST", "start") => {
                    records[index]["State"]["Running"] = json!(true);
                    ResponseTemplate::new(204)
                }
                ("DELETE", _) => {
                    records.remove(index);
                    ResponseTemplate::new(204)
                }
                _ => ResponseTemplate::new(500),
            }
        }
    }
    async fn engine(swap: &Swap, old: bool, new: bool) -> (MockServer, Engine, docker::Docker) {
        let mut records = vec![];
        if old {
            records.push(inspect("old", &swap.name, "old-image"));
        }
        if new {
            records.push(inspect(
                "new",
                &swap.temporary,
                &format!("{}@{}", update::UPDATER_IMAGE, swap.digest),
            ));
        }
        let engine = Engine(Arc::new(Mutex::new(records)));
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(engine.clone())
            .mount(&server)
            .await;
        let api =
            docker::Docker::fixture(server.uri(), Err("must not re-resolve a journaled digest"));
        (server, engine, api)
    }

    #[test]
    fn only_newer_companions_are_installed_and_identity_ignores_inherited_hostname() {
        assert!(wants_update("0.40.0", "0.41.3").unwrap());
        assert!(!wants_update("0.41.3", "0.41.3").unwrap());
        assert!(!wants_update("0.41.3", "0.40.0").unwrap());
        let id = "a".repeat(64);
        assert_eq!(
            mount_identity(&format!(
                "1 2 3 /docker/containers/{id}/hostname /etc/hostname rw - ext4 /dev/vda rw"
            )),
            Some(id)
        );
        assert!(mount_identity("1 2 3 /hostile/path /etc/hostname rw").is_none());
    }

    #[test]
    fn successor_requires_expected_official_digest_runtime_image_and_release() {
        let swap = swap(Step::Released);
        let valid = inspect(
            "new",
            &swap.temporary,
            &format!("{}@{}", update::UPDATER_IMAGE, swap.digest),
        );
        verify_successor(&valid, &swap).unwrap();
        for (field, value) in [
            ("image", "attacker/image"),
            ("runtime", "wrong-id"),
            ("version", "0.0.1"),
        ] {
            let mut state = valid.clone();
            let mut swap = swap.clone();
            match field {
                "image" => state["Config"]["Image"] = json!(value),
                "runtime" => state["Image"] = json!(value),
                _ => swap.target = value.into(),
            }
            assert_eq!(
                classified(&verify_successor(&state, &swap).unwrap_err()).code(),
                "update_companion:successor_image_mismatch"
            );
        }
    }

    #[test]
    fn companion_config_changes_only_image_and_retains_all_security_and_identity_settings() {
        let state = inspect("old", "machine-updater", "old-image");
        validate(&state, "machine").unwrap();
        let mut copy = docker::copy_config(&state).unwrap();
        copy["Image"] = json!(format!(
            "{}@sha256:{}",
            update::UPDATER_IMAGE,
            "a".repeat(64)
        ));
        for (key, value) in state["Config"].as_object().unwrap() {
            if key != "Image" {
                assert_eq!(&copy[key], value);
            }
        }
        assert_eq!(copy["HostConfig"], state["HostConfig"]);
        let mut bad = state.clone();
        bad["Mounts"][1]["Name"] = json!("another-volume");
        assert!(validate(&bad, "machine").is_err());
    }

    #[tokio::test]
    async fn every_precommit_crash_boundary_rolls_back_to_one_controller() {
        for step in [
            Step::Prepared,
            Step::Released,
            Step::Healthy,
            Step::RollingBack,
        ] {
            for created in [false, true] {
                let root = tempfile::tempdir().unwrap();
                let mut swap = swap(step);
                // create succeeded but saving its ID may have failed.
                if step == Step::Prepared {
                    swap.successor_id = None;
                }
                let (_server, engine, api) = engine(&swap, true, created).await;
                save(root.path(), &swap).unwrap();
                write(
                    root.path(),
                    "companion-heartbeat.json",
                    &serde_json::to_vec(&Pulse {
                        epoch: 0,
                        id: "new".into(),
                        digest: swap.digest.clone(),
                        at_ms: 0,
                    })
                    .unwrap(),
                )
                .unwrap();
                let guard = acquire(&api, root.path(), "machine", "old").await.unwrap();
                assert!(try_lock(root.path(), "controller.lock").unwrap().is_none());
                assert_eq!(engine.0.lock().unwrap().len(), 1);
                assert_eq!(engine.0.lock().unwrap()[0]["Id"], "old");
                assert!(load(root.path()).unwrap().is_none());
                assert_eq!(
                    status(root.path()).unwrap().code.as_deref(),
                    Some("update_companion:successor_unhealthy")
                );
                drop(guard);
                drop(super::relock_eventually(root.path()));
                // A second recovery is a no-op, including after journal deletion.
                drop(acquire(&api, root.path(), "machine", "old").await.unwrap());
            }
        }
    }

    #[tokio::test]
    async fn every_committed_cleanup_boundary_is_idempotent_and_never_restores_old() {
        for boundary in 0..4 {
            let root = tempfile::tempdir().unwrap();
            let swap = swap(Step::Committed);
            let (_server, engine, api) = engine(&swap, boundary < 3, true).await;
            {
                let mut records = engine.0.lock().unwrap();
                for state in records.iter_mut() {
                    if state["Id"] == "old" && boundary >= 1 {
                        state["Name"] = json!(format!("/{}", swap.retired));
                    }
                    if state["Id"] == "new" && boundary >= 2 {
                        state["Name"] = json!(format!("/{}", swap.name));
                    }
                }
            }
            save(root.path(), &swap).unwrap();
            let guard = acquire(&api, root.path(), "machine", "new").await.unwrap();
            assert!(try_lock(root.path(), "controller.lock").unwrap().is_none());
            assert_eq!(engine.0.lock().unwrap().len(), 1);
            assert_eq!(engine.0.lock().unwrap()[0]["Name"], "/machine-updater");
            assert_eq!(status(root.path()).unwrap().version, release_version());
            assert!(load(root.path()).unwrap().is_none());
            drop(guard);
            drop(acquire(&api, root.path(), "machine", "new").await.unwrap());
        }
    }

    #[tokio::test]
    async fn successor_waits_for_predecessor_and_commits_only_its_fenced_heartbeat() {
        let root = tempfile::tempdir().unwrap();
        let mut swap = swap(Step::Released);
        swap.deadline_ms = now_ms() + HANDOFF_MS;
        let (_server, engine, api) = engine(&swap, true, true).await;
        save(root.path(), &swap).unwrap();
        let old = lock(root.path()).unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(50),
                acquire(&api, root.path(), "machine", "new")
            )
            .await
            .is_err()
        );
        assert_eq!(load(root.path()).unwrap().unwrap().step, Step::Released);
        drop(old);
        let _successor = acquire(&api, root.path(), "machine", "new").await.unwrap();
        assert_eq!(engine.0.lock().unwrap().len(), 1);
        assert_eq!(status(root.path()).unwrap().phase, "current");
    }

    #[test]
    fn companion_failure_preserves_connected_machine_and_reports_only_fixed_metadata() {
        let root = tempfile::tempdir().unwrap();
        report(
            root.path(),
            &Progress {
                target: release_version().into(),
                phase: Phase::Connected,
                started_at_ms: 1,
                code: None,
                updater: None,
            },
        )
        .unwrap();
        let current = swap(Step::Prepared).old_status;
        failed(
            root.path(),
            &current,
            release_version(),
            &Failure::new("verify_updater_image", "attestation_invalid").into(),
        )
        .unwrap();
        let progress: Progress =
            serde_json::from_slice(&read(root.path(), "progress.json", 4096).unwrap()).unwrap();
        assert_eq!(progress.phase, Phase::Connected);
        assert_eq!(progress.code, None);
        assert_eq!(
            progress.updater.unwrap().code.as_deref(),
            Some("update_companion:attestation_invalid")
        );
        failed(
            root.path(),
            &current,
            release_version(),
            &anyhow::anyhow!("SECRET_DOCKER_ENV"),
        )
        .unwrap();
        assert_eq!(
            status(root.path()).unwrap().code.as_deref(),
            Some("update_companion:operation_failed")
        );
        assert!(
            !String::from_utf8(read(root.path(), "updater.json", 2048).unwrap())
                .unwrap()
                .contains("SECRET")
        );
    }

    #[test]
    fn stale_or_foreign_heartbeats_cannot_commit() {
        let swap = swap(Step::Healthy);
        let mut pulse = Pulse {
            epoch: 1,
            id: "new".into(),
            digest: swap.digest.clone(),
            at_ms: 100,
        };
        assert!(pulse_matches(&swap, &pulse, 101));
        assert!(!pulse_matches(&swap, &pulse, 10_000));
        pulse.epoch = 0;
        assert!(!pulse_matches(&swap, &pulse, 101));
        pulse.epoch = 1;
        pulse.id = "old".into();
        assert!(!pulse_matches(&swap, &pulse, 101));
        pulse.id = "new".into();
        pulse.digest = format!("sha256:{}", "f".repeat(64));
        assert!(!pulse_matches(&swap, &pulse, 101));
    }
}

#[cfg(test)]
mod container_e2e {
    use super::*;

    // Runs in real predecessor and successor containers. Only this test binary
    // replaces provenance with a local image fixture; production has no hooks.
    #[tokio::test]
    #[ignore = "entry point for real companion containers"]
    async fn companion_process() -> Result<()> {
        let image = std::env::var("NYXID_TEST_COMPANION_IMAGE")?;
        let name = std::env::var("NYXID_TEST_MACHINE")?;
        ensure!(
            image.starts_with("nyxid-self-update-e2e:"),
            "Test image required"
        );
        ensure!(
            name.starts_with("nyxid-update-e2e-"),
            "Test namespace required"
        );
        let api = docker::Docker::local_fixture(&image)?;
        let state = api.inspect(&own_id()?).await?;
        if state["Config"]["Image"]
            .as_str()
            .is_some_and(|s| s.ends_with("-bad"))
        {
            bail!("test successor exits before heartbeat");
        }
        run(update::UPDATE_VOLUME.into(), name, api).await
    }

    #[tokio::test]
    #[ignore = "real Docker self-update; cli/tests/machine_updater_e2e.sh"]
    async fn real_companion_handoff_and_rollback() -> Result<()> {
        let name = std::env::var("NYXID_TEST_MACHINE")?;
        let image = std::env::var("NYXID_TEST_COMPANION_IMAGE")?;
        let target = std::env::var("NYXID_TEST_TARGET_VERSION")?;
        ensure!(
            name.starts_with("nyxid-update-e2e-"),
            "Test namespace required"
        );
        ensure!(
            image.starts_with("nyxid-self-update-e2e:"),
            "Test image required"
        );
        let root = Path::new(update::UPDATE_VOLUME);
        let api = docker::Docker::new()?;
        let companion = format!("{name}-updater");
        // The migration case has removed its real machine. This health-only
        // fixture supplies the node's connection marker; the companion swap,
        // process crash, image identity and controller locks remain real.
        let fixture = api.create(&name, &json!({
            "Image": format!("{image}-old"), "Entrypoint": ["/bin/sh"],
            "Cmd": ["-c", format!("while true; do printf '{{\"version\":\"{target}\",\"at_ms\":%s000}}' \"$(date +%s)\" > {}/connected.json; sleep 1; done", update::UPDATE_VOLUME)],
            "Healthcheck": {"Test": ["CMD", "true"], "Interval": 1_000_000_000u64, "Timeout": 1_000_000_000u64},
            "HostConfig": {"Binds": [format!("{name}-nyxid-update:{}", update::UPDATE_VOLUME)]}
        })).await?;
        api.start(&fixture).await?;
        for bad in [false, true] {
            remove_file(root, "updater.json")?;
            remove_file(root, "progress.json")?;
            let successor_image = format!("{image}-{}", if bad { "bad" } else { "new" });
            let config = json!({
                "Image": format!("{image}-old"), "Entrypoint": ["/updater-tests"],
                "Cmd": ["companion_process", "--ignored", "--nocapture"],
                "Env": [format!("NYXID_TEST_MACHINE={name}"), format!("NYXID_TEST_COMPANION_IMAGE={successor_image}")],
                "Labels": {"dev.nyxid.machine.updater": name, "custom": "preserved"},
                "HostConfig": {
                    "ReadonlyRootfs": true, "CapDrop": ["ALL"],
                    "Tmpfs": {"/tmp": "rw,noexec,nosuid,size=16m"},
                    "SecurityOpt": ["no-new-privileges:true", format!("seccomp={}", std::fs::read_to_string("/test/seccomp.json")?)],
                    "RestartPolicy": {"Name": "unless-stopped"},
                    "Binds": ["/var/run/docker.sock:/var/run/docker.sock", format!("{name}-nyxid-update:{}", update::UPDATE_VOLUME)]
                }
            });
            let old = api.create(&companion, &config).await?;
            let before = api.inspect(&old).await?;
            api.start(&old).await?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(100);
            let result = async {
                loop {
                    let status = status(root);
                    if status.as_ref().is_some_and(|s| if bad { s.phase == "failed" } else { s.phase == "current" && s.version == target })
                        && !root.join(JOURNAL).exists()
                    { break; }
                    ensure!(tokio::time::Instant::now() < deadline, "Companion did not settle; status {:?}", status);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                let after = api.inspect(&companion).await?;
                ensure!(after["State"]["Running"] == true, "Active companion missing");
                if bad {
                    ensure!(after["Id"] == old, "Failed successor replaced old companion");
                    ensure!(status(root).unwrap().code.as_deref() == Some("update_companion:successor_unhealthy"), "Missing fixed rollback reason");
                    // Explicit same-version retry: prove the old controller owns
                    // the mailbox again and attempts the companion, not the machine.
                    let machine_before = api.inspect(&name).await?["Id"].clone();
                    write(root, "request", target.as_bytes())?;
                    let retry = tokio::time::Instant::now() + Duration::from_secs(10);
                    while root.join("request").exists() {
                        ensure!(tokio::time::Instant::now() < retry, "Rollback controller did not resume");
                        tokio::time::sleep(POLL).await;
                    }
                    // Wait until create/start is durably recorded before test
                    // cleanup; no Docker create may finish after the EXIT sweep.
                    while !load(root)?.is_some_and(|s| s.step == Step::Released) {
                        ensure!(tokio::time::Instant::now() < retry, "Companion-only retry did not hand off");
                        tokio::time::sleep(POLL).await;
                    }
                    ensure!(api.inspect(&name).await?["Id"] == machine_before, "Companion-only retry restarted machine");
                    println!("Companion failed successor: previous controller resumed; same-version retry preserved machine");
                } else {
                    ensure!(after["Id"] != old, "Successor identity unchanged");
                    ensure!(inspect_optional(&api, &old).await?.is_none(), "Committed old companion retained");
                    for (key, value) in before["Config"].as_object().context("config")? {
                        if key != "Image" { ensure!(&after["Config"][key] == value, "Companion config changed: {key}"); }
                    }
                    for key in ["Binds", "Tmpfs", "ReadonlyRootfs", "CapDrop", "SecurityOpt", "RestartPolicy", "NetworkMode"] {
                        ensure!(before["HostConfig"][key] == after["HostConfig"][key], "Companion host setting changed: {key}");
                    }
                    ensure!(status(root).unwrap().digest.is_some(), "Missing reported companion digest");
                    println!("Companion self-update: old → fixture successor → old removed; config and controller fence preserved");
                }
                Ok(())
            }.await;
            // The shell's cleanup also removes temporary candidates on failures.
            result?;
            api.remove(&companion).await?;
        }
        api.remove(&fixture).await?;
        Ok(())
    }
}
