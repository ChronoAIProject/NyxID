//! Manager intent mapping; the shared updater crate owns the only HTTP adapter.
use crate::models::remote_machine::ManagedMachine;
use nyxid_machine_updater::sandbox;

/// Compatibility name helper for persisted manager intents.
pub struct SandboxIntent;
impl SandboxIntent {
    pub fn deterministic_name(machine_id: &str) -> String {
        // New Machine IDs are validated UUIDv4s. Legacy malformed rows never
        // reach the API: sandbox::SandboxIntent::validate rejects them.
        sandbox::deterministic_name(machine_id)
            .unwrap_or_else(|_| format!("invalid-machine-{}", machine_id.replace('-', "")))
    }
}

pub fn intent(
    machine: &ManagedMachine,
    operation_id: &str,
    storage_class: Option<&str>,
    running: bool,
    bootstrap_base_url: &str,
) -> sandbox::SandboxIntent {
    sandbox::SandboxIntent {
        bootstrap_base_url: bootstrap_base_url.into(),
        namespace: machine.sandbox_namespace.clone(),
        name: machine.sandbox_name.clone(),
        owner_id: machine.owner_id.clone(),
        machine_id: machine.node_id.clone(),
        operation_id: operation_id.into(),
        profile: machine.profile.profile_version.clone(),
        image: machine.profile.image_digest.clone(),
        runtime_class: machine.profile.runtime_class.clone(),
        workspace_storage_class: storage_class.map(str::to_owned),
        workspace_size: format!("{}Gi", machine.profile.workspace_gib),
        cpu_request: format!("{}m", machine.profile.cpu_millis),
        cpu_limit: format!("{}m", machine.profile.cpu_millis),
        memory_request: format!("{}Mi", machine.profile.memory_mib),
        memory_limit: format!("{}Mi", machine.profile.memory_mib),
        runtime_generation: machine.runtime_generation,
        credential_epoch: machine.credential_epoch,
        writer_fence: machine.writer_fence.clone(),
        running,
        // Native Sandbox expiry is bounded desired control-plane state, not
        // an independent physical deadline under node/control-plane partition.
        // Paid extension remains limited to an atomically funded window.
        shutdown_time: if !running {
            None
        } else if machine.free_slot {
            machine.native_run_not_after.map(|at| at.to_rfc3339())
        } else {
            machine.funded_until.map(|at| at.to_rfc3339())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapping_uses_the_shared_identity_and_fixed_resources() {
        let id = uuid::Uuid::new_v4().to_string();
        let m = crate::services::remote_machine_service::CreateMachineInput::fixed(
            &uuid::Uuid::new_v4().to_string(),
            true,
            &format!("registry.test/node@sha256:{}", "a".repeat(64)),
        )
        .preview(&id, chrono::Utc::now());
        let object = intent(
            &m,
            &uuid::Uuid::new_v4().to_string(),
            Some("test-csi"),
            false,
            "https://nyxid.test",
        )
        .apply_object();
        assert_eq!(
            object["metadata"]["name"],
            sandbox::deterministic_name(&id).unwrap()
        );
        assert_eq!(object["spec"]["operatingMode"], "Suspended");
        assert_eq!(object["spec"]["service"], false);
        assert_eq!(
            object["spec"]["volumeClaimTemplates"][0]["spec"]["resources"]["requests"]["storage"],
            "20Gi"
        );
        assert_eq!(
            object["spec"]["podTemplate"]["spec"]["containers"][0]["resources"]["limits"]["memory"],
            "2Gi"
        );
        assert!(
            object
                .pointer("/spec/podTemplate/spec/containers/0/resources/limits/ephemeral-storage")
                .is_none()
        );
        assert!(
            object
                .pointer("/spec/podTemplate/spec/volumes/2/emptyDir/sizeLimit")
                .is_none()
        );
        let mut retained = m;
        retained.runtime_generation = 1;
        retained.credential_epoch = 1;
        retained.profile.runtime_class = Some("saved-runtime".into());
        let resumed = intent(
            &retained,
            &uuid::Uuid::new_v4().to_string(),
            Some("test-csi"),
            true,
            "https://nyxid.test",
        );
        resumed.validate().unwrap();
        assert_eq!(resumed.image, retained.profile.image_digest);
        assert_eq!(resumed.runtime_class, retained.profile.runtime_class);
        assert_eq!(resumed.profile, nyxid_machine::managed::PROFILE);
        for obsolete in ["remote-shell-files-v1", "remote-shell-files-v2"] {
            retained.profile.profile_version = obsolete.into();
            assert!(
                intent(
                    &retained,
                    &uuid::Uuid::new_v4().to_string(),
                    Some("test-csi"),
                    true,
                    "https://nyxid.test"
                )
                .validate()
                .is_err()
            );
        }
    }
}
