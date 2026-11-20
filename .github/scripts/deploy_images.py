"""Deploy SHA-tagged backend and frontend images through the NyxID proxy."""

import argparse
from dataclasses import dataclass
import http.client
import json
import os
import re
import subprocess
import sys
import time
from urllib.parse import quote_plus, urlencode, urlsplit


TOKEN_URL = "https://api.nyxid.ai/oauth/token"
APPS = "https://api.nyxid.ai/api/v1/proxy/s/chrono-cd/api/v1/projects/nyxid/apps"
POLL_SECONDS = 10
REQUIRED_SAMPLES = 3


@dataclass(frozen=True)
class App:
    name: str
    service: str
    version_url: str


@dataclass(frozen=True)
class Target:
    name: str
    branch: str
    org_id: str
    backend: App
    frontend: App
    timeout: int


STAGING = Target(
    "staging", "dev", "internal:chrono-ai-staging",
    App("staging-api", "nyxid-backend", "https://staging-api.nyxid.ai/health"),
    App("staging", "nyxid-frontend", "https://staging.nyxid.ai/build-version.json"),
    900,
)
PRODUCTION = Target(
    "production", "main", "internal:chrono-nyxid",
    App("prod-api", "nyxid-backend-production-api-svc", "https://api.nyxid.ai/health"),
    App("prod", "nyxid-frontend", "https://nyxid.ai/build-version.json"),
    1800,
)


class DeployError(Exception):
    pass


class NewerCommit(Exception):
    pass


def already_current(app, observed, target):
    # Match object IDs only. A hex-named ref or an ambiguous prefix cannot
    # authorize a write. The deploy job checks out full commit history.
    resolved = subprocess.run(["git", "rev-parse", "--disambiguate=" + observed],
                              capture_output=True, text=True)
    full = resolved.stdout.strip()
    if resolved.returncode or not re.fullmatch(r"[0-9a-f]{40}", full):
        raise DeployError(f"{app.name} commit {observed} cannot be resolved uniquely in the checkout")
    if full == target:
        return True
    base = subprocess.run(["git", "merge-base", full, target], capture_output=True, text=True)
    if base.returncode not in (0, 1):
        raise DeployError(f"cannot compare {app.name} commit {observed} with {target[:12]}")
    ancestor = base.stdout.strip()
    if ancestor == full:
        return False
    if ancestor == target:
        raise NewerCommit(f"{app.name} already runs newer commit {observed}")
    raise DeployError(f"{app.name} commit {observed} and candidate {target[:12]} have diverged")


class RequestError(DeployError):
    def __init__(self, message, retryable, status=None):
        super().__init__(message)
        self.retryable = retryable
        self.status = status


def exchange(method, url, headers, body, timeout):
    # HTTPSConnection never follows redirects carrying a bearer or form credentials.
    target = urlsplit(url)
    conn = http.client.HTTPSConnection(target.netloc, timeout=timeout)
    try:
        path = target.path + ("?" + target.query if target.query else "")
        conn.request(method, path, body, headers)
        response = conn.getresponse()
        return response.status, response.read()
    finally:
        conn.close()


def mask_token(token):
    if os.environ.get("GITHUB_ACTIONS") == "true":
        print("::add-mask::" + token.replace("%", "%25"), flush=True)


class Rollout:
    def __init__(self, target, *, send=exchange, clock=time.monotonic, sleep=time.sleep, timeout=None):
        self.target = target
        self.send = send
        self.clock = clock
        self.sleep = sleep
        self.deadline = clock() + (target.timeout if timeout is None else timeout)
        self.token = ""
        self.redactions = []
        self.status = {}
        self.last_error = ""

    def remaining(self):
        left = self.deadline - self.clock()
        if left <= 0:
            raise DeployError(f"{self.target.name} rollout timed out")
        return left

    def request(self, method, url, *, headers=None, body=None):
        try:
            code, raw = self.send(method, url, headers or {}, body, min(30, self.remaining()))
        except (OSError, http.client.HTTPException) as error:
            raise RequestError(f"{method} {url}: {type(error).__name__}", method == "GET") from None
        try:
            data = json.loads(raw) if raw else {}
        except (ValueError, UnicodeError):
            data = None
        if not 200 <= code < 300:
            fields = data if isinstance(data, dict) else {}
            raise RequestError(
                f"{method} {url}: HTTP {code} {fields.get('error', '')} {fields.get('message', '')}",
                method == "GET" and (code in (408, 429) or code >= 500),
                status=code,
            )
        if not isinstance(data, dict):
            raise RequestError(f"{method} {url}: invalid JSON object", False, status=code)
        return data

    def login(self, client_id, secret):
        if not client_id or not secret:
            raise DeployError(f"{self.target.name} deployer credentials are missing")
        self.redactions.extend((client_id, quote_plus(client_id), secret, quote_plus(secret)))
        form = urlencode({"grant_type": "client_credentials", "client_id": client_id,
                          "client_secret": secret, "scope": "proxy"})
        data = self.request("POST", TOKEN_URL, body=form.encode(),
                            headers={"Content-Type": "application/x-www-form-urlencoded"})
        token = data.get("access_token")
        if not isinstance(token, str) or not re.fullmatch(r"[A-Za-z0-9._~+/=-]+", token):
            raise DeployError("token endpoint returned no usable access token")
        self.token = token
        self.redactions.extend((token, quote_plus(token)))
        mask_token(token)

    def app_request(self, app, method, *, body=None):
        url = f"{APPS}/{app.name}" + ("/status" if method == "GET" else "")
        url += "?" + urlencode({"org_id": self.target.org_id})
        headers = {"Authorization": "Bearer " + self.token}
        if body is not None:
            headers["Content-Type"] = "application/json"
        return self.request(method, url, headers=headers, body=body)

    def version(self, app):
        try:
            data = self.request("GET", app.version_url, headers={"Cache-Control": "no-cache"})
        except RequestError as error:
            # Old images and rolling replicas can temporarily lack the document.
            detail = f"HTTP {error.status}" if error.status else "connection error"
            raise RequestError(f"{app.name} version unavailable ({detail})",
                               error.retryable or error.status in (200, 404)) from None
        commit = data.get("commit")
        if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{12}", commit):
            raise RequestError(f"{app.name} version lacks a 12-hex commit", True)
        return data

    def check_pair(self, sha):
        while True:
            try:
                return {app: already_current(app, self.version(app)["commit"], sha)
                        for app in (self.target.backend, self.target.frontend)}
            except RequestError as error:
                if not error.retryable:
                    raise
                self.last_error = str(error)
                self.sleep(min(POLL_SECONDS, self.remaining()))

    def sample(self, app, sha):
        state = self.app_request(app, "GET")
        phase = state.get("phase")
        if not isinstance(phase, str) or not phase:
            raise DeployError(f"{app.name} status is missing the rollout phase")
        try:
            services = state.get("services") or {}
            pods = (services.get(app.service) or {}).get("pods") or []
            reasons = sorted({str(pod["reason"]) for pod in pods if pod.get("reason")})
        except (TypeError, AttributeError):
            raise DeployError(f"{app.name} phase={phase} returned invalid pod status") from None
        self.status[app] = f"{app.name} phase={phase} pod reasons={', '.join(reasons) or 'none'}"
        version = self.version(app)
        observed = version["commit"]
        self.status[app] += f" commit={observed}"
        current = already_current(app, observed, sha)
        ready = phase == "ready" and (app != self.target.backend or version.get("status") == "ok")
        return current, current and ready

    def ensure(self, app, sha, was_current):
        component = "backend" if app == self.target.backend else "frontend"
        self.status[app] = f"{app.name} phase=unknown pod reasons=unknown"
        patched = False
        consecutive = 0
        while consecutive < REQUIRED_SAMPLES:
            try:
                current, ready = self.sample(app, sha)
                if not was_current and not patched and not current:
                    image = f"ghcr.io/chronoaiproject/nyxid/{component}:{self.target.branch}-{sha[:7]}"
                    body = json.dumps({"services": {app.service: {"image": image}}}).encode()
                    self.app_request(app, "PATCH", body=body)
                    patched = True
                if ready and app == self.target.frontend:
                    _, ready = self.sample(self.target.backend, sha)
                consecutive = consecutive + 1 if ready else 0
                self.last_error = ""
            except RequestError as error:
                # A timed-out PATCH may have taken effect. Let a rerun reconcile it.
                if not error.retryable:
                    raise
                self.last_error = str(error)
                consecutive = 0
            if consecutive < REQUIRED_SAMPLES:
                self.sleep(min(POLL_SECONDS, self.remaining()))
        return "updated" if patched else "already current"

    def run(self, sha, client_id, secret):
        if not re.fullmatch(r"[0-9a-f]{40}", sha):
            raise DeployError(f"expected the full {self.target.branch} commit SHA")
        try:
            current = self.check_pair(sha)
            self.login(client_id, secret)
            backend = self.ensure(self.target.backend, sha, current[self.target.backend])
            frontend = self.ensure(self.target.frontend, sha, current[self.target.frontend])
            self.remaining()
        except NewerCommit as error:
            print(f"Skipping further {self.target.name} updates for {sha[:12]}: {error}.")
            return
        except DeployError as error:
            detail = " | ".join(filter(None, [str(error), *self.status.values(), self.last_error]))
            for private in sorted(self.redactions, key=len, reverse=True):
                detail = detail.replace(private, "***")
            raise DeployError(" ".join(detail.splitlines())) from None
        summary = (f"Skipping deployment, {self.target.name} already ready" if all(current.values())
                   else f"{self.target.name.capitalize()} ready")
        print(f"{summary} at {sha[:12]} (backend {backend}, frontend {frontend}).")


def main():
    targets = {target.name: target for target in (STAGING, PRODUCTION)}
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", choices=targets)
    target = targets[parser.parse_args().target]
    credentials = f"NYXID_{target.name.upper()}_DEPLOY_CLIENT"
    try:
        Rollout(target).run(os.environ["GITHUB_SHA"], os.getenv(credentials + "_ID"),
                            os.getenv(credentials + "_SECRET"))
    except DeployError as exc:
        sys.exit(f"Deploy failed: {exc}")


if __name__ == "__main__":
    main()
