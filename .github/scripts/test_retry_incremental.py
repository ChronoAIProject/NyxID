import unittest
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time

from retry_incremental import should_retry

SCRIPT = Path(__file__).with_name('retry_incremental.py').resolve()


class RetryIncrementalTests(unittest.TestCase):
    def test_internal_compiler_error_retries_case_insensitively(self):
        self.assertTrue(should_retry("error: Internal Compiler Error: stale query"))

    def test_incremental_markers_retry(self):
        for output in (
            "error: incremental compilation failure",
            "failed to load dep graph: metadata is corrupt",
            "could not read dep-graph file",
        ):
            with self.subTest(output=output):
                self.assertTrue(should_retry(output))

    def test_ordinary_compile_error_does_not_retry(self):
        self.assertFalse(should_retry("error[E0308]: mismatched types"))

    def test_test_failure_does_not_retry(self):
        self.assertFalse(should_retry("test foo ... FAILED\nthread panicked at assertion"))

    def test_memory_markers_and_kill_statuses_retry(self):
        for code in (137, 143, -signal.SIGKILL, -signal.SIGTERM):
            self.assertTrue(should_retry('', code))
        for text in ('fatal: out of memory', 'memory allocation of 123 bytes failed',
                     'Cannot allocate memory', 'process failed (signal: 9, SIGKILL: kill)',
                     'process failed (signal: 15, SIGTERM: termination)'):
            self.assertTrue(should_retry(text, 101))

    def test_real_child_retries_once_without_graph_or_incremental(self):
        for failure in ('137', '143', 'sigkill', 'sigterm', 'oom', 'stale', 'ordinary', 'twice'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                for name in ('debug', 'llvm-cov-target/debug'):
                    (root / 'target' / name / 'incremental').mkdir(parents=True)
                child = root / 'child.py'
                child.write_text('''import json, os, pathlib, signal, sys
p = pathlib.Path('attempts.json')
attempts = json.loads(p.read_text()) if p.exists() else []
attempts.append({'incremental': os.environ['CARGO_INCREMENTAL'],
                 'graphs': [str(p) for p in pathlib.Path('target').rglob('incremental')]})
p.write_text(json.dumps(attempts))
mode = sys.argv[1]
if len(attempts) == 2:
    sys.exit(137 if mode == 'twice' else 0)
if mode.startswith('sig'):
    os.kill(os.getpid(), signal.SIGKILL if mode == 'sigkill' else signal.SIGTERM)
print({'oom': 'out of memory', 'stale': 'failed to load dep graph',
       'ordinary': 'error[E0308]: mismatched types'}.get(mode, ''), flush=True)
sys.exit(int(mode) if mode.isdigit() else (137 if mode == 'twice' else 101))
''')
                result = subprocess.run([sys.executable, str(SCRIPT), '--log-file', 'build.log', '--',
                                         sys.executable, str(child), failure], cwd=root,
                                        env={**os.environ, 'CARGO_INCREMENTAL': '1',
                                             'GITHUB_ENV': str(root / 'environment')},
                                        capture_output=True, text=True, timeout=30)
                attempts = json.loads((root / 'attempts.json').read_text())
                if failure == 'ordinary':
                    self.assertEqual(result.returncode, 101, result.stdout + result.stderr)
                    self.assertEqual(len(attempts), 1)
                    self.assertFalse((root / 'environment').exists())
                else:
                    self.assertEqual(result.returncode, 137 if failure == 'twice' else 0,
                                     result.stdout + result.stderr)
                    self.assertEqual(attempts, [{'incremental': '1', 'graphs': attempts[0]['graphs']},
                                                {'incremental': '0', 'graphs': []}])
                    self.assertEqual(len(attempts[0]['graphs']), 2)
                    self.assertEqual((root / 'environment').read_text(), 'CARGO_INCREMENTAL=0\n')

    def test_supervisor_cancellation_does_not_retry(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            child = root / 'child.py'
            child.write_text('import pathlib, signal\n'
                             'with pathlib.Path("attempts").open("a") as f: f.write("attempt\\n")\n'
                             'signal.pause()\n')
            with (root / 'output').open('w') as output:
                process = subprocess.Popen([sys.executable, str(SCRIPT), '--log-file', 'build.log', '--',
                                            sys.executable, str(child)], cwd=root, start_new_session=True,
                                           stdout=output, stderr=subprocess.STDOUT)
                try:
                    deadline = time.monotonic() + 10
                    while not (root / 'attempts').exists() and time.monotonic() < deadline:
                        time.sleep(.02)
                    self.assertTrue((root / 'attempts').exists())
                    os.killpg(process.pid, signal.SIGTERM)
                    self.assertEqual(process.wait(timeout=10), -signal.SIGTERM)
                    self.assertEqual((root / 'attempts').read_text(), 'attempt\n')
                finally:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    process.wait(timeout=10)


if __name__ == "__main__":
    unittest.main()
