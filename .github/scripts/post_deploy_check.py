"""Check one private NyxBot turn and channel delivery inside the deployment slot."""

import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import http.client
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time
from urllib.parse import urlsplit
import uuid

from deploy_images import DIGEST, DeployError, RestoreError, Rollout, TARGETS, https_url, load_target, resolve_commit


INVOCATIONS = "/api/v1/assistant/nyxagent/invocations"
CHECK_SECONDS = 90
MAX_RESPONSE = 256 * 1024


class CheckError(Exception):
    pass


@contextmanager
def time_limit(seconds):
    # Socket timeouts alone allow a slow trickle to hold the deploy slot indefinitely.
    def expired(*_):
        raise TimeoutError

    previous = signal.signal(signal.SIGALRM, expired)
    signal.alarm(seconds)
    try:
        yield
    finally:
        signal.alarm(0)
        signal.signal(signal.SIGALRM, previous)


def http_request(method, url, body=None, token=None, *, json_response=True):
    parsed = urlsplit(url)
    connection = http.client.HTTPSConnection(parsed.netloc, timeout=10)
    headers = {"Cache-Control": "no-store"}
    if token:
        headers["Authorization"] = "Bearer " + token
    if body is not None:
        headers["Content-Type"] = "application/json"
    try:
        connection.request(method, parsed.path, json.dumps(body) if body is not None else None, headers)
        response = connection.getresponse()
        if not 200 <= response.status < 300:
            raise CheckError
        if json_response:
            raw = response.read(MAX_RESPONSE + 1)
            if len(raw) > MAX_RESPONSE:
                raise CheckError
            value = json.loads(raw)
            if not isinstance(value, dict):
                raise CheckError
            return value
    finally:
        connection.close()


def account_settings(raw):
    fields = json.loads(raw)
    required = ("email", "password", "owner_id", "conversation_id", "channel_id")
    if not isinstance(fields, dict):
        raise ValueError
    if any(not fields.get(k) for k in required):
        return None
    if not all(isinstance(fields[k], str) for k in required):
        raise ValueError
    for name in ("owner_id", "channel_id"):
        if str(uuid.UUID(fields[name])) != fields[name]:
            raise ValueError
    if not re.fullmatch(r"nyxa-[a-f0-9]{32}", fields["conversation_id"]):
        raise ValueError
    return fields


def verify_reply(row, root_turn, conversation):
    if row.get("status") == "completed" and row.get("turn_count") == 1 and not row.get("error_code"):
        for reply in row.get("replies", []):
            if (isinstance(reply, dict) and reply.get("conversation_id") == conversation
                    and reply.get("turn_id") == root_turn and reply.get("status") == "completed"
                    and not reply.get("error_code") and isinstance(reply.get("text"), str) and reply["text"].strip()):
                return
    raise CheckError


def probe(origin, account, record, *, request=http_request, sleep=time.sleep):
    key = str(uuid.uuid4())
    receipt_url = origin + INVOCATIONS + "/" + key
    token = None
    may_be_running = False
    try:
        with time_limit(CHECK_SECONDS):
            record["stage"] = "authenticate"
            login = request("POST", origin + "/api/v1/auth/login", {
                "client": "cli", "email": account["email"], "password": account["password"],
            })
            token = login.get("access_token")
            if login.get("user_id") != account["owner_id"] or not isinstance(token, str) or not token:
                raise CheckError

            record["stage"] = "admit"
            may_be_running = True
            row = request("POST", origin + INVOCATIONS, {
                "idempotency_key": key, "conversation_id": account["conversation_id"],
                "text": "Reply with one short greeting. Use no tools.",
            }, token)
            record["stage"] = "model_reply"
            root_turn = row.get("turn_id")
            while True:
                if (not isinstance(root_turn, str) or not root_turn or row.get("turn_id") != root_turn
                        or row.get("invocation_id") != key or row.get("conversation_id") != account["conversation_id"]):
                    raise CheckError
                if row.get("terminal") is True:
                    may_be_running = False
                    break
                if row.get("status") not in ("running", "waiting") or row.get("turn_count") != 1:
                    raise CheckError
                delay = row.get("retry_after_ms")
                if type(delay) is not int or not 0 <= delay <= CHECK_SECONDS * 1000:
                    raise CheckError
                sleep(max(1, delay / 1000))
                row = request("GET", receipt_url, token=token)
            verify_reply(row, root_turn, account["conversation_id"])

            record["stage"] = "channel"
            sent = request("POST", origin + "/api/v1/channel-relay/send", {
                "conversation_id": account["channel_id"], "idempotency_key": key,
                "message": {"text": "NyxID deployment check."},
            }, token)
            if not all(isinstance(sent.get(k), str) and sent[k].strip() for k in ("message_id", "platform_message_id")):
                raise CheckError
    finally:
        if may_be_running:
            try:
                with time_limit(5):
                    request("POST", receipt_url + "/cancel", token=token)
                record["cancel"] = "requested"
            except (CheckError, OSError, http.client.HTTPException, ValueError, RecursionError):
                record["cancel"] = "unconfirmed"
        if isinstance(token, str) and token:
            try:
                with time_limit(5):
                    request("POST", origin + "/api/v1/auth/logout", token=token)
            except (CheckError, OSError, http.client.HTTPException, ValueError, RecursionError):
                record["logout"] = "unconfirmed"


def github_read(endpoint):
    response = subprocess.run(["gh", "api", "repos/{owner}/{repo}/" + endpoint],
                              capture_output=True, timeout=10, check=True)
    return json.loads(response.stdout)


def prior_images(target, sha, read=github_read):
    workflow = "publish-dev-images.yml" if target.name == "staging" else "publish-images.yml"
    runs = read(f"actions/workflows/{workflow}/runs?branch={target.branch}&head_sha={sha}&per_page=1")
    if not runs["workflow_runs"]:
        return None
    run = runs["workflow_runs"][0]
    if (run["head_sha"] != sha or run["head_branch"] != target.branch or run["conclusion"] != "success"
            or run["event"] not in ("push", "workflow_dispatch")):
        return None
    prefix = f"nyxbot-{target.name}-passed-{run['run_attempt']}-"
    artifacts = read(f"actions/runs/{run['id']}/artifacts?per_page=100")
    pairs = []
    for artifact in artifacts["artifacts"]:
        match = re.fullmatch(prefix + r"([a-f0-9]{64})-([a-f0-9]{64})", artifact["name"])
        if match and artifact["expired"] is False:
            pairs.append({"backend": "sha256:" + match[1], "frontend": "sha256:" + match[2]})
    return pairs[0] if len(pairs) == 1 else None


def rollback(target, sha, observed, record):
    if not re.fullmatch(r"[a-f0-9]{12}", observed) or observed == sha[:12]:
        return "blocked_no_previous_pair"
    previous = resolve_commit(target.backend, observed)
    record["previous"] = previous
    if os.getenv("NYXID_ROLLBACK_PAIR") != f"{previous}..{sha}":
        return "blocked_compatibility"
    if subprocess.run(["git", "merge-base", "--is-ancestor", previous, sha], capture_output=True).returncode:
        return "blocked_history"
    digests = prior_images(target, previous)
    if not digests:
        return "blocked_no_passing_deploy"
    record["rollback"] = "failed"
    credentials = f"NYXID_{target.name.upper()}_DEPLOY_CLIENT"
    with time_limit(target.timeout + 5):
        Rollout(target).restore(sha, previous, digests, os.getenv(credentials + "_ID"), os.getenv(credentials + "_SECRET"))
    return "restored"


def notify(record, webhook):
    run_url = f"https://github.com/{os.environ['GITHUB_REPOSITORY']}/actions/runs/{os.environ['GITHUB_RUN_ID']}"
    disposition = record["rollback"]
    if record.get("rollback_stage"):
        disposition += " at " + record["rollback_stage"]
    message = (f"NyxID {record['environment']} {record['commit'][:12]} failed at {record['stage']}. "
               f"Rollback {disposition}. {run_url}")
    try:
        with time_limit(10):
            http_request("POST", webhook, {"text": message}, json_response=False)
        record["notification"] = "sent"
    except (OSError, CheckError, http.client.HTTPException, ValueError):
        record["notification"] = "failed"
        print("::error::NyxBot deploy alert delivery failed.")


def run(name):
    sha = os.environ["GITHUB_SHA"]
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("Expected full deploy SHA")
    record = {"timestamp": datetime.now(timezone.utc).isoformat(), "environment": name, "commit": sha,
              "outcome": "skipped", "stage": "configuration", "rollback": "none"}
    raw, webhook = os.getenv("NYXID_CHECK_ACCOUNT"), os.getenv("NYXID_CHECK_ALERT_URL")
    if not raw or not webhook:
        print("::warning::NyxBot deployment check skipped. Configure both NYXID_CHECK_ACCOUNT "
              "and NYXID_CHECK_ALERT_URL in this deployment environment; NyxBot is unverified.")
        return record
    target = load_target(name, os.getenv("NYXID_DEPLOY_TARGET"))
    try:
        https_url(webhook, "NYXID_CHECK_ALERT_URL")
    except DeployError:
        print("::error::Invalid NyxBot alert URL.")
        record.update(outcome="failed", notification="unavailable")
        return record
    try:
        account = account_settings(raw)
    except (ValueError, RecursionError):
        print("::error::Invalid NyxBot check account configuration.")
        record["outcome"] = "failed"
        notify(record, webhook)
        return record
    if account is None:
        print("::notice::NyxBot deployment check skipped. Test account settings are incomplete.")
        return record
    origin = "https://" + urlsplit(target.backend.version_url).netloc
    try:
        probe(origin, account, record)
    except (CheckError, OSError, http.client.HTTPException, ValueError, TypeError, RecursionError) as error:
        record["outcome"] = "failed"
        if isinstance(error, TimeoutError):
            record["reason"] = "timeout"
        print(f"::error::NyxBot deployment check failed at {record['stage']}.", flush=True)
        record["rollback"] = "blocked_evidence_unavailable"
        try:
            record["rollback"] = rollback(target, sha, os.getenv("PREVIOUS_COMMIT", ""), record)
        except RestoreError as error:
            record["rollback_stage"] = str(error)
        except (DeployError, OSError, subprocess.SubprocessError, http.client.HTTPException,
                KeyError, ValueError, TypeError, RecursionError):
            pass  # Keep the evidence/restore phase. Provider diagnostics can contain private data.
        notify(record, webhook)
    else:
        record.update(outcome="passed", stage="complete")
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", choices=TARGETS)
    record = run(parser.parse_args().target)
    suffix = ""
    if record["outcome"] == "passed":
        images = {role: os.getenv(role.upper() + "_DEPLOY_DIGEST", "") for role in ("backend", "frontend")}
        if all(re.fullmatch(DIGEST, value) for value in images.values()):
            record["images"] = images
            suffix = "-" + "-".join(value.removeprefix("sha256:") for value in images.values())
    serialized = json.dumps(record, sort_keys=True)
    print(serialized)
    (Path(os.environ["RUNNER_TEMP"]) / "nyxbot-deploy.json").write_text(serialized + "\n")
    with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
        print("NyxBot deployment check\n\n`" + serialized + "`", file=summary)
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        print("outcome=" + record["outcome"], "image_suffix=" + suffix, sep="\n", file=output)
    return int(record["outcome"] == "failed")


if __name__ == "__main__":
    raise SystemExit(main())
