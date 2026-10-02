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

/// Accept only fixed diagnostic identifiers from nodes; never relay arbitrary
/// upstream text into owner notifications, tool results or conversation wakes.
pub fn failure_guidance(code: &str) -> Option<&'static str> {
    let reason = if let Some((stage, reason)) = code.split_once(':') {
        if !matches!(
            stage,
            "verify_updater_image"
                | "verify_machine_image"
                | "verify_images"
                | "bootstrap"
                | "watch"
                | "startup"
                | "install_companion"
                | "download_image"
        ) {
            return None;
        }
        reason
    } else {
        code
    };
    Some(match reason {
        "docker_socket_unavailable" => {
            "Start Docker and check that /var/run/docker.sock is mounted into the updater, then retry the command from Machines."
        }
        "container_not_found" | "source_name_changed" => {
            "Check the machine container's current name on the Docker host, then copy a command targeting that container. Do not create a new identity volume."
        }
        "invalid_container_name" => {
            "Use the machine's Docker container name, with letters, numbers, dots, underscores or hyphens; copy the command from Machines."
        }
        "source_not_official_image" => {
            "This updater supports only the official NyxID machine image. Migrate to the official image while preserving the identity and workspace volumes."
        }
        "source_auto_remove" => {
            "Recreate the machine container without --rm, retaining its identity and workspace volumes, then retry. Rollback requires keeping the stopped container."
        }
        "source_owner_mismatch" => {
            "The updater's machine label does not match this container. Check the container name and reinstall its companion using the Machines command."
        }
        "source_version_unknown" => {
            "The installed version cannot be verified. Use a versioned official machine image or restore its version label before updating; keep the existing volumes."
        }
        "companion_name_taken" => {
            "Another container uses the required updater name. Check its ownership on the Docker host and resolve the name conflict before retrying."
        }
        "update_volume_mismatch" => {
            "The update volume or machine label belongs to a different container. Check the mounts and use this machine's dedicated update volume; retain the identity and workspace volumes."
        }
        "downgrade_refused" => {
            "Choose the current or a newer release. An older release requires an explicit owner-confirmed rollback through the installed updater."
        }
        "invalid_target_version" => {
            "Use the supported release version in the current Machines command, then retry."
        }
        "trust_store_unavailable" => {
            "Check that the updater's update volume is writable and private to its root user, then retry the command from Machines."
        }
        "trust_root_unavailable" => {
            "Retry shortly and check access to GitHub's trust-root mirror. For a 0.41.0 updater, copy the current Machines command, which includes the required /tmp tmpfs."
        }
        "attestation_unavailable" => {
            "Retry shortly; check GitHub connectivity or API rate limits. Image verification must succeed before updating."
        }
        "attestation_invalid" => {
            "No valid official release attestation was found. Wait for the release to finish publishing, then retry; do not bypass verification."
        }
        "image_resolution_failed" => {
            "Check that Docker can reach GHCR and the requested NyxID release has been published, then retry."
        }
        "operation_failed" | "verification_or_update_failed" | "update_failed_or_rolled_back" => {
            "Check Docker and the updater's private update volume, then retry from Machines. Keep the existing identity and workspace volumes."
        }
        "replacement_did_not_reconnect" => {
            "The previous container was restored. Check the machine's connection and retry from Machines."
        }
        _ => return None,
    })
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
        "--tmpfs".into(),
        "/tmp:rw,noexec,nosuid,size=16m".into(),
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
    fn migration_mounts_tmpfs_for_already_published_updaters() {
        let image = format!("{UPDATER_IMAGE}@sha256:{}", "a".repeat(64));
        let args = migration_args("test-machine", "0.41.0", &image).unwrap();
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--tmpfs", "/tmp:rw,noexec,nosuid,size=16m"])
        );
        assert!(args.contains(&"--read-only".into()));
        assert!(args.contains(&"--cap-drop=ALL".into()));
        assert!(args.contains(&"--security-opt=no-new-privileges".into()));
    }

    #[test]
    fn only_fixed_failure_codes_have_guidance() {
        assert!(
            failure_guidance("verify_updater_image:trust_root_unavailable")
                .unwrap()
                .contains("tmpfs")
        );
        assert!(
            failure_guidance("verify_machine_image:attestation_invalid")
                .unwrap()
                .contains("do not bypass")
        );
        assert!(failure_guidance("SECRET:trust_root_unavailable").is_none());
        assert!(failure_guidance("bootstrap:SECRET").is_none());
    }

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
