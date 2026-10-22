import json
from pathlib import Path
import tempfile
import unittest

from coverage_base_lookup import BASELINE_ARTIFACT, find_baseline, validate_baseline


class CoverageBaseLookupTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.run = {"id": 42, "head_sha": "base-sha", "conclusion": "success",
                    "event": "push", "path": ".github/workflows/publish-images.yml"}
        self.artifact = {"id": 84, "name": BASELINE_ARTIFACT, "expired": False}
        self.identity = self.root / "coverage-backend.identity.json"
        self.identity.write_text(json.dumps({"fingerprint": "recipe", "reusable": True}))
        (self.root / "coverage-backend.base.json").write_text(json.dumps({
            "data": [{"totals": {"lines": {"percent": 75.25}}}]}))

    def test_no_run_reports_unavailable_without_artifact_requests(self):
        def unexpected_request(run_id):
            self.fail(f"artifact lookup for absent run {run_id}")
        self.assertIsNone(find_baseline([], "base-sha", unexpected_request))

    def test_run_without_artifact_reports_unavailable(self):
        self.assertIsNone(find_baseline([self.run], "base-sha", lambda _: []))

    def test_fingerprint_mismatch_is_rejected(self):
        self.assertFalse(validate_baseline(self.root, "different-recipe", True))

    def test_non_reusable_baseline_or_current_runner_is_rejected(self):
        self.assertFalse(validate_baseline(self.root, "recipe", False))
        for value in (False, "true", None):
            with self.subTest(value=value):
                self.identity.write_text(json.dumps({"fingerprint": "recipe", "reusable": value}))
                self.assertFalse(validate_baseline(self.root, "recipe", True))

    def test_success_selects_exact_run_and_validates_identity(self):
        self.assertEqual(find_baseline([self.run], "base-sha", lambda _: [self.artifact]),
                         {"run-id": 42, "artifact-id": 84})
        self.assertTrue(validate_baseline(self.root, "recipe", True))

    def test_pr_failed_wrong_sha_and_untrusted_workflow_runs_are_rejected(self):
        def unexpected_request(run_id):
            self.fail(f"artifact lookup for untrusted run {run_id}")
        for changes in ({"event": "pull_request"}, {"head_sha": "head-sha"},
                        {"conclusion": "failure"}, {"path": ".github/workflows/other.yml"}):
            with self.subTest(changes=changes):
                self.assertIsNone(find_baseline([{**self.run, **changes}], "base-sha",
                                                unexpected_request))

    def test_expired_or_other_artifacts_are_rejected(self):
        for changes in ({"expired": True}, {"name": "backend-coverage-head"}):
            with self.subTest(changes=changes):
                self.assertIsNone(find_baseline([self.run], "base-sha", lambda _: [
                    {**self.artifact, **changes}]))

    def test_newest_usable_run_is_selected_even_when_newer_run_has_no_artifact(self):
        requests = []

        def artifacts(run_id):
            requests.append(run_id)
            return [self.artifact] if run_id == 42 else []

        self.assertEqual(find_baseline([self.run, {**self.run, "id": 43}], "base-sha", artifacts),
                         {"run-id": 42, "artifact-id": 84})
        self.assertEqual(requests, [43, 42])

    def test_missing_or_malformed_identity_or_report_is_rejected(self):
        for invalid in ("invalid json", "[]", "null"):
            self.identity.write_text(invalid)
            self.assertFalse(validate_baseline(self.root, "recipe", True))
        self.identity.write_text(json.dumps({"fingerprint": "recipe", "reusable": True}))
        report = self.root / "coverage-backend.base.json"
        report.write_text("{}")
        self.assertFalse(validate_baseline(self.root, "recipe", True))
        report.unlink()
        self.assertFalse(validate_baseline(self.root, "recipe", True))
        self.identity.unlink()
        self.assertFalse(validate_baseline(self.root, "recipe", True))


if __name__ == "__main__":
    unittest.main()
