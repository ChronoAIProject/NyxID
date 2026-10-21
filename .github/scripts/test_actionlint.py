"""Execute the pinned GitHub semantic validator against real negative fixtures."""
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


checker = module("check-actionlint")
bootstrap = module("bootstrap-build-tool")

CALLED = """name: Called
on:
  workflow_call:
    inputs:
      enabled: {type: boolean, required: true}
    secrets:
      credential: {required: false}
permissions: {contents: read}
jobs:
  build:
    runs-on: ubuntu-24.04
    steps:
      - run: echo "runner=${{ runner.temp }}"
"""
CALLER = """name: Caller
on: pull_request
permissions: {contents: read}
jobs:
  call:
    uses: ./.github/workflows/called.yml
    with:
      enabled: true
"""


class ActionlintSemantics(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tools = tempfile.TemporaryDirectory(prefix="nyxid-actionlint-tests-")
        cls.addClassCleanup(cls.tools.cleanup)
        cls.binary = checker.install(Path(cls.tools.name))

    def fixture(self, caller=CALLER, called=CALLED, expected=None):
        with tempfile.TemporaryDirectory(prefix="nyxid-actionlint-fixture-") as temporary:
            root = Path(temporary)
            (root / ".git").mkdir()
            workflows = root / ".github/workflows"
            workflows.mkdir(parents=True)
            (workflows / "caller.yml").write_text(caller)
            (workflows / "called.yml").write_text(called)
            result = checker.validate(self.binary, root)
        if expected is None:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(expected, result.stdout + result.stderr)

    def test_all_repository_workflows_have_valid_semantics(self):
        result = checker.validate(self.binary, ROOT)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_reusable_inputs_secrets_and_step_runner_context_pass(self):
        self.fixture()

    def test_runner_context_in_job_env_is_rejected_before_dispatch(self):
        called = CALLED.replace("    steps:", "    env:\n      CARGO_TARGET_DIR: ${{ runner.temp }}/target\n    steps:")
        self.fixture(called=called, expected='context "runner" is not allowed here')

    def test_runner_context_in_step_env_is_valid(self):
        self.fixture(called=CALLED.replace('      - run:', '      - env: {TARGET: "${{ runner.temp }}/target"}\n        run:'))

    def test_unknown_needs_is_rejected(self):
        self.fixture(caller=CALLER.replace("    uses:", "    needs: missing\n    uses:"), expected='missing')

    def test_invalid_if_expression_is_rejected(self):
        self.fixture(caller=CALLER.replace("    uses:", "    if: ${{ needs. }}\n    uses:"), expected='[expression]')

    def test_missing_local_reusable_workflow_is_rejected(self):
        self.fixture(caller=CALLER.replace("called.yml", "missing.yml"), expected='missing.yml')

    def test_callee_without_workflow_call_is_rejected(self):
        self.fixture(called=CALLED.replace("  workflow_call:", "  workflow_dispatch:"), expected='workflow_call')

    def test_undeclared_input_is_rejected(self):
        self.fixture(caller=CALLER.replace("      enabled: true", "      enabled: true\n      surprise: true"), expected='input "surprise"')

    def test_invalid_declared_input_type_is_rejected(self):
        self.fixture(called=CALLED.replace("type: boolean", "type: integer"), expected='type')

    def test_missing_required_input_is_rejected(self):
        self.fixture(caller=CALLER.replace("    with:\n      enabled: true\n", ""), expected='input "enabled" is required')

    def test_undeclared_secret_is_rejected(self):
        self.fixture(caller=CALLER + '    secrets:\n      surprise: ${{ secrets.GITHUB_TOKEN }}\n', expected='secret "surprise"')

    def test_invalid_permission_level_is_rejected(self):
        self.fixture(caller=CALLER.replace('contents: read', 'contents: execute'), expected='permission')

    def test_missing_required_secret_is_rejected(self):
        self.fixture(called=CALLED.replace('credential: {required: false}', 'credential: {required: true}'),
                     expected='secret "credential" is required')


class ActionlintPins(unittest.TestCase):
    def test_every_supported_host_has_official_archive_checksum_and_refresh(self):
        catalog = json.loads((ROOT / "security/build-tools.json").read_text())["actionlint"]
        self.assertEqual(set(catalog["targets"]), {"aarch64-apple-darwin", "x86_64-apple-darwin",
                         "aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"})
        for target in catalog["targets"].values():
            self.assertIn("https://github.com/rhysd/actionlint/releases/download/v" + catalog["version"] + "/", target["url"])
            self.assertRegex(target["sha256"], r"^[0-9a-f]{64}$")
        renovate = json.loads((ROOT / "renovate.json").read_text())
        self.assertTrue(any(manager.get("depNameTemplate") == "rhysd/actionlint" for manager in renovate["customManagers"]))

    def test_corrupt_tool_archive_is_rejected_before_execution(self):
        with tempfile.TemporaryDirectory() as temporary:
            response = io.BytesIO(b"corrupted executable")
            response.url = "https://github.com/rhysd/actionlint/releases/download/v1.7.12/archive"
            with patch.object(bootstrap.urllib.request, "urlopen", return_value=response), \
                    patch.object(bootstrap.subprocess, "run") as execute:
                with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                    bootstrap.download_verified({"url": response.url, "sha256": "0" * 64}, Path(temporary) / "archive")
                execute.assert_not_called()


if __name__ == "__main__":
    unittest.main()
