#!/usr/bin/env python3
"""Require the complete production browser suite through packaged nginx startup."""
import argparse
from contextlib import contextmanager
import hashlib
from http.server import ThreadingHTTPServer
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time
from urllib.error import URLError
from urllib.request import urlopen
import uuid

from image_artifacts import copy, image_container
from source_artifacts import verify_frontend, verify_labels

ROOT = Path(__file__).resolve().parents[1]
CASES = {tuple(item[key] for key in ('project', 'file', 'title')) for item in json.loads(
    (ROOT / 'frontend/e2e-production/required-cases.json').read_text())}


def require_results(path):
    report = json.loads(path.read_text())
    stats = report['stats']
    if report.get('errors') or stats.get('expected') != len(CASES) or any(stats.get(key) != 0 for key in ('unexpected', 'flaky', 'skipped')):
        raise ValueError('all packaged-image browser cases must pass without skips/retries/errors')
    specs = []

    def walk(suite):
        specs.extend(suite.get('specs', []))
        for child in suite.get('suites', []):
            walk(child)

    for suite in report['suites']:
        walk(suite)
    seen = []
    for spec in specs:
        tests = spec['tests']
        if len(tests) != 1 or tests[0].get('status') != 'expected' or tests[0].get('expectedStatus') != 'passed' or \
                len(tests[0]['results']) != 1 or tests[0]['results'][0].get('status') != 'passed' or \
                tests[0]['results'][0].get('retry') != 0 or tests[0]['results'][0].get('errors'):
            raise ValueError('packaged-image browser case failed, retried or was skipped')
        seen.append((tests[0].get('projectName'), spec.get('file'), spec['title']))
    if len(seen) != len(CASES) or set(seen) != CASES:
        raise ValueError('missing, duplicate or unexpected packaged-image browser case/project/file')


@contextmanager
def runtime(image_id):
    name = 'nyxid-packaged-csp-' + uuid.uuid4().hex[:12]
    try:
        # Intentionally inherit the shipped Entrypoint/Cmd. No application,
        # nginx config or renderer is replaced by a fixture or bind mount.
        subprocess.run(['docker', 'run', '--detach', '--name', name, '--network=host', '--read-only',
                        '--cap-drop=ALL', '--security-opt=no-new-privileges', '--tmpfs', '/tmp',
                        '--tmpfs', '/etc/nginx/nyxid/generated:uid=101,gid=101,mode=0700',
                        '--env', 'BACKEND_URL=http://127.0.0.1:18489',
                        '--env', 'NYXID_BROWSER_CONNECT_ORIGINS=https://api.nyxid.example http://127.0.0.1:18489',
                        '--env', 'NYXID_PUBLIC_ORIGIN=http://127.0.0.1:8080', image_id], check=True)
        yield name
    finally:
        subprocess.run(['docker', 'logs', name], check=False)
        subprocess.run(['docker', 'rm', '-f', '-v', name], check=True, stdout=subprocess.DEVNULL)


def probe(base, dist):
    with urlopen(base + '/login', timeout=3) as response:
        policy = response.headers.get_all('Content-Security-Policy', [])
        if len(policy) != 1 or any(value not in policy[0] for value in ("script-src 'self'", "script-src-attr 'none'", "worker-src 'self'")) or \
                response.headers.get('Content-Security-Policy-Report-Only') or \
                'microphone=(self)' not in response.headers.get('Permissions-Policy', ''):
            raise ValueError('packaged nginx must serve enforcing worklet/microphone CSP')
        if response.read() != (dist / 'index.html').read_bytes():
            raise ValueError('nginx served different application bytes')
    worklets = sorted((dist / 'assets').glob('grok-audio-worklet-*.js'))
    if len(worklets) != 1:
        raise ValueError('expected one actual hashed production AudioWorklet')
    with urlopen(base + '/assets/' + worklets[0].name, timeout=3) as response:
        data = response.read()
        if 'javascript' not in response.headers.get('Content-Type', '') or data != worklets[0].read_bytes():
            raise ValueError('nginx did not serve the exact packaged AudioWorklet bytes')
    print(f'Packaged worklet served: {worklets[0].name}, sha256={hashlib.sha256(data).hexdigest()}', flush=True)


@contextmanager
def nginx_access_log(name, path):
    # The shipped access log goes to stdout. Stream the actual container log
    # into browser evidence; do not replace nginx's config or log destination.
    with path.open('wb') as output:
        process = subprocess.Popen(['docker', 'logs', '--follow', name], stdout=output, stderr=subprocess.STDOUT)
        try:
            yield
            if process.poll() is not None:
                raise ValueError('packaged nginx log stream ended during browser acceptance')
        finally:
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=5)


def transport_module(source, filename='production-policy-server.py'):
    # The source export is immutable evidence. Importing its fixture must not
    # add __pycache__ (including on a failed import) to the verified inventory.
    previous = sys.dont_write_bytecode
    try:
        sys.dont_write_bytecode = True
        spec = importlib.util.spec_from_file_location('packaged_transport', source / 'frontend/scripts' / filename)
        fixture = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(fixture)
        return fixture
    finally:
        sys.dont_write_bytecode = previous


def verify_offline_fixtures(source):
    pins = transport_module(source, 'prepare-production-policy-fixtures.py').FIXTURES
    directory = source / 'frontend/e2e-production/fixtures'
    receipts = json.loads((directory / 'sources.json').read_text())
    if len(receipts) != len(pins) or {item['file'] for item in receipts} != set(pins):
        raise ValueError('incomplete or duplicate offline policy fixtures')
    for item in receipts:
        data = (directory / item['file']).read_bytes()
        origin, digest = pins[item['file']]
        if item['source'] != origin or item['sha256'] != digest or item['size'] != len(data) or hashlib.sha256(data).hexdigest() != digest:
            raise ValueError('offline policy fixture differs from its reviewed pin: ' + item['file'])


def run(image, browser_image, source, output, arch, commit, epoch):
    output.mkdir(parents=True, exist_ok=False)
    verify_offline_fixtures(source)
    with image_container(image) as (container, info):
        verify_labels(info, arch, commit, epoch)
        dist = output / 'image-dist'
        copy(container, '/usr/share/nginx/html/.', dist)
        verify_frontend(dist, source, commit, epoch)
        image_id = info['Id']
    fixture = transport_module(source)
    server = ThreadingHTTPServer(('127.0.0.1', 18489), fixture.transport(dist))
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        with runtime(image_id) as name:
            deadline = time.monotonic() + 60
            while True:
                try:
                    probe('http://127.0.0.1:8080', dist)
                    break
                except (URLError, TimeoutError):
                    if time.monotonic() >= deadline:
                        raise ValueError('packaged frontend did not become ready')
                    time.sleep(.5)
            actual = json.loads(subprocess.check_output(['docker', 'inspect', name]))[0]
            if actual['Image'] != image_id or any(actual['Config'].get(key) != info['Config'].get(key) for key in ('Entrypoint', 'Cmd')) or \
                    any(mount['Type'] != 'tmpfs' for mount in actual.get('Mounts', [])):
                raise ValueError('runtime must use the exact shipped image/startup with only tmpfs mounts')
            browser_id = json.loads(subprocess.check_output(['docker', 'image', 'inspect', browser_image]))[0]['Id']
            print(f'Browser testing frontend image {image_id} with browser image {browser_id}; SOURCE_COMMIT={commit}', flush=True)
            results = output / 'browser'
            results.mkdir()
            with nginx_access_log(name, results / 'nginx.log'):
                subprocess.run(['docker', 'run', '--rm', '--network=host', '--shm-size=256m', '--cap-drop=ALL',
                                '--security-opt=no-new-privileges', '--user', f'{os.getuid()}:{os.getgid()}',
                                '--env', 'HOME=/tmp', '--env', 'CI=true',
                                '--env', 'NYXID_PRODUCTION_TEST_URL=http://127.0.0.1:8080',
                                '--env', 'NYXID_PRODUCTION_DIST=/image-dist',
                                '--env', 'NYXID_PRODUCTION_RESULTS=/results/playwright.json',
                                '--env', 'NYXID_NGINX_ACCESS_LOG=/results/nginx.log',
                                '--env', 'NYXID_TEST_FONT=/tests/frontend/e2e-production/fixtures/jetbrainsmono-v24.ttf',
                                '--mount', f'type=bind,source={dist},target=/image-dist,readonly',
                                '--mount', f'type=bind,source={results},target=/results', browser_id], check=True)
            require_results(results / 'playwright.json')
            print(f'Built Image Production CSP: {len(CASES)} passed, 0 failed/skipped/flaky/retried', flush=True)
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True)
    parser.add_argument('--browser-image', required=True)
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--arch', choices=('amd64', 'arm64'), required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--epoch', type=int, required=True)
    args = parser.parse_args()
    run(args.image, args.browser_image, args.source_root.resolve(), args.output.resolve(), args.arch, args.commit, args.epoch)


if __name__ == '__main__':
    main()
