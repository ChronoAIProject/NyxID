"""Behavioral checks for Docker-policy export and immutable node installation."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


EXPORT = Path(__file__).resolve().parents[1] / "seccomp" / "export.py"


class SeccompProfileTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("machine_seccomp_export", EXPORT)
        cls.exporter = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.exporter)
        cls.source = json.loads(cls.exporter.SOURCE.read_text())

    def test_export_resolves_docker_conditions_without_granting_other_capabilities(self):
        policy = self.exporter.export_policy(self.source)
        self.assertEqual(policy["defaultAction"], "SCMP_ACT_ERRNO")
        self.assertEqual(policy["architectures"], ["SCMP_ARCH_X86_64", "SCMP_ARCH_AARCH64"])
        allowed = set()
        for rule in policy["syscalls"]:
            self.assertLessEqual(set(rule), {"names", "action", "args", "errnoRet"})
            if rule["action"] == "SCMP_ACT_ALLOW":
                allowed.update(rule["names"])
        self.assertTrue({"clone", "unshare", "setns", "chroot", "mount", "umount2", "pivot_root"} <= allowed)
        self.assertFalse({"bpf", "perf_event_open", "init_module", "reboot", "open_by_handle_at"} & allowed)
        clone3 = [r for r in policy["syscalls"] if r["names"] == ["clone3"]]
        self.assertEqual(clone3, [{"names": ["clone3"], "action": "SCMP_ACT_ERRNO", "errnoRet": 38}])

    def test_unknown_or_newer_conditions_fail_instead_of_becoming_unconditional(self):
        for selector in [{"unknown": True}, {"minKernel": "99.0"}, {"minKernel": "4.8.1"}]:
            source = copy.deepcopy(self.source)
            source["syscalls"].append({"names": ["bpf"], "action": "SCMP_ACT_ALLOW", "includes": selector})
            with self.assertRaises(ValueError):
                self.exporter.export_policy(source)

    def test_capability_includes_require_every_capability(self):
        source = copy.deepcopy(self.source)
        source["syscalls"].append({"names": ["requires_both"], "action": "SCMP_ACT_ALLOW",
                                   "includes": {"caps": ["CAP_CHOWN", "CAP_SYS_ADMIN"]}})
        self.assertFalse(any("requires_both" in rule["names"]
                             for rule in self.exporter.export_policy(source)["syscalls"]))

    def test_architecture_conditions_cannot_widen_common_native_syscalls(self):
        source = copy.deepcopy(self.source)
        source["syscalls"].append({"names": ["bpf"], "action": "SCMP_ACT_ALLOW",
                                   "includes": {"arches": ["amd64"]}})
        with self.assertRaises(ValueError):
            self.exporter.export_policy(source)

    def test_artifact_name_binds_exact_bytes_and_is_deterministic(self):
        policy = self.exporter.export_policy(self.source)
        reference, content = self.exporter.artifact(policy)
        self.assertEqual((reference, content), self.exporter.artifact(policy))
        self.assertEqual(reference, "nyxid/remote-machine-v1-" + hashlib.sha256(content).hexdigest() + ".json")

    def test_install_is_immutable_and_refuses_symlinks_and_different_content(self):
        reference, content = self.exporter.artifact(self.exporter.export_policy(self.source))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            installed = self.exporter.install_artifact(root, reference, content)
            self.assertEqual(installed.read_bytes(), content)
            self.assertEqual(installed.stat().st_mode & 0o777, 0o644)
            self.assertEqual(self.exporter.install_artifact(root, reference, content), installed)
            installed.write_bytes(b"different")
            with self.assertRaises(ValueError):
                self.exporter.install_artifact(root, reference, content)
            installed.unlink()
            outside = root / "outside"
            outside.write_bytes(content)
            installed.symlink_to(outside)
            with self.assertRaises(ValueError):
                self.exporter.install_artifact(root, reference, content)

    def test_install_binds_exact_bytes_not_just_equivalent_json(self):
        reference, content = self.exporter.artifact(self.exporter.export_policy(self.source))
        noncanonical = json.dumps(json.loads(content), separators=(",", ":")).encode()
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(ValueError):
                self.exporter.install_artifact(Path(temporary).resolve(), reference, noncanonical)

    def test_install_does_not_publish_before_contents_are_synced(self):
        reference, content = self.exporter.artifact(self.exporter.export_policy(self.source))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()

            def fail_sync(_descriptor):
                self.assertFalse((root / reference).exists(), "partial profile became visible")
                raise OSError("simulated disk failure")

            with patch.object(self.exporter.os, "fsync", side_effect=fail_sync):
                with self.assertRaises(OSError):
                    self.exporter.install_artifact(root, reference, content)
            self.assertEqual(list((root / "nyxid").iterdir()), [])

    def test_concurrent_install_never_overwrites_or_removes_an_existing_profile(self):
        reference, content = self.exporter.artifact(self.exporter.export_policy(self.source))
        for competing in [content, b"different"]:
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary).resolve()
                target = root / reference

                def concurrent_publish(_source, _target):
                    target.write_bytes(competing)
                    raise FileExistsError("another installer published first")

                with patch.object(self.exporter.os, "link", side_effect=concurrent_publish):
                    if competing == content:
                        self.exporter.install_artifact(root, reference, content)
                    else:
                        with self.assertRaises(ValueError):
                            self.exporter.install_artifact(root, reference, content)
                self.assertEqual(target.read_bytes(), competing)
                self.assertEqual(list(target.parent.iterdir()), [target])


if __name__ == "__main__":
    unittest.main()
