//! Update protocol contains release identifiers and metadata, never commands.
use serde::{Deserialize, Serialize};

pub const MACHINE_IMAGE: &str = "ghcr.io/chronoaiproject/nyxid/nyxid-node-machine";
pub const UPDATER_IMAGE: &str = "ghcr.io/chronoaiproject/nyxid/nyxid-machine-updater";
pub const CONTAINER_LABEL: &str = "dev.nyxid.machine";
pub const UPDATE_VOLUME: &str = "/var/lib/nyxid-machine-update";
pub const RECONNECT_SECONDS: u64 = 180;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Installation {
    Container,
    Native,
}

pub fn version(value: &str) -> Result<semver::Version, &'static str> {
    if value.len() > 80 || value.starts_with('v') {
        return Err("Use a release version such as 0.41.0");
    }
    semver::Version::parse(value).map_err(|_| "Invalid release version")
}

pub fn validate_target(
    current: &str,
    target: &str,
    owner_rollback: bool,
) -> Result<(), &'static str> {
    let current = version(current)?;
    let target = version(target)?;
    if target < current && !owner_rollback {
        return Err("Downgrade requires an owner-confirmed rollback");
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Queued,
    Verifying,
    Downloading,
    Restarting,
    Connected,
    RolledBack,
    Failed,
}
impl Phase {
    pub fn terminal(&self) -> bool {
        matches!(self, Self::Connected | Self::RolledBack | Self::Failed)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub target: String,
    pub phase: Phase,
    pub started_at_ms: u64,
    /// A fixed local identifier; never an upstream response or Docker config.
    pub code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Connected {
    pub version: String,
    pub at_ms: u64,
}

pub fn container_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_alphanumeric() || i > 0 && matches!(b, b'_' | b'-' | b'.'))
}

/// Arguments only: callers invoke Docker directly, never interpolate a shell.
pub fn migration_args(name: &str, target: &str, image: &str) -> Result<Vec<String>, &'static str> {
    if !container_name(name) {
        return Err("Invalid container name");
    }
    version(target)?;
    let digest = image
        .strip_prefix(&format!("{UPDATER_IMAGE}@sha256:"))
        .ok_or("Verified updater image digest required")?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("Verified updater image digest required");
    }
    Ok(vec![
        "run".into(),
        "--rm".into(),
        "--read-only".into(),
        "--cap-drop=ALL".into(),
        "--security-opt=no-new-privileges".into(),
        "--mount".into(),
        "type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock".into(),
        "--mount".into(),
        format!("type=volume,src={name}-nyxid-update,dst={UPDATE_VOLUME}"),
        image.into(),
        "bootstrap".into(),
        name.into(),
        target.into(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_semver_can_choose_an_official_image_and_downgrades_need_confirmation() {
        for bad in [
            "latest",
            "v0.41.0",
            "0.41.0;id",
            "other/image:1.0.0",
            "1.2.3\n",
            "01.2.3",
        ] {
            assert!(version(bad).is_err());
        }
        assert!(version("0.41.0-rc.1").is_ok());
        assert!(validate_target("0.40.0", "0.41.0", false).is_ok());
        assert!(validate_target("0.41.0", "0.40.0", false).is_err());
        assert!(validate_target("0.41.0", "0.40.0", true).is_ok());
    }
}
