#!/usr/bin/env python3
"""Negative controls for actual-image startup, served bytes and browser acceptance."""
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
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
        self.data = dict(errors=[], stats=dict(expected=len(csp.CASES), unexpected=0, flaky=0, skipped=0), suites=[dict(suites=[dict(specs=[
            dict(title=title, file=file, tests=[dict(projectName=project, status='expected', expectedStatus='passed',
                 results=[dict(status='passed', retry=0, errors=[])])])
            for project, file, title in sorted(csp.CASES)])])])

    def check(self):
        self.report.write_text(json.dumps(self.data))
        csp.require_results(self.report)

    def test_complete_suite_passes_once(self):
        self.assertEqual(len(csp.CASES), 33)
        self.assertEqual({item[0] for item in csp.CASES}, {'production-policy', 'production-oauth-meta', 'production-telegram-privacy'})
        self.assertTrue({
            'real microphone, shipped AudioWorklet, fonts and allowed websocket',
            'rejects injected inline scripts, event handlers and unlisted connections',
        } <= {item[2] for item in csp.CASES})
        self.check()

    def test_old_two_case_report_rejects(self):
        self.data['stats']['expected'] = 2
        self.data['suites'][0]['suites'][0]['specs'] = self.data['suites'][0]['suites'][0]['specs'][:2]
        with self.assertRaises(ValueError): self.check()

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
        for mutation in ('missing', 'replaced', 'duplicate', 'project', 'file'):
            original = copy.deepcopy(self.data)
            specs = self.data['suites'][0]['suites'][0]['specs']
            if mutation == 'missing': specs.pop()
            elif mutation == 'replaced': specs[0]['title'] = 'different test'
            elif mutation == 'duplicate': specs[1] = copy.deepcopy(specs[0])
            elif mutation == 'project': specs[0]['tests'][0]['projectName'] = 'different-project'
            else: specs[0]['file'] = 'different.spec.ts'
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): self.check()
            self.data = original

    def test_case_level_skip_failure_expected_failure_or_retry_rejects(self):
        for mutation in ('skipped', 'failed', 'expected-failure', 'retry', 'retry-number', 'errors', 'duplicate-test'):
            original = copy.deepcopy(self.data)
            test = self.data['suites'][0]['suites'][0]['specs'][0]['tests'][0]
            if mutation == 'expected-failure': test['expectedStatus'] = 'failed'
            elif mutation == 'retry': test['results'].insert(0, dict(status='failed'))
            elif mutation == 'retry-number': test['results'][0]['retry'] = 1
            elif mutation == 'errors': test['results'][0]['errors'] = [dict(message='fixture teardown failed')]
            elif mutation == 'duplicate-test': self.data['suites'][0]['suites'][0]['specs'][0]['tests'].append(copy.deepcopy(test))
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
        self.assertIn('NYXID_BROWSER_CONNECT_ORIGINS=https://api.nyxid.example http://127.0.0.1:18489', calls[0])
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
    def test_log_stream_uses_actual_container_and_cleans_up_on_browser_failure(self):
        from unittest.mock import Mock
        process = Mock()
        process.poll.return_value = None
        with tempfile.TemporaryDirectory() as temporary, patch.object(csp.subprocess, 'Popen', return_value=process) as start:
            path = Path(temporary) / 'nginx.log'
            with self.assertRaisesRegex(ValueError, 'browser failed'):
                with csp.nginx_access_log('actual-image-container', path):
                    command = start.call_args.args[0]
                    self.assertEqual(command, ['docker', 'logs', '--follow', 'actual-image-container'])
                    stream = start.call_args.kwargs['stdout']
                    stream.write(b'"GET /channel-bots HTTP/1.1"\n')
                    stream.flush()
                    self.assertIn('GET /channel-bots', path.read_text())
                    raise ValueError('browser failed')
            process.terminate.assert_called_once()
            process.wait.assert_called_once()

    def test_premature_log_stream_exit_rejects(self):
        from unittest.mock import Mock
        process = Mock()
        process.poll.return_value = 1
        with tempfile.TemporaryDirectory() as temporary, patch.object(csp.subprocess, 'Popen', return_value=process):
            with self.assertRaisesRegex(ValueError, 'log stream ended'):
                with csp.nginx_access_log('actual-image-container', Path(temporary) / 'nginx.log'): pass
            process.terminate.assert_not_called()

    def test_offline_fixture_missing_or_modified_rejects(self):
        import shutil
        root = Path(__file__).resolve().parents[1]
        csp.verify_offline_fixtures(root)
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary)
            scripts = source / 'frontend/scripts'
            scripts.mkdir(parents=True)
            shutil.copyfile(root / 'frontend/scripts/prepare-production-policy-fixtures.py', scripts / 'prepare-production-policy-fixtures.py')
            fixtures = source / 'frontend/e2e-production/fixtures'
            shutil.copytree(root / 'frontend/e2e-production/fixtures', fixtures)
            target = fixtures / 'telegram-widget-v22.js'
            target.write_bytes(target.read_bytes() + b'altered')
            with self.assertRaisesRegex(ValueError, 'reviewed pin'): csp.verify_offline_fixtures(source)
            target.unlink()
            with self.assertRaises(OSError): csp.verify_offline_fixtures(source)

    def test_fixture_import_preserves_exact_export_and_bytecode_setting(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary)
            fixture = source / 'frontend/scripts/production-policy-server.py'
            fixture.parent.mkdir(parents=True)
            for body in ('VALUE = 42\n', 'raise ValueError("fixture import failed")\n'):
                fixture.write_text(body)
                before = {path.relative_to(source): path.read_bytes() for path in source.rglob('*') if path.is_file()}
                with patch.object(sys, 'dont_write_bytecode', False):
                    if body.startswith('raise'):
                        with self.assertRaisesRegex(ValueError, 'import failed'): csp.transport_module(source)
                    else:
                        self.assertEqual(csp.transport_module(source).VALUE, 42)
                    self.assertFalse(sys.dont_write_bytecode)
                after = {path.relative_to(source): path.read_bytes() for path in source.rglob('*') if path.is_file()}
                self.assertEqual(before, after)

    def test_isolated_browser_copies_complete_runtime_import_closure(self):
        import re
        root = Path(__file__).resolve().parents[1]
        text = (root / 'frontend/Dockerfile.policy-tests').read_text()
        available = set()
        for line in text.splitlines():
            if line.startswith('COPY frontend/'):
                for source in line.split()[1:-1]:
                    path = root / source
                    available.update(path.rglob('*') if path.is_dir() else [path])
        pending = [root / 'frontend' / file for _, file, _ in csp.CASES]
        visited = set()
        while pending:
            path = pending.pop().resolve()
            if path in visited: continue
            visited.add(path)
            self.assertIn(path, available, f'isolated browser runner omits {path.relative_to(root)}')
            # Type-only imports are erased by Playwright's TS transform.
            for relative in re.findall(r'''(?m)^import\s+(?!type\b)(?:[^;]*?\sfrom\s+)?["'](\.[^"']+)["']''', path.read_text()):
                target = path.parent / relative
                resolved = next((candidate.resolve() for candidate in (target, target.with_suffix('.ts'), target.with_suffix('.mjs'))
                                 if candidate.is_file()), None)
                self.assertIsNotNone(resolved, f'unresolved runtime import {relative} from {path}')
                pending.append(resolved)

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
        entrypoint = json.loads(next(line.removeprefix('ENTRYPOINT ') for line in text.splitlines() if line.startswith('ENTRYPOINT ')))
        self.assertEqual(entrypoint, ['node', 'node_modules/@playwright/test/cli.js', 'test',
                                     '--config=playwright.production.config.ts', '--output=/results/artifacts'])
        self.assertIn('directory: /frontend', (root / '.github/dependabot.yml').read_text())


if __name__ == '__main__':
    unittest.main()
