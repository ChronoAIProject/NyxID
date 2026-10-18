import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("policy", Path(__file__).with_name("check-node-install-policy.py"))
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)


class InstallPolicyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "security").mkdir()
        (self.root / "app").mkdir()
        self.entry = {"lock_path": "node_modules/fixture", "name": "fixture", "version": "1.0.0",
                      "integrity": "sha512-fixture", "allow": False, "reason": "No generated runtime files required"}
        self.write("security/node-install-policy.json", {"schema": "nyxid.node-install-policy.v1", "roots": [{"path": "app", "install_scripts": [self.entry]}]})
        self.write("app/package.json", {"allowScripts": {"fixture": False}})
        self.lock = {"packages": {"node_modules/fixture": {"version": "1.0.0", "integrity": "sha512-fixture", "hasInstallScript": True}}}
        self.write("app/package-lock.json", self.lock)
        (self.root / "app/.npmrc").write_text("ignore-scripts=true\n")

    def write(self, name, data):
        (self.root / name).write_text(json.dumps(data))

    def check(self):
        return policy.check(self.root, {"app"})

    def test_reviewed_denial(self):
        self.assertEqual(self.check(), [])

    def test_unknown_or_changed_install_script_identity_fails(self):
        for field in ("version", "integrity"):
            changed = json.loads(json.dumps(self.lock))
            changed["packages"]["node_modules/fixture"][field] = "changed"
            self.write("app/package-lock.json", changed)
            self.assertTrue(any("missing exact" in error for error in self.check()))
        self.lock["packages"]["node_modules/new-package"] = {"version": "1", "hasInstallScript": True}
        self.write("app/package-lock.json", self.lock)
        self.assertTrue(any("new-package" in error for error in self.check()))

    def test_enabling_lifecycle_or_blanket_approval_fails(self):
        (self.root / "app/.npmrc").write_text("ignore-scripts=false\n")
        self.write("app/package.json", {"allowScripts": {"*": True}})
        errors = self.check()
        self.assertTrue(any("execution must be disabled" in error for error in errors))
        self.assertTrue(any("explicit reviewed denials" in error for error in errors))

    def test_missing_root_review_fails(self):
        self.write("security/node-install-policy.json", {"schema": "nyxid.node-install-policy.v1", "roots": []})
        self.assertTrue(any("every required root" in error for error in self.check()))


if __name__ == "__main__":
    unittest.main()
