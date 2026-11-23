import io
import json
import os
import signal
from pathlib import Path
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import Mock, patch

import post_deploy_check as check
from deploy_images import RestoreError, load_target
from test_deploy_images import DIGESTS, fake_config


SHA = "1" * 40
PREVIOUS = "2" * 40
ACCOUNT = {"email": "runner@example.test", "password": "secret-password", "owner_id": "ad3df9fa-eb5f-4190-a329-f5cb683b36ac",
           "conversation_id": "nyxa-" + "a" * 32, "channel_id": "caa90242-05dd-4f70-bea1-6dcab8b21c27"}
ORIGIN = "https://backend-a.example.test"
WEBHOOK = "https://hooks.example.test/private-target"


class CheckTests(unittest.TestCase):
    def setUp(self):
        self.files = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.enterContext(patch.dict(os.environ, {
            "NYXID_CHECK_ACCOUNT": json.dumps(ACCOUNT), "NYXID_CHECK_ALERT_URL": WEBHOOK,
            "GITHUB_SHA": SHA, "GITHUB_REPOSITORY": "example/repo", "GITHUB_RUN_ID": "7",
            "NYXID_DEPLOY_TARGET": json.dumps(fake_config("a")), "PREVIOUS_COMMIT": PREVIOUS[:12],
            "NYXID_ROLLBACK_PAIR": f"{PREVIOUS}..{SHA}", "RUNNER_TEMP": str(self.files),
            "GITHUB_STEP_SUMMARY": str(self.files / "summary"), "GITHUB_OUTPUT": str(self.files / "outputs"),
        }, clear=True))
        self.output = self.enterContext(redirect_stdout(io.StringIO()))
        self.enterContext(patch("sys.argv", ["post_deploy_check.py", "staging"]))
        self.calls = []
        self.key = None
        self.terminal = {}
        self.auth_id = ACCOUNT["owner_id"]
        self.uncertain = None
        self.channel = {"message_id": "opaque", "platform_message_id": "delivered"}
        self.alert = self.enterContext(patch.object(check, "http_request"))
        self.sleep = Mock()
        probe = check.probe
        self.enterContext(patch.object(check, "probe", side_effect=lambda origin, account, result:
                                      probe(origin, account, result, request=self.request, sleep=self.sleep)))
        self.enterContext(patch.object(check, "resolve_commit", return_value=PREVIOUS))
        self.enterContext(patch.object(check.subprocess, "run", return_value=Mock(returncode=0)))
        self.prior_images = self.enterContext(patch.object(check, "prior_images", return_value=DIGESTS))
        self.restore = self.enterContext(patch.object(check.Rollout, "restore"))

    def request(self, method, url, body=None, token=None):
        self.assertTrue(url.startswith(ORIGIN + "/"))
        path = url[len(ORIGIN):]
        self.calls.append((method, path))
        if path == "/api/v1/auth/login":
            self.assertIsNone(token)
            self.assertEqual(body, {"client": "cli", "email": ACCOUNT["email"], "password": ACCOUNT["password"]})
            return {"user_id": self.auth_id, "access_token": "private-token"}
        self.assertEqual(token, "private-token")
        if path == check.INVOCATIONS:
            self.assertEqual(self.key, None, "paid admission was retried")
            self.key = body["idempotency_key"]
            self.assertEqual(body["conversation_id"], ACCOUNT["conversation_id"])
            if self.uncertain:
                raise self.uncertain
            return dict(self.receipt(), terminal=False, status="running", retry_after_ms=1000)
        if path == f"{check.INVOCATIONS}/{self.key}":
            return dict(self.receipt(), **self.terminal)
        if path == "/api/v1/channel-relay/send":
            self.assertEqual((body["conversation_id"], body["idempotency_key"]), (ACCOUNT["channel_id"], self.key))
            return self.channel
        self.assertIn(path, (f"{check.INVOCATIONS}/{self.key}/cancel", "/api/v1/auth/logout"))
        return {}

    def receipt(self):
        reply = {"conversation_id": ACCOUNT["conversation_id"], "turn_id": "root", "status": "completed",
                 "text": "private-model-reply"}
        return {"conversation_id": ACCOUNT["conversation_id"], "invocation_id": self.key, "turn_id": "root",
                "status": "completed", "turn_count": 1, "terminal": True, "replies": [reply]}

    def execute(self, expected_exit):
        self.assertEqual(check.main(), expected_exit)
        result = (self.files / "nyxbot-deploy.json").read_text()
        visible = result + self.output.getvalue() + (self.files / "summary").read_text()
        if self.alert.called:
            args = self.alert.call_args.args
            self.assertEqual(args[:2], ("POST", WEBHOOK))
            visible += json.dumps(args[2])
        for value in [*ACCOUNT.values(), WEBHOOK, "private-model-reply", "private-token", "private-diagnostic",
                      "Reply with one short greeting", "NyxID deployment check."]:
            self.assertNotIn(value, visible)
        row = json.loads(result)
        self.assertIn("outcome=" + row["outcome"], (self.files / "outputs").read_text().splitlines())
        self.assertEqual(row["commit"], SHA)
        return row

    def test_pass_requires_completed_root_model_reply_and_platform_receipt(self):
        os.environ.update({role.upper() + "_DEPLOY_DIGEST": digest for role, digest in DIGESTS.items()})
        row = self.execute(0)
        self.assertEqual(row["images"], DIGESTS)
        self.assertIn("image_suffix=-" + "a" * 64 + "-" + "b" * 64, (self.files / "outputs").read_text())
        self.assertEqual(row["outcome"], "passed")
        self.assertEqual(len(self.calls), 5)
        self.assertIn(("POST", "/api/v1/channel-relay/send"), self.calls)
        self.restore.assert_not_called()
        self.alert.assert_not_called()

    def test_model_failure_rolls_back_and_notifies(self):
        self.terminal = {"status": "failed", "error_code": "private-diagnostic"}
        row = self.execute(1)
        self.assertEqual((row["stage"], row["rollback"], row["notification"]), ("model_reply", "restored", "sent"))
        self.restore.assert_called_once()
        self.assertNotIn(("POST", "/api/v1/channel-relay/send"), self.calls)

    def test_admission_timeout_cancels_exact_invocation_without_retry(self):
        self.uncertain = TimeoutError("private-diagnostic")
        row = self.execute(1)
        self.assertEqual((row["stage"], row["reason"], row["cancel"]), ("admit", "timeout", "requested"))
        self.assertIn(("POST", f"{check.INVOCATIONS}/{self.key}/cancel"), self.calls)

    def test_total_deadline_while_polling_cancels_the_root(self):
        self.sleep.side_effect = lambda _: signal.raise_signal(signal.SIGALRM)
        row = self.execute(1)
        self.assertEqual((row["stage"], row["reason"], row["cancel"]), ("model_reply", "timeout", "requested"))
        self.assertIn(("POST", f"{check.INVOCATIONS}/{self.key}/cancel"), self.calls)

    def test_invalid_account_alerts_without_rollback(self):
        os.environ["NYXID_CHECK_ACCOUNT"] = "private-diagnostic"
        row = self.execute(1)
        self.assertEqual((row["stage"], row["rollback"], row["notification"]), ("configuration", "none", "sent"))
        self.assertEqual(self.calls, [])
        self.restore.assert_not_called()

    def test_incomplete_or_cross_turn_reply_fails(self):
        for change in ({"text": " "}, {"turn_id": "other"}, {"conversation_id": "other"}, {"status": "failed"}):
            reply = dict(self.receipt()["replies"][0], **change)
            with self.subTest(change=change), self.assertRaises(check.CheckError):
                check.verify_reply({"status": "completed", "turn_count": 1, "replies": [reply]}, "root",
                                   ACCOUNT["conversation_id"])

    def test_wrong_owner_stops_before_paid_admission(self):
        self.auth_id = "other"
        row = self.execute(1)
        self.assertEqual(row["stage"], "authenticate")
        self.assertIsNone(self.key)

    def test_missing_receipt_and_failed_restore_alert(self):
        self.channel = {"message_id": "opaque"}
        self.restore.side_effect = RestoreError("frontend")
        row = self.execute(1)
        self.assertEqual((row["stage"], row["rollback"], row["notification"]), ("channel", "failed", "sent"))
        self.assertEqual(row["rollback_stage"], "frontend")

    def test_unapproved_pair_leaves_failed_release_in_place_and_notifies(self):
        self.channel = {}
        os.environ["NYXID_ROLLBACK_PAIR"] = f"{PREVIOUS}..{'3' * 40}"
        row = self.execute(1)
        self.assertEqual((row["rollback"], row["notification"]), ("blocked_compatibility", "sent"))
        self.prior_images.assert_not_called()
        self.restore.assert_not_called()

    def test_missing_passing_deploy_blocks_even_an_approved_pair(self):
        self.channel = {}
        self.prior_images.return_value = False
        row = self.execute(1)
        self.assertEqual(row["rollback"], "blocked_no_passing_deploy")
        self.restore.assert_not_called()

    def test_unknown_previous_pair_blocks(self):
        self.channel = {}
        os.environ["PREVIOUS_COMMIT"] = ""
        row = self.execute(1)
        self.assertEqual(row["rollback"], "blocked_no_previous_pair")
        self.restore.assert_not_called()

    def test_evidence_and_notification_errors_are_visible_without_provider_text(self):
        self.channel = {}
        self.prior_images.side_effect = TimeoutError("private-diagnostic")
        self.alert.side_effect = OSError("private-diagnostic")
        row = self.execute(1)
        self.assertEqual((row["rollback"], row["notification"]), ("blocked_evidence_unavailable", "failed"))
        self.restore.assert_not_called()

    def test_missing_secrets_skip_with_warning_and_no_io(self):
        for values in ({"NYXID_CHECK_ACCOUNT": ""}, {"NYXID_CHECK_ALERT_URL": ""}, {"NYXID_CHECK_ACCOUNT": "{}"}):
            with self.subTest(values=values), patch.dict(os.environ, values):
                self.assertEqual(check.run("staging")["outcome"], "skipped")
        self.assertIn("::warning::", self.output.getvalue())
        self.assertIn("NyxBot is unverified", self.output.getvalue())
        self.assertEqual(self.calls, [])
        self.restore.assert_not_called()
        self.alert.assert_not_called()


class EvidenceTests(unittest.TestCase):
    def test_matching_successful_publish_run_and_nonexpired_attempt_artifact(self):
        target = load_target("staging", json.dumps(fake_config("a")))
        run = {"head_sha": PREVIOUS, "head_branch": "dev", "conclusion": "success", "event": "push",
               "id": 42, "run_attempt": 3}
        artifact = {"name": "nyxbot-staging-passed-3-" + "a" * 64 + "-" + "b" * 64, "expired": False}
        for run_change, artifact_change, expected in [
            ({}, {}, True), ({"head_branch": "main"}, {}, False), ({"head_sha": SHA}, {}, False),
            ({"conclusion": "failure"}, {}, False), ({"event": "pull_request"}, {}, False),
            ({}, {"expired": True}, False), ({}, {"name": "nyxbot-production-passed-3"}, False),
            ({}, {"name": "nyxbot-staging-skipped-3"}, False), ({}, {"name": "nyxbot-staging-passed-2"}, False),
        ]:
            read = Mock(side_effect=[{"workflow_runs": [dict(run, **run_change)]},
                                     {"artifacts": [dict(artifact, **artifact_change)]}])
            self.assertEqual(check.prior_images(target, PREVIOUS, read), DIGESTS if expected else None)
            self.assertIn(f"publish-dev-images.yml/runs?branch=dev&head_sha={PREVIOUS}", read.call_args_list[0].args[0])
            if read.call_count == 2:
                self.assertIn("actions/runs/42/artifacts?per_page=100", read.call_args.args[0])
