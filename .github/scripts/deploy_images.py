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
TARGETS = {"staging": ("dev", 900), "production": ("main", 1800)}
TARGET_SECRET = "NYXID_DEPLOY_TARGET"
POLL_SECONDS = 10
REQUIRED_SAMPLES = 3
DIGEST = r"sha256:[0-9a-f]{64}"


@dataclass(frozen=True)
class App:
    role: str
    name: str
    service: str
    version_url: str


@dataclass(frozen=True)
class Target:
    name: str
    branch: str
    timeout: int
    apps_url: str
    org_id: str
    backend: App
    frontend: App


class DeployError(Exception):
    pass


class RestoreError(DeployError):
    """Only the restoration stage is safe to publish."""


class ObjectPairs(list):
    """Keep JSON objects distinct from arrays and preserve duplicate keys for validation."""


def object_fields(value, path, required):
    if not isinstance(value, ObjectPairs):
        raise DeployError(f"{path} must be an object")
    fields = {}
    for key, item in value:
        if key not in required:
            raise DeployError(f"{path} contains an unexpected field")
        if key in fields:
            raise DeployError(f"{path}.{key} is duplicated")
        fields[key] = item
    for key in required:
        if key not in fields:
            raise DeployError(f"{path}.{key} is missing")
    return fields


def identifier(value, path, pattern):
    if not isinstance(value, str) or not re.fullmatch(pattern, value):
        raise DeployError(f"{path} is not a valid identifier")
    return value


def https_url(value, path):
    error = DeployError(f"{path} must be an HTTPS URL without credentials, query or fragment")
    if not isinstance(value, str) or not value.startswith("https://"):
        raise error
    if re.search(r"[^!-~]|[?#\\]", value):
        raise error
    try:
        parts = urlsplit(value)
        port = parts.port
    except ValueError:
        raise error from None
    # urlsplit alone accepts malformed authorities, including junk after an IPv6 literal.
    label = r"[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?"
    host = rf"(?:{label}(?:\.{label})*\.?|\[[a-fA-F0-9:.]+\])"
    if not re.fullmatch(host + r"(?::[0-9]+)?", parts.netloc) or port == 0:
        raise error
    if not re.fullmatch(r"(?:[a-zA-Z0-9/._~!$&'()*+,;=:@-]|%[0-9a-fA-F]{2})*", parts.path):
        raise error
    return value


def load_target(name, raw):
    if not raw or "\n" in raw or "\r" in raw:
        raise DeployError(f"{TARGET_SECRET} must be a single-line JSON object")
    try:
        obj = json.loads(raw, object_pairs_hook=ObjectPairs)
    except (ValueError, RecursionError):
        raise DeployError(f"{TARGET_SECRET} must be a JSON object") from None
    fields = object_fields(obj, TARGET_SECRET, ("apps_url", "org_id", "backend", "frontend"))

    def app(role):
        path = f"{TARGET_SECRET}.{role}"
        settings = object_fields(fields[role], path, ("app", "service", "version_url"))
        pattern = r"[A-Za-z0-9][A-Za-z0-9._-]{0,62}"
        return App(
            role, identifier(settings["app"], path + ".app", pattern),
            identifier(settings["service"], path + ".service", pattern),
            https_url(settings["version_url"], path + ".version_url"),
        )

    branch, timeout = TARGETS[name]
    return Target(
        name, branch, timeout,
        https_url(fields["apps_url"], TARGET_SECRET + ".apps_url"),
        identifier(fields["org_id"], TARGET_SECRET + ".org_id", r"[A-Za-z0-9][A-Za-z0-9:._-]{0,127}"),
        app("backend"), app("frontend"),
    )


class NewerCommit(Exception):
    pass


def resolve_commit(app, observed):
    # Match object IDs only. A hex-named ref or an ambiguous prefix cannot
    # authorize a write. The deploy job checks out full commit history.
    resolved = subprocess.run(["git", "rev-parse", "--disambiguate=" + observed],
                              capture_output=True, text=True)
    full = resolved.stdout.strip()
    if resolved.returncode or not re.fullmatch(r"[0-9a-f]{40}", full):
        raise DeployError(f"{app.role} commit {observed} cannot be resolved uniquely in the checkout")
    return full


def already_current(app, observed, target):
    full = resolve_commit(app, observed)
    if full == target:
        return True
    base = subprocess.run(["git", "merge-base", full, target], capture_output=True, text=True)
    if base.returncode not in (0, 1):
        raise DeployError(f"cannot compare {app.role} commit {observed} with {target[:12]}")
    ancestor = base.stdout.strip()
    if ancestor == full:
        return False
    if ancestor == target:
        raise NewerCommit(f"{app.role} already runs newer commit {observed}")
    raise DeployError(f"{app.role} commit {observed} and candidate {target[:12]} have diverged")


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


def mask(value):
    if os.environ.get("GITHUB_ACTIONS") == "true":
        print("::add-mask::" + value.replace("%", "%25"), flush=True)


def image_tag(role, branch, sha):
    return f"ghcr.io/chronoaiproject/nyxid/{role}:{branch}-{sha[:7]}"


def published_digests(branch, sha):
    digests = {}
    for role in ("backend", "frontend"):
        try:
            result = subprocess.run(["docker", "buildx", "imagetools", "inspect", image_tag(role, branch, sha)],
                                    capture_output=True, text=True, check=True, timeout=15)
        except (OSError, subprocess.SubprocessError):
            return {}
        match = re.search(rf"^Digest:[ \t]+({DIGEST})[ \t]*$", result.stdout, re.MULTILINE)
        if not match:
            return {}
        digests[role] = match[1]
    return digests


class Rollout:
    def __init__(self, target, *, send=exchange, clock=time.monotonic, sleep=time.sleep, timeout=None):
        self.target = target
        self.send = send
        self.clock = clock
        self.sleep = sleep
        self.deadline = clock() + (target.timeout if timeout is None else timeout)
        self.token = ""
        apps = urlsplit(target.apps_url)
        values = {target.apps_url, target.org_id, apps.netloc + apps.path}
        for app in (target.backend, target.frontend):
            values.update((app.name, app.service, app.version_url))
        # Masking a substring of a public label would obscure that label throughout the job log.
        public_labels = (target.name, "backend", "frontend")
        self.private_values = {
            value for value in values if not any(value.lower() in label for label in public_labels)
        }
        # The proxy can echo a relative request path or a form-encoded identifier.
        self.redactions = self.private_values.copy()
        if apps.path.rstrip("/"):
            self.redactions.add(apps.path.rstrip("/"))
        self.redactions |= {quote_plus(value) for value in self.redactions}
        self.applied_digests = {}
        self.status = {}
        self.last_error = ""

    def redact(self, message):
        for private in sorted(self.redactions, key=len, reverse=True):
            message = message.replace(private, "***")
        return " ".join(message.splitlines())

    def remaining(self):
        left = self.deadline - self.clock()
        if left <= 0:
            raise DeployError("rollout timed out")
        return left

    def request(self, method, url, label, *, headers=None, body=None):
        try:
            code, raw = self.send(method, url, headers or {}, body, min(30, self.remaining()))
        except (OSError, http.client.HTTPException) as error:
            raise RequestError(f"{method} {label}: {type(error).__name__}", method == "GET") from None
        try:
            data = json.loads(raw) if raw else {}
        except (ValueError, UnicodeError):
            data = None
        if not 200 <= code < 300:
            fields = data if isinstance(data, dict) else {}
            raise RequestError(
                f"{method} {label}: HTTP {code} {fields.get('error', '')} {fields.get('message', '')}",
                method == "GET" and (code in (408, 429) or code >= 500),
                status=code,
            )
        if not isinstance(data, dict):
            raise RequestError(f"{method} {label}: invalid JSON object", False, status=code)
        return data

    def login(self, client_id, secret):
        if not client_id or not secret:
            raise DeployError("deployer credentials are missing")
        self.redactions.update((client_id, quote_plus(client_id), secret, quote_plus(secret)))
        form = urlencode({"grant_type": "client_credentials", "client_id": client_id,
                          "client_secret": secret, "scope": "proxy"})
        data = self.request("POST", TOKEN_URL, "token endpoint", body=form.encode(),
                            headers={"Content-Type": "application/x-www-form-urlencoded"})
        token = data.get("access_token")
        if not isinstance(token, str) or not re.fullmatch(r"[A-Za-z0-9._~+/=-]+", token):
            raise DeployError("token endpoint returned no usable access token")
        self.token = token
        self.redactions.update((token, quote_plus(token)))
        mask(token)

    def app_request(self, app, method, *, body=None):
        url = f"{self.target.apps_url.rstrip('/')}/{app.name}" + ("/status" if method == "GET" else "")
        url += "?" + urlencode({"org_id": self.target.org_id})
        headers = {"Authorization": "Bearer " + self.token}
        if body is not None:
            headers["Content-Type"] = "application/json"
        return self.request(method, url, app.role, headers=headers, body=body)

    def version(self, app):
        try:
            data = self.request("GET", app.version_url, app.role, headers={"Cache-Control": "no-cache"})
        except RequestError as error:
            # Old images and rolling replicas can temporarily lack the document.
            detail = f"HTTP {error.status}" if error.status else "connection error"
            raise RequestError(f"{app.role} version unavailable ({detail})",
                               error.retryable or error.status in (200, 404)) from None
        commit = data.get("commit")
        if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{12}", commit):
            raise RequestError(f"{app.role} version lacks a 12-hex commit", True)
        return data

    def check_pair(self, sha):
        while True:
            try:
                observed, current = {}, {}
                for app in (self.target.backend, self.target.frontend):
                    observed[app] = self.version(app)["commit"]
                    current[app] = already_current(app, observed[app], sha)
                return current, observed
            except RequestError as error:
                if not error.retryable:
                    raise
                self.last_error = str(error)
                self.sleep(min(POLL_SECONDS, self.remaining()))

    def sample(self, app, sha, rollback_from=None):
        state = self.app_request(app, "GET")
        phase = state.get("phase")
        if not isinstance(phase, str) or not phase:
            raise DeployError(f"{app.role} status is missing the rollout phase")
        try:
            services = state.get("services") or {}
            pods = (services.get(app.service) or {}).get("pods") or []
            reasons = sorted({str(pod["reason"]) for pod in pods if pod.get("reason")})
        except (TypeError, AttributeError):
            raise DeployError(f"{app.role} phase={phase} returned invalid pod status") from None
        self.status[app] = f"{app.role} phase={phase} pod reasons={', '.join(reasons) or 'none'}"
        version = self.version(app)
        observed = version["commit"]
        self.status[app] += f" commit={observed}"
        if rollback_from is None:
            current = already_current(app, observed, sha)
        else:
            if observed not in (rollback_from[:12], sha[:12]):
                raise DeployError("live release changed during rollback")
            current = observed == sha[:12]
        ready = phase == "ready" and (app != self.target.backend or version.get("status") == "ok")
        return observed, current, current and ready

    def ensure(self, app, sha, was_current, rollback_from=None, digest=None):
        self.status[app] = f"{app.role} phase=unknown pod reasons=unknown"
        replaced = None
        consecutive = 0
        while consecutive < REQUIRED_SAMPLES:
            try:
                observed, current, ready = self.sample(app, sha, rollback_from)
                if not was_current and replaced is None and not current:
                    if rollback_from:
                        peer = self.target.frontend if app == self.target.backend else self.target.backend
                        expected = rollback_from if app == self.target.backend else sha
                        if self.version(peer)["commit"] != expected[:12]:
                            raise DeployError("peer release changed before rollback write")
                    image = image_tag(app.role, self.target.branch, sha)
                    if digest:
                        image += "@" + digest
                    body = json.dumps({"services": {app.service: {"image": image}}}).encode()
                    self.app_request(app, "PATCH", body=body)
                    replaced = observed
                if ready and app == self.target.frontend:
                    _, _, ready = self.sample(self.target.backend, sha, rollback_from)
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
        return replaced

    def run(self, sha, client_id, secret):
        for value in sorted(self.private_values):
            mask(value)
        try:
            if not re.fullmatch(r"[0-9a-f]{40}", sha):
                raise DeployError(f"expected the full {self.target.branch} commit SHA")
            current, observed = self.check_pair(sha)
            self.login(client_id, secret)
            digests = {}
            if os.getenv("NYXID_PIN_CHECK_IMAGES") == "true" and not any(current.values()):
                digests = published_digests(self.target.branch, sha)
                if not digests:
                    print("::notice::Image digests unavailable. This rollout cannot become a rollback baseline.")
            backend = self.ensure(self.target.backend, sha, current[self.target.backend], digest=digests.get("backend"))
            frontend = self.ensure(self.target.frontend, sha, current[self.target.frontend], digest=digests.get("frontend"))
            if backend and frontend:
                self.applied_digests = digests
            self.remaining()
        except NewerCommit as error:
            print(self.redact(f"Skipping further {self.target.name} updates for {sha[:12]}: {error}."))
            return
        except DeployError as error:
            detail = " | ".join(filter(None, [str(error), *self.status.values(), self.last_error]))
            raise DeployError(self.redact(f"{self.target.name} {detail}")) from None
        summary = (f"Skipping deployment, {self.target.name} already ready" if all(current.values())
                   else f"{self.target.name.capitalize()} ready")
        backend_status = "updated" if backend else "already current"
        frontend_status = "updated" if frontend else "already current"
        print(self.redact(f"{summary} at {sha[:12]} (backend {backend_status}, frontend {frontend_status})."))
        if backend and backend == frontend and set(observed.values()) == {backend}:
            return backend
        return ""

    def restore(self, failed, previous, digests, client_id, secret):
        if not all(re.fullmatch(DIGEST, digests.get(role, "")) for role in ("backend", "frontend")):
            raise RestoreError("image_digests")
        stage = "verify_candidate"
        try:
            for app in (self.target.backend, self.target.frontend):
                if self.version(app)["commit"] != failed[:12]:
                    raise DeployError("live pair changed")
            stage = "authenticate_deployer"
            self.login(client_id, secret)
            for app in (self.target.backend, self.target.frontend):
                stage = app.role
                self.ensure(app, previous, False, rollback_from=failed, digest=digests[app.role])
            self.remaining()
        except (DeployError, OSError):
            raise RestoreError(stage) from None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", choices=TARGETS)
    name = parser.parse_args().target
    credentials = f"NYXID_{name.upper()}_DEPLOY_CLIENT"
    try:
        target = load_target(name, os.getenv(TARGET_SECRET))
        rollout = Rollout(target)
        previous = rollout.run(os.getenv("GITHUB_SHA", ""), os.getenv(credentials + "_ID"),
                               os.getenv(credentials + "_SECRET"))
        if previous is not None and os.getenv("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a") as output:
                print("deployed=true", f"previous={previous}", sep="\n", file=output)
                for role, digest in rollout.applied_digests.items():
                    print(f"{role}_digest={digest}", file=output)
    except DeployError as exc:
        sys.exit(f"Deploy failed: {exc}")


if __name__ == "__main__":
    main()
