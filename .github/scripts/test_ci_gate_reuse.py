import copy
import fnmatch
import unittest
from pathlib import Path

from ci_gate_reuse import (
    CI_GATE_PREFIX,
    WorkflowIdentityError,
    WorkflowSchemaError,
    analyze_workflow,
    assess_tree_and_jobs,
    decide_from_api,
    newest_completed_successful_run,
    strict_load_workflow,
    _has_event_sensitive_steps,
    _load_workflow_bundle,
)


NESTED = {
    "on": {"workflow_call": {"inputs": {
        "coverage": {"type": "boolean", "default": False},
    }}},
    "jobs": {
        "build": {"name": "Build test archive"},
        "billing-smoke": {"name": "Billing route coverage smoke",
                          "if": "!inputs.coverage"},
        "run": {
            "name": "Test shard ${{ matrix.shard }}",
            "strategy": {"matrix": {"shard": [1, 2]}},
        },
        "report": {"name": "Verify complete suite"},
    }
}

WORKFLOW = {
    "jobs": {
        "changes": {"name": "Detect Changes"},
        "backend-test": {"name": "Backend Test", "if": "inputs.reuse-jobs",
                          "uses": "./.github/workflows/backend-tests.yml"},
        "coverage-backend": {"name": "Coverage (Backend)", "if": "inputs.reuse-jobs",
                             "uses": "./.github/workflows/backend-tests.yml",
                             "with": {"coverage": True}},
        "frontend": {"name": "Frontend", "if": "inputs.reuse-jobs"},
        "ci-pass": {"name": "CI Pipeline", "needs": [
            "changes", "backend-test", "coverage-backend", "frontend"]},
    }
}
WORKFLOWS = {
    ".github/workflows/ci.yml": WORKFLOW,
    ".github/workflows/backend-tests.yml": NESTED,
}


def job(name, conclusion="success", status="completed"):
    return {"name": name, "status": status, "conclusion": conclusion}


def decision_jobs():
    return [
        job("Detect Changes"),
        job("Frontend"),
        job("Backend Test / Build test archive"),
        job("Backend Test / Billing route coverage smoke"),
        job("Backend Test / Test shard 1"),
        job("Backend Test / Test shard 2"),
        job("Backend Test / Verify complete suite"),
        job("Coverage (Backend) / Build test archive"),
        job("Coverage (Backend) / Test shard 1"),
        job("Coverage (Backend) / Test shard 2"),
        job("Coverage (Backend) / Verify complete suite"),
    ]


class GateReuseTests(unittest.TestCase):
    def test_machine_pr_paths_include_backend_cli_and_ci_without_broad_docs_or_frontend(self):
        workflows, _ = _load_workflow_bundle(Path("."), ".github/workflows/ci.yml")
        workflow = workflows[".github/workflows/ci.yml"]
        filter_step = next(step for step in workflow["jobs"]["changes"]["steps"]
                           if step.get("id") == "filter")
        filters = strict_load_workflow(filter_step["with"]["filters"])
        condition = workflow["jobs"]["machine-container"]["if"]
        self.assertEqual(condition.strip(),
                         "!(inputs.force-all && contains(fromJSON(inputs.reuse-jobs || '[]'), 'machine-container')) && "
                         "(inputs.force-all || needs.changes.outputs.ci == 'true' || needs.changes.outputs.machine == 'true')")
        cases = {
            "backend/src/handlers/nyxbot.rs": True,
            "backend/Cargo.toml": True,
            "cli/src/commands/auth.rs": True,
            "cli/Dockerfile.machine": True,
            "cli/tests/machine_updater_e2e.sh": True,
            "cli/resources/machine-container/seccomp.json": True,
            "machine/src/main.rs": True,
            "machine-updater/src/main.rs": True,
            ".github/workflows/ci.yml": True,
            ".github/actions/ci-rust-cache/action.yml": True,
            ".github/scripts/ci_gate_reuse.py": True,
            ".github/CODEOWNERS": True,
            "Cargo.toml": True,
            "Cargo.lock": True,
            # Preserve the existing narrow machine integration triggers.
            "frontend/scripts/machine-seccomp.ts": True,
            "frontend/src/schemas/machines.ts": True,
            "frontend/src/routes/index.tsx": False,
            "frontend/package-lock.json": False,
            "docs/AI_AGENT_PLAYBOOK.md": False,
            "docs/ci/ACTIONS_CACHE_BUDGET.md": False,
            "README.md": False,
            "sdk/src/index.ts": False,
            "cloud-auth/src/lib.rs": False,
        }
        for path, expected in cases.items():
            with self.subTest(path=path):
                # These filters contain only positive globs. The fixtures use
                # literal paths and ** descendants, with no extglob semantics.
                matches = any(fnmatch.fnmatchcase(path, pattern)
                              for lane in ("machine", "ci") for pattern in filters[lane])
                self.assertEqual(matches, expected)

    def test_real_machine_success_is_reused_with_input_disabled_skipped_leaf(self):
        workflows, _ = _load_workflow_bundle(Path("."), ".github/workflows/ci.yml")
        workflow = workflows[".github/workflows/ci.yml"]
        analysis = analyze_workflow(workflow, workflows)
        self.assertEqual(analysis.reusable["machine-container"].exact_names,
                         ("Machine Container E2E",))
        self.assertFalse(_has_event_sensitive_steps(workflow["jobs"]["machine-container"]))
        observed = [job(name) for spec in analysis.reusable.values() for name in spec.exact_names]
        observed.append(job("Coverage (Backend) / Billing route coverage smoke", "skipped"))
        for prefix in ("", CI_GATE_PREFIX):
            with self.subTest(prefix=prefix):
                prefixed = [{**item, "name": prefix + item["name"]} for item in observed]
                decision = assess_tree_and_jobs(
                    workflow=workflow, reusable_workflows=workflows,
                    current_tree="tree", tested_tree="tree", jobs=prefixed,
                    source="pull_request" if not prefix else "main_gate", prefix=prefix,
                    run={"id": 7},
                )
                self.assertTrue(decision.reuse, decision.reason)
                self.assertIn("machine-container", decision.jobs)
                self.assertIn("coverage-backend", decision.jobs)
                self.assertNotIn("claude-plugin-validate", decision.jobs)

    def test_real_machine_skipped_failed_missing_or_different_tree_is_not_reused(self):
        workflows, _ = _load_workflow_bundle(Path("."), ".github/workflows/ci.yml")
        workflow = workflows[".github/workflows/ci.yml"]
        for conclusion in ("success", "skipped", "failure", "cancelled", None, "missing"):
            for same_tree in (True, False):
                with self.subTest(conclusion=conclusion, same_tree=same_tree):
                    observed = [] if conclusion == "missing" else [job("Machine Container E2E", conclusion)]
                    decision = assess_tree_and_jobs(
                        workflow=workflow, reusable_workflows=workflows,
                        current_tree="tree", tested_tree="tree" if same_tree else "other",
                        jobs=observed, source="pull_request", run={"id": 7},
                    )
                    self.assertEqual("machine-container" in decision.jobs,
                                     same_tree and conclusion == "success")

    def test_real_rust_jobs_have_event_insensitive_exact_identities(self):
        workflows, _ = _load_workflow_bundle(Path("."), ".github/workflows/ci.yml")
        workflow = workflows[".github/workflows/ci.yml"]
        analysis = analyze_workflow(workflow, workflows)
        expected = {
            "rust-clippy": ("Rust Clippy",),
            "rust-features": ("Rust Features (aws-kms)", "Rust Features (gcp-kms)",
                              "Rust Features (aws-and-gcp-kms)"),
            "cli-test": ("CLI Test",),
            "permissions-test": ("Google Permissions",),
            "coverage-cli": ("Coverage (CLI)",),
        }
        for job_id, names in expected.items():
            with self.subTest(job=job_id):
                self.assertFalse(_has_event_sensitive_steps(workflow["jobs"][job_id]))
                self.assertEqual(analysis.reusable[job_id].exact_names, names)
        self.assertEqual(len(analysis.leaf_names), len(set(analysis.leaf_names)))
        self.assertNotIn("claude-plugin-validate", analysis.reusable)
        self.assertTrue(_has_event_sensitive_steps(workflow["jobs"]["claude-plugin-validate"]))
        for job_id in ("backend-image-smoke", "coverage-cli-base", "coverage-cli-summary"):
            self.assertNotIn(job_id, analysis.candidates)

    def test_real_feature_matrix_requires_every_exact_successful_variant(self):
        workflows, _ = _load_workflow_bundle(Path("."), ".github/workflows/ci.yml")
        workflow = workflows[".github/workflows/ci.yml"]
        names = analyze_workflow(workflow, workflows).reusable["rust-features"].exact_names
        for conclusion in ("success", "failure", "skipped", "cancelled", None):
            observed = [job(name) for name in names[:-1]] + [job(names[-1], conclusion)]
            decision = assess_tree_and_jobs(
                workflow=workflow, reusable_workflows=workflows,
                current_tree="tree", tested_tree="tree", jobs=observed,
                source="pull_request", run={"id": 7},
            )
            self.assertEqual("rust-features" in decision.jobs, conclusion == "success")
        # A result under the old display name cannot stand in for the new one.
        decision = assess_tree_and_jobs(
            workflow=workflow, reusable_workflows=workflows,
            current_tree="tree", tested_tree="tree",
            jobs=[job(name) for name in names[:-1]] + [job("Rust Features (aws-kms,gcp-kms)")],
            source="pull_request", run={"id": 7},
        )
        self.assertNotIn("rust-features", decision.jobs)

    def test_real_rust_jobs_lose_reuse_if_event_sensitive_saves_return(self):
        workflows, _ = _load_workflow_bundle(Path("."), ".github/workflows/ci.yml")
        for job_id in ("rust-clippy", "rust-features", "cli-test", "permissions-test"):
            workflow = copy.deepcopy(workflows[".github/workflows/ci.yml"])
            workflow["jobs"][job_id]["steps"].append({
                "if": "github.ref == 'refs/heads/main' && success()",
                "uses": "actions/cache/save@v4",
            })
            self.assertNotIn(job_id, analyze_workflow(workflow, workflows).reusable)

    def test_real_nested_eight_shards_and_billing_leaf_stay_exact(self):
        workflows, _ = _load_workflow_bundle(Path("."), ".github/workflows/ci.yml")
        analysis = analyze_workflow(workflows[".github/workflows/ci.yml"], workflows)
        for job_id, label in (("backend-test", "Backend Test"),
                              ("coverage-backend", "Coverage (Backend)")):
            names = analysis.reusable[job_id].exact_names
            for shard in range(1, 9):
                self.assertEqual(names.count(f"{label} / Test shard {shard}"), 1)
        self.assertIn("Backend Test / Billing route coverage smoke",
                      analysis.reusable["backend-test"].exact_names)

    def test_nested_input_condition_excludes_ordinary_only_coverage_leaf(self):
        analysis = analyze_workflow(WORKFLOW, WORKFLOWS)
        backend = analysis.reusable["backend-test"].exact_names
        coverage = analysis.reusable["coverage-backend"].exact_names
        self.assertIn("Backend Test / Billing route coverage smoke", backend)
        self.assertNotIn("Coverage (Backend) / Billing route coverage smoke", coverage)

    def test_input_disabled_leaf_listed_as_skipped_is_tolerated(self):
        # GitHub lists a reusable child's disabled job as skipped (seen on
        # main b6ae99ce8 / PR #2070); it must not block reuse.
        jobs = decision_jobs() + [
            job("Coverage (Backend) / Billing route coverage smoke", conclusion="skipped"),
        ]
        decision = assess_tree_and_jobs(
            workflow=WORKFLOW, reusable_workflows=WORKFLOWS,
            current_tree="tree", tested_tree="tree", jobs=jobs,
            source="pull_request", run={"id": 7},
        )
        self.assertTrue(decision.reuse)
        self.assertEqual(set(decision.jobs), {"frontend", "backend-test", "coverage-backend"})

    def test_input_disabled_leaf_that_actually_ran_fails_closed(self):
        for conclusion in ("success", "failure", "cancelled", None):
            for prefix in ("", CI_GATE_PREFIX):
                with self.subTest(conclusion=conclusion, prefix=prefix):
                    jobs = decision_jobs() + [
                        job("Coverage (Backend) / Billing route coverage smoke", conclusion=conclusion),
                    ]
                    jobs = [{**item, "name": prefix + item["name"]} for item in jobs]
                    decision = assess_tree_and_jobs(
                        workflow=WORKFLOW, reusable_workflows=WORKFLOWS,
                        current_tree="tree", tested_tree="tree", jobs=jobs,
                        source="pull_request", prefix=prefix, run={"id": 7},
                    )
                    self.assertFalse(decision.reuse)
                    self.assertEqual(decision.reason, "api_unexpected_reusable_job")

    def test_unknown_skipped_leaf_under_prefix_still_fails_closed(self):
        for prefix in ("", CI_GATE_PREFIX):
            with self.subTest(prefix=prefix):
                jobs = decision_jobs() + [
                    job("Coverage (Backend) / Something unexpected", conclusion="skipped"),
                ]
                jobs = [{**item, "name": prefix + item["name"]} for item in jobs]
                decision = assess_tree_and_jobs(
                    workflow=WORKFLOW, reusable_workflows=WORKFLOWS,
                    current_tree="tree", tested_tree="tree", jobs=jobs,
                    source="pull_request", prefix=prefix, run={"id": 7},
                )
                self.assertFalse(decision.reuse)
                self.assertEqual(decision.reason, "api_unexpected_reusable_job")

    def test_tree_match_reuses_complete_direct_and_nested_jobs(self):
        decision = assess_tree_and_jobs(
            workflow=WORKFLOW, reusable_workflows=WORKFLOWS,
            current_tree="tree", tested_tree="tree", jobs=decision_jobs(),
            source="pull_request", run={"id": 7},
        )
        self.assertTrue(decision.reuse)
        self.assertEqual(set(decision.jobs), {"frontend", "backend-test", "coverage-backend"})

    def test_tree_mismatch_disables_reuse(self):
        decision = assess_tree_and_jobs(
            workflow=WORKFLOW, reusable_workflows=WORKFLOWS,
            current_tree="main-tree", tested_tree="pr-tree", jobs=[job("Frontend")],
            source="pull_request", run={"id": 7},
        )
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "tree_mismatch")

    def test_event_sensitive_steps_stay_on_main_even_when_job_passed(self):
        workflow = {
            "jobs": {
                "sensitive": {"name": "Sensitive", "if": "inputs.reuse-jobs",
                              "env": {"CHECK": "${{ github.event_name }}"}},
                "ci-pass": {"needs": ["sensitive"]},
            }
        }
        decision = assess_tree_and_jobs(
            workflow=workflow, current_tree="tree", tested_tree="tree",
            jobs=[job("Sensitive")], source="pull_request", run={"id": 7},
        )
        self.assertFalse(decision.reuse)

    def test_skipped_or_missing_pr_jobs_are_run_on_main(self):
        jobs = [job("Detect Changes"), job("Frontend", conclusion="skipped")]
        decision = assess_tree_and_jobs(
            workflow=WORKFLOW, reusable_workflows=WORKFLOWS,
            current_tree="tree", tested_tree="tree", jobs=jobs,
            source="pull_request", run={"id": 7},
        )
        self.assertFalse(decision.reuse)

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
        def api(path):
            if "/pulls" in path:
                return [{"head": {"sha": "h"}}, {"head": {"sha": "h2"}}]
            return []

        decision = self._decide(api, sha="main")
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "main_commit_pr_ambiguous")

    def _pr(self, *, base="main", merge="main", head_repo="o/r", head="head"):
        return {
            "number": 2063,
            "base": {"ref": base, "repo": {"full_name": "o/r"}},
            "head": {"sha": head, "repo": {"full_name": head_repo}},
            "merged_at": "2026-10-27T00:00:00Z",
            "merge_commit_sha": merge,
        }

    def _main_api(self, *, head_tree="tree", ancestor=True, pr=None,
                  head_pr=None, run=None, extra_head_prs=None):
        pr = pr or self._pr()
        head_pr = head_pr or pr
        run = run or {
            "id": 7, "head_sha": "head", "event": "pull_request",
            "status": "completed", "conclusion": "success", "run_number": 1,
            "path": ".github/workflows/ci.yml",
            "head_repository": {"full_name": "o/r"},
            "pull_requests": [],
        }

        def api(path):
            if "/commits/main/pulls" in path:
                return [pr]
            if "/commits/head/pulls" in path:
                return [head_pr] + list(extra_head_prs or [])
            if "actions/workflows/ci.yml/runs?" in path:
                return {"workflow_runs": [run]}
            if "/actions/runs/7/jobs" in path:
                return {"jobs": decision_jobs()}
            raise AssertionError(path)

        tree = lambda commit: "tree" if commit == "main" else head_tree
        return api, tree, (lambda commit: "parent"), (lambda parent, commit: ancestor)

    def _decide(self, api, *, sha="main", tree=None, parent=None, ancestor=None,
                pr_workflows=WORKFLOWS):
        tree = tree or (lambda commit: "tree")
        parent = parent or (lambda commit: "parent")
        ancestor = ancestor or (lambda parent_sha, commit: True)
        return decide_from_api(
            api=api, git_tree=tree, git_parent=parent, git_is_ancestor=ancestor,
            git_blob=lambda commit, path: "workflow-blob", workflow_blob="workflow-blob",
            workflow=WORKFLOW, reusable_workflows=pr_workflows,
            repository="o/r", sha=sha, event="push", ref="refs/heads/main",
        )

    def test_up_to_date_pr_reuses_without_using_reported_base_sha(self):
        api, tree, parent, ancestor = self._main_api()
        decision = self._decide(api, tree=tree, parent=parent, ancestor=ancestor)
        self.assertTrue(decision.reuse)
        self.assertEqual(decision.tested_tree, "tree")
        self.assertNotIn("base_sha", decision.payload())

    def test_missing_ci_workflow_path_is_ambiguous(self):
        api, tree, parent, ancestor = self._main_api()
        original = api

        def missing_path(path):
            result = original(path)
            if "actions/workflows/ci.yml/runs?" in path:
                result["workflow_runs"][0] = {
                    key: value for key, value in result["workflow_runs"][0].items()
                    if key != "path"
                }
            return result

        decision = self._decide(missing_path, tree=tree, parent=parent, ancestor=ancestor)
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "workflow_path_mismatch")

    def test_pr_behind_main_runs_full_gate(self):
        api, tree, parent, ancestor = self._main_api(ancestor=False)
        decision = self._decide(api, tree=tree, parent=parent, ancestor=ancestor)
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "pr_behind_main")

    def test_squash_tree_mismatch_runs_full_gate(self):
        api, tree, parent, ancestor = self._main_api(head_tree="different")
        decision = self._decide(api, tree=tree, parent=parent, ancestor=ancestor)
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "tree_mismatch")

    def test_same_head_with_second_dev_pr_is_rejected(self):
        api, tree, parent, ancestor = self._main_api(
            extra_head_prs=[self._pr(base="dev", merge="other")])
        decision = self._decide(api, tree=tree, parent=parent, ancestor=ancestor)
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "head_commit_pr_ambiguous")

    def test_rollup_target_is_rejected(self):
        pr = self._pr(base="rollup-x", merge="main")
        api, tree, parent, ancestor = self._main_api(pr=pr, head_pr=pr)
        decision = self._decide(api, tree=tree, parent=parent, ancestor=ancestor)
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "main_pr_base_mismatch")

    def test_merge_commit_mismatch_is_rejected(self):
        pr = self._pr(merge="other")
        api, tree, parent, ancestor = self._main_api(pr=pr, head_pr=pr)
        decision = self._decide(api, tree=tree, parent=parent, ancestor=ancestor)
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "main_pr_merge_mismatch")

    def test_fork_run_head_repository_mismatch_is_rejected(self):
        pr = self._pr(head_repo="fork/r")
        run = {"id": 7, "head_sha": "head", "event": "pull_request",
               "status": "completed", "conclusion": "success", "run_number": 1,
               "path": ".github/workflows/ci.yml",
               "head_repository": {"full_name": "o/r"}, "pull_requests": []}
        api, tree, parent, ancestor = self._main_api(pr=pr, head_pr=pr, run=run)
        decision = self._decide(api, tree=tree, parent=parent, ancestor=ancestor)
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "run_head_repository_mismatch")

    def test_run_pull_request_metadata_must_name_main_pr(self):
        pr = self._pr()
        run = {"id": 7, "head_sha": "head", "event": "pull_request",
               "status": "completed", "conclusion": "success", "run_number": 1,
               "path": ".github/workflows/ci.yml",
               "head_repository": {"full_name": "o/r"},
               "pull_requests": [{"number": 999, "base": {"ref": "dev"},
                                   "head": {"sha": "head"}}]}
        api, tree, parent, ancestor = self._main_api(pr=pr, head_pr=pr, run=run)
        decision = self._decide(api, tree=tree, parent=parent, ancestor=ancestor)
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "run_pull_request_mismatch")

    def test_workflow_blob_mismatch_runs_full_gate(self):
        api, tree, parent, ancestor = self._main_api()
        decision = decide_from_api(
            api=api, git_tree=tree, git_parent=parent, git_is_ancestor=ancestor,
            git_blob=lambda commit, path: "different", workflow_blob="workflow-blob",
            workflow=WORKFLOW, reusable_workflows=WORKFLOWS, repository="o/r",
            sha="main", event="push", ref="refs/heads/main")
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "workflow_blob_mismatch")

    def test_tag_reuses_successful_main_publish_gate(self):
        run = {"id": 42, "head_sha": "main", "head_branch": "main",
               "status": "completed", "conclusion": "success", "run_number": 8,
               "event": "push"}
        tag_jobs = [job(f"{CI_GATE_PREFIX}CI Pipeline"), job(f"{CI_GATE_PREFIX}Frontend")]

        def api(path):
            if "publish-images.yml" in path and "/runs?" in path:
                return {"workflow_runs": [run]}
            if path.endswith("/jobs?per_page=100"):
                return {"jobs": tag_jobs}
            raise AssertionError(path)

        decision = decide_from_api(
            api=api, git_tree=lambda commit: "tree",
            git_blob=lambda commit, path: "workflow-blob", workflow_blob="workflow-blob",
            workflow=WORKFLOW, reusable_workflows=WORKFLOWS, repository="o/r",
            sha="main", event="push", ref="refs/tags/v1.2.3")
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
            api=api, git_tree=lambda commit: "tree",
            git_blob=lambda commit, path: "workflow-blob", workflow_blob="workflow-blob",
            workflow=WORKFLOW, reusable_workflows=WORKFLOWS, repository="o/r",
            sha="main", event="push", ref="refs/tags/v1.2.3")
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "main_publish_gate_missing")

    def test_api_error_falls_back_to_full_gate(self):
        decision = self._decide(lambda path: (_ for _ in ()).throw(RuntimeError("API unavailable")))
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.source, "error")
        self.assertEqual(decision.reason, "lookup_error:RuntimeError")

    def test_workflow_dispatch_always_runs_full_gate(self):
        decision = decide_from_api(
            api=lambda path: self.fail(f"API call for dispatch: {path}"),
            git_tree=lambda commit: self.fail("git lookup for dispatch"),
            workflow=WORKFLOW, repository="o/r", sha="main",
            event="workflow_dispatch", ref="refs/heads/main")
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "workflow_dispatch")

    def test_duplicate_key_in_jobs_fails_closed(self):
        with self.assertRaisesRegex(WorkflowSchemaError, "yaml_duplicate_mapping_key"):
            strict_load_workflow("jobs:\n  one: {}\n  one: {}\n")

    def test_duplicate_key_nested_in_job_fails_closed(self):
        with self.assertRaisesRegex(WorkflowSchemaError, "yaml_duplicate_mapping_key"):
            strict_load_workflow("jobs:\n  one:\n    steps:\n      - run: a\n        run: b\n")

    def test_alias_and_merge_keys_fail_closed(self):
        with self.assertRaisesRegex(WorkflowSchemaError, "yaml_anchor"):
            strict_load_workflow("defaults: &x {runs-on: ubuntu}\njobs: {}\n")
        with self.assertRaisesRegex(WorkflowSchemaError, "yaml_alias"):
            strict_load_workflow("jobs:\n  one: *x\n")
        with self.assertRaisesRegex(WorkflowSchemaError, "yaml_merge_key"):
            strict_load_workflow("jobs:\n  one:\n    <<: {runs-on: ubuntu}\n")

    def test_non_string_keys_and_explicit_tags_fail_closed(self):
        with self.assertRaisesRegex(WorkflowSchemaError, "yaml_non_string_mapping_key"):
            strict_load_workflow("jobs:\n  one:\n    true: value\n")
        with self.assertRaisesRegex(WorkflowSchemaError, "yaml_tag"):
            strict_load_workflow("jobs: !!map {}\n")

    def test_non_string_if_expression_fails_closed(self):
        workflow = {"jobs": {
            "one": {"name": "One", "if": True},
            "ci-pass": {"needs": ["one"]},
        }}
        decision = assess_tree_and_jobs(
            workflow=workflow, current_tree="tree", tested_tree="tree",
            jobs=[job("One")], source="pull_request", run={"id": 7})
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "workflow_identity_if_expression")

    def test_colliding_display_names_fail_closed(self):
        workflow = {"jobs": {
            "a": {"name": "Same", "if": "inputs.reuse-jobs"},
            "b": {"name": "Same", "if": "inputs.reuse-jobs"},
            "ci-pass": {"needs": ["a", "b"]},
        }}
        decision = assess_tree_and_jobs(
            workflow=workflow, current_tree="tree", tested_tree="tree",
            jobs=[job("Same")], source="pull_request", run={"id": 7})
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "workflow_identity_duplicate_display_name")

    def test_reusable_prefix_overlap_with_plain_job_fails_closed(self):
        workflow = {"jobs": {
            "group": {"name": "Backend Test", "if": "inputs.reuse-jobs",
                      "uses": "./.github/workflows/backend-tests.yml"},
            "plain": {"name": "Backend Test / Other child",
                      "if": "inputs.reuse-jobs"},
            "ci-pass": {"needs": ["group", "plain"]},
        }}
        decision = assess_tree_and_jobs(
            workflow=workflow, reusable_workflows=WORKFLOWS,
            current_tree="tree", tested_tree="tree", jobs=[],
            source="pull_request", run={"id": 7})
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "workflow_identity_prefix_overlap")

    def test_duplicate_api_job_name_fails_closed(self):
        decision = assess_tree_and_jobs(
            workflow=WORKFLOW, reusable_workflows=WORKFLOWS,
            current_tree="tree", tested_tree="tree",
            jobs=decision_jobs() + [job("Frontend")],
            source="pull_request", run={"id": 7})
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "api_duplicate_job_name")

    def test_unexpected_reusable_prefix_job_fails_closed(self):
        decision = assess_tree_and_jobs(
            workflow=WORKFLOW, reusable_workflows=WORKFLOWS,
            current_tree="tree", tested_tree="tree",
            jobs=decision_jobs() + [job("Backend Test / Extra")],
            source="pull_request", run={"id": 7})
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.reason, "api_unexpected_reusable_job")

    def test_include_and_exclude_matrices_are_not_reusable(self):
        for matrix in (
            {"include": [{"value": "one"}]},
            {"value": ["one"], "exclude": [{"value": "one"}]},
        ):
            workflow = {"jobs": {
                "dynamic": {"name": "Dynamic ${{ matrix.value }}",
                            "if": "inputs.reuse-jobs", "strategy": {"matrix": matrix}},
                "ci-pass": {"needs": ["dynamic"]},
            }}
            decision = assess_tree_and_jobs(
                workflow=workflow, current_tree="tree", tested_tree="tree",
                jobs=[job("Dynamic one")], source="pull_request", run={"id": 7})
            self.assertFalse(decision.reuse)

    def test_non_matrix_expression_name_is_not_reusable(self):
        workflow = {"jobs": {
            "dynamic": {"name": "Dynamic ${{ github.job }}",
                        "if": "inputs.reuse-jobs"},
            "ci-pass": {"needs": ["dynamic"]},
        }}
        decision = assess_tree_and_jobs(
            workflow=workflow, current_tree="tree", tested_tree="tree",
            jobs=[job("Dynamic something")], source="pull_request", run={"id": 7})
        self.assertFalse(decision.reuse)

    def test_pull_request_only_job_is_not_a_force_all_candidate(self):
        workflow = {"jobs": {
            "smoke": {"name": "Backend Release Image",
                      "if": "github.event_name == 'pull_request'"},
            "ci-pass": {"needs": ["smoke"]},
        }}
        decision = assess_tree_and_jobs(
            workflow=workflow, current_tree="tree", tested_tree="tree",
            jobs=[job("Backend Release Image")], source="pull_request", run={"id": 7})
        self.assertFalse(decision.reuse)
        self.assertEqual(decision.candidates, ())

    def test_reuse_failure_cannot_skip_the_publish_gate(self):
        publish = Path(".github/workflows/publish-images.yml").read_text()
        ci = Path(".github/workflows/ci.yml").read_text()
        self.assertIn("if: ${{ !cancelled() }}", publish)
        self.assertIn("needs.gate-reuse.result == 'success'", publish)
        self.assertIn("fromJSON(inputs.reuse-jobs || '[]')", ci)


if __name__ == "__main__":
    unittest.main()
