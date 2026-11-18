import io
import json
import os
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch
from urllib.parse import parse_qs, urlsplit

from deploy_staging import BACKEND, FRONTEND, DeployError, Rollout, mask_token


SHA = "1234567890abcdef" * 2 + "12345678"
OLD = "0" * 12
SECRET = "fake +&secret"
TOKEN = "fake.bearer"
APPS = "/api/v1/proxy/s/chrono-cd/api/v1/projects/nyxid/apps/"


class Staging:
    def __init__(self, case):
        self.case = case
        self.now = 0
        self.events = []
        self.commits = {BACKEND: OLD, FRONTEND: OLD}
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
                    "image": f"ghcr.io/chronoaiproject/nyxid/{component}:dev-{SHA[:7]}",
                }}})
                if self.patch_failure:
                    return self.patch_failure
                self.commits[app] = SHA[:12]
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
    def setUp(self):
        self.staging = Staging(self)
        self.output = io.StringIO()
        self.enterContext(redirect_stdout(self.output))
        self.enterContext(patch.dict(os.environ, {"GITHUB_ACTIONS": "false"}))

    def deploy(self, timeout=900):
        s = self.staging
        Rollout(send=s.send, sleep=s.sleep, clock=lambda: s.now, timeout=timeout).run(
            SHA, "staging-deployer", SECRET)

    def writes(self):
        return [e[1] for e in self.staging.events if e[0] == "patch"]

    def test_order_and_consistent_health_across_mixed_replicas_and_a_transient_error(self):
        target = SHA[:12]
        observations = [OLD, target, target, OLD, target, 503, target, target, target]
        self.staging.health_sequence = observations.copy()
        self.deploy()
        self.assertEqual(self.writes(), [BACKEND, FRONTEND])
        before_frontend = self.staging.events[:self.staging.events.index(("patch", FRONTEND))]
        self.assertEqual([e[1] for e in before_frontend if e[0] == "health"], observations)
        self.assertIn("frontend updated", self.output.getvalue())
        self.assertNotIn(SECRET, self.output.getvalue())
        self.assertNotIn(TOKEN, self.output.getvalue())

    def test_both_apps_already_deployed(self):
        self.staging.commits = dict.fromkeys([BACKEND, FRONTEND], SHA[:12])
        self.deploy()
        self.assertEqual(self.writes(), [])
        self.assertIn("frontend already current", self.output.getvalue())

    def test_partial_rollout_resumes_only_frontend(self):
        self.staging.commits[BACKEND] = SHA[:12]
        self.deploy()
        self.assertEqual(self.writes(), [FRONTEND])

    def test_old_frontend_missing_version_document_can_be_upgraded(self):
        self.staging.commits[BACKEND] = SHA[:12]
        self.staging.frontend_versions = [(404, b"missing")] * 3
        self.deploy()
        self.assertEqual(self.writes(), [FRONTEND])

    def test_unavailable_old_backend_can_be_upgraded(self):
        self.staging.health_sequence = [503, 503, 503]
        self.deploy()
        self.assertEqual(self.writes(), [BACKEND, FRONTEND])

    def test_ready_frontend_serving_old_build_cannot_finish_rollout(self):
        self.staging.frontend_versions = [(200, json.dumps({"commit": OLD}).encode())] * 9
        with self.assertRaisesRegex(DeployError, f"timed out.*staging phase=ready.*commit={OLD}"):
            self.deploy(timeout=75)
        self.assertEqual(self.writes(), [BACKEND, FRONTEND])

    def test_transient_health_error_on_rerun_does_not_trigger_a_write(self):
        self.staging.commits = dict.fromkeys([BACKEND, FRONTEND], SHA[:12])
        self.staging.health_sequence = [SHA[:12], 503, SHA[:12], SHA[:12], SHA[:12]]
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
