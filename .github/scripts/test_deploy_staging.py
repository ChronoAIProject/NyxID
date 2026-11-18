import io
import json
import os
import subprocess
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch
from urllib.parse import parse_qs, urlsplit

from deploy_staging import BACKEND, FRONTEND, DeployError, Rollout, mask_token


SECRET = "fake +&secret"
TOKEN = "fake.bearer"
APPS = "/api/v1/proxy/s/chrono-cd/api/v1/projects/nyxid/apps/"


class Staging:
    def __init__(self, case):
        self.case = case
        self.now = 0
        self.events = []
        self.commits = {BACKEND: case.old, FRONTEND: case.old}
        self.phases = {BACKEND: "ready", FRONTEND: "ready"}
        self.health_sequence = []
        self.frontend_versions = []
        self.patch_failure = None
        self.status_failure = None

    def sleep(self, seconds):
        self.now += seconds

    def send(self, method, url, headers, body, timeout):
        case = self.case
        case.assertGreater(timeout, 0)
        case.assertLessEqual(timeout, 30)
        parsed = urlsplit(url)
        case.assertEqual(parsed.scheme, "https")
        if parsed.hostname == "api.nyxid.ai" and parsed.path == "/oauth/token":
            case.assertEqual(method, "POST")
            case.assertNotIn("Authorization", headers)
            case.assertEqual(headers["Content-Type"], "application/x-www-form-urlencoded")
            case.assertEqual(parse_qs(body.decode()), {
                "client_id": ["staging-deployer"], "client_secret": [SECRET],
                "scope": ["proxy"], "grant_type": ["client_credentials"],
            })
            return 200, json.dumps({"access_token": TOKEN}).encode()

        if parsed.hostname == "api.nyxid.ai":
            case.assertEqual(parse_qs(parsed.query), {"org_id": ["internal:chrono-ai-staging"]})
            case.assertEqual(headers["Authorization"], "Bearer " + TOKEN)
            case.assertTrue(parsed.path.startswith(APPS))
            app = parsed.path[len(APPS):].split("/")[0]
            component = {BACKEND: "backend", FRONTEND: "frontend"}[app]
            service = "nyxid-" + component
            if method == "PATCH":
                self.events.append(("patch", app))
                case.assertEqual(parsed.path, APPS + app)
                case.assertEqual(headers["Content-Type"], "application/json")
                case.assertEqual(json.loads(body), {"services": {service: {
                    "image": f"ghcr.io/chronoaiproject/nyxid/{component}:dev-{case.sha[:7]}",
                }}})
                if self.patch_failure:
                    return self.patch_failure
                self.commits[app] = case.sha[:12]
                return 200, b"{}"
            case.assertEqual(method, "GET")
            case.assertEqual(parsed.path, APPS + app + "/status")
            if self.status_failure:
                return self.status_failure
            phase = self.phases[app]
            self.events.append(("status", app, phase))
            return 200, json.dumps({"phase": phase,
                                    "services": {service: {"pods": [{"reason": "FixturePod"}]}}}).encode()

        case.assertEqual(method, "GET")
        case.assertNotIn("Authorization", headers)
        case.assertEqual(headers.get("Cache-Control"), "no-cache")
        if url == "https://staging-api.nyxid.ai/health":
            if self.health_sequence:
                answer = self.health_sequence.pop(0)
                if isinstance(answer, int):
                    self.events.append(("health", answer))
                    return answer, b"{}"
                self.events.append(("health", answer))
                return 200, json.dumps({"status": "ok", "commit": answer}).encode()
            return 200, json.dumps({"status": "ok", "commit": self.commits[BACKEND]}).encode()
        case.assertEqual(url, "https://staging.nyxid.ai/build-version.json")
        if self.frontend_versions:
            return self.frontend_versions.pop(0)
        return 200, json.dumps({"commit": self.commits[FRONTEND]}).encode()


class DeployTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Exercise the same abbreviated object lookup and merge-base as Actions.
        repo = cls.enterClassContext(tempfile.TemporaryDirectory())
        subprocess.run(["git", "init", "--bare", "--quiet", repo], check=True)
        cls.enterClassContext(patch.dict(os.environ, {"GIT_DIR": repo}))

        def git(*args):
            return subprocess.check_output(["git", "-c", "user.name=Fixture",
                                            "-c", "user.email=fixture@example.com", *args],
                                           input="", text=True).strip()

        tree = git("mktree")
        base = git("commit-tree", tree, "-m", "base")
        cls.old = base[:12]
        cls.sha = git("commit-tree", tree, "-p", base, "-m", "candidate")
        cls.newer = git("commit-tree", tree, "-p", cls.sha, "-m", "newer")[:12]
        cls.other = git("commit-tree", tree, "-p", base, "-m", "manual branch")[:12]

    def setUp(self):
        self.staging = Staging(self)
        self.output = io.StringIO()
        self.enterContext(redirect_stdout(self.output))
        self.enterContext(patch.dict(os.environ, {"GITHUB_ACTIONS": "false"}))

    def deploy(self, timeout=900):
        s = self.staging
        Rollout(send=s.send, sleep=s.sleep, clock=lambda: s.now, timeout=timeout).run(
            self.sha, "staging-deployer", SECRET)

    def writes(self):
        return [e[1] for e in self.staging.events if e[0] == "patch"]

    def test_order_and_consistent_health_across_mixed_replicas_and_a_transient_error(self):
        target = self.sha[:12]
        observations = [self.old, target, target, self.old, target, 503, target, target, target]
        self.staging.health_sequence = observations.copy()
        self.deploy()
        self.assertEqual(self.writes(), [BACKEND, FRONTEND])
        before_frontend = self.staging.events[:self.staging.events.index(("patch", FRONTEND))]
        self.assertEqual([e[1] for e in before_frontend if e[0] == "health"], observations)
        self.assertIn("frontend updated", self.output.getvalue())
        self.assertNotIn(SECRET, self.output.getvalue())
        self.assertNotIn(TOKEN, self.output.getvalue())

    def test_both_apps_already_deployed(self):
        self.staging.commits = dict.fromkeys([BACKEND, FRONTEND], self.sha[:12])
        self.deploy()
        self.assertEqual(self.writes(), [])
        self.assertIn("frontend already current", self.output.getvalue())
        self.assertIn("Skipping deployment", self.output.getvalue())

    def test_late_older_run_skips_pair_if_either_component_is_newer(self):
        for app in (BACKEND, FRONTEND):
            with self.subTest(app=app):
                self.staging.commits = dict.fromkeys((BACKEND, FRONTEND), self.old)
                self.staging.commits[app] = self.newer
                self.deploy()
                self.assertEqual(self.writes(), [])
                self.assertIn(f"{app} already runs newer commit {self.newer}", self.output.getvalue())

    def test_diverged_frontend_blocks_backend(self):
        self.staging.commits[FRONTEND] = self.other
        with self.assertRaisesRegex(DeployError, f"{self.other}.*diverged"):
            self.deploy()
        self.assertEqual(self.writes(), [])

    def test_unknown_frontend_commit_blocks_backend(self):
        self.staging.commits[FRONTEND] = "f" * 12
        with self.assertRaisesRegex(DeployError, "cannot be resolved uniquely"):
            self.deploy()
        self.assertEqual(self.writes(), [])

    def test_persistently_unreadable_frontend_blocks_backend(self):
        for response, reason in (
            ((404, b"missing"), "HTTP 404"),
            ((200, json.dumps({"commit": TOKEN}).encode()), "12-hex commit"),
        ):
            with self.subTest(response=response):
                self.staging.frontend_versions = [response] * 5
                with self.assertRaisesRegex(DeployError, f"timed out.*{reason}") as raised:
                    self.deploy(timeout=35)
                self.assertNotIn(TOKEN, str(raised.exception) + self.output.getvalue())
                self.assertEqual(self.writes(), [])

    def test_frontend_advances_during_backend_rollout(self):
        self.staging.frontend_versions = [
            (200, json.dumps({"commit": commit}).encode()) for commit in (self.old, self.newer)
        ]
        self.deploy()
        self.assertEqual(self.writes(), [BACKEND])
        self.assertIn(f"already runs newer commit {self.newer}", self.output.getvalue())

    def test_last_read_before_patch_must_still_identify_the_commit(self):
        self.staging.health_sequence = [self.old] + [503] * 5
        with self.assertRaisesRegex(DeployError, "timed out.*version unavailable"):
            self.deploy(timeout=35)
        self.assertEqual(self.writes(), [])

    def test_partial_rollout_resumes_only_frontend(self):
        self.staging.commits[BACKEND] = self.sha[:12]
        self.deploy()
        self.assertEqual(self.writes(), [FRONTEND])

    def test_old_frontend_missing_version_document_can_be_upgraded(self):
        self.staging.commits[BACKEND] = self.sha[:12]
        self.staging.frontend_versions = [(404, b"missing")] * 3
        self.deploy()
        self.assertEqual(self.writes(), [FRONTEND])

    def test_unavailable_old_backend_can_be_upgraded(self):
        self.staging.health_sequence = [503, 503, 503]
        self.deploy()
        self.assertEqual(self.writes(), [BACKEND, FRONTEND])

    def test_ready_frontend_serving_old_build_cannot_finish_rollout(self):
        self.staging.frontend_versions = [(200, json.dumps({"commit": self.old}).encode())] * 9
        with self.assertRaisesRegex(DeployError, f"timed out.*staging phase=ready.*commit={self.old}"):
            self.deploy(timeout=75)
        self.assertEqual(self.writes(), [BACKEND, FRONTEND])

    def test_transient_health_error_on_rerun_does_not_trigger_a_write(self):
        self.staging.commits = dict.fromkeys([BACKEND, FRONTEND], self.sha[:12])
        self.staging.health_sequence = [self.sha[:12], 503, self.sha[:12], self.sha[:12], self.sha[:12]]
        self.deploy()
        self.assertEqual(self.writes(), [])

    def test_backend_not_ready_blocks_frontend_and_reports_pod_reason(self):
        self.staging.phases[BACKEND] = "progressing"
        with self.assertRaisesRegex(DeployError, "timed out.*staging-api phase=progressing.*FixturePod"):
            self.deploy(timeout=37)
        self.assertEqual(self.staging.now, 37)
        self.assertEqual(self.writes(), [BACKEND])

    def test_frontend_shares_the_backend_deadline(self):
        self.staging.phases[FRONTEND] = "degraded"
        with self.assertRaisesRegex(DeployError, "timed out.*staging phase=degraded.*FixturePod"):
            self.deploy(timeout=55)
        self.assertEqual(self.staging.now, 55)
        self.assertEqual(self.writes(), [BACKEND, FRONTEND])

    def test_permanent_proxy_error_surfaces_details_and_redacts_secrets(self):
        self.staging.status_failure = 403, json.dumps({
            "error": "wrong_namespace", "error_code": 4, "message": f"forbidden {TOKEN} {SECRET}",
        }).encode()
        with self.assertRaisesRegex(DeployError, "HTTP 403 wrong_namespace forbidden") as result:
            self.deploy()
        self.assertNotIn(SECRET, str(result.exception))
        self.assertNotIn(TOKEN, str(result.exception))
        self.assertEqual(self.writes(), [])

    def test_ambiguous_patch_fails_without_blind_retry(self):
        self.staging.patch_failure = 502, b'{"error":"proxy_failed","message":"upstream closed"}'
        with self.assertRaisesRegex(DeployError, "502 proxy_failed upstream closed"):
            self.deploy()
        self.assertEqual(self.writes(), [BACKEND])

    def test_token_registered_with_actions_runner(self):
        with patch.dict(os.environ, {"GITHUB_ACTIONS": "true"}):
            with patch("deploy_staging.mask_token", wraps=mask_token) as mask:
                self.deploy()
                mask.assert_called_once_with(TOKEN)
        protocol, summary = self.output.getvalue().splitlines()
        self.assertEqual(protocol, "::add-mask::" + TOKEN)
        self.assertNotIn(TOKEN, summary)
        self.assertNotIn(SECRET, summary)
