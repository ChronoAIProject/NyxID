import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from coverage_identity import INPUTS, fingerprint, matching_identities


SCRIPT = Path(__file__).with_name("coverage_identity.py").resolve()


class CoverageIdentityTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        for name in INPUTS:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name)

    def identity(self, image="20260927.320.1", arch="x86_64"):
        return fingerprint(self.root, "ubuntu24", image, arch)

    def test_image_and_architecture_changes_invalidate_reports(self):
        self.assertNotEqual(self.identity(), self.identity(image="next-image"))
        self.assertNotEqual(self.identity(), self.identity(arch="aarch64"))

    def test_each_measurement_input_invalidates_reports(self):
        original = self.identity()
        for name in INPUTS:
            with self.subTest(name=name):
                (self.root / name).write_text("changed")
                self.assertNotEqual(original, self.identity())
                (self.root / name).write_text(name)

    def test_nextest_profile_changes_invalidate_reports(self):
        original = self.identity()
        config = self.root / ".config/nextest.toml"
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text("[profile.ci]\ntest-threads = 1\n")
        self.assertNotEqual(original, self.identity())

    def test_all_measurement_runners_must_match_for_report_reuse(self):
        paths = [self.root / f"runner-{index}.json" for index in range(4)]
        for path in paths:
            path.write_text(json.dumps({"fingerprint": self.identity(), "reusable": True}))
        self.assertTrue(matching_identities(paths, self.identity()))
        for identity in ({"fingerprint": "old-image", "reusable": True},
                         {"fingerprint": self.identity(), "reusable": False}, []):
            with self.subTest(identity=identity):
                paths[-1].write_text(json.dumps(identity))
                self.assertFalse(matching_identities(paths, self.identity()))

    def test_missing_or_invalid_runner_identity_disables_reuse(self):
        path = self.root / "identity.json"
        self.assertFalse(matching_identities([path], self.identity()))
        path.write_text("invalid json")
        self.assertFalse(matching_identities([path], self.identity()))

    def test_missing_runner_identity_disables_reuse(self):
        for image_os, image_version, reusable in [("ubuntu24", "v1", "true"),
                                                   ("", "v1", "false"),
                                                   ("ubuntu24", "", "false")]:
            output = self.root / "output"
            output.write_text("")
            result = subprocess.run(
                [sys.executable, str(SCRIPT)], cwd=self.root,
                env={**os.environ, "ImageOS": image_os, "ImageVersion": image_version,
                     "GITHUB_OUTPUT": str(output)}, capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"reusable={reusable}\n", output.read_text())


if __name__ == "__main__":
    unittest.main()
