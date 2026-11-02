use super::hosted_context::*;
use nyxid_machine::authority::Authority;
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn authority() -> Authority {
    Authority {
        require_v2: true,
        context_id: id(),
        generation: 1,
        mode: "separated".into(),
        agent_id: id(),
        owner_id: id(),
        actor_id: id(),
        group_id: None,
        runtime_id: id(),
        conversation_id: id(),
        turn_id: id(),
        lease_id: id(),
        revision: 1,
        expires_at_ms: 1,
        capabilities: Default::default(),
    }
}
fn pool() -> AccountPool {
    serde_json::from_value(serde_json::json!({"version":1,"first_uid":20000,"count":2048})).unwrap()
}
fn fixture() -> (tempfile::TempDir, VolumeBinding, String) {
    assert_eq!(
        unsafe { libc::geteuid() },
        0,
        "run filesystem tests in the disposable Linux container"
    );
    fs::create_dir_all("/var/lib/nyxid-hosted-tests").unwrap();
    let root = tempfile::tempdir_in("/var/lib/nyxid-hosted-tests").unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o711)).unwrap();
    (
        root,
        VolumeBinding {
            node_id: id(),
            pvc_uid: id(),
        },
        id(),
    )
}
fn open(
    root: &Path,
    binding: &VolumeBinding,
    runtime: &str,
    init: bool,
) -> anyhow::Result<HostedStore> {
    HostedStore::open(root, binding.clone(), runtime, init, pool())
}
fn read_as(uid: u32, path: &Path) -> bool {
    std::process::Command::new("setpriv")
        .args([
            "--reuid",
            &uid.to_string(),
            "--regid",
            &uid.to_string(),
            "--clear-groups",
            "cat",
        ])
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[test]
fn restore_requires_original_volume_and_complete_journal() {
    let (root, binding, runtime) = fixture();
    assert!(open(root.path(), &binding, &runtime, false).is_err());
    assert!(!root.path().join("metadata").exists());
    let store = open(root.path(), &binding, &runtime, true).unwrap();
    drop(store);
    let wrong = VolumeBinding {
        node_id: binding.node_id.clone(),
        pvc_uid: id(),
    };
    assert!(open(root.path(), &wrong, &runtime, false).is_err());
    fs::remove_file(root.path().join("metadata/journal.json")).unwrap();
    assert!(open(root.path(), &binding, &runtime, true).is_err());
    assert!(open(root.path(), &binding, &runtime, false).is_err());
}
#[test]
fn workspace_marker_uid_quarantine_and_generation_survive_runtime_replacement() {
    let (root, binding, runtime) = fixture();
    let mut a = authority();
    let mut store = open(root.path(), &binding, &runtime, true).unwrap();
    let (first, path) = store.provision(&a).unwrap();
    // Write as the actual command UID; root has no DAC_OVERRIDE in this test.
    assert!(
        std::process::Command::new("setpriv")
            .args([
                "--reuid",
                &first.command_uid.to_string(),
                "--regid",
                &first.command_uid.to_string(),
                "--clear-groups",
                "sh",
                "-c",
                "printf retained > \"$1/marker\"",
                "sh"
            ])
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    assert!(read_as(first.command_uid, &path.join("marker")));
    assert!(!read_as(1000, &path.join("marker")));
    assert!(!read_as(
        first.browsers[&1].secure_uid,
        &path.join("marker")
    ));
    assert!(!read_as(
        first.command_uid,
        &root.path().join("metadata/journal.json")
    ));
    let users = store.quarantine(&a.agent_id, 2).unwrap();
    assert_eq!(users.len(), 2);
    drop(store);
    let mut store = open(root.path(), &binding, &id(), false).unwrap();
    assert!(store.provision(&a).is_err());
    a.generation = 2;
    a.revision = 2;
    let (next, next_path) = store.provision(&a).unwrap();
    assert_eq!(next_path, path);
    assert_eq!(first.command_uid, next.command_uid);
    assert!(read_as(next.command_uid, &path.join("marker")));
    assert_ne!(first.browsers[&1], next.browsers[&2]);
    a.generation = 1;
    assert!(store.provision(&a).is_err());
}
#[test]
fn profile_recreation_requires_a_new_authorized_generation() {
    let (root, binding, runtime) = fixture();
    let a = authority();
    let mut store = open(root.path(), &binding, &runtime, true).unwrap();
    store.provision(&a).unwrap();
    drop(store);
    let mut store = open(root.path(), &binding, &id(), false).unwrap();
    assert!(
        store
            .provision(&a)
            .unwrap_err()
            .to_string()
            .contains("context_profile_generation_required")
    );
}
#[test]
fn missing_workspace_symlink_owner_drift_and_orphan_directory_refuse_recovery() {
    for defect in [
        "missing",
        "symlink",
        "owner",
        "orphan",
        "journal-symlink",
        "journal-corrupt",
        "journal-missing",
    ] {
        let (root, binding, runtime) = fixture();
        let a = authority();
        let mut store = open(root.path(), &binding, &runtime, true).unwrap();
        let (_, path) = store.provision(&a).unwrap();
        drop(store);
        match defect {
            "missing" => fs::remove_dir(&path).unwrap(),
            "symlink" => {
                fs::remove_dir(&path).unwrap();
                std::os::unix::fs::symlink("/tmp", &path).unwrap();
            }
            "owner" => std::os::unix::fs::chown(&path, Some(1000), Some(1000)).unwrap(),
            "orphan" => fs::create_dir(root.path().join("contexts/99999")).unwrap(),
            "journal-corrupt" => {
                fs::write(root.path().join("metadata/journal.json"), b"{}").unwrap()
            }
            other => {
                let journal = root.path().join("metadata/journal.json");
                fs::remove_file(&journal).unwrap();
                if other == "journal-symlink" {
                    std::os::unix::fs::symlink("/etc/passwd", journal).unwrap();
                }
            }
        }
        assert!(
            open(root.path(), &binding, &runtime, false).is_err(),
            "accepted {defect}"
        );
    }
}
#[test]
fn exclusive_supervisor_lock_and_empty_rebuild_are_not_adoption() {
    let (root, binding, runtime) = fixture();
    let store = open(root.path(), &binding, &runtime, true).unwrap();
    assert!(open(root.path(), &binding, &runtime, false).is_err());
    drop(store);
    let (replacement, _, _) = fixture();
    assert!(open(replacement.path(), &binding, &runtime, false).is_err());
}
#[test]
fn bounded_pool_exhaustion_never_reuses_generation_tombstones() {
    let (root, binding, runtime) = fixture();
    let mut store = open(root.path(), &binding, &runtime, true).unwrap();
    let mut a = authority();
    let first = store.provision(&a).unwrap().0;
    let mut journal: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("metadata/journal.json")).unwrap())
            .unwrap();
    drop(store);
    // Exhaust the monotonic allocator without manufacturing thousands of contexts.
    journal["registry"]["next_uid"] = serde_json::json!(22047);
    fs::write(
        root.path().join("metadata/journal.json"),
        serde_json::to_vec(&journal).unwrap(),
    )
    .unwrap();
    let mut store = open(root.path(), &binding, &runtime, false).unwrap();
    a.generation = 2;
    a.revision = 2;
    assert!(store.provision(&a).is_err());
    a.generation = 1;
    a.revision = 1;
    assert_eq!(
        store.provision(&a).unwrap().0.command_uid,
        first.command_uid
    );
    store.quarantine(&a.agent_id, 2).unwrap();
    assert!(store.provision(&a).is_err());
}
#[test]
fn interrupted_provisioning_only_finishes_its_journaled_allocation() {
    let (root, binding, runtime) = fixture();
    let a = authority();
    let mut store = open(root.path(), &binding, &runtime, true).unwrap();
    let (allocation, path) = store.provision(&a).unwrap();
    drop(store);
    // Crash image: allocation durable, workspace not yet created/committed.
    let file = root.path().join("metadata/journal.json");
    let mut journal: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    journal["workspaces"][&a.context_id]["ready"] = serde_json::json!(false);
    fs::write(&file, serde_json::to_vec(&journal).unwrap()).unwrap();
    fs::remove_dir(&path).unwrap();
    let mut store = open(root.path(), &binding, &runtime, false).unwrap();
    let (replayed, recovered) = store.provision(&a).unwrap();
    assert_eq!(allocation, replayed);
    assert_eq!(path, recovered);
    assert!(path.is_dir());
}
#[test]
fn account_metadata_rejects_uid_name_primary_or_supplementary_group_collisions() {
    let mut passwd = String::new();
    let mut groups = String::new();
    for uid in 20000..22048 {
        passwd += &format!(
            "nyxc{uid}:x:{uid}:{uid}:nyxid-hosted-context-v1:/nonexistent:/usr/sbin/nologin\n"
        );
        groups += &format!("nyxc{uid}:x:{uid}:\n");
    }
    pool().validate(&passwd, &groups).unwrap();
    for wrong in [
        passwd.replace("nyxc20000:x:20000:", "other:x:20000:"),
        passwd.clone() + "collision:x:20000:99::/:/bin/sh\n",
        passwd.clone() + "other:x:99:20000::/:/bin/sh\n",
        passwd.replace("/nonexistent", "/home/shared"),
    ] {
        assert!(pool().validate(&wrong, &groups).is_err());
    }
    assert!(
        pool()
            .validate(&passwd, &(groups.clone() + "privileged:x:999:nyxc20000\n"))
            .is_err()
    );
    assert!(
        pool()
            .validate(
                &passwd,
                &groups.replace("nyxc20000:x:20000:", "other:x:20000:")
            )
            .is_err()
    );
    assert!(!pool().contains(22048));
    assert!(!pool().contains(19999));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn split_workspace_sandbox_uses_retained_cwd_and_ephemeral_home_tmp() {
    use super::process::{Identity, context_sandbox::Sandbox};
    use std::sync::Arc;
    let (root, binding, runtime) = fixture();
    let a = authority();
    let mut store = open(root.path(), &binding, &runtime, true).unwrap();
    let (allocation, workspace) = store.provision(&a).unwrap();
    let uid = allocation.command_uid;
    let ephemeral = root.path().join("ephemeral");
    fs::create_dir(&ephemeral).unwrap();
    fs::set_permissions(&ephemeral, fs::Permissions::from_mode(0o710)).unwrap();
    std::os::unix::fs::chown(&ephemeral, Some(0), Some(uid)).unwrap();
    for name in ["workspace", "home", "tmp"] {
        let path = ephemeral.join(name);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::chown(&path, Some(uid), Some(uid)).unwrap();
    }
    let sandbox = Arc::new(Sandbox::open_with_workspace(&ephemeral, uid, &workspace).unwrap());
    let identity = Identity {
        uid,
        gid: uid,
        name: format!("nyxc{uid}"),
        home: ephemeral.join("home"),
        sandbox: Some(sandbox.clone()),
        policy_groups: vec![],
        desktop: None,
    };
    let sibling = authority();
    let (other, sibling_workspace) = store.provision(&sibling).unwrap();
    assert!(
        std::process::Command::new("setpriv")
            .args([
                "--reuid",
                &other.command_uid.to_string(),
                "--regid",
                &other.command_uid.to_string(),
                "--clear-groups",
                "sh",
                "-c",
                "printf private > \"$1/private\"",
                "sh"
            ])
            .arg(&sibling_workspace)
            .status()
            .unwrap()
            .success()
    );
    assert!(read_as(
        other.command_uid,
        &sibling_workspace.join("private")
    ));
    let mut command = tokio::process::Command::new("/bin/sh");
    identity.prepare_agent(&mut command).unwrap();
    sandbox.prepare(&mut command);
    command.args(["-ec","printf retained > marker; printf ephemeral > \"$HOME/home-marker\"; printf temporary > \"$TMPDIR/tmp-marker\"; test \"$PWD\" = \"$1\"; test ! -r \"$2\"; test ! -r \"$3\"; test ! -w \"$4\"","sh"]).arg(&workspace).arg(root.path().join("metadata/journal.json")).arg(sibling_workspace.join("private")).arg(root.path());
    assert!(command.status().await.unwrap().success());
    assert!(read_as(uid, &workspace.join("marker")));
    assert!(!read_as(other.command_uid, &workspace.join("marker")));
    assert!(read_as(uid, &ephemeral.join("home/home-marker")));
    // Pinning rejects replaced/misowned retained workspaces before exec.
    std::os::unix::fs::chown(&workspace, Some(1000), Some(1000)).unwrap();
    assert!(Sandbox::open_with_workspace(&ephemeral, uid, &workspace).is_err());
}

#[test]
fn image_account_supply_matches_immutable_system_databases() {
    let supply = AccountPool::system().unwrap();
    assert!(supply.contains(20000));
    assert!(supply.contains(22047));
    assert!(!supply.contains(22048));
}
