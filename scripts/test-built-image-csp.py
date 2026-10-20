#!/usr/bin/env python3
"""Negative controls for actual-image startup, served bytes and browser acceptance."""
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('built_image_csp', Path(__file__).with_name('check-built-image-csp.py'))
csp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(csp)


class BrowserAcceptance(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.report = Path(self.temporary.name) / 'results.json'
        self.data = dict(errors=[], stats=dict(expected=2, unexpected=0, flaky=0, skipped=0), suites=[dict(suites=[dict(specs=[
            dict(title=name, tests=[dict(status='expected', expectedStatus='passed', results=[dict(status='passed')])])
            for name in sorted(csp.CASES)])])])

    def check(self):
        self.report.write_text(json.dumps(self.data))
        csp.require_results(self.report)

    def test_both_original_cases_pass_once(self):
        self.check()

    def test_skipped_failed_flaky_or_report_errors_reject(self):
        original = copy.deepcopy(self.data)
        for key in ('skipped', 'unexpected', 'flaky'):
            self.data = copy.deepcopy(original)
            self.data['stats'][key] = 1
            with self.subTest(key=key), self.assertRaises(ValueError): self.check()
        self.data = original
        self.data['errors'] = [dict(message='browser setup failed')]
        with self.assertRaises(ValueError): self.check()

    def test_missing_or_replaced_case_rejects_even_with_success_stats(self):
        for mutation in ('missing', 'replaced', 'duplicate'):
            original = copy.deepcopy(self.data)
            specs = self.data['suites'][0]['suites'][0]['specs']
            if mutation == 'missing': specs.pop()
            elif mutation == 'replaced': specs[0]['title'] = 'different test'
            else: specs[1]['title'] = specs[0]['title']
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): self.check()
            self.data = original

    def test_case_level_skip_failure_expected_failure_or_retry_rejects(self):
        for mutation in ('skipped', 'failed', 'expected-failure', 'retry'):
            original = copy.deepcopy(self.data)
            test = self.data['suites'][0]['suites'][0]['specs'][0]['tests'][0]
            if mutation == 'expected-failure': test['expectedStatus'] = 'failed'
            elif mutation == 'retry': test['results'].insert(0, dict(status='failed'))
            else: test['results'][0]['status'] = mutation
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): self.check()
            self.data = original

    def test_missing_report_fails(self):
        with self.assertRaises(OSError): csp.require_results(self.report)


class RuntimeStartup(unittest.TestCase):
    def test_default_entrypoint_no_code_mount_and_cleanup_on_browser_failure(self):
        calls = []

        def run(command, **kwargs):
            calls.append(command)
            return subprocess.CompletedProcess(command, 0)

        with patch.object(csp.subprocess, 'run', side_effect=run):
            with self.assertRaisesRegex(ValueError, 'browser failed'):
                with csp.runtime('sha256:' + 'a' * 64):
                    raise ValueError('browser failed')
        self.assertEqual(calls[0][-1], 'sha256:' + 'a' * 64)
        self.assertNotIn('--entrypoint', calls[0])
        self.assertNotIn('--mount', calls[0])
        self.assertNotIn('--volume', calls[0])
        self.assertIn('--read-only', calls[0])
        self.assertEqual(calls[-1][:5], ['docker', 'rm', '-f', '-v', calls[0][calls[0].index('--name') + 1]])


class ServedArtifact(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.dist = Path(temporary.name)
        (self.dist / 'assets').mkdir()
        (self.dist / 'index.html').write_bytes(b'<html>actual packaged application</html>')
        self.worklet = self.dist / 'assets/grok-audio-worklet-fixture.js'
        self.worklet.write_bytes(b'registerProcessor("nyx-grok-audio", class {})')
        self.bad = None
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_): pass

            def do_GET(self):
                is_html = self.path == '/login'
                data = (fixture.dist / 'index.html' if is_html else fixture.worklet).read_bytes()
                if fixture.bad == ('html' if is_html else 'worklet'): data += b'altered'
                self.send_response(200)
                self.send_header('Content-Type', 'text/html' if is_html else 'text/javascript')
                if is_html:
                    policy = "default-src 'none'; script-src 'self'; script-src-attr 'none'; worker-src 'self'"
                    if fixture.bad == 'unsafe-policy': policy = "default-src * 'unsafe-inline'"
                    name = 'Content-Security-Policy-Report-Only' if fixture.bad == 'report-only' else 'Content-Security-Policy'
                    self.send_header(name, policy)
                    if fixture.bad != 'microphone': self.send_header('Permissions-Policy', 'microphone=(self)')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.url = 'http://127.0.0.1:' + str(self.server.server_port)

    def test_actual_html_and_hashed_worklet_bytes(self):
        csp.probe(self.url, self.dist)

    def test_altered_served_html_or_worklet_reject(self):
        for self.bad in ('html', 'worklet'):
            with self.subTest(which=self.bad), self.assertRaises(ValueError): csp.probe(self.url, self.dist)

    def test_report_only_unsafe_policy_or_missing_microphone_reject(self):
        for self.bad in ('report-only', 'unsafe-policy', 'microphone'):
            with self.subTest(which=self.bad), self.assertRaisesRegex(ValueError, 'enforcing'): csp.probe(self.url, self.dist)

    def test_missing_or_multiple_worklets_reject(self):
        self.worklet.unlink()
        with self.assertRaisesRegex(ValueError, 'one actual'): csp.probe(self.url, self.dist)
        self.worklet.write_bytes(b'worklet')
        (self.worklet.parent / 'grok-audio-worklet-extra.js').write_bytes(b'extra')
        with self.assertRaisesRegex(ValueError, 'one actual'): csp.probe(self.url, self.dist)


class BrowserInputs(unittest.TestCase):
    def test_digest_and_locked_playwright_versions_match_and_are_maintained(self):
        root = Path(__file__).resolve().parents[1]
        text = (root / 'frontend/Dockerfile.policy-tests').read_text()
        import re
        version, digest = re.search(r'playwright:v([0-9.]+)-noble@(sha256:[0-9a-f]{64})', text).groups()
        lock = json.loads((root / 'frontend/package-lock.json').read_text())
        self.assertEqual(version, lock['packages']['node_modules/playwright']['version'])
        pins = json.loads((root / 'security/build-input-pins.json').read_text())
        record = next(item for item in pins['images'] if item['image'] == 'mcr.microsoft.com/playwright:v' + version + '-noble')
        self.assertEqual(record['digest'], digest)
        self.assertEqual({item['platform']['architecture'] for item in record['platforms']}, {'amd64', 'arm64'})
        self.assertIn('npm ci --ignore-scripts', text)
        for title in csp.CASES: self.assertIn(title, text)
        self.assertIn('directory: /frontend', (root / '.github/dependabot.yml').read_text())


if __name__ == '__main__':
    unittest.main()
