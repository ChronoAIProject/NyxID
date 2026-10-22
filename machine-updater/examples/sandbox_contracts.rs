//! Export actual manager-rendered objects for the repository CEL contract test.
//! This only prints fixtures; it does not connect to Kubernetes.
use nyxid_machine::managed::PROFILE;
use nyxid_machine_updater::sandbox::SandboxIntent;
use serde_json::json;

fn main() -> anyhow::Result<()> {
    let current = SandboxIntent {
        namespace: "nyxid-machine".into(),
        name: "machine-00000000000040008000000000000001".into(),
        owner_id: "00000000-0000-4000-8000-000000000002".into(),
        machine_id: "00000000-0000-4000-8000-000000000001".into(),
        operation_id: "00000000-0000-4000-8000-000000000003".into(),
        runtime_generation: 1,
        credential_epoch: 1,
        writer_fence: "00000000-0000-4000-8000-000000000004".into(),
        profile: PROFILE.into(),
        image: format!("node@sha256:{}", "a".repeat(64)),
        runtime_class: Some("sandboxed".into()),
        workspace_storage_class: Some("verified".into()),
        workspace_size: "20Gi".into(),
        cpu_request: "2".into(),
        cpu_limit: "2".into(),
        memory_request: "2Gi".into(),
        memory_limit: "2Gi".into(),
        running: true,
        shutdown_time: None,
        bootstrap_base_url: "https://nyxid.example".into(),
    };
    current.validate()?;
    let mut current_default_storage = current.clone();
    current_default_storage.workspace_storage_class = None;
    current_default_storage.validate()?;
    let mut retained_current = current.clone();
    retained_current.image = format!("node@sha256:{}", "b".repeat(64));
    retained_current.validate()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "current": current.apply_object(),
            "current_default_storage": current_default_storage.apply_object(),
            "retained_current": retained_current.apply_object(),
        }))?
    );
    Ok(())
}
