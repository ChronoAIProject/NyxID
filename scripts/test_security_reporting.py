"""Regression cases for managed chooser output and private bug-report routing."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("reconcile_security", ROOT / "scripts/reconcile-security-reporting.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class SecurityReportingTests(unittest.TestCase):
    def test_generated_chooser_retains_marker_and_existing_links(self):
        generated = """# fkst-issue-templates-version: 14
# Managed by fkst-hosted; do not edit.
blank_issues_enabled: false
contact_links:
  - name: fkst docs & help
    url: https://github.com/ChronoAIProject/fkst-hosted
    about: Get help.
"""
        result = module.reconcile(generated)
        self.assertEqual(result.replace(module.CONTACT, ""), generated)
        self.assertEqual(module.reconcile(result), result)

    def test_upstream_policy_contact_preserves_versions_without_duplicates(self):
        for version in [13, 14, 27]:
            generated = (f"# fkst-issue-templates-version: {version}\n"
                         "blank_issues_enabled: false\ncontact_links:\n" + module.POLICY_CONTACT)
            self.assertEqual(module.reconcile(generated), generated)
            self.assertNotIn(module.CONTACT, module.reconcile(generated))

    def test_unexpected_generator_shape_fails_for_manual_review(self):
        for generated in ["blank_issues_enabled: false\n", "contact_links: []\n"]:
            with self.assertRaises(ValueError):
                module.reconcile(generated)

    def test_repository_surfaces_link_to_private_reporting(self):
        chooser = (ROOT / ".github/ISSUE_TEMPLATE/config.yml").read_text()
        self.assertEqual(module.reconcile(chooser), chooser)
        for path in ["SECURITY.md", ".github/ISSUE_TEMPLATE/bug_report.yml"]:
            content = (ROOT / path).read_text()
            self.assertIn("https://github.com/ChronoAIProject/NyxID/security/advisories/new", content)
        self.assertRegex(chooser, r"(?m)^# fkst-issue-templates-version: [1-9][0-9]*$")


if __name__ == "__main__":
    unittest.main()
