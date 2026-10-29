//! Dispatch-gate evidence only: complete browser prerequisites are supplied test
//! data. Real DAC/ACL, provisioning, signed authority and sandbox failures remain
//! enforced; these tests do not certify the full hosted image.
use super::*;
use crate::node::machine::{Operation, Request, process::Identity};
use nyxid_machine::{config::Config, context::Evidence, managed::*};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[test]
fn context_preparation_failure_projection_uses_fixed_metadata() {
    let correlation = id();
    let error = anyhow::anyhow!("private path=/var/lib/secret cookie=secret").context(
        PreparationFailure::new(
            "workspace_probe",
            "workspace_probe_failed",
            37,
            &correlation,
            9,
        ),
    );
    let diagnostic = failure_diagnostic(&error).expect("typed context diagnostic");
    assert_eq!(diagnostic.stage, "workspace_probe");
    assert_eq!(diagnostic.reason, "workspace_probe_failed");
    assert_eq!(diagnostic.elapsed_ms, 37);
    assert_eq!(diagnostic.correlation_id, correlation);
    assert_eq!(diagnostic.generation, 9);
    assert!(!diagnostic.to_string().contains("secret"));
}

#[test]
fn context_preparation_failure_classification_does_not_use_error_text() {
    let correlation = id();
    let first =
        anyhow::anyhow!("permission denied at /private/a").context(PreparationFailure::new(
            "browser_resources",
            "browser_resources_unavailable",
            1,
            &correlation,
            2,
        ));
    let second =
        anyhow::anyhow!("permission denied at /private/b").context(PreparationFailure::new(
            "browser_resources",
            "browser_resources_unavailable",
            99,
            &correlation,
            2,
        ));
    let first = failure_diagnostic(&first).unwrap();
    let second = failure_diagnostic(&second).unwrap();
    assert_eq!(first.stage, second.stage);
    assert_eq!(first.reason, second.reason);
    assert_eq!(first.correlation_id, second.correlation_id);
    assert_ne!(first.elapsed_ms, second.elapsed_ms);
}

#[test]
fn context_preparation_stage_reason_mapping_is_fixed() {
    for (stage, reason) in [
        ("authority", "context_authority_invalid"),
        ("binding", "hosted_context_binding_required"),
        ("identity", "context_identity_mismatch"),
        ("generation", "context_generation_quarantined"),
        ("capability", "context_backend_unavailable"),
        ("custody", "node_custody_unverified"),
        ("legacy_roots", "legacy_roots_unavailable"),
        ("legacy_browser_home", "legacy_browser_home_unavailable"),
        ("allocation_store", "context_store_unavailable"),
        ("provision_users", "context_user_provision_failed"),
        ("reap_secure", "context_secure_reap_failed"),
        ("reap_developer", "context_developer_reap_failed"),
        ("workspace_probe", "workspace_probe_failed"),
        ("native_policy", "native_browser_policy_failed"),
        ("browser_resources", "browser_resources_unavailable"),
        ("context_runtime", "context_runtime_unavailable"),
        ("context_connect", "context_node_offline"),
    ] {
        assert_eq!(stage_reason(stage), reason, "stage {stage}");
    }
}

#[test]
fn context_support_failures_keep_custody_distinct_from_capability() {
    assert_eq!(
        support_failure_stage(Some("node_custody_unverified")),
        "custody"
    );
    assert_eq!(
        support_failure_stage(Some("separated_requires_landlock_abi_6")),
        "capability"
    );
    assert_eq!(support_failure_stage(None), "capability");
}

fn evidence() -> Evidence {
    Evidence {
        version: 4,
        os: "linux".into(),
        landlock_abi: Some(6),
        separate_users: Some(true),
        credential_custody: Some(true),
        managed_browser_configured: Some(true),
        command_seccomp: Some(true),
        landlock_enforced: Some(true),
        managed_browser_sandbox: Some(true),
        managed_browser_policy: Some(true),
        ..Default::default()
    }
}
fn hardened_envelope() {
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let status = fs::read_to_string("/proc/self/status").unwrap();
    for field in ["CapEff", "CapPrm", "CapBnd"] {
        let value = status
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{field}:")))
            .unwrap()
            .trim();
        assert_eq!(
            u64::from_str_radix(value, 16).unwrap(),
            0xe1,
            "capability ceiling changed"
        );
    }
    assert!(status.lines().any(|line| line == "NoNewPrivs:\t1"));
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    assert_eq!(
        unsafe { libc::statvfs(c"/".as_ptr(), stat.as_mut_ptr()) },
        0
    );
    assert_ne!(unsafe { stat.assume_init() }.f_flag & libc::ST_RDONLY, 0);
}
fn worker_access(uid: u32, groups: &[u32], path: &Path, allowed: bool) {
    let mut command = std::process::Command::new("setpriv");
    command.args(["--reuid", &uid.to_string(), "--regid", &uid.to_string()]);
    if groups.is_empty() {
        command.arg("--clear-groups");
    } else {
        command.args([
            "--groups",
            &groups
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(","),
        ]);
    }
    let check = if allowed {
        "test -r \"$1\" && test -w \"$1\" && test -x \"$1\""
    } else {
        "test ! -r \"$1\" && test ! -w \"$1\" && test ! -x \"$1\""
    };
    let result = command
        .args(["sh", "-c", check, "sh"])
        .arg(path)
        .status()
        .unwrap();
    assert!(result.success(), "unexpected access for UID {uid}");
}
async fn dispatch(
    runtime: &Arc<Runtime>,
    authority: &Authority,
    operation: Operation,
) -> serde_json::Value {
    let parameters = serde_json::json!({
        "conversation_id":authority.conversation_id, "turn_id":authority.turn_id,
        "runtime_id":authority.runtime_id, "job_id":id(), "services":[],
        "path":"/workspace/shared-marker", "command":"printf fallback > /workspace/fallback", "timeout_secs":5
    });
    let mut request = Request {
        version: 2,
        authority: Some(Box::new(Authority {
            lease_id: id(),
            ..authority.clone()
        })),
        external_authority: None,
        request_id: id(),
        node_id: runtime.node_id.clone(),
        operation,
        parameters,
        timestamp: chrono::Utc::now().timestamp(),
        nonce: id(),
        signature: String::new(),
    };
    request.signature = nyxid_machine::signing::sign(&request, &[7; 32]);
    runtime.handle(request, &[7; 32]).await
}

#[tokio::test]
#[ignore = "requires disposable hardened Linux startup homes and immutable hosted account pool"]
async fn hosted_first_dispatch_prepares_startup_private_homes() {
    hardened_envelope();
    // WSS admission hands the shared workspace to the command user too. Other
    // suites retain the pre-admission root-owned mount to test that WSS fence.
    std::os::unix::fs::chown("/workspace", Some(1000), Some(1000)).unwrap();
    for (uid, home) in [
        (1000, "/home/agent"),
        (1001, "/home/browser"),
        (1002, "/home/devbrowser"),
    ] {
        let meta = fs::metadata(home).unwrap();
        assert_eq!(
            (meta.uid(), meta.gid(), meta.mode() & 0o777),
            (uid, uid, 0o700)
        );
        assert!(
            fs::File::open(home).is_err(),
            "fixture must reproduce missing DAC override"
        );
    }
    let parent = tempfile::tempdir_in("/var/lib/nyxid-hosted-tests").unwrap();
    fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o711)).unwrap();
    let directory = parent.path().join("node");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let configuration = RequiredConfiguration {
        version: CONFIG_VERSION,
        revision: 1,
        separation_choice: nyxid_machine::context::Choice::RequireSeparated,
        capabilities: vec!["files".into(), "shell".into()],
    };
    let config: Config =
        crate::node::managed_config::configuration_machine(&configuration).unwrap();
    let binding = Binding {
        node_id: id(),
        sandbox_uid: id(),
        pod_uid: id(),
        runtime_generation: 1,
        credential_epoch: 1,
        writer_fence: id(),
        operation_id: id(),
        delivery_id: id(),
    };
    let report = ConfigurationReport {
        digest: configuration.digest().unwrap(),
        configuration,
        runtime_generation: 1,
    };
    let runtime = Runtime::new(&config, &binding.node_id, &directory).unwrap();
    runtime
        .bind_managed_configuration(&binding, &report)
        .unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let window = SignedRuntimeWindow::sign(
        RuntimeWindow {
            binding: binding.clone(),
            control_revision: 1,
            run_not_after_ms: now + 120_000,
            issued_at_ms: now,
        },
        &[7; 32],
    )
    .unwrap();
    runtime.apply_managed_window(&window, &[7; 32]).unwrap();
    let mut authority = Authority {
        require_v2: true,
        context_id: id(),
        generation: 1,
        mode: "separated".into(),
        agent_id: id(),
        owner_id: id(),
        actor_id: id(),
        group_id: None,
        runtime_id: runtime.runtime_id.clone(),
        conversation_id: id(),
        turn_id: id(),
        lease_id: id(),
        revision: 1,
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 40_000,
        capabilities: nyxid_machine::authority::Capabilities {
            shell: true,
            files: true,
            ..Default::default()
        },
    };
    // Even complete prerequisite data cannot bypass missing retained authority.
    runtime.contexts.lock().await.test_evidence = Some(evidence());
    let denied = dispatch(&runtime, &authority, Operation::ReadFile).await;
    assert_eq!(denied["error"]["code"], 12419, "{denied}");
    assert!(!Path::new("/var/lib/nyxid-contexts/metadata/journal.json").exists());
    runtime
        .install_hosted_context_store(&id(), true)
        .await
        .unwrap();
    // Unknown/incomplete evidence must refuse before mutating startup homes.
    for invalid in [
        Evidence {
            version: 3,
            ..evidence()
        },
        Evidence {
            managed_browser_sandbox: None,
            ..evidence()
        },
    ] {
        runtime.contexts.lock().await.test_evidence = Some(invalid);
        let denied = dispatch(&runtime, &authority, Operation::ReadFile).await;
        assert_eq!(denied["error"]["code"], 12419, "{denied}");
        assert_eq!(fs::metadata("/home/agent").unwrap().uid(), 1000);
    }
    runtime.contexts.lock().await.test_evidence = Some(evidence());
    // Preparation diagnostics must preserve the installed retained store when
    // provisioning refuses an invalid generation. A subsequent request still
    // needs that exact store and its exclusive writer lock.
    let invalid = Authority {
        generation: 0,
        ..authority.clone()
    };
    let failure = match runtime.context_instance(&invalid).await {
        Ok(_) => panic!("invalid generation unexpectedly provisioned"),
        Err(failure) => failure,
    };
    assert_eq!(failure_stage(&failure), "provision_users");
    assert!(
        runtime
            .contexts
            .lock()
            .await
            .store
            .as_ref()
            .is_some_and(Store::is_hosted),
        "provisioning refusal must retain the hosted store and writer lock"
    );
    for operation in [Operation::ReadFile, Operation::Exec] {
        authority.lease_id = id();
        let result = dispatch(&runtime, &authority, operation).await;
        // This browser-less fixture must reach the real post-provision probe,
        // then fail closed. It cannot execute the operation in shared mode.
        assert_eq!(result["error"]["code"], 12419, "{result}");
        assert!(
            matches!(
                result["error"]["stage"].as_str(),
                Some("workspace_probe" | "browser_resources")
            ),
            "first dispatch stopped before provisioning: {result}"
        );
        eprintln!(
            "signed dispatch passed home preparation and provisioning; remaining fixture stage={}",
            result["error"]["stage"]
        );
        assert!(!Path::new("/workspace/fallback").exists());
        assert!(runtime.contexts.lock().await.runtimes.is_empty());
        let command = fs::metadata("/home/agent").unwrap();
        assert_eq!(
            (command.uid(), command.gid(), command.mode() & 0o777),
            (0, 1000, 0o770)
        );
        worker_access(1000, &[], Path::new("/home/agent"), true);
        for (uid, home) in [(1001, "/home/browser"), (1002, "/home/devbrowser")] {
            let browser = fs::metadata(home).unwrap();
            // ACL mask occupies group mode bits; group_obj has no access.
            assert_eq!(
                (browser.uid(), browser.gid(), browser.mode() & 0o777),
                (0, 0, 0o770)
            );
            worker_access(uid, &[], Path::new(home), true);
            worker_access(20000, &[uid], Path::new(home), false);
            worker_access(1000, &[], Path::new(home), false);
            let status = std::process::Command::new("setpriv")
                .args([
                    "--reuid",
                    &uid.to_string(),
                    "--regid",
                    &uid.to_string(),
                    "--clear-groups",
                    "chmod",
                    "0777",
                    home,
                ])
                .output()
                .unwrap();
            assert!(
                !status.status.success(),
                "legacy user must not be able to widen the gate"
            );
        }
        let journal: serde_json::Value = serde_json::from_slice(
            &fs::read("/var/lib/nyxid-contexts/metadata/journal.json").unwrap(),
        )
        .unwrap();
        assert_eq!(journal["workspaces"][&authority.context_id]["ready"], true);
    }
    // Infrastructure preparation grants no graphical capability.
    for operation in [Operation::Browser, Operation::Computer] {
        let denied = dispatch(&runtime, &authority, operation).await;
        assert_eq!(denied["error"]["code"], 12420);
    }
    // Existing explicit shared execution still works after the owner-selected
    // transition protects the homes. This is a separate signed shared context,
    // never a fallback for either refused separated operation above.
    let (sender, _receiver) = tokio::sync::mpsc::channel(100);
    runtime.connect(sender, &[7; 32]).await.unwrap();
    let shared = Authority {
        context_id: id(),
        mode: "shared_legacy".into(),
        ..authority
    };
    let result = dispatch(&runtime, &shared, Operation::Exec).await;
    assert_eq!(result["exit_code"], 0, "{result}");
    assert!(Path::new("/workspace/fallback").exists());
    runtime.shutdown().await;
    hardened_envelope();
}

#[test]
#[ignore = "requires disposable hardened Linux supervisor"]
fn legacy_home_transition_rejects_unsafe_targets_and_parents() {
    use super::super::context_runtime::{protect_legacy_browser_home, protect_legacy_roots};
    hardened_envelope();
    let root = tempfile::tempdir_in("/var/lib/nyxid-hosted-tests").unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let mut identity = Identity::resolve(Some("agent")).unwrap();
    let target = root.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    identity.home = link.clone();
    assert!(protect_legacy_roots(&[link.clone()], &identity).is_err());
    assert!(protect_legacy_browser_home(&identity).is_err());
    assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o700);
    std::os::unix::fs::chown(&target, Some(1002), Some(1002)).unwrap();
    identity.home = target.clone();
    assert!(protect_legacy_roots(&[target.clone()], &identity).is_err());
    assert!(protect_legacy_browser_home(&identity).is_err());
    assert_eq!(
        fs::metadata(&target).unwrap().uid(),
        1002,
        "wrong owner's inode must remain untouched"
    );
    std::os::unix::fs::chown(&target, Some(1000), Some(1000)).unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o777)).unwrap();
    assert!(protect_legacy_roots(&[target.clone()], &identity).is_err());
    assert!(protect_legacy_browser_home(&identity).is_err());
    assert_eq!(fs::metadata(&target).unwrap().uid(), 1000);
    hardened_envelope();
}
