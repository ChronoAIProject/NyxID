import contextlib
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

import ci_resources


SCRIPT = Path(ci_resources.__file__).resolve()


class ResourceWrapperTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ci resources ")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.environment = {**os.environ, "RUNNER_TEMP": str(self.root)}

    def invoke(self, source, *args):
        return subprocess.run(
            [sys.executable, str(SCRIPT), "test", sys.executable, "-c", source, *args],
            env=self.environment, capture_output=True, text=True, timeout=20,
        )

    def test_success_and_nonzero_exit_are_preserved(self):
        for code in (0, 7, 101):
            with self.subTest(code=code):
                result = self.invoke(f"raise SystemExit({code})")
                self.assertEqual(result.returncode, code, result.stderr)
                self.assertIn(f'"exit_code": {code}', result.stdout)

    def test_arguments_environment_and_working_directory_are_preserved(self):
        output = self.root / "argument output.json"
        args = ["argument with spaces", "$(false)", "literal;argument", "--flag"]
        result = self.invoke(
            "import json,os,sys; "
            "json.dump([sys.argv[2:], os.getcwd(), os.environ['RUNNER_TEMP']], open(sys.argv[1], 'w'))",
            str(output), *args,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(output.read_text()), [args, os.getcwd(), str(self.root)])
        self.assertNotIn("literal;argument", result.stdout)

    def test_child_signal_is_a_failure(self):
        result = self.invoke("import os,signal; os.kill(os.getpid(), signal.SIGTERM)")
        self.assertEqual(result.returncode, 143, result.stderr)

    def test_unavailable_telemetry_and_artifact_preserve_failure(self):
        blocked = self.root / "not a directory"
        blocked.write_text("blocked")
        with patch.dict(os.environ, {"RUNNER_TEMP": str(blocked)}), \
             patch.object(ci_resources, "snapshot", side_effect=OSError("unavailable")), \
             patch.object(ci_resources, "metadata", return_value={}), \
             contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(ci_resources.run("test", [sys.executable, "-c", "raise SystemExit(23)"]), 23)

    def test_periodic_samples_are_written_while_command_runs(self):
        with patch.dict(os.environ, self.environment, clear=True), \
             patch.object(ci_resources, "metadata", return_value={}), \
             contextlib.redirect_stdout(io.StringIO()):
            code = ci_resources.run("periodic", [sys.executable, "-c", "import time; time.sleep(.12)"], .02)
        self.assertEqual(code, 0)
        events = [json.loads(line) for line in
                  (self.root / "nyxid-ci-resources/periodic.jsonl").read_text().splitlines()]
        self.assertGreaterEqual(sum(event["event"] == "resources" for event in events), 3)

    def test_wrapper_cancellation_reaches_descendants_and_cannot_succeed(self):
        ready = self.root / "ready"
        terminated = self.root / "terminated"
        # Even a child that handles TERM by returning success must leave the gate cancelled.
        source = (
            "import os,signal,sys,time; from pathlib import Path; "
            "pid=os.fork(); "
            "signal.signal(signal.SIGTERM, lambda *_: "
            "(Path(sys.argv[2]).write_text('stopped') if pid == 0 else None, sys.exit(0))); "
            "Path(sys.argv[1]).write_text('ready') if pid == 0 else None; time.sleep(120)"
        )
        child = subprocess.Popen(
            [sys.executable, str(SCRIPT), "cancel", sys.executable, "-c", source,
             str(ready), str(terminated)], env=self.environment, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True,
        )
        try:
            deadline = time.monotonic() + 15
            while not ready.exists() and child.poll() is None and time.monotonic() < deadline:
                time.sleep(.02)
            self.assertTrue(ready.exists(), "wrapped descendant did not start")
            child.send_signal(signal.SIGTERM)
            stdout, stderr = child.communicate(timeout=15)
            self.assertEqual(child.returncode, 143, stderr)
            self.assertTrue(terminated.exists(), stdout)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()

    def test_uncooperative_descendants_are_killed_within_bounded_grace(self):
        ready = self.root / "uncooperative"
        source = (
            "import os,signal,sys,time; from pathlib import Path; "
            "signal.signal(signal.SIGTERM, signal.SIG_IGN); pid=os.fork(); "
            "Path(sys.argv[1]).write_text(str(os.getpid())) if pid == 0 else None; time.sleep(120)"
        )
        wrapper = (
            "import sys; sys.path.insert(0,sys.argv[1]); from ci_resources import run; "
            "sys.exit(run('ignore', [sys.executable, '-c', sys.argv[2], sys.argv[3]], cancellation_grace=.1))"
        )
        child = subprocess.Popen(
            [sys.executable, "-c", wrapper, str(SCRIPT.parent), source, str(ready)],
            env=self.environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        try:
            deadline = time.monotonic() + 15
            while not ready.exists() and child.poll() is None and time.monotonic() < deadline:
                time.sleep(.02)
            self.assertTrue(ready.exists(), "wrapped descendant did not start")
            child.send_signal(signal.SIGTERM)
            _, stderr = child.communicate(timeout=5)
            self.assertEqual(child.returncode, 143, stderr)
            state = subprocess.run(["ps", "-o", "stat=", "-p", ready.read_text()],
                                   capture_output=True, text=True).stdout.strip()
            self.assertTrue(not state or state.startswith("Z"), f"descendant survived: {state}")
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            if ready.exists():
                try:
                    os.kill(int(ready.read_text()), signal.SIGKILL)
                except ProcessLookupError:
                    pass


if __name__ == "__main__":
    unittest.main()
