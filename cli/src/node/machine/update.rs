//! Supervisor-only mailbox. Only a verified machine command reaches request().
use anyhow::{Context, Result, bail};
use nyxid_machine::update::{Connected, Installation, Progress};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn installation(config: &nyxid_machine::config::Config) -> Installation {
    if std::env::var_os("NYXID_MACHINE_CONTAINER").is_some()
        || config.managed_browser.as_ref().is_some_and(|b| b.container)
    {
        Installation::Container
    } else {
        Installation::Native
    }
}

pub fn directory(config: &nyxid_machine::config::Config, config_dir: &Path) -> PathBuf {
    match installation(config) {
        Installation::Container => nyxid_machine::update::UPDATE_VOLUME.into(),
        Installation::Native => config_dir.join("machine-update"),
    }
}

pub fn ready(root: &Path) -> bool {
    protected(root).is_ok()
        && std::fs::symlink_metadata(root.join("heartbeat"))
            .ok()
            .filter(|m| m.is_file() && !m.file_type().is_symlink())
            .and_then(|m| m.modified().ok())
            .and_then(|at| at.elapsed().ok())
            .is_some_and(|age| age < Duration::from_secs(90))
}

fn protected(root: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(root)?;
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
    {
        bail!("Machine updater directory must be private to the supervisor");
    }
    Ok(())
}

fn write(root: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    protected(root)?;
    let mut tmp = tempfile::NamedTempFile::new_in(root)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(root.join(name))?;
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}

pub fn progress(root: &Path) -> Option<Progress> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    protected(root).ok()?;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(root.join("progress.json"))
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.by_ref().take(4097).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

pub fn request(root: &Path, target: &str, owner_rollback: bool) -> Result<()> {
    nyxid_machine::update::validate_target(env!("CARGO_PKG_VERSION"), target, owner_rollback)
        .map_err(anyhow::Error::msg)?;
    if !ready(root) {
        bail!(
            "Machine updater is not installed or running; open Assistant → Machines to enable it"
        );
    }
    if progress(root).is_some_and(|p| !p.phase.terminal())
        || root.join("request").exists()
        || root.join("rollback-request").exists()
    {
        bail!("A machine update is already running");
    }
    // One atomic durable admission. The helper writes progress after reading it;
    // a crash must never leave queued progress without a request to process.
    // Rollback is a separate, owner-authorized mailbox. Each request's entire
    // content is a SemVer: no image, command, arguments, URL or environment.
    write(
        root,
        if owner_rollback {
            "rollback-request"
        } else {
            "request"
        },
        target.as_bytes(),
    )
}

pub fn connected(root: &Path) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    write(
        root,
        "connected.json",
        &serde_json::to_vec(&Connected {
            version: env!("CARGO_PKG_VERSION").into(),
            at_ms: now_ms(),
        })?,
    )
    .context("Could not report machine reconnection to updater")
}

/// Docker access uses the agent command identity, never supervisor privileges.
pub async fn container_operation(
    identity: &super::process::Identity,
    config: &nyxid_machine::config::Config,
    parameters: &serde_json::Value,
    migrate: bool,
) -> Result<serde_json::Value> {
    use serde_json::json;
    if installation(config) == Installation::Container {
        bail!("Run migration on the Docker host, never inside a machine container");
    }
    let name = parameters["container"]
        .as_str()
        .context("Container name required")?;
    if !nyxid_machine::update::container_name(name) {
        bail!("Invalid container name");
    }
    let mut inspect = tokio::process::Command::new("docker");
    identity.prepare_agent(&mut inspect)?;
    inspect
        .args([
            "inspect",
            "--type=container",
            "--format",
            r#"{{json .Id}} {{json .Name}} {{json .Config.Image}}"#,
            name,
        ])
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(10), inspect.output()).await??;
    if !output.status.success() || output.stdout.len() > 4096 {
        bail!("Docker access or target container verification failed");
    }
    let fields = serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter::<String>()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if fields.len() != 3
        || fields[1] != format!("/{name}")
        || !(fields[2].starts_with(&format!("{}:", nyxid_machine::update::MACHINE_IMAGE))
            || fields[2].starts_with(&format!("{}@sha256:", nyxid_machine::update::MACHINE_IMAGE)))
    {
        bail!("Target is not the named official machine container");
    }
    if !migrate {
        return Ok(
            json!({"container_id":fields[0],"name":name,"image":fields[2],"docker_access":true}),
        );
    }
    if parameters["container_id"].as_str() != Some(fields[0].as_str()) {
        bail!("Target container changed since owner approval; inspect and confirm again");
    }
    let target = parameters["version"]
        .as_str()
        .context("Release version required")?;
    let image = parameters["updater_image"]
        .as_str()
        .context("Verified updater image required")?;
    let args =
        nyxid_machine::update::migration_args(name, target, image).map_err(anyhow::Error::msg)?;
    let mut command = tokio::process::Command::new("docker");
    identity.prepare_agent(&mut command)?;
    command
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    // A detached task owns the child: the signed response acknowledges admission;
    // the target's durable reconnect watch reports completion or failure.
    let mut child = command.spawn()?;
    tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(1200), child.wait()).await;
    });
    Ok(
        json!({"accepted":true,"note":"Verified host migration started; watch the target machine reconnect."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn request_contains_only_a_version_and_requires_a_private_live_companion() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(request(dir.path(), "99.0.0", false).is_err());
        std::fs::write(dir.path().join("heartbeat"), []).unwrap();
        assert!(request(dir.path(), "evil/image:99.0.0", false).is_err());
        request(dir.path(), "99.0.0", false).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("request")).unwrap(),
            "99.0.0"
        );
        assert!(request(dir.path(), "99.0.1", false).is_err());
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(!ready(dir.path()));
    }
}
