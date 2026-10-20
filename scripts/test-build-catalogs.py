#!/usr/bin/env python3
"""Portable freshness, HTTPS candidate and scheduled publication boundaries."""
from datetime import datetime, timezone
import hashlib
import json
import importlib.util
import os
import re
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import urllib.request

from apk_archive_release import stage_payload
from apk_inputs import ARCHES
from build_catalogs import (CA_URL, CA_CHECKSUM_URL, MAX_CA_SOURCE_AGE_DAYS, ROOT,
                            SnapshotRedirect, ca_provenance, fetch, fetch_ca,
                            freshness, refresh_apt, refresh_ca)
from resolve_apk_requests import REQUESTS

spec = importlib.util.spec_from_file_location('catalog_publisher', ROOT / 'scripts/publish-build-input-candidates.py')
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)
NOW = datetime(2026, 10, 20, 12, tzinfo=timezone.utc)
CA_DATA = re.sub(rb'^## Certificate data from Mozilla as of: .+ GMT$',
                 b'## Certificate data from Mozilla as of: Fri Sep 25 03:12:01 2026 GMT',
                 (ROOT / 'security/apt-transport-ca.pem').read_bytes(), flags=re.M)


def digest(data):
    return hashlib.sha256(data).hexdigest()


class Candidates(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.security = self.root / 'security'
        self.catalogs = self.security / 'apk'
        self.catalogs.mkdir(parents=True)
        self.apt = self.security / 'apt-snapshots.tsv'
        self.pins = self.security / 'apt-transport-ca.tsv'
        self.bundle = self.security / 'apt-transport-ca.pem'
        for name in ('apt-snapshots.tsv', 'apt-transport-ca.tsv', 'apt-transport-ca.pem'):
            shutil.copyfile(ROOT / 'security' / name, self.security / name)
        # Fixed dates: tests must not expire when the real catalogs are refreshed.
        apt = self.apt.read_text()
        self.apt.write_text(re.sub(r'20[0-9]{6}T[0-9]{6}Z', '20261019T000000Z', apt))
        source = '2026-09-25T03:12:01Z'
        self.bundle.write_bytes(CA_DATA)
        self.pins.write_text('# nyxid.apt-transport-ca.v1\n' + '\t'.join((source, '2026-10-20', CA_URL, CA_CHECKSUM_URL, digest(CA_DATA))) + '\n')
        archives = self.root / 'archives'
        archives.mkdir()
        (archives / 'fixture-1-r0.apk').write_bytes(b'signed archive fixture; native replay verifies signatures')
        checksum = digest((archives / 'fixture-1-r0.apk').read_bytes())
        for label in REQUESTS:
            for arch, native in ARCHES.items():
                catalog = self.catalogs / (label + '-' + arch + '.tsv')
                catalog.write_text(f'''# nyxid.apk-inputs.v2
# base-image alpine:3.22@sha256:{'a' * 64}
# arch {native}
# refreshed-on 2026-10-20
# packages fixture
# before-sha256 {'b' * 64}
# after-sha256 {'c' * 64}
fixture\t1-r0\t{native}\t{checksum}\thttps://github.com/ChronoAIProject/NyxID/releases/download/build-inputs-apk-2026-10-20/{checksum}-{native}-fixture-1-r0.apk\thttps://dl-cdn.alpinelinux.org/alpine/v3.22/main/{native}/fixture-1-r0.apk
''')
                stage_payload(catalog, arch, archives, self.root / 'payload')
        shutil.rmtree(archives)

    def check(self, now=NOW):
        return freshness(self.apt, self.catalogs, self.pins, self.bundle, now)

    def test_fresh_complete_inputs(self):
        self.assertEqual(self.check(), [])

    def test_stale_apt_security_snapshots_fail(self):
        self.apt.write_text(self.apt.read_text().replace('20261019', '20261001'))
        self.assertEqual(sum('stale input' in error for error in self.check()), 4)

    def test_stale_apk_catalogs_fail(self):
        for path in self.catalogs.iterdir():
            path.write_text(path.read_text().replace('2026-10-20', '2026-10-01'))
        self.assertTrue(any('APK catalogs: stale' in error for error in self.check()))

    def test_stale_ca_check_fails_even_with_current_distro_catalogs(self):
        self.pins.write_text(self.pins.read_text().replace('\t2026-10-20\t', '\t2026-10-01\t'))
        self.assertTrue(any('CA upstream check: stale' in error for error in self.check()))

    def test_old_ca_source_fails_despite_a_recent_check(self):
        # Move time forward while keeping all check/catalog dates fresh.
        future = datetime(2027, 1, 1, tzinfo=timezone.utc)
        self.apt.write_text(self.apt.read_text().replace('20261019', '20261231'))
        for path in self.catalogs.iterdir():
            path.write_text(path.read_text().replace('2026-10-20', '2027-01-01'))
        self.pins.write_text(self.pins.read_text().replace('\t2026-10-20\t', '\t2027-01-01\t'))
        self.assertEqual(self.check(future), [f'APT transport CA: stale upstream bundle exceeds {MAX_CA_SOURCE_AGE_DAYS} days'])

    def test_future_catalog_dates_fail(self):
        self.assertTrue(any('future' in error for error in self.check(datetime(2026, 10, 18, tzinfo=timezone.utc))))

    def test_missing_or_inconsistent_catalogs_fail(self):
        next(self.catalogs.iterdir()).unlink()
        with self.assertRaises((OSError, ValueError)):
            self.check()

    def test_manifest_tamper_missing_duplicate_origin_and_hash_fail(self):
        original = self.pins.read_text()
        for changed in (original.replace('https://curl.se/', 'http://curl.se/'), original + original.splitlines()[1] + '\n',
                        original.replace(digest(CA_DATA), '0' * 64), original.replace('\t2026-10-20\t', '\t2026-13-20\t')):
            with self.subTest(changed=changed[:60]):
                self.pins.write_text(changed)
                with self.assertRaises(ValueError): ca_provenance(self.pins, self.bundle)

    def test_ca_bundle_tamper_and_source_mismatch_fail(self):
        self.bundle.write_bytes(CA_DATA + b'poison')
        with self.assertRaisesRegex(ValueError, 'differs from pins'): ca_provenance(self.pins, self.bundle)
        self.bundle.write_bytes(CA_DATA)
        self.pins.write_text(self.pins.read_text().replace('T03:12:01Z', 'T03:12:02Z'))
        with self.assertRaisesRegex(ValueError, 'timestamp'): ca_provenance(self.pins, self.bundle)

    def test_ca_symlink_rejected(self):
        self.bundle.unlink()
        self.bundle.symlink_to(ROOT / 'security/apt-transport-ca.pem')
        with self.assertRaisesRegex(ValueError, 'regular files'): ca_provenance(self.pins, self.bundle)

    def test_apt_refresh_rewrites_all_twelve_hashes_and_https_urls(self):
        urls = []
        def download(url):
            urls.append(url)
            return ('signed fixture ' + url).encode()
        output = self.root / 'candidate.tsv'
        text = refresh_apt(output, '20261019T000000Z', download)
        self.assertEqual(len(urls), 12)
        self.assertTrue(all(url.startswith('https://snapshot.') and '20261019T000000Z/dists/' in url for url in urls))
        for row in text.splitlines()[1:]:
            distro, code, suite, components, url, expected = row.split('\t')
            self.assertEqual(expected, digest(('signed fixture ' + url + '/dists/' + suite + '/InRelease').encode()))

    def test_invalid_timestamp_or_failed_fetch_never_writes_candidate(self):
        output = self.root / 'candidate.tsv'
        with self.assertRaises(ValueError): refresh_apt(output, '2026-10-19')
        with self.assertRaises(RuntimeError): refresh_apt(output, '20261019T000000Z', lambda url: (_ for _ in ()).throw(RuntimeError('offline')))
        self.assertFalse(output.exists())

    def ca_download(self, url):
        return CA_DATA if url == CA_URL else (digest(CA_DATA) + '  cacert.pem\n').encode()

    def test_ca_refresh_binds_official_checksum_and_provenance(self):
        output = self.root / 'ca-candidate'
        refresh_ca(output, '2026-10-20', self.ca_download)
        self.assertEqual((output / 'apt-transport-ca.pem').read_bytes(), CA_DATA)
        self.assertEqual(ca_provenance(output / 'apt-transport-ca.tsv', output / 'apt-transport-ca.pem')[1].date().isoformat(), '2026-10-20')

    def test_corrupt_ca_checksum_never_writes_candidate(self):
        output = self.root / 'ca-candidate'
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            refresh_ca(output, '2026-10-20', lambda url: CA_DATA if url == CA_URL else b'0' * 64 + b'  cacert.pem\n')
        self.assertFalse(output.exists())

    def test_corrupt_ca_body_never_writes_candidate(self):
        data = b'not a CA bundle'
        with self.assertRaises(ValueError):
            refresh_ca(self.root / 'ca-candidate', '2026-10-20', lambda url: data if url == CA_URL else (digest(data) + '  cacert.pem\n').encode())

    def test_complete_candidate_payload_validates(self):
        with patch('build_catalogs.datetime') as clock:
            clock.now.return_value = NOW
            clock.strptime = datetime.strptime
            self.assertEqual(publisher.validate(self.root)[0], 'build-inputs-apk-2026-10-20')

    def test_extra_candidate_file_or_directory_rejected(self):
        for extra in ('unreviewed.py', 'extra-dir'):
            target = self.root / extra
            target.write_text('poison') if '.' in extra else target.mkdir()
            with self.subTest(extra=extra), patch('build_catalogs.datetime') as clock:
                clock.now.return_value = NOW
                clock.strptime = datetime.strptime
                with self.assertRaisesRegex(ValueError, 'unexpected'): publisher.validate(self.root)
            target.unlink() if target.is_file() else target.rmdir()

    def test_missing_or_corrupted_payload_rejected(self):
        next((self.root / 'payload').glob('*.apk')).write_bytes(b'corrupt')
        with patch('build_catalogs.datetime') as clock:
            clock.now.return_value = NOW
            clock.strptime = datetime.strptime
            with self.assertRaisesRegex(ValueError, 'checksum mismatch'): publisher.validate(self.root)

    def test_candidate_symlink_rejected_before_reading(self):
        (self.root / 'outside').symlink_to('/nonexistent/poison')
        with self.assertRaisesRegex(ValueError, 'symlinks'): publisher.validate(self.root)


class Transport(unittest.TestCase):
    def test_official_https_only_before_any_request(self):
        for url in ('http://snapshot.debian.org/a', 'https://snapshot.debian.org.evil/a', 'https://user@snapshot.debian.org/a',
                    'https://snapshot.debian.org/a?q=x', 'https://curl.se.evil/ca/cacert.pem'):
            with self.subTest(url=url), patch('urllib.request.build_opener') as opener:
                with self.assertRaises(ValueError): fetch(url)
                opener.assert_not_called()
        with patch('urllib.request.build_opener') as opener:
            with self.assertRaises(ValueError): fetch_ca(CA_URL + '?mutable')
            opener.assert_not_called()

    def test_redirect_downgrade_or_origin_change_rejected_before_following(self):
        request = urllib.request.Request('https://snapshot.debian.org/archive/a')
        for url in ('http://snapshot.debian.org/archive/a', 'https://evil.example/a', 'https://user@snapshot.debian.org/a'):
            with self.subTest(url=url), self.assertRaises(ValueError):
                SnapshotRedirect().redirect_request(request, None, 302, 'found', {}, url)
        redirected = SnapshotRedirect().redirect_request(request, None, 302, 'found', {}, 'https://snapshot.debian.org/archive/b')
        self.assertEqual(redirected.full_url, 'https://snapshot.debian.org/archive/b')

    def test_unsigned_metadata_rejected(self):
        with patch('build_catalogs.fetch_https', return_value=b'unsigned metadata'):
            with self.assertRaisesRegex(ValueError, 'unsigned'): fetch('https://snapshot.debian.org/a')


class Publication(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.candidate = self.root / 'candidate'
        self.checkout = self.root / 'checkout'
        for name in publisher.paths():
            for root in (self.candidate, self.checkout):
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(('verified fixture ' + name).encode())
        self.env = {'GITHUB_EVENT_NAME': 'schedule', 'GITHUB_REF': 'refs/heads/main', 'GITHUB_REPOSITORY': publisher.REPOSITORY}
        self.commands = []

    def run_publisher(self, existing=False, changed=True, conflict=False, existing_pr=False, foreign_changes=False):
        def run(command, **kwargs):
            self.commands.append(command)
            result = int(changed) if command[:3] == ['git', 'diff', '--cached'] else 0
            return subprocess.CompletedProcess(command, result)
        def output(command, **kwargs):
            self.commands.append(command)
            if command[:2] == ['git', 'ls-remote']: return ('a' * 40 + '\trefs/heads/automation/build-inputs-2026-10-20') if existing else ''
            if command[:3] == ['git', 'diff', '--name-only']:
                return 'backend/src/main.rs\n' if foreign_changes else '\n'.join(publisher.paths())
            if command[:2] == ['git', 'show']:
                return b'conflict' if conflict else (self.candidate / command[2].split(':', 1)[1]).read_bytes()
            if command[:2] == ['git', 'rev-parse']: return 'a' * 40
            if command[:3] == ['gh', 'pr', 'list']: return '[{"url":"https://github.com/fixture"}]' if existing_pr else '[]'
            self.fail('unexpected external command: ' + str(command))
        stamps = {name: datetime(2026, 10, 19, tzinfo=timezone.utc) for name in ('APT debian:bookworm', 'APT ubuntu:noble')}
        with patch.dict(os.environ, self.env), patch.object(publisher, 'ROOT', self.checkout), \
             patch.object(publisher, 'validate', return_value=('build-inputs-apk-2026-10-20', [])), \
             patch.object(publisher, 'dates', return_value=stamps), \
             patch.object(publisher, 'ca_provenance', return_value=(None, datetime(2026, 10, 20, tzinfo=timezone.utc))), \
             patch.object(publisher.subprocess, 'run', side_effect=run), \
             patch.object(publisher.subprocess, 'check_output', side_effect=output), \
             patch.object(publisher, 'publish_archives') as archive:
            publisher.publish(self.candidate, '2026-10-20')
            return archive.call_args_list

    def test_pr_nonmain_and_foreign_repo_cannot_publish(self):
        for changed in ({'GITHUB_EVENT_NAME': 'pull_request'}, {'GITHUB_REF': 'refs/heads/dev'}, {'GITHUB_REPOSITORY': 'foreign/repo'}):
            with self.subTest(changed=changed), patch.dict(os.environ, {**self.env, **changed}), patch.object(publisher.subprocess, 'run') as run:
                with self.assertRaisesRegex(ValueError, 'official main schedule'): publisher.publish(self.candidate, '2026-10-20')
                run.assert_not_called()

    def test_explicit_catalog_only_commit_and_nonforce_push(self):
        self.assertEqual(len(self.run_publisher()), 1)
        commit = next(c for c in self.commands if c[0] == 'git' and 'commit' in c)
        self.assertEqual(commit[commit.index('--') + 1:], publisher.paths())
        push = next(c for c in self.commands if c[:2] == ['git', 'push'])
        self.assertFalse(any('force' in arg for arg in push))
        self.assertIn('HEAD:refs/heads/automation/build-inputs-2026-10-20', push)
        self.assertTrue(any(c[:3] == ['gh', 'pr', 'create'] and '--body-file' in c for c in self.commands))

    def test_existing_identical_candidate_is_idempotent(self):
        self.assertEqual(len(self.run_publisher(existing=True, existing_pr=True)), 1)
        self.assertFalse(any(c[:2] in (['git', 'push'], ['git', 'switch']) for c in self.commands))
        self.assertFalse(any(c[:3] == ['gh', 'pr', 'create'] for c in self.commands))

    def test_existing_changed_candidate_is_never_replaced(self):
        with self.assertRaisesRegex(ValueError, 'refusing branch/asset replacement'): self.run_publisher(existing=True, conflict=True)
        self.assertFalse(any(c[:2] == ['git', 'push'] for c in self.commands))

    def test_existing_candidate_with_non_catalog_changes_rejected(self):
        with self.assertRaisesRegex(ValueError, 'non-catalog paths'):
            self.run_publisher(existing=True, foreign_changes=True)
        self.assertFalse(any(c[:2] == ['git', 'push'] or c[0] == 'gh' for c in self.commands))

    def test_unchanged_inputs_do_not_publish(self):
        self.assertEqual(self.run_publisher(changed=False), [])
        self.assertFalse(any(c[:2] == ['git', 'push'] or c[0] == 'gh' for c in self.commands))


class UpdateCoverage(unittest.TestCase):
    def test_standalone_update_managers_cover_current_exact_pins(self):
        config = json.loads((ROOT / 'renovate.json').read_text())
        self.assertFalse(config['automerge'])
        self.assertIn('monday', ' '.join(config['schedule']))
        dependencies = set()
        files = ['security/build-tools.json', 'cli/Dockerfile.machine-hosted', 'rust-toolchain.toml',
                 '.node-version', 'cli/resources/cua/release.json', 'security/hosted-context.tsv']
        for manager in config['customManagers']:
            dependencies.add(manager['depNameTemplate'])
            matches = []
            for name in files:
                if any(re.search(pattern[1:-1], name) for pattern in manager['managerFilePatterns']):
                    for pattern in manager['matchStrings']:
                        pattern = pattern.replace('(?<currentValue>', '(?P<currentValue>').replace('(?<currentDigest>', '(?P<currentDigest>')
                        matches.extend(re.finditer(pattern, (ROOT / name).read_text()))
            self.assertEqual(len(matches), 1, manager['depNameTemplate'])
        self.assertEqual(dependencies, {'axodotdev/cargo-dist', 'rust-lang/rustup', 'rust-secure-code/cargo-auditable',
                                       'anchore/syft', 'gitleaks/gitleaks', 'aquasecurity/trivy', 'rustsec/rustsec', 'regclient/regclient', 'npm', 'node', 'rust', '@openai/codex', '@agentclientprotocol/codex-acp',
                                       'getheca/heca-artifacts', 'trycua/cua', 'https://github.com/ChronoAIProject/cma.git'})


if __name__ == '__main__':
    unittest.main()
