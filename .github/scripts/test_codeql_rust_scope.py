import copy
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import yaml

from ci_gate_reuse import strict_load_workflow
from codeql_rust_scope import LANGUAGES, WORKFLOW_FILE, prepare_config, select_languages

REPO = Path(__file__).resolve().parents[2]


def glob_match(pattern: str, path: str) -> bool:
    """Match the *, ** and **/ subset used in the action's filters, including dots."""
    parts = []
    index = 0
    while index < len(pattern):
        if pattern.startswith("**/", index):
            parts.append("(?:.*/)?")
            index += 3
        elif pattern.startswith("**", index):
            parts.append(".*")
            index += 2
        else:
            char = pattern[index]
            parts.append("[^/]*" if char == "*" else re.escape(char))
            index += 1
    return re.fullmatch("".join(parts), path) is not None


class LanguageSelectionTests(unittest.TestCase):
    def test_only_successful_explicit_negative_pr_detection_omits_rust(self):
        for event in ("pull_request", "push", "schedule", "workflow_dispatch", "unknown", ""):
            for outcome in ("success", "failure", "cancelled", "skipped", ""):
                for changed in ("true", "false", "", "invalid"):
                    with self.subTest(event=event, outcome=outcome, changed=changed):
                        omit = (event, outcome, changed) == ("pull_request", "success", "false")
                        self.assertEqual(select_languages(event, outcome, changed),
                                         LANGUAGES[:-1] if omit else LANGUAGES)

    def test_matrix_command_writes_json_and_missing_detection_includes_rust(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "output"
            env = {**os.environ, "GITHUB_OUTPUT": str(output), "CODEQL_EVENT": "pull_request",
                   "DETECTION_OUTCOME": "", "RUST_CHANGED": ""}
            subprocess.run([sys.executable, str(REPO / ".github/scripts/codeql_rust_scope.py"), "matrix"],
                           env=env, check=True, capture_output=True)
            self.assertEqual(json.loads(output.read_text().removeprefix("languages=")), LANGUAGES)


class ConfigTests(unittest.TestCase):
    def test_only_prs_append_proven_test_files_and_preserve_query_settings(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            base_path = root / ".github/codeql/codeql-config.yml"
            base_path.parent.mkdir(parents=True)
            base = {"name": "fixture", "paths-ignore": ["vendor/**"],
                    "queries": [{"uses": "security-extended"}],
                    "packs": ["codeql/rust-queries@0.1.46"]}
            base_path.write_text(yaml.safe_dump(base))
            (root / "Cargo.toml").write_text('[package]\nname="fixture"\nversion="0.1.0"\n')
            (root / "src").mkdir()
            (root / "src/lib.rs").write_text('#[cfg(test)] mod cases; mod tests;')
            (root / "src/cases.rs").write_text('fn test_only() {}')
            (root / "src/tests.rs").write_text('pub fn shipped_despite_name() {}')
            for event in ("pull_request", "push", "schedule", "workflow_dispatch", "unknown"):
                with self.subTest(event=event):
                    dest = root / f"{event}.yml"
                    paths = prepare_config(root, event, dest)
                    expected = copy.deepcopy(base)
                    if event == "pull_request":
                        expected["paths-ignore"].append("src/cases.rs")
                    self.assertEqual(yaml.safe_load(dest.read_text()), expected)
                    self.assertEqual(paths, ["src/cases.rs"] if event == "pull_request" else [])
                    self.assertEqual(dest.with_suffix('.excluded.txt').read_text().splitlines(), paths)

    def test_generator_failure_propagates_without_publishing_config(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "config.yml"
            with patch("codeql_rust_scope.analyze_tree", side_effect=RuntimeError("fixture failure")):
                with self.assertRaisesRegex(RuntimeError, "fixture failure"):
                    prepare_config(REPO, "pull_request", output)
            self.assertFalse(output.exists())
            self.assertFalse(output.with_suffix('.excluded.txt').exists())
            # An actual CLI/config error must also produce a failed step.
            result = subprocess.run([sys.executable, str(REPO / ".github/scripts/codeql_rust_scope.py"),
                                     "config", "--root", tmp, "--event", "pull_request", "--output", str(output)],
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())


class WorkflowTests(unittest.TestCase):
    def setUp(self):
        self.workflow = strict_load_workflow((REPO / WORKFLOW_FILE).read_text())
        self.detection = next(step for step in self.workflow["jobs"]["changes"]["steps"]
                              if step.get("uses") == "dorny/paths-filter@v4")
        self.patterns = yaml.safe_load(self.detection["with"]["filters"])["rust"]

    def matches(self, paths):
        return any(glob_match(pattern, path) for path in paths for pattern in self.patterns)

    def test_workflow_uses_uncapped_git_detection_and_fails_open(self):
        events = self.workflow["on"]
        self.assertEqual(set(events), {"pull_request", "push", "schedule", "workflow_dispatch"})
        for event in events.values():
            if isinstance(event, dict):
                self.assertNotIn("paths", event)
                self.assertNotIn("paths-ignore", event)
        self.assertEqual(self.detection["with"]["token"], "")
        self.assertEqual(self.detection["with"]["base"], "${{ github.event.pull_request.base.sha }}")
        self.assertTrue(self.detection["continue-on-error"])
        self.assertIn("github.event_name == 'pull_request'", self.detection["if"])
        self.assertIn("github.event.pull_request.base.sha != ''", self.detection["if"])
        steps = self.workflow["jobs"]["changes"]["steps"]
        select = next(step for step in steps if step.get("id") == "matrix")
        self.assertEqual(select["if"], "always()")
        self.assertEqual(select["env"]["DETECTION_OUTCOME"], "${{ steps.filter.outcome }}")
        job = self.workflow["jobs"]["codeql"]
        self.assertEqual(job["needs"], "changes")
        self.assertEqual(job["if"], "${{ always() && !cancelled() }}")
        self.assertEqual(job["name"], "CodeQL (${{ matrix.language }})")
        matrix = job["strategy"]["matrix"]["language"]
        self.assertEqual(matrix, '${{ fromJSON(needs.changes.outputs.languages || '\
                         '\'["actions","javascript-typescript","python","rust"]\') }}')
        self.assertFalse((REPO / ".github/workflows/codeql-languages.yml").exists())

    def test_default_categories_single_upload_and_no_install_or_duplicate_tests(self):
        job = self.workflow["jobs"]["codeql"]
        steps = job["steps"]
        init = next(step for step in steps if step.get("uses") == "github/codeql-action/init@v3")
        self.assertEqual(init["with"]["config-file"],
                         "${{ matrix.language == 'rust' && './target/codeql-rust.yml' || './.github/codeql/codeql-config.yml' }}")
        analyze = next(step for step in steps if step.get("uses") == "github/codeql-action/analyze@v3")
        self.assertNotIn("category", analyze.get("with", {}))
        self.assertFalse(any(step.get("uses", "").startswith(("github/codeql-action/upload-sarif", "actions/setup-python"))
                             for step in steps))
        prepare = next(step for step in steps if step.get("name") == "Prepare Rust scope")
        self.assertEqual(prepare["if"], "matrix.language == 'rust'")
        self.assertNotIn("continue-on-error", prepare)
        self.assertIn('python3 .github/scripts/codeql_rust_scope.py config', prepare["run"])
        self.assertNotIn('pip', prepare["run"])
        self.assertNotIn('unittest', prepare["run"])
        upload = next(step for step in steps if step.get("name") == "Upload analysis scope")
        self.assertEqual(upload["uses"], "actions/upload-artifact@v4")
        self.assertIn("matrix.language == 'rust'", upload["if"])

    def test_filters_cover_rust_inputs_and_leave_docs_and_other_languages(self):
        relevant = ["a.rs", "backend/src/lib.rs", "Cargo.toml", "backend/Cargo.toml",
                    "Cargo.lock", "cli/Cargo.lock", "rust-toolchain", "rust-toolchain.toml",
                    "cli/rust-toolchain.toml", ".cargo/config.toml", "cli/.cargo/config.toml",
                    WORKFLOW_FILE, ".github/codeql/codeql-config.yml",
                    ".github/scripts/codeql_rust_scope.py", ".github/scripts/test_codeql_rust_scope.py",
                    ".github/scripts/codeql_cfg_test_files.py"]
        irrelevant = ["frontend/src/App.tsx", "docs/CI_PERFORMANCE.md", "README.md",
                      ".github/workflows/ci.yml", ".github/scripts/ci_gate_reuse.py"]
        for path in relevant + irrelevant:
            with self.subTest(path=path):
                self.assertEqual(self.matches([path]), path in relevant)

    def test_real_git_diff_includes_rust_after_300_and_3000_files_and_on_rename(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, stderr=subprocess.DEVNULL)
            git("init", "-q")
            git("config", "user.email", "fixture@example.invalid")
            git("config", "user.name", "Fixture")
            git("commit", "--allow-empty", "-qm", "base")
            base = git("rev-parse", "HEAD").decode().strip()
            (root / "docs").mkdir()
            for i in range(3100):
                (root / f"docs/{i:04d}.md").write_text("fixture")
            (root / "z.rs").write_text("fn main() {}")
            git("add", ".")
            git("commit", "-qm", "large")
            # The action uses this exact uncapped Git command in token:'' mode.
            output = git("diff", "--no-renames", "--name-status", "-z", f"{base}..HEAD")
            paths = output.decode().split("\0")[1::2]
            self.assertEqual(len(paths), 3101)
            self.assertFalse(self.matches(paths[:300]))
            self.assertFalse(self.matches(paths[:3000]))
            self.assertTrue(self.matches(paths))
            self.assertEqual(select_languages("pull_request", "success", str(self.matches(paths)).lower()), LANGUAGES)
            git("mv", "z.rs", "z.md")
            git("commit", "-qm", "rename")
            renamed = git("diff", "--no-renames", "--name-status", "-z", "HEAD~1..HEAD").decode().split("\0")[1::2]
            self.assertIn("z.rs", renamed)
            self.assertTrue(self.matches(renamed))


if __name__ == "__main__":
    unittest.main()
