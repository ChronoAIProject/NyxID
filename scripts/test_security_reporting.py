"""Regression cases for managed chooser output and private bug-report routing."""
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("check_security", ROOT / "scripts/check-security-reporting.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class SecurityReportingTests(unittest.TestCase):
    def test_missing_contact_fails_without_changing_managed_output(self):
        generated = """# fkst-issue-templates-version: 14
# Managed by fkst-hosted; do not edit.
blank_issues_enabled: false
contact_links:
  - name: fkst docs & help
    url: https://github.com/ChronoAIProject/fkst-hosted
    about: Get help.
"""
        self.assertFalse(module.has_security_contact(generated))
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.yml"
            path.write_text(generated)
            result = subprocess.run(
                [sys.executable, str(ROOT / "scripts/check-security-reporting.py"), str(path)],
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 1)
            self.assertIn("contact missing", result.stderr)
            self.assertEqual(path.read_text(), generated)

    def test_upstream_policy_contact_preserves_versions_without_duplicates(self):
        for version in [13, 14, 27]:
            generated = (f"# fkst-issue-templates-version: {version}\n"
                         "blank_issues_enabled: false\ncontact_links:\n" + module.POLICY_CONTACT)
            with self.subTest(version=version), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "config.yml"
                path.write_text(generated)
                result = subprocess.run(
                    [sys.executable, str(ROOT / "scripts/check-security-reporting.py"), str(path)],
                    capture_output=True, text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(path.read_text(), generated)
                self.assertNotIn(module.CONTACT, path.read_text())

    def test_missing_contact_in_unexpected_generator_shape_fails(self):
        for generated in ["blank_issues_enabled: false\n", "contact_links: []\n"]:
            self.assertFalse(module.has_security_contact(generated))

    def test_repository_surfaces_link_to_private_reporting(self):
        chooser = (ROOT / ".github/ISSUE_TEMPLATE/config.yml").read_text()
        self.assertTrue(module.has_security_contact(chooser))
        for path in ["SECURITY.md", ".github/ISSUE_TEMPLATE/bug_report.yml"]:
            content = (ROOT / path).read_text()
            self.assertIn("https://github.com/ChronoAIProject/NyxID/security/advisories/new", content)
        self.assertRegex(chooser, r"(?m)^# fkst-issue-templates-version: [1-9][0-9]*$")


if __name__ == "__main__":
    unittest.main()
