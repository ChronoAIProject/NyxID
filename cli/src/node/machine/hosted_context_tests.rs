//! Opt-in filesystem tests run in an isolated Linux container, with the formal
//! supervisor capabilities and actual dropped worker credentials.
use super::*;
use crate::node::machine::process::{Identity, context_sandbox::Sandbox};
use nyxid_machine::authority::Authority;
use std::{fs, os::unix::fs::PermissionsExt, process::Command, sync::Arc};

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn fixture() -> (tempfile::TempDir, PathBuf, VolumeBinding, String) {
    assert_eq!(
        std::env::var("NYXID_TEST_HOSTED_STORAGE").as_deref(),
        Ok("1")
    );
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let status = fs::read_to_string("/proc/self/status").unwrap();
    for field in ["CapEff", "CapPrm", "CapBnd"] {
        let actual = status
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{field}:")))
            .unwrap();
        assert_eq!(u64::from_str_radix(actual.trim(), 16).unwrap(), 0xe1);
    }
    assert!(status.lines().any(|line| line == "NoNewPrivs:\t1"));
    assert!(status.lines().any(|line| line == "Umask:\t0077"));
    let base = tempfile::tempdir_in("/var/lib/nyxid-hosted-tests").unwrap();
    fs::set_permissions(base.path(), fs::Permissions::from_mode(0o711)).unwrap();
    let root = base.path().join("retained");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o711)).unwrap();
    for ancestor in root.ancestors() {
        let info = fs::metadata(ancestor).unwrap();
        println!(
            "ancestor={} mode={:o}",
            ancestor.display(),
            info.mode() & 0o7777
        );
    }
    let binding = VolumeBinding {
        node_id: id(),
        pvc_uid: id(),
    };
    (base, root, binding, id())
}
fn open(
    root: &Path,
    binding: &VolumeBinding,
    runtime: &str,
    initialize: bool,
) -> Result<HostedStore> {
    let pool =
        serde_json::from_value(serde_json::json!({"version":1,"first_uid":20000,"count":2048}))
            .unwrap();
    HostedStore::open(root, binding.clone(), runtime, initialize, pool)
}
fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().mode() & 0o7777
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

#[test]
#[ignore = "requires isolated Linux root filesystem and NYXID_TEST_HOSTED_STORAGE=1"]
fn retained_volume_contract_fresh_authority_initializes_only_public_root() {
    for initial in [0o700, 0o711, 0o755, 0o777] {
        let (_base, root, binding, runtime) = fixture();
        fs::set_permissions(&root, fs::Permissions::from_mode(initial)).unwrap();
        let legacy = root.join("legacy");
        fs::create_dir(&legacy).unwrap();
        fs::set_permissions(&legacy, fs::Permissions::from_mode(0o770)).unwrap();
        fs::write(legacy.join("marker"), "retained").unwrap();
        let before = fs::metadata(&root).unwrap();
        assert!(open(&root, &binding, &runtime, false).is_err());
        assert_eq!(mode(&root), initial);
        assert!(!root.join("metadata").exists());
        drop(open(&root, &binding, &runtime, true).unwrap());
        let after = fs::metadata(&root).unwrap();
        assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
        assert_eq!((after.uid(), after.gid(), mode(&root)), (0, 0, 0o711));
        assert_eq!(mode(&root.join("contexts")), 0o711);
        assert_eq!(mode(&root.join("metadata")), 0o700);
        assert_eq!(mode(&legacy), 0o770);
        assert_eq!(
            fs::read_to_string(legacy.join("marker")).unwrap(),
            "retained"
        );
        drop(open(&root, &binding, &id(), false).unwrap());
    }
}

#[test]
#[ignore = "requires isolated Linux root filesystem and NYXID_TEST_HOSTED_STORAGE=1"]
fn retained_volume_contract_recovery_never_repairs_anomalies() {
    for defect in [
        "root700",
        "root755",
        "root777",
        "root-special",
        "root-group",
        "contexts700",
        "contexts-special",
        "contexts-group",
        "metadata-group",
        "missing-journal",
        "wrong-volume",
        "gate-special",
        "workspace-special",
    ] {
        let (_base, root, binding, runtime) = fixture();
        let mut store = open(&root, &binding, &runtime, true).unwrap();
        let (allocation, workspace) = store.provision(&authority()).unwrap();
        drop(store);
        let journal = root.join("metadata/journal.json");
        let original = fs::read(&journal).unwrap();
        match defect {
            "root700" => fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap(),
            "root755" => fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap(),
            "root777" => fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap(),
            "root-special" => {
                fs::set_permissions(&root, fs::Permissions::from_mode(0o1711)).unwrap()
            }
            "root-group" => std::os::unix::fs::chown(&root, Some(0), Some(1000)).unwrap(),
            "contexts700" => {
                fs::set_permissions(root.join("contexts"), fs::Permissions::from_mode(0o700))
                    .unwrap()
            }
            "contexts-special" => {
                fs::set_permissions(root.join("contexts"), fs::Permissions::from_mode(0o1711))
                    .unwrap()
            }
            "contexts-group" => {
                std::os::unix::fs::chown(root.join("contexts"), Some(0), Some(1000)).unwrap()
            }
            "metadata-group" => {
                std::os::unix::fs::chown(root.join("metadata"), Some(0), Some(1000)).unwrap()
            }
            "missing-journal" => fs::remove_file(&journal).unwrap(),
            "wrong-volume" => {}
            "gate-special" => {
                // Linux clears SGID without FSETID unless the caller belongs
                // to the file's group. Change only this fixture's effective
                // group with SETGID, then restore the supervisor immediately.
                assert_eq!(unsafe { libc::setegid(allocation.command_uid) }, 0);
                let changed = fs::set_permissions(
                    workspace.parent().unwrap(),
                    fs::Permissions::from_mode(0o2710),
                );
                assert_eq!(unsafe { libc::setegid(0) }, 0);
                changed.unwrap();
                assert_eq!(mode(workspace.parent().unwrap()), 0o2710);
            }
            "workspace-special" => {
                assert!(
                    Command::new("setpriv")
                        .args([
                            "--reuid",
                            &allocation.command_uid.to_string(),
                            "--regid",
                            &allocation.command_uid.to_string(),
                            "--clear-groups",
                            "chmod",
                            "1700"
                        ])
                        .arg(&workspace)
                        .status()
                        .unwrap()
                        .success()
                );
            }
            _ => unreachable!(),
        }
        let before = fs::metadata(&root).unwrap();
        let wrong = VolumeBinding {
            node_id: binding.node_id.clone(),
            pvc_uid: id(),
        };
        let selected = if defect == "wrong-volume" {
            &wrong
        } else {
            &binding
        };
        for initialize in [false, true] {
            assert!(
                open(&root, selected, &runtime, initialize).is_err(),
                "accepted {defect} / {initialize}"
            );
        }
        let after = fs::metadata(&root).unwrap();
        assert_eq!(
            (
                after.dev(),
                after.ino(),
                after.uid(),
                after.gid(),
                after.mode()
            ),
            (
                before.dev(),
                before.ino(),
                before.uid(),
                before.gid(),
                before.mode()
            )
        );
        if defect == "missing-journal" {
            assert!(!journal.exists());
        } else {
            assert_eq!(fs::read(&journal).unwrap(), original);
        }
    }
}

#[test]
#[ignore = "requires isolated Linux root filesystem and NYXID_TEST_HOSTED_STORAGE=1"]
fn retained_volume_contract_fresh_anomaly_is_not_initialization_permission() {
    for defect in [
        "unknown",
        "contexts",
        "metadata-symlink",
        "legacy-symlink",
        "root-symlink",
        "root-file",
        "root-owner",
        "root-group",
        "root-special",
    ] {
        let (base, root, binding, runtime) = fixture();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
        match defect {
            "unknown" => fs::write(root.join("unknown"), "preserve").unwrap(),
            "contexts" => fs::create_dir(root.join("contexts")).unwrap(),
            "metadata-symlink" => {
                std::os::unix::fs::symlink(base.path(), root.join("metadata")).unwrap()
            }
            "legacy-symlink" => {
                std::os::unix::fs::symlink(base.path(), root.join("legacy")).unwrap()
            }
            "root-symlink" => {
                fs::remove_dir(&root).unwrap();
                std::os::unix::fs::symlink(base.path(), &root).unwrap();
            }
            "root-file" => {
                fs::remove_dir(&root).unwrap();
                fs::write(&root, "preserve").unwrap();
            }
            "root-owner" => std::os::unix::fs::chown(&root, Some(1000), Some(0)).unwrap(),
            "root-group" => std::os::unix::fs::chown(&root, Some(0), Some(1000)).unwrap(),
            "root-special" => {
                fs::set_permissions(&root, fs::Permissions::from_mode(0o1777)).unwrap()
            }
            _ => unreachable!(),
        }
        let before = fs::symlink_metadata(&root).unwrap();
        assert!(
            open(&root, &binding, &runtime, true).is_err(),
            "accepted {defect}"
        );
        let after = fs::symlink_metadata(&root).unwrap();
        assert_eq!(
            (
                after.dev(),
                after.ino(),
                after.uid(),
                after.gid(),
                after.mode()
            ),
            (
                before.dev(),
                before.ino(),
                before.uid(),
                before.gid(),
                before.mode()
            )
        );
        if defect == "unknown" {
            assert_eq!(fs::read(root.join("unknown")).unwrap(), b"preserve");
        }
    }
}

#[tokio::test]
#[ignore = "requires isolated Linux root filesystem and NYXID_TEST_HOSTED_STORAGE=1"]
async fn retained_volume_contract_real_worker_and_sandbox_preserve_isolation() {
    let (base, root, binding, runtime) = fixture();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
    let mut store = open(&root, &binding, &runtime, true).unwrap();
    println!(
        "landlock_abi={}",
        crate::node::machine::process::context_sandbox::abi().unwrap()
    );
    let mut a = authority();
    let (allocation, workspace) = store.provision(&a).unwrap();
    let (other, other_workspace) = store.provision(&authority()).unwrap();
    let uid = allocation.command_uid;
    let gate = workspace.parent().unwrap();
    assert_eq!(
        (
            fs::metadata(gate).unwrap().uid(),
            fs::metadata(gate).unwrap().gid(),
            mode(gate)
        ),
        (0, uid, 0o710)
    );
    assert_eq!(
        (
            fs::metadata(&workspace).unwrap().uid(),
            fs::metadata(&workspace).unwrap().gid(),
            mode(&workspace)
        ),
        (uid, uid, 0o700)
    );
    let ephemeral = base.path().join("ephemeral");
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
    let mut command = tokio::process::Command::new("/bin/sh");
    identity.prepare_agent(&mut command).unwrap();
    command.args(["-ec", r#"
        test "$(id -u)" = "$1"; test "$(id -g)" = "$1"; test "$(id -G)" = "$1"
        python3 -c 'import ctypes,os; class_header=type("Header",(ctypes.Structure,),{"_fields_":[("version",ctypes.c_uint32),("pid",ctypes.c_int)]}); class_data=type("Data",(ctypes.Structure,),{"_fields_":[("effective",ctypes.c_uint32),("permitted",ctypes.c_uint32),("inheritable",ctypes.c_uint32)]}); header=class_header(0x20080522,0); data=(class_data*2)(); assert ctypes.CDLL(None).capget(ctypes.byref(header),ctypes.byref(data)) == 0; assert all(row.effective == row.permitted == row.inheritable == 0 for row in data); print("worker_capabilities=0")'
        ! cat /proc/self/status 2>/dev/null
        printf initial > relative; test "$(cat relative)" = initial
        printf absolute > "$2/absolute"; test "$(cat "$2/absolute")" = absolute
        printf modified > relative; test "$(cat "$2/relative")" = modified
        ls . > /dev/null; ls "$2" > /dev/null
        ! ls "$3" 2>/dev/null; ! ls "$3/contexts" 2>/dev/null
        ! touch "$3/parent-write" 2>/dev/null; ! touch "$3/contexts/parent-write" 2>/dev/null
        ! ls "$4" 2>/dev/null; ! touch "$4/other-write" 2>/dev/null
        ! cat "$3/metadata/journal.json" 2>/dev/null
        ! touch ../../escape 2>/dev/null; ! cat /etc/shadow 2>/dev/null
        ln -s "$3/metadata/journal.json" metadata-link
        ! cat metadata-link 2>/dev/null
        printf 'worker_uid=%s worker_gid=%s groups=%s sandbox=active\n' "$(id -u)" "$(id -g)" "$(id -G)"
    "#, "sh"]).arg(uid.to_string()).arg(&workspace).arg(&root).arg(&other_workspace);
    assert!(command.status().await.unwrap().success());
    let denied = Command::new("setpriv")
        .args([
            "--reuid",
            &other.command_uid.to_string(),
            "--regid",
            &other.command_uid.to_string(),
            "--clear-groups",
            "cat",
        ])
        .arg(workspace.join("relative"))
        .status()
        .unwrap();
    assert!(!denied.success());
    drop(store);
    let journal = fs::read(root.join("metadata/journal.json")).unwrap();
    let mut recovered = open(&root, &binding, &id(), false).unwrap();
    assert_eq!(
        fs::read(root.join("metadata/journal.json")).unwrap(),
        journal
    );
    assert!(recovered.provision(&a).is_err());
    a.generation = 2;
    a.revision = 2;
    let (next, next_workspace) = recovered.provision(&a).unwrap();
    assert_eq!(next.command_uid, uid);
    assert_eq!(next_workspace, workspace);
    assert!(
        Command::new("setpriv")
            .args([
                "--reuid",
                &uid.to_string(),
                "--regid",
                &uid.to_string(),
                "--clear-groups",
                "sh",
                "-ec",
                "test \"$(cat \"$1/relative\")\" = modified",
                "sh"
            ])
            .arg(&workspace)
            .status()
            .unwrap()
            .success()
    );
    println!("recovery_workspace generation=2 same_workspace=true retained_marker=true");
    println!(
        "supervisor_status={}",
        fs::read_to_string("/proc/self/status")
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("Uid:")
                || line.starts_with("Gid:")
                || line.starts_with("Groups:")
                || line.starts_with("CapEff:")
                || line.starts_with("NoNewPrivs:")
                || line.starts_with("Seccomp:"))
            .collect::<Vec<_>>()
            .join(";")
    );
}
