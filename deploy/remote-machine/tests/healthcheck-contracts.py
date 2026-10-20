#!/usr/bin/env python3
"""Exercise the real read-only script with local kubectl API/permission fixtures."""
import copy
import json
import os
from pathlib import Path
import subprocess
import tempfile

SCRIPT = Path(__file__).resolve().parents[1] / "healthcheck.sh"
MANAGER = "system:serviceaccount:nyxid-machine:nyxid-machine-manager"
EXPLICIT = {"NYXID_REMOTE_MACHINE_STORAGE_CLASS": "existing-csi"}
CLASS = {"apiVersion": "storage.k8s.io/v1", "kind": "StorageClass",
         "metadata": {"name": "existing-csi", "uid": "class-uid", "creationTimestamp": "2026-10-01T00:00:00Z"},
         "provisioner": "csi.example.com"}
DEFAULT = copy.deepcopy(CLASS)
DEFAULT["metadata"]["annotations"] = {"storageclass.kubernetes.io/is-default-class": "true"}
CASES = 0


def check(name, selected, *, expected=0, contains=(), excludes=(), denied="", storage=None, error=""):
    global CASES
    with tempfile.TemporaryDirectory(prefix="nyxid-healthcheck-contract-") as directory:
        root = Path(directory)
        calls = root / "commands.jsonl"
        kubectl = root / "kubectl"
        kubectl.write_text("""#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
with open(os.environ['NYXID_TEST_COMMANDS'], 'a') as target:
    target.write(json.dumps(args) + '\\n')
joined = ' '.join(args)
if os.environ.get('NYXID_TEST_DENIED') and os.environ['NYXID_TEST_DENIED'] in joined:
    sys.exit(1)
if args[:2] == ['auth', 'can-i'] and any(value in joined for value in
    ['create pods', 'create persistentvolumeclaims', 'patch pods', 'patch persistentvolumeclaims']):
    sys.exit(1)
if args[:2] == ['get', 'validatingadmissionpolicy']:
    print('[]')
if args[:2] in (['get', 'storageclass'], ['get', 'storageclasses']):
    print(os.environ['NYXID_TEST_STORAGE'])
""")
        kubectl.chmod(0o700)
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith(("NYXID_REMOTE_MACHINE_", "NYXID_MACHINE_"))}
        default_storage = CLASS if selected.get("NYXID_REMOTE_MACHINE_STORAGE_CLASS") else {"items": [DEFAULT]}
        environment.update({"PATH": str(root) + os.pathsep + os.environ["PATH"],
                            "NYXID_TEST_COMMANDS": str(calls), "NYXID_TEST_DENIED": denied,
                            "NYXID_TEST_STORAGE": json.dumps(default_storage if storage is None else storage)})
        environment.update(selected)
        result = subprocess.run(["sh", str(SCRIPT)], env=environment, capture_output=True, text=True, timeout=20)
        assert result.returncode == expected, (name, result.returncode, result.stdout, result.stderr)
        arguments = [json.loads(line) for line in calls.read_text().splitlines()]
        commands = [" ".join(args) for args in arguments]
        # The script may inspect permissions, but it must never issue a write.
        assert all(args[0] == "get" or args[:2] in (["auth", "can-i"], ["rollout", "status"])
                   for args in arguments), (name, "mutating command", commands)
        for fragment in contains:
            assert any(fragment in command for command in commands), (name, "missing", fragment)
        for fragment in excludes:
            assert not any(fragment in command for command in commands), (name, "unexpected", fragment)
        if expected == 0:
            assert "native_api_contract_healthy" in result.stdout
        if expected != 0:
            assert "Traceback" not in result.stderr, (name, result.stderr)
        if error:
            assert error in result.stderr, (name, "missing error", error, result.stderr)
        CASES += 1
        print(name + ": passed")


check("unset storage selects actual cluster default without runtime or provider selection", {},
      contains=(f"get storageclasses --as={MANAGER}", "get storageclasses.storage.k8s.io",
                "list storageclasses.storage.k8s.io", "get nodes", "get persistentvolumes", "create tokenreviews"),
      excludes=("runtimeclass", "deployment", "nodes/stats", "nodes/proxy", "get storageclass existing-csi"))
check("delete recovery requires residual deletes and complete read permissions", EXPLICIT,
      contains=("delete pods", "delete persistentvolumeclaims", "list pods --all-namespaces",
                "list persistentvolumes", "get leases.coordination.k8s.io -n kube-node-lease",
                "get volumeattachments.storage.k8s.io", "list volumeattachments.storage.k8s.io"))
for permission in ("delete pods", "delete persistentvolumeclaims", "list pods --all-namespaces",
                   "list persistentvolumes", "get leases.coordination.k8s.io",
                   "get volumeattachments.storage.k8s.io", "list volumeattachments.storage.k8s.io"):
    check(f"delete recovery missing {permission} permission fails", EXPLICIT, expected=1, denied=permission)
check("explicit existing storage is read as manager", EXPLICIT,
      contains=(f"get storageclass existing-csi --as={MANAGER}",), excludes=("get storageclasses --as=",))
check("blank optional storage also selects cluster default", {"NYXID_REMOTE_MACHINE_STORAGE_CLASS": ""},
      contains=(f"get storageclasses --as={MANAGER}",))
check("whitespace-only optional storage selects cluster default", {"NYXID_REMOTE_MACHINE_STORAGE_CLASS": " \t\n "},
      storage={"items": [DEFAULT]}, contains=(f"get storageclasses --as={MANAGER}",))
check("configured storage name trims surrounding whitespace", {"NYXID_REMOTE_MACHINE_STORAGE_CLASS": " existing-csi \t"},
      contains=(f"get storageclass existing-csi --as={MANAGER}",))
check("legacy storage alias cannot override unset canonical cluster default", {"NYXID_MACHINE_STORAGE_CLASS": "legacy-csi"},
      contains=(f"get storageclasses --as={MANAGER}",), excludes=("get storageclass legacy-csi",))
check("legacy storage alias cannot make a missing default healthy", {"NYXID_MACHINE_STORAGE_CLASS": "legacy-csi"},
      storage={"items": [CLASS]}, expected=2, error="storageclass_default_unverified")
check("legacy storage alias cannot override blank canonical default", {
    "NYXID_REMOTE_MACHINE_STORAGE_CLASS": "", "NYXID_MACHINE_STORAGE_CLASS": "legacy-csi"},
      contains=(f"get storageclasses --as={MANAGER}",), excludes=("get storageclass legacy-csi",))
check("canonical explicit storage controls conflicting legacy alias", {
    **EXPLICIT, "NYXID_MACHINE_STORAGE_CLASS": "legacy-csi"},
      contains=(f"get storageclass existing-csi --as={MANAGER}",), excludes=("get storageclass legacy-csi",))
check("missing cluster default fails clearly", {}, storage={"items": [CLASS]}, expected=2,
      error="storageclass_default_unverified")
check("no classes fails clearly", {}, storage={"items": []}, expected=2, error="storageclass_default_unverified")
check("explicit unknown StorageClass is rejected", EXPLICIT, expected=2,
      denied="get storageclass existing-csi", error="storageclass_unverified")
check("default API permission failure is rejected", {}, expected=2,
      denied=f"get storageclasses --as={MANAGER}", error="storageclass_default_unverified")
for verb in ("get", "list"):
    check(f"required StorageClass {verb} permission is checked", EXPLICIT, expected=2,
          denied=f"{verb} storageclasses.storage.k8s.io", error="storageclass_read_permission_unverified")
for selected, prefix in ((EXPLICIT, "explicit"), ({}, "default")):
    for label, value in (("deleting", "2026-10-15T00:00:00Z"), ("empty deletion timestamp", "")):
        broken = copy.deepcopy(DEFAULT)
        broken["metadata"]["deletionTimestamp"] = value
        check(f"{prefix} {label} class rejected", selected,
              storage=broken if selected else {"items": [broken]}, expected=2)
    for provisioner in ("", "bad provisioner", "bad//driver", None):
        broken = copy.deepcopy(DEFAULT)
        broken["provisioner"] = provisioner
        check(f"{prefix} invalid provisioner {provisioner!r} rejected", selected,
              storage=broken if selected else {"items": [broken]}, expected=2)
    broken = copy.deepcopy(DEFAULT)
    del broken["metadata"]["uid"]
    check(f"{prefix} missing object identity rejected", selected,
          storage=broken if selected else {"items": [broken]}, expected=2)
beta = copy.deepcopy(DEFAULT)
beta["metadata"]["annotations"] = {"storageclass.beta.kubernetes.io/is-default-class": "true"}
check("legacy default annotation selects existing class", {}, storage={"items": [beta]})
newer = copy.deepcopy(DEFAULT)
newer["metadata"].update(name="new-default", creationTimestamp="2026-10-15T00:00:00Z")
check("changed default selects newest existing class", {}, storage={"items": [DEFAULT, newer]})
newer["metadata"]["deletionTimestamp"] = "2026-10-15T01:00:00Z"
check("unusable newest default does not fall back to old default", {},
      storage={"items": [DEFAULT, newer]}, expected=2)
wrong = copy.deepcopy(CLASS)
wrong["metadata"]["name"] = "wrong-name"
check("explicit response must match requested name", EXPLICIT, storage=wrong, expected=2)
check("malformed StorageClass response fails", {}, storage={"items": None}, expected=2)
check("non-object API response fails without a traceback", {}, storage=[], expected=2,
      error="storageclass_default_unverified")
for selected, prefix in ((EXPLICIT, "explicit"), ({}, "default")):
    valid = copy.deepcopy(DEFAULT)
    valid["metadata"]["deletionTimestamp"] = None
    valid["provisioner"] = "kubernetes.io/no-provisioner"
    check(f"{prefix} qualified provisioner and null deletion timestamp are valid metadata only", selected,
          storage=valid if selected else {"items": [valid]})
    invalid = copy.deepcopy(DEFAULT)
    invalid["metadata"]["name"] = "invalid/Class"
    check(f"{prefix} invalid class name rejected", selected,
          storage=invalid if selected else {"items": [invalid]}, expected=2)
invalid_time = copy.deepcopy(DEFAULT)
invalid_time["metadata"]["creationTimestamp"] = "not-a-timestamp"
check("default class requires a verifiable creation timestamp", {}, storage={"items": [invalid_time]}, expected=2)
tied = copy.deepcopy(DEFAULT)
tied["metadata"].update(name="z-default", deletionTimestamp="2026-10-15T01:00:00Z")
check("same-time default selection uses name ascending", {}, storage={"items": [tied, DEFAULT]})
fractional = copy.deepcopy(DEFAULT)
fractional["metadata"]["creationTimestamp"] = "2026-10-15T00:00:00.123+00:00"
check("RFC3339 fractional default creation timestamp is supported", {}, storage={"items": [fractional]})
check("explicit RuntimeClass is checked", {**EXPLICIT, "NYXID_REMOTE_MACHINE_RUNTIME_CLASS": "verified-handler"},
      contains=("get runtimeclass verified-handler",))
check("unknown configured RuntimeClass stays rejected", {**EXPLICIT, "NYXID_REMOTE_MACHINE_RUNTIME_CLASS": "missing"},
      expected=2, denied="get runtimeclass missing")
check("required Node read permission stays rejected", EXPLICIT, expected=1, denied="get nodes")
check("optional detailed Deployment checks run only on request", {
    **EXPLICIT, "NYXID_REMOTE_MACHINE_CHECK_CONTROLLER": "1",
    "NYXID_REMOTE_MACHINE_CONTROLLER_NAMESPACE": "controller-system",
    "NYXID_REMOTE_MACHINE_CONTROLLER_DEPLOYMENT": "agent-sandbox"},
    contains=("get deployment/agent-sandbox", "rollout status deployment/agent-sandbox"))
print(f"healthcheck contract: {CASES} cases passed; local kubectl fixture only; no resource writes")
