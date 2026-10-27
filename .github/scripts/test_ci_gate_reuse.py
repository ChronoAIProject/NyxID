import unittest
from pathlib import Path

from ci_gate_reuse import (
    CI_GATE_PREFIX,
    assess_tree_and_jobs,
    decide_from_api,
    newest_completed_successful_run,
    reusable_job_ids,
)


WORKFLOW = {
    "jobs": {
        "changes": {"name": "Detect Changes"},
        "backend-test": {"name": "Backend Test", "if": "inputs.reuse-jobs", "uses": "./backend-tests.yml"},
        "coverage-backend": {"name": "Coverage (Backend)", "if": "inputs.reuse-jobs",
                             "uses": "./backend-tests.yml", "with": {"coverage": True}},
        "rust-features": {
            "name": "Rust Features (${{ matrix.features }})",
            "if": "inputs.reuse-jobs",
            "strategy": {"matrix": {"features": ["one", "two"]}},
        },
        "frontend": {"name": "Frontend", "if": "inputs.reuse-jobs"},
        "ci-pass": {"name": "CI Pipeline", "needs": ["changes", "backend-test", "coverage-backend",
                                                        "rust-features", "frontend"]},
    }
}


def job(name, conclusion="success", status="completed"):
    return {"name": name, "status": status, "conclusion": conclusion}


class GateReuseTests(unittest.TestCase):
    def test_tree_match_reuses_complete_direct_matrix_and_nested_jobs(self):
        jobs = [
            job("Detect Changes"),
            job("Frontend"),
            job("Rust Features (one)"),
            job("Rust Features (two)"),
            job("Backend Test / Build test archive"),
            job("Backend Test / Test shard 1"),
            job("Backend Test / Verify complete suite"),
            job("Coverage (Backend) / Build test archive"),
            job("Coverage (Backend) / Test shard 1"),
            job("Coverage (Backend) / Verify complete suite"),
        ]
        decision = assess_tree_and_jobs(
            workflow=WORKFLOW,
            current_tree="tree",
            tested_tree="tree",
            jobs=jobs,
            source="pull_request",
            run={"id": 7},
        )
        self.assertTrue(decision.reuse)
        self.assertEqual(set(decision.jobs), {"frontend", "rust-features", "backend-test", "coverage-backend"})

    def test_tree_mismatch_disables_reuse(self):
        decision = assess_tree_and_jobs(
            workflow=WORKFLOW,
            current_tree="main-tree",
            tested_tree="pr-tree",
            jobs=[job("Frontend")],
            source="pull_request",
            run={"id": 7},
        )
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "tree_mismatch")

    def test_event_sensitive_steps_stay_on_main_even_when_job_passed(self):
        workflow = {
            "jobs": {
                "sensitive": {
                    "name": "Sensitive",
                    "if": "inputs.reuse-jobs",
                    "env": {"CHECK": "${{ github.event_name }}"},
                },
                "ci-pass": {"needs": ["sensitive"]},
            }
        }
        decision = assess_tree_and_jobs(
            workflow=workflow,
            current_tree="tree",
            tested_tree="tree",
            jobs=[job("Sensitive")],
            source="pull_request",
            run={"id": 7},
        )
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.candidates, ("sensitive",))
        self.assertEqual(decision.jobs, ())

    def test_skipped_or_missing_pr_jobs_are_run_on_main(self):
        jobs = [
            job("Detect Changes"),
            job("Frontend", conclusion="skipped"),
            job("Rust Features (one)"),
            # A missing matrix value is never treated as a complete matrix.
        ]
        self.assertEqual(reusable_job_ids(WORKFLOW, jobs), ())

    def test_newest_failed_or_in_progress_attempt_invalidates_old_success(self):
        old = {"head_sha": "h", "run_number": 10, "run_attempt": 1,
               "status": "completed", "conclusion": "success"}
        for newer in (
            {"head_sha": "h", "run_number": 10, "run_attempt": 2,
             "status": "in_progress"},
            {"head_sha": "h", "run_number": 11, "run_attempt": 1,
             "status": "completed", "conclusion": "failure"},
        ):
            with self.subTest(newer=newer):
                self.assertIsNone(newest_completed_successful_run([old, newer], "h"))

    def test_missing_successful_run_is_rejected(self):
        self.assertIsNone(newest_completed_successful_run([], "h"))

    def test_multiple_prs_are_ambiguous(self):
        api = lambda path: [{"head": {"sha": "h"}, "base": {"sha": "b"}},
                            {"head": {"sha": "h2"}, "base": {"sha": "b2"}}] if "/pulls" in path else []
        decision = decide_from_api(
            api=api,
            git_tree=lambda commit: "tree",
            workflow=WORKFLOW,
            repository="o/r",
            sha="main",
            event="push",
            ref="refs/heads/main",
        )
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "ambiguous_pull_request")

    def _main_api(self, head_tree="tree", ancestor=True):
        pr = {"number": 2063, "head": {"sha": "head"},
              "base": {"sha": "old-base"}}
        run = {"id": 7, "head_sha": "head", "event": "pull_request",
               "status": "completed", "conclusion": "success", "run_number": 1,
               "path": ".github/workflows/ci.yml",
               # Merged runs commonly expose no pull_requests entries. The
               # unique commit-to-PR mapping above remains the association.
               "pull_requests": []}

        def api(path):
            if "/commits/main/pulls" in path:
                return [pr]
            if "actions/workflows/ci.yml/runs?" in path:
                return {"workflow_runs": [run]}
            if "/actions/runs/7/jobs" in path:
                return {"jobs": [job("Frontend")]}
            raise AssertionError(path)

        return api, pr, run, (lambda commit: "tree" if commit == "main" else head_tree), \
            (lambda commit: "parent"), (lambda parent, commit: ancestor)

    def test_up_to_date_pr_reuses_without_using_reported_base_sha(self):
        api, _, _, tree, parent, ancestor = self._main_api()
        decision = decide_from_api(
            api=api, git_tree=tree, git_parent=parent, git_is_ancestor=ancestor,
            workflow=WORKFLOW, repository="o/r", sha="main", event="push",
            ref="refs/heads/main")
        self.assertTrue(decision.reuse)
        self.assertEqual(decision.tested_tree, "tree")
        self.assertNotIn("base_sha", decision.payload())

    def test_missing_ci_workflow_path_is_ambiguous(self):
        api, _, run, tree, parent, ancestor = self._main_api()
        run = {key: value for key, value in run.items() if key != "path"}

        def api_without_path(path):
            result = api(path)
            if "actions/workflows/ci.yml/runs?" in path:
                return {"workflow_runs": [run]}
            return result

        decision = decide_from_api(
            api=api_without_path, git_tree=tree, git_parent=parent,
            git_is_ancestor=ancestor, workflow=WORKFLOW, repository="o/r",
            sha="main", event="push", ref="refs/heads/main")
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "workflow_path_mismatch")

    def test_pr_behind_main_runs_full_gate(self):
        api, _, _, tree, parent, ancestor = self._main_api(ancestor=False)
        decision = decide_from_api(
            api=api, git_tree=tree, git_parent=parent, git_is_ancestor=ancestor,
            workflow=WORKFLOW, repository="o/r", sha="main", event="push",
            ref="refs/heads/main")
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "pr_behind_main")

    def test_squash_tree_mismatch_runs_full_gate(self):
        api, _, _, tree, parent, ancestor = self._main_api(head_tree="different")
        decision = decide_from_api(
            api=api, git_tree=tree, git_parent=parent, git_is_ancestor=ancestor,
            workflow=WORKFLOW, repository="o/r", sha="main", event="push",
            ref="refs/heads/main")
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "tree_mismatch")

    def test_tag_reuses_successful_main_publish_gate(self):
        run = {"id": 42, "head_sha": "main", "head_branch": "main",
               "status": "completed", "conclusion": "success", "run_number": 8,
               "event": "push"}
        tag_jobs = [
            job(f"{CI_GATE_PREFIX}CI Pipeline"),
            job(f"{CI_GATE_PREFIX}Detect Changes"),
            job(f"{CI_GATE_PREFIX}Frontend"),
        ]

        def api(path):
            if "publish-images.yml" in path and "/runs?" in path:
                return {"workflow_runs": [run]}
            if path.endswith("/jobs?per_page=100"):
                return {"jobs": tag_jobs}
            raise AssertionError(path)

        decision = decide_from_api(
            api=api,
            git_tree=lambda commit: "tree",
            workflow=WORKFLOW,
            repository="o/r",
            sha="main",
            event="push",
            ref="refs/tags/v1.2.3",
        )
        self.assertTrue(decision.reuse)
        self.assertEqual(decision.source, "tag_main_publish")
        self.assertIn("frontend", decision.jobs)

    def test_tag_does_not_use_an_older_success_when_newer_publish_failed(self):
        runs = [
            {"id": 42, "head_sha": "main", "head_branch": "main", "event": "push",
             "status": "completed", "conclusion": "success", "run_number": 8},
            {"id": 43, "head_sha": "main", "head_branch": "main", "event": "push",
             "status": "completed", "conclusion": "failure", "run_number": 9},
        ]

        def api(path):
            if "publish-images.yml" in path and "/runs?" in path:
                return {"workflow_runs": runs}
            raise AssertionError(path)

        decision = decide_from_api(
            api=api,
            git_tree=lambda commit: "tree",
            workflow=WORKFLOW,
            repository="o/r",
            sha="main",
            event="push",
            ref="refs/tags/v1.2.3",
        )
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "main_publish_gate_missing")

    def test_api_error_falls_back_to_full_gate(self):
        def api(_path):
            raise RuntimeError("API unavailable")

        decision = decide_from_api(
            api=api,
            git_tree=lambda commit: "tree",
            workflow=WORKFLOW,
            repository="o/r",
            sha="main",
            event="push",
            ref="refs/heads/main",
        )
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.source, "error")
        self.assertEqual(decision.reason, "lookup_error:RuntimeError")

    def test_workflow_dispatch_always_runs_full_gate(self):
        decision = decide_from_api(
            api=lambda path: self.fail(f"API call for dispatch: {path}"),
            git_tree=lambda commit: self.fail("git lookup for dispatch"),
            workflow=WORKFLOW,
            repository="o/r",
            sha="main",
            event="workflow_dispatch",
            ref="refs/heads/main",
        )
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "workflow_dispatch")

    def test_reuse_failure_cannot_skip_the_publish_gate(self):
        publish = Path(".github/workflows/publish-images.yml").read_text()
        ci = Path(".github/workflows/ci.yml").read_text()
        self.assertIn("if: ${{ !cancelled() }}", publish)
        self.assertIn("needs.gate-reuse.result == 'success'", publish)
        self.assertIn("fromJSON(inputs.reuse-jobs || '[]')", ci)


if __name__ == "__main__":
    unittest.main()
