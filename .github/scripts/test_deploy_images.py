import io
import json
import os
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from unittest.mock import patch
from urllib.parse import parse_qs, quote_plus, urlsplit

from deploy_images import DeployError, Rollout, load_target, main, published_digests
import post_deploy_check


SECRET = "fake +&secret"
TOKEN = "fake.bearer"
DIGESTS = {"backend": "sha256:" + "a" * 64, "frontend": "sha256:" + "b" * 64}


def fake_config(suffix):
    return {
        "apps_url": f"https://deploy-{suffix}.example.test:8443/team%20{suffix}/apps",
        "org_id": f"org:{suffix}",
        **{role: {"app": f"app-{role}-{suffix}", "service": f"svc-{role}-{suffix}",
                  "version_url": f"https://{role}-{suffix}.example.test/version"}
           for role in ("backend", "frontend")},
    }


def private_values(config):
    return {config["apps_url"], config["apps_url"].removeprefix("https://"), config["org_id"],
            *config["backend"].values(), *config["frontend"].values()}


class TargetConfigTests(unittest.TestCase):
    def assert_rejected(self, raw, field, *invalid_values):
        output = io.StringIO()
        errors = io.StringIO()
        env = {"GITHUB_ACTIONS": "true"}
        if raw is not None:
            env["NYXID_DEPLOY_TARGET"] = raw
        with patch.dict(os.environ, env, clear=True), patch("sys.argv", ["deploy_images.py", "staging"]), \
                redirect_stdout(output), redirect_stderr(errors), patch("deploy_images.Rollout") as rollout:
            with self.assertRaises(SystemExit) as failure:
                main()
        rollout.assert_not_called()
        message = str(failure.exception)
        self.assertIn("NYXID_DEPLOY_TARGET" + field, message)
        self.assertEqual(output.getvalue() + errors.getvalue(), "")
        for value in private_values(fake_config("a")) | set(invalid_values):
            self.assertNotIn(value, message)

    def test_manifest_lookup_requires_two_top_level_digests(self):
        outputs = [subprocess.CompletedProcess([], 0, f"Name: image\nDigest: {digest}\n")
                   for digest in DIGESTS.values()]
        with patch("deploy_images.subprocess.run", side_effect=outputs) as inspect:
            self.assertEqual(published_digests("dev", "1" * 40), DIGESTS)
        self.assertTrue(inspect.call_args_list[0].args[0][-1].endswith("/backend:dev-1111111"))
        with patch("deploy_images.subprocess.run", side_effect=[outputs[0], subprocess.CompletedProcess([], 0, "bad")]):
            self.assertEqual(published_digests("dev", "1" * 40), {})

    def test_requires_single_line_json_object(self):
        for raw in (None, "", "bad-private-json", "null", "[]", json.dumps(fake_config("a"), indent=2)):
            with self.subTest(raw=raw):
                self.assert_rejected(raw, "", "bad-private-json")

    def test_exact_object_fields(self):
        for role in ("", "backend", "frontend"):
            path = "." + role if role else ""
            fields = ("app", "service", "version_url") if role else ("org_id", "apps_url", "backend", "frontend")
            for key in (*fields, "private-extra-key"):
                with self.subTest(role=role, key=key):
                    config = fake_config("a")
                    obj = config[role] if role else config
                    if key in fields:
                        del obj[key]
                        self.assert_rejected(json.dumps(config), f"{path}.{key}")
                    else:
                        obj[key] = "private-extra-value"
                        self.assert_rejected(json.dumps(config), path, key, obj[key])
            if role:
                config = fake_config("a")
                config[role] = list(config[role].items())
                self.assert_rejected(json.dumps(config), path)

    def test_duplicate_field(self):
        raw = json.dumps(fake_config("a")).replace('"app":', '"app": "private-duplicate", "app":', 1)
        self.assert_rejected(raw, ".backend.app", "private-duplicate")

    def test_invalid_identifiers(self):
        for role, key, limit in (("", "org_id", 128), ("backend", "app", 63), ("frontend", "service", 63)):
            for value in (None, 42, [], "", " ", "bad/private", "..", "x" * (limit + 1), "bad\nprivate"):
                with self.subTest(role=role, key=key, value=value):
                    config = fake_config("a")
                    obj = config[role] if role else config
                    obj[key] = value
                    field = f".{role}.{key}" if role else f".{key}"
                    self.assert_rejected(json.dumps(config), field, "bad/private", "x" * (limit + 1), "private")

    def test_invalid_urls(self):
        for role in ("", "backend", "frontend"):
            for url in (None, 42, "", "https://", "http://private.test/apps",
                        "https://user:pass@private.test", "https://@private.test", "https://private.test?",
                        "https://private.test/#", "https://private.test/a?b", "https://private.test/a#b",
                        "https://private.test:99999", "https://[private.test", "https://private.test/\npath",
                        "https://private.test/bad path", "https://private.test/%zz"):
                with self.subTest(role=role, url=url):
                    config = fake_config("a")
                    if role:
                        config[role]["version_url"] = url
                    else:
                        config["apps_url"] = url
                    self.assert_rejected(json.dumps(config), f".{role}.version_url" if role else ".apps_url",
                                         "private.test", "user:pass")


class FakeDeployment:
    def __init__(self, case):
        self.case = case
        self.now = 0
        self.events = []
        self.commits = {case.backend: case.old, case.frontend: case.old}
        self.phases = {case.backend: "ready", case.frontend: "ready"}
        self.health_sequence = []
        self.frontend_versions = []
        self.pod_reason = "FixturePod"
        self.image_sha = case.sha
        self.digests = {}
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
                "client_id": [case.client_id], "client_secret": [SECRET],
                "scope": ["proxy"], "grant_type": ["client_credentials"],
            })
            return 200, json.dumps({"access_token": TOKEN}).encode()

        apps = urlsplit(case.config["apps_url"])
        prefix = apps.path.rstrip("/") + "/"
        if parsed.netloc == apps.netloc and parsed.path.startswith(prefix):
            case.assertEqual(parse_qs(parsed.query), {"org_id": [case.config["org_id"]]})
            case.assertEqual(headers["Authorization"], "Bearer " + TOKEN)
            app = parsed.path[len(prefix):].split("/")[0]
            component = {case.backend: "backend", case.frontend: "frontend"}[app]
            service = case.config[component]["service"]
            if method == "PATCH":
                self.events.append(("patch", app))
                case.assertEqual(parsed.path, prefix + app)
                case.assertEqual(headers["Content-Type"], "application/json")
                expected = f"ghcr.io/chronoaiproject/nyxid/{component}:{case.branch}-{self.image_sha[:7]}"
                if self.digests:
                    expected += "@" + self.digests[component]
                case.assertEqual(json.loads(body), {"services": {service: {"image": expected}}})
                if self.patch_failure:
                    return self.patch_failure
                self.commits[app] = self.image_sha[:12]
                return 200, b"{}"
            case.assertEqual(method, "GET")
            case.assertEqual(parsed.path, prefix + app + "/status")
            if self.status_failure:
                return self.status_failure
            phase = self.phases[app]
            self.events.append(("status", app, phase))
            return 200, json.dumps({"phase": phase,
                                    "services": {service: {"pods": [{"reason": self.pod_reason}]}}}).encode()

        case.assertEqual(method, "GET")
        case.assertNotIn("Authorization", headers)
        case.assertEqual(headers.get("Cache-Control"), "no-cache")
        if url == case.config["backend"]["version_url"]:
            if self.health_sequence:
                answer = self.health_sequence.pop(0)
                if isinstance(answer, int):
                    self.events.append(("health", answer))
                    return answer, b"{}"
                self.events.append(("health", answer))
                return 200, json.dumps({"status": "ok", "commit": answer}).encode()
            return 200, json.dumps({"status": "ok", "commit": self.commits[case.backend]}).encode()
        case.assertEqual(url, case.config["frontend"]["version_url"])
        if self.frontend_versions:
            return self.frontend_versions.pop(0)
        return 200, json.dumps({"commit": self.commits[case.frontend]}).encode()


class StagingDeployTests(unittest.TestCase):
    name = "staging"
    suffix = "a"
    credential_prefix = "NYXID_STAGING_DEPLOY_CLIENT"
    branch = "dev"
    client_id = "fixture-a-deployer"
    rollout_seconds = 900

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
        cls.previous = base
        cls.old = base[:12]
        cls.sha = git("commit-tree", tree, "-p", base, "-m", "candidate")
        cls.newer = git("commit-tree", tree, "-p", cls.sha, "-m", "newer")[:12]
        cls.other = git("commit-tree", tree, "-p", base, "-m", "manual branch")[:12]

    def setUp(self):
        self.configure(fake_config(self.suffix))
        self.output = io.StringIO()
        self.errors = io.StringIO()
        self.enterContext(redirect_stderr(self.errors))
        self.enterContext(redirect_stdout(self.output))
        self.enterContext(patch.dict(os.environ, {"GITHUB_ACTIONS": "false"}))

    def configure(self, config, public_values=()):
        self.config = config
        self.expected_private = private_values(config).difference(public_values)
        self.backend = config["backend"]["app"]
        self.frontend = config["frontend"]["app"]
        self.target = load_target(self.name, json.dumps(config))
        self.server = FakeDeployment(self)

    def configure_public_labels(self):
        config = fake_config(self.suffix)
        frontend = self.name if self.name == "staging" else "product"
        config["frontend"]["app"] = frontend
        config["backend"]["app"] = f"{frontend}-worker-{self.suffix}"
        config["backend"]["service"] = "Back"
        config["frontend"]["service"] = "rOnT"
        self.configure(config, public_values=(frontend, "Back", "rOnT"))

    def assert_private_values_absent(self, text):
        for value in self.expected_private | {TOKEN, SECRET, self.client_id}:
            self.assertNotIn(value, text)
            self.assertNotIn(quote_plus(value), text)

    def tearDown(self):
        # Only Actions mask commands may carry the individual values.
        lines = self.output.getvalue().splitlines()
        self.assert_private_values_absent("\n".join(line for line in lines if not line.startswith("::add-mask::")))
        self.assert_private_values_absent(self.errors.getvalue())

    def deploy(self, timeout=None):
        s = self.server
        rollout = Rollout(self.target, send=s.send, sleep=s.sleep, clock=lambda: s.now, timeout=timeout)
        try:
            return rollout.run(self.sha, self.client_id, SECRET)
        except DeployError as error:
            self.assert_private_values_absent(str(error))
            raise

    def writes(self):
        return [e[1] for e in self.server.events if e[0] == "patch"]

    def test_order_and_consistent_health_across_mixed_replicas_and_a_transient_error(self):
        target = self.sha[:12]
        observations = [self.old, target, target, self.old, target, 503, target, target, target]
        self.server.health_sequence = observations.copy()
        self.deploy()
        self.assertEqual(self.writes(), [self.backend, self.frontend])
        before_frontend = self.server.events[:self.server.events.index(("patch", self.frontend))]
        self.assertEqual([e[1] for e in before_frontend if e[0] == "health"], observations)
        self.assertEqual(self.output.getvalue(), f"{self.target.name.capitalize()} ready at {target} "
                         "(backend updated, frontend updated).\n")
        self.assertNotIn(SECRET, self.output.getvalue())
        self.assertNotIn(TOKEN, self.output.getvalue())

    def restore(self):
        server = self.server
        server.image_sha = self.previous
        server.digests = DIGESTS
        rollout = Rollout(self.target, send=server.send, sleep=server.sleep, clock=lambda: server.now)
        rollout.restore(self.sha, self.previous, DIGESTS, self.client_id, SECRET)

    def test_pinned_forward_patches_record_both_applied_digests(self):
        self.server.digests = DIGESTS
        s = self.server
        rollout = Rollout(self.target, send=s.send, sleep=s.sleep, clock=lambda: s.now)
        with patch.dict(os.environ, {"NYXID_PIN_CHECK_IMAGES": "true"}), \
                patch("deploy_images.published_digests", return_value=DIGESTS):
            self.assertEqual(rollout.run(self.sha, self.client_id, SECRET), self.old)
        self.assertEqual(rollout.applied_digests, DIGESTS)

    def test_restore_previous_passing_pair_backend_then_frontend(self):
        self.assertEqual(self.deploy(), self.old)
        self.server.events.clear()
        self.restore()
        self.assertEqual(self.writes(), [self.backend, self.frontend])
        self.assertEqual(set(self.server.commits.values()), {self.old})
        last = self.server.events[self.server.events.index(("patch", self.frontend)):]
        self.assertEqual(last.count(("status", self.backend, "ready")), 3)

    def test_failed_model_restores_observed_pair_before_sending_alert(self):
        observed = self.deploy()
        account = {"email": "runner@example.test", "password": "fixture", "owner_id": "ad3df9fa-eb5f-4190-a329-f5cb683b36ac",
                   "conversation_id": "nyxa-" + "c" * 32, "channel_id": "e082a538-c99d-458d-8bc0-81d3505788e6"}
        server = self.server
        server.image_sha = self.previous
        server.digests = DIGESTS

        def failed_probe(_origin, _account, record):
            record["stage"] = "model_reply"
            raise post_deploy_check.CheckError

        def alert(method, url, body, **kwargs):
            self.assertEqual(set(server.commits.values()), {self.old})
            self.assertIn("restored", body["text"])

        env = {"GITHUB_SHA": self.sha, "GITHUB_REPOSITORY": "example/repo", "GITHUB_RUN_ID": "1",
               self.credential_prefix + "_ID": self.client_id, self.credential_prefix + "_SECRET": SECRET,
               "NYXID_CHECK_ACCOUNT": json.dumps(account), "NYXID_CHECK_ALERT_URL": "https://test.invalid/hook",
               "NYXID_DEPLOY_TARGET": json.dumps(self.config), "PREVIOUS_COMMIT": observed,
               "NYXID_ROLLBACK_PAIR": f"{self.previous}..{self.sha}"}
        with patch.dict(os.environ, env), patch.object(post_deploy_check, "probe", side_effect=failed_probe), \
                patch.object(post_deploy_check, "prior_images", return_value=DIGESTS), \
                patch.object(post_deploy_check, "http_request", side_effect=alert), \
                patch.object(post_deploy_check, "Rollout", return_value=Rollout(
                    self.target, send=server.send, sleep=server.sleep, clock=lambda: server.now)):
            result = post_deploy_check.run(self.name)
        self.assertEqual(self.writes(), [self.backend, self.frontend] * 2)
        self.assertEqual((result["outcome"], result["rollback"], result["notification"]), ("failed", "restored", "sent"))

    def test_restore_backend_patch_failure_stops_before_frontend(self):
        self.deploy()
        self.server.patch_failure = 500, b"{}"
        with self.assertRaisesRegex(DeployError, "^backend$"):
            self.restore()
        self.assertEqual(self.writes(), [self.backend, self.frontend, self.backend])

    def test_restore_checks_peer_before_first_patch(self):
        self.deploy()
        self.server.frontend_versions = [(200, json.dumps({"commit": value}).encode())
                                         for value in (self.sha[:12], self.newer)]
        with self.assertRaises(DeployError):
            self.restore()
        self.assertEqual(self.writes(), [self.backend, self.frontend])

    def test_restore_refuses_changed_pair(self):
        self.deploy()
        self.server.commits[self.backend] = self.other
        with self.assertRaises(DeployError):
            self.restore()
        self.assertEqual(self.writes(), [self.backend, self.frontend])

    def test_both_apps_already_deployed(self):
        self.server.commits = dict.fromkeys([self.backend, self.frontend], self.sha[:12])
        self.assertEqual(self.deploy(), "")
        self.assertEqual(self.writes(), [])
        self.assertEqual(self.output.getvalue(), f"Skipping deployment, {self.target.name} already ready "
                         f"at {self.sha[:12]} (backend already current, frontend already current).\n")

    def test_late_older_run_skips_pair_if_either_component_is_newer(self):
        for role, app in (("backend", self.backend), ("frontend", self.frontend)):
            with self.subTest(role=role):
                self.server.commits = dict.fromkeys((self.backend, self.frontend), self.old)
                self.server.commits[app] = self.newer
                self.assertIsNone(self.deploy())
                self.assertEqual(self.writes(), [])
                self.assertIn(f"{role} already runs newer commit {self.newer}", self.output.getvalue())

    def test_diverged_frontend_blocks_backend(self):
        self.server.commits[self.frontend] = self.other
        with self.assertRaisesRegex(DeployError, f"{self.other}.*diverged"):
            self.deploy()
        self.assertEqual(self.writes(), [])

    def test_unknown_frontend_commit_blocks_backend(self):
        self.server.commits[self.frontend] = "f" * 12
        with self.assertRaisesRegex(DeployError, "cannot be resolved uniquely"):
            self.deploy()
        self.assertEqual(self.writes(), [])

    def test_persistently_unreadable_frontend_blocks_backend(self):
        for response, reason in (
            ((404, b"missing"), "HTTP 404"),
            ((200, json.dumps({"commit": TOKEN}).encode()), "12-hex commit"),
        ):
            with self.subTest(response=response):
                self.server.frontend_versions = [response] * 5
                with self.assertRaisesRegex(DeployError, f"timed out.*{reason}") as raised:
                    self.deploy(timeout=35)
                self.assertNotIn(TOKEN, str(raised.exception) + self.output.getvalue())
                self.assertEqual(self.writes(), [])

    def test_frontend_advances_during_backend_rollout(self):
        self.server.frontend_versions = [
            (200, json.dumps({"commit": commit}).encode()) for commit in (self.old, self.newer)
        ]
        self.deploy()
        self.assertEqual(self.writes(), [self.backend])
        self.assertIn(f"already runs newer commit {self.newer}", self.output.getvalue())

    def test_last_read_before_patch_must_still_identify_the_commit(self):
        self.server.health_sequence = [self.old] + [503] * 5
        with self.assertRaisesRegex(DeployError, "timed out.*version unavailable"):
            self.deploy(timeout=35)
        self.assertEqual(self.writes(), [])

    def test_partial_rollout_resumes_only_frontend(self):
        self.server.commits[self.backend] = self.sha[:12]
        self.assertEqual(self.deploy(), "")
        self.assertEqual(self.writes(), [self.frontend])

    def test_old_frontend_missing_version_document_can_be_upgraded(self):
        self.server.commits[self.backend] = self.sha[:12]
        self.server.frontend_versions = [(404, b"missing")] * 3
        self.deploy()
        self.assertEqual(self.writes(), [self.frontend])

    def test_unavailable_old_backend_can_be_upgraded(self):
        self.server.health_sequence = [503, 503, 503]
        self.deploy()
        self.assertEqual(self.writes(), [self.backend, self.frontend])

    def test_ready_frontend_serving_old_build_cannot_finish_rollout(self):
        self.server.frontend_versions = [(200, json.dumps({"commit": self.old}).encode())] * 9
        with self.assertRaisesRegex(DeployError, f"timed out.*frontend phase=ready.*commit={self.old}"):
            self.deploy(timeout=75)
        self.assertEqual(self.writes(), [self.backend, self.frontend])

    def test_transient_health_error_on_rerun_does_not_trigger_a_write(self):
        self.server.commits = dict.fromkeys([self.backend, self.frontend], self.sha[:12])
        self.server.health_sequence = [self.sha[:12], 503, self.sha[:12], self.sha[:12], self.sha[:12]]
        self.deploy()
        self.assertEqual(self.writes(), [])

    def test_backend_not_ready_blocks_frontend_and_reports_pod_reason(self):
        self.server.phases[self.backend] = "progressing"
        with self.assertRaisesRegex(DeployError, f"timed out.*backend phase=progressing.*FixturePod"):
            self.deploy(timeout=37)
        self.assertEqual(self.server.now, 37)
        self.assertEqual(self.writes(), [self.backend])

    def test_frontend_shares_the_backend_deadline(self):
        self.server.phases[self.frontend] = "degraded"
        with self.assertRaisesRegex(DeployError, f"timed out.*frontend phase=degraded.*FixturePod"):
            self.deploy(timeout=55)
        self.assertEqual(self.server.now, 55)
        self.assertEqual(self.writes(), [self.backend, self.frontend])

    def test_permanent_proxy_error_surfaces_details_and_redacts_secrets(self):
        self.server.status_failure = 403, json.dumps({
            "error": "wrong_namespace", "error_code": 4,
            "message": "forbidden " + " ".join([
                TOKEN, self.client_id, SECRET, quote_plus(SECRET), *private_values(self.config),
                quote_plus(self.config["org_id"]), urlsplit(self.config["apps_url"]).path,
            ]),
        }).encode()
        with self.assertRaisesRegex(DeployError, "HTTP 403 wrong_namespace forbidden") as result:
            self.deploy()
        self.assertNotIn(urlsplit(self.config["apps_url"]).path, str(result.exception))
        self.assertEqual(self.writes(), [])

    def test_ambiguous_patch_fails_without_blind_retry(self):
        self.server.patch_failure = 502, b'{"error":"proxy_failed","message":"upstream closed"}'
        with self.assertRaisesRegex(DeployError, "502 proxy_failed upstream closed"):
            self.deploy()
        self.assertEqual(self.writes(), [self.backend])

    def test_actions_masks_precede_rollout_output(self):
        with patch.dict(os.environ, {"GITHUB_ACTIONS": "true"}):
            self.deploy()
        *commands, summary = self.output.getvalue().splitlines()
        expected = private_values(self.config) | {TOKEN}
        self.assertCountEqual(commands, ["::add-mask::" + value.replace("%", "%25") for value in expected])
        self.assertIn("ready at", summary)

    def test_public_labels_survive_in_summary_and_job_log(self):
        self.configure_public_labels()
        self.server.commits = dict.fromkeys((self.backend, self.frontend), self.sha[:12])
        with patch.dict(os.environ, {"GITHUB_ACTIONS": "true"}):
            self.deploy()
        *commands, summary = self.output.getvalue().splitlines()
        self.assertEqual(summary, f"Skipping deployment, {self.name} already ready at {self.sha[:12]} "
                         "(backend already current, frontend already current).")
        self.assertCountEqual(commands, ["::add-mask::" + value.replace("%", "%25")
                                         for value in self.expected_private | {TOKEN}])

    def test_public_labels_survive_in_failure_details(self):
        self.configure_public_labels()
        self.server.phases[self.backend] = "progressing"
        self.server.pod_reason = f"{self.frontend} Back rOnT " + " ".join(sorted(self.expected_private))
        with self.assertRaisesRegex(DeployError, f"{self.name} rollout timed out") as failure:
            self.deploy(timeout=5)
        self.assertIn(f"backend phase=progressing pod reasons={self.frontend} Back rOnT ", str(failure.exception))

    def test_actions_masks_registered_even_when_preflight_skips_login(self):
        self.server.commits[self.frontend] = self.newer
        with patch.dict(os.environ, {"GITHUB_ACTIONS": "true"}):
            self.deploy()
        *commands, summary = self.output.getvalue().splitlines()
        self.assertCountEqual(commands, ["::add-mask::" + value.replace("%", "%25")
                                         for value in private_values(self.config)])
        self.assertIn("frontend already runs newer commit", summary)

    def test_timeout_redacts_phase_and_pod_reasons(self):
        self.server.phases[self.backend] = "waiting-" + self.backend
        self.server.pod_reason = "PodFailure " + " ".join(private_values(self.config))
        with self.assertRaisesRegex(DeployError, f"backend phase=waiting-.*PodFailure.*commit={self.old}"):
            self.deploy(timeout=5)

    def test_cli_uses_the_selected_targets_configuration_and_credentials(self):
        s = self.server
        env = {"GITHUB_SHA": self.sha, self.credential_prefix + "_ID": self.client_id,
               self.credential_prefix + "_SECRET": SECRET, "NYXID_DEPLOY_TARGET": json.dumps(self.config)}
        with patch.dict(os.environ, env), patch("sys.argv", ["deploy_images.py", self.name]):
            with patch("deploy_images.Rollout", side_effect=lambda target: Rollout(
                    target, send=s.send, sleep=s.sleep, clock=lambda: s.now)):
                main()
        self.assertEqual(self.writes(), [self.backend, self.frontend])

    def test_configured_deadline(self):
        rollout = Rollout(self.target, clock=lambda: 50)
        self.assertEqual(rollout.remaining(), self.rollout_seconds)

    def test_missing_credentials(self):
        for client_id, secret in ((None, SECRET), (self.client_id, None)):
            with self.subTest(client_id=client_id):
                rollout = Rollout(self.target, send=self.server.send)
                with self.assertRaisesRegex(DeployError, f"{self.target.name} deployer credentials are missing"):
                    rollout.run(self.sha, client_id, secret)
                self.assertEqual(self.writes(), [])


class ProductionDeployTests(StagingDeployTests):
    name = "production"
    suffix = "b"
    credential_prefix = "NYXID_PRODUCTION_DEPLOY_CLIENT"
    branch = "main"
    client_id = "fixture-b-deployer"
    rollout_seconds = 1800
