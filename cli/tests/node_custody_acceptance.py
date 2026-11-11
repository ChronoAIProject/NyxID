#!/usr/bin/env python3
"""Disposable native Linux custody acceptance; never use an owner's machine.

No accounts, host policies, keyring services, or installed definitions are changed.
The fixture owns a private D-Bus, GNOME Secret Service, HOME and public sentinels.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--disposable", action="store_true", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--identities", required=True, help="Three existing disposable worker users, comma separated")
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    if os.geteuid() != 0 or os.uname().sysname != "Linux":
        parser.error("requires root in a disposable Linux VM")
    args.evidence.mkdir(parents=True, exist_ok=True)
    passed = 0
    connections = []

    class Trap(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            connections.append(True)
            self.send_response(500)
            self.end_headers()

        do_POST = do_GET

        def log_message(self, *_):
            pass

    trap = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Trap)
    threading.Thread(target=trap.serve_forever, daemon=True).start()
    # Dropped identities must be able to execute the helper. Evidence may live
    # below a private HOME; do not mistake that traversal denial for custody.
    with tempfile.TemporaryDirectory(prefix="w95-custody-", dir="/var/tmp") as fixture:
        root = Path(fixture)
        root.chmod(0o755)
        binary = root / "nyxid"
        test_binary = root / "nyxid-tests"
        for source, target in [(args.binary, binary), (args.test_binary, test_binary)]:
            shutil.copyfile(source, target)
            target.chmod(0o755)
        home = root / "home"
        home.mkdir(mode=0o700)
        private = root / "private"
        private.mkdir(mode=0o700)
        env = dict(os.environ, HOME=str(home), XDG_DATA_HOME=str(home / "data"),
                   XDG_CONFIG_HOME=str(home / "config"), XDG_RUNTIME_DIR=str(private),
                   NYXID_SHARE_ANALYTICS="true",
                   NYXID_TELEMETRY_DSN=f"http://public@127.0.0.1:{trap.server_port}/1")
        control = root / "public-control"
        control.write_bytes(b"public custody control")
        control.chmod(0o644)

        def probe(request):
            return subprocess.run([str(binary), "node", "custody-probe"],
                                  input=json.dumps(request).encode(), env=env,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                  timeout=5, check=False)

        for request, code in [({"paths": [str(control)], "keychain": None}, 41),
                              ({"paths": [str(root / "absent")], "keychain": None}, 42),
                              ({"paths": [], "keychain": None}, 42)]:
            result = probe(request)
            assert result.returncode == code and result.stdout == b"" and result.stderr == b"", "fixed helper outcome/output"
            passed += 1
        assert not list(home.rglob("*")), "helper wrote outside its read-only probe targets"
        assert not connections, "helper contacted optional telemetry/update network hooks"
        passed += 2
        bus_pid = None
        keyring = None
        log = (args.evidence / "native-custody.log").open("wb")
        try:
            bus = subprocess.run(["dbus-daemon", "--session", "--fork",
                                  f"--address=unix:path={private / 'bus'}",
                                  "--print-address=1", "--print-pid=1"], env=env,
                                 stdout=subprocess.PIPE, stderr=log, timeout=5, check=True)
            address, pid = bus.stdout.decode().strip().splitlines()
            bus_pid = int(pid)
            env["DBUS_SESSION_BUS_ADDRESS"] = address
            keyring = subprocess.Popen(["gnome-keyring-daemon", "--foreground", "--unlock",
                                        "--components=secrets", f"--control-directory={private / 'keyring'}"],
                                       env=env, stdin=subprocess.PIPE, stdout=log, stderr=log)
            keyring.stdin.write(b"public disposable fixture passphrase\n")
            keyring.stdin.close()
            # Start readiness is independent of each five-second access probe.
            deadline = time.monotonic() + 5
            while not (private / "keyring").exists() and time.monotonic() < deadline:
                assert keyring.poll() is None, "disposable Secret Service exited"
                time.sleep(0.025)
            target = root / "native"
            target.mkdir(mode=0o755)
            env.update(NYXID_TEST_CUSTODY_DIRECTORY=str(target),
                       NYXID_TEST_CUSTODY_IDENTITIES=args.identities)

            def rust_test(name, bound):
                result = subprocess.run([str(test_binary), "--exact", name, "--nocapture"],
                                        env=env, stdout=subprocess.PIPE, stderr=log,
                                        timeout=bound, check=False)
                log.write(result.stdout)
                log.flush()
                assert result.returncode == 0, f"native test failed: {name}"
                assert b"test result: ok. 1 passed; 0 failed; 0 ignored;" in result.stdout, "exact native test must execute once"

            rust_test("node::custody::tests::native_linux_custody", 120)
            passed += 1
            for test in ["node::machine::authority_runtime_tests::shared_legacy_exec_runs_with_unknown_custody_and_a_false_claim",
                         "node::machine::authority_runtime_tests::separated_context_refuses_unknown_custody_before_any_job_effect"]:
                rust_test(test, 30)
                passed += 1
        finally:
            if keyring is not None:
                keyring.terminate()
                try:
                    keyring.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    keyring.kill()
                    keyring.wait(timeout=5)
            if bus_pid is not None:
                try:
                    os.kill(bus_pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            log.close()
    trap.shutdown()
    trap.server_close()
    print(f"native custody acceptance: {passed} passed, 0 failed, 0 ignored")


if __name__ == "__main__":
    main()
