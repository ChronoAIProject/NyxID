#!/usr/bin/env python3
"""Linux/root acceptance of the real executable; no shared accounts/settings.

Run in a disposable Linux guest:
  sudo python3 backend/tests/audit_witness_permissions.py /absolute/path/to/nyxid-audit-witness
Uses numeric UIDs 65533 (sink) and 65534 (append client), empty supplementary
groups and private /tmp fixtures. Fails, rather than skips, without prerequisites.
Only fsync/link syscalls are traced, never read/write buffers or credentials.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.request
import uuid


BINARY = str(Path(sys.argv.pop(1)).resolve()) if len(sys.argv) > 1 else ""
SINK = 65533
APP = 65534
ZERO = "0" * 64

APP_HTTP = r"""
import json, pathlib, sys, urllib.request, urllib.error
base, token_path, route = sys.argv[1:]
token = pathlib.Path(token_path).read_text()
req = urllib.request.Request(base + route, data=sys.stdin.buffer.read(),
    headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'}, method='POST')
try:
    with urllib.request.urlopen(req, timeout=5) as r:
        print(json.dumps({'status': r.status, 'body': json.load(r)}))
except urllib.error.HTTPError as e:
    print(json.dumps({'status': e.code, 'body': json.load(e) if e.code == 409 else None}))
"""

STORAGE_ATTACK = r"""
import os, pathlib, sys
operation, target, other = sys.argv[1:]
try:
    if operation == 'read': pathlib.Path(target).read_bytes()
    elif operation == 'rewrite':
        with open(target, 'r+b') as f: f.write(b'forged')
    elif operation == 'truncate':
        with open(target, 'wb'): pass
    elif operation == 'delete': os.unlink(target)
    elif operation == 'rename': os.rename(target, other)
    elif operation == 'replace': os.replace(other, target)
    elif operation == 'create':
        with open(target, 'xb'): pass
    elif operation == 'chmod': os.chmod(target, 0o666)
    elif operation == 'link': os.link(target, other)
    else: raise AssertionError('unknown test operation')
except PermissionError:
    sys.exit(0)
sys.exit(10)  # The OS must refuse; an API error does not satisfy this assertion.
"""


def demote(uid):
    def apply():
        os.setgroups([])
        os.setgid(uid)
        os.setuid(uid)
    return apply


class WitnessPermissions(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if sys.platform != "linux" or os.geteuid() != 0:
            raise RuntimeError("requires root in a disposable Linux guest; no checks were skipped")
        for tool in [BINARY, shutil.which("strace"), shutil.which("setpriv")]:
            if not tool or not os.access(tool, os.X_OK):
                raise RuntimeError("requires real witness executable, strace and setpriv")
        cls.fixture = Path(tempfile.mkdtemp(prefix="nyxid-w8-witness-permissions-"))
        os.chmod(cls.fixture, 0o755)
        cls.process = None
        cls.addClassCleanup(cls.cleanup)
        cls.admin = cls.fixture / "admin"
        cls.journal = cls.fixture / "journal"
        cls.token = cls.fixture / "append.credential"
        cls.chain = str(uuid.uuid4())
        cls.enroll_args = [BINARY, "enroll", "--admin-dir", str(cls.admin), "--journal-dir", str(cls.journal),
            "--token-output", str(cls.token), "--sink-uid", str(SINK), "--sink-gid", str(SINK),
            "--chain-id", cls.chain, "--key-version", "1", "--seq", "0", "--prev-hash", ZERO, "--entry-hash", ZERO]
        subprocess.run(cls.enroll_args, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        os.chown(cls.token, APP, APP)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            cls.port = sock.getsockname()[1]
        cls.base = f"http://127.0.0.1:{cls.port}"
        cls.start()

    @classmethod
    def start(cls):
        trace = cls.fixture / f"fsync-{uuid.uuid4()}"
        cls.process = subprocess.Popen(["strace", "-ff", "-ttt", "-yy", "-e", "trace=fsync,link,linkat", "-o", str(trace),
            "setpriv", f"--reuid={SINK}", f"--regid={SINK}", "--clear-groups", "--no-new-privs",
            BINARY, "serve", "--admin-dir", str(cls.admin), "--journal-dir", str(cls.journal), "--listen", f"127.0.0.1:{cls.port}"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if cls.process.poll() is not None:
                raise RuntimeError("real witness failed startup (no credentials logged)")
            try:
                request = urllib.request.Request(cls.base + "/v1/observe", data=b'{}', headers={"Content-Type": "application/json"})
                urllib.request.urlopen(request, timeout=0.2).close()
            except urllib.error.HTTPError:
                return
            except OSError:
                time.sleep(0.02)
        raise RuntimeError("real witness did not listen")

    @classmethod
    def stop(cls):
        if cls.process is not None and cls.process.poll() is None:
            os.killpg(cls.process.pid, signal.SIGKILL)
            cls.process.wait(timeout=5)

    @classmethod
    def cleanup(cls):
        cls.stop()
        shutil.rmtree(cls.fixture)

    def request(self, route, body):
        result = subprocess.run([sys.executable, "-c", APP_HTTP, self.base, str(self.token), route],
            input=json.dumps(body), text=True, capture_output=True, preexec_fn=demote(APP), check=True)
        return json.loads(result.stdout)

    def observe(self):
        result = self.request("/v1/observe", {"chain_id": self.chain, "challenge": "c" * 64})
        self.assertEqual(result["status"], 200)
        return result["body"]["body"]["latest"]

    def attack(self, operation, target, uid=APP):
        other = self.fixture / f"attacker-{uuid.uuid4()}"
        if operation == "replace":
            other.write_bytes(b"forged")
            os.chown(other, uid, uid)
        result = subprocess.run([sys.executable, "-c", STORAGE_ATTACK, operation, str(target), str(other)],
            preexec_fn=demote(uid), capture_output=True)
        self.assertEqual(result.returncode, 0, f"OS did not deny {operation}; no API assertion substituted")
        if other.exists():
            other.unlink()

    def test_01_real_append_credential_works(self):
        head = self.observe()
        # Obtain the exact protected predecessor digest using a tiny protocol
        # probe from the real Rust client in the separate host tests. Here the
        # live append is sent by the app UID using the server's canonical receipt
        # digest, computed independently below.
        request = {"chain_id": self.chain, "request_id": str(uuid.uuid4()), "previous_receipt": digest(head),
            "links": [{"key_version": 1, "seq": 1, "prev_hash": ZERO, "entry_hash": "1" * 64}]}
        result = self.request("/v1/advance", request)
        self.assertEqual(result["status"], 200)
        self.assertEqual(result["body"]["body"]["head"]["seq"], 1)
        self.assertEqual(self.request("/v1/advance", request), result)
        fork = dict(request, request_id=str(uuid.uuid4()))
        self.assertEqual(self.request("/v1/advance", fork)["status"], 409)

    def test_02_app_cannot_read_journal_or_signing_key(self):
        self.attack("read", self.journal / "00000000000000000001.json")
        self.attack("read", self.admin / "signing.key")

    def test_03_app_cannot_rewrite(self):
        self.attack("rewrite", self.journal / "00000000000000000001.json")

    def test_04_app_cannot_truncate(self):
        self.attack("truncate", self.journal / "00000000000000000001.json")

    def test_05_app_cannot_delete(self):
        self.attack("delete", self.journal / "00000000000000000001.json")

    def test_06_app_cannot_rename_replace_chmod_or_link(self):
        for operation in ["rename", "replace", "chmod", "link"]:
            with self.subTest(operation=operation):
                self.attack(operation, self.journal / "00000000000000000001.json")

    def test_07_app_cannot_reenroll_at_the_storage_or_command_boundary(self):
        for operation in ["rewrite", "truncate", "delete", "replace"]:
            with self.subTest(operation=operation):
                self.attack(operation, self.admin / "enrollment.json")
        self.attack("create", self.admin / "another-enrollment.json")
        result = subprocess.run(self.enroll_args, preexec_fn=demote(APP), capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        # The same offline admin command also refuses to overwrite enrollment.
        self.assertNotEqual(subprocess.run(self.enroll_args, capture_output=True).returncode, 0)

    def test_08_sink_uid_cannot_change_admin_enrollment(self):
        for operation in ["rewrite", "truncate", "delete", "replace"]:
            with self.subTest(operation=operation):
                self.attack(operation, self.admin / "enrollment.json", SINK)
        self.assertNotEqual(subprocess.run(self.enroll_args, preexec_fn=demote(SINK), capture_output=True).returncode, 0)

    def test_09_dispatch_claim_cannot_be_reassigned(self):
        head = self.observe()
        claim = {"chain_id": self.chain, "operation_id": str(uuid.uuid4()), "attempt_id": str(uuid.uuid4()),
            "process_id": str(uuid.uuid4()), "intent_seq": 1, "intent_hash": "1" * 64, "previous_receipt": digest(head)}
        accepted = self.request("/v1/dispatch", claim)
        self.assertEqual(accepted["status"], 200)
        self.assertEqual(self.request("/v1/dispatch", claim), accepted)
        reclaimed = dict(claim, process_id=str(uuid.uuid4()), previous_receipt=digest(accepted["body"]))
        self.assertEqual(self.request("/v1/dispatch", reclaimed)["status"], 409)

    def test_10_ack_survives_sigkill_and_restart(self):
        before = self.observe()
        self.stop()
        self.start()
        self.assertEqual(self.observe(), before)

    def test_11_fsync_file_publish_directory_order_is_real(self):
        lines = []
        for trace in self.fixture.glob("fsync-*.*"):
            lines.extend(trace.read_text().splitlines())
        lines.sort(key=lambda line: float(line.split()[0]))
        text = "\n".join(lines)
        self.assertIn("00000000000000000001.json", text)
        # For every link publication, a successful record fsync precedes it and
        # a successful directory fsync follows it. No write buffers were traced.
        publications = 0
        for index, line in enumerate(lines):
            if "link(" not in line and "linkat(" not in line:
                continue
            self.assertIn(" = 0", line)
            pending = line.split('"')[1]
            self.assertTrue(any("fsync(" in earlier and pending in earlier and " = 0" in earlier for earlier in lines[:index]))
            self.assertTrue(any("fsync(" in later and f"<{self.journal}>" in later and " = 0" in later for later in lines[index + 1:]))
            publications += 1
        self.assertGreaterEqual(publications, 2)


def digest(receipt):
    """Independent implementation of the documented v1 witness framing."""
    import struct
    out = bytearray()
    def n(value): out.extend(struct.pack(">Q", value))
    def f(value):
        value = value.encode() if isinstance(value, str) else value
        n(len(value)); out.extend(value)
    def u(value): f(uuid.UUID(value).bytes)
    def link(value):
        n(value["key_version"]); n(value["seq"]); f(value["prev_hash"]); f(value["entry_hash"])
    b = receipt["body"]
    f("nyxid.audit-witness.receipt"); n(b["protocol"]); u(b["chain_id"]); n(b["receipt_seq"])
    f(b["previous_receipt"]); n(b["witness_time_ms"]); link(b["head"])
    event = b["event"]
    if event["kind"] == "enrollment":
        n(0); link(event["head"])
    elif event["kind"] == "advance":
        r = event["request"]; n(1); u(r["chain_id"]); u(r["request_id"]); f(r["previous_receipt"]); n(len(r["links"]))
        for value in r["links"]: link(value)
    elif event["kind"] == "dispatch":
        r = event["request"]; n(2); u(r["chain_id"]); u(r["operation_id"]); u(r["attempt_id"]); u(r["process_id"])
        n(r["intent_seq"]); f(r["intent_hash"]); f(r["previous_receipt"])
    else:
        raise AssertionError("unsupported witness event")
    f(receipt["signature"])
    return hashlib.sha256(out).hexdigest()


if __name__ == "__main__":
    unittest.main(verbosity=2)
