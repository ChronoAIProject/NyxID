#!/usr/bin/env python3
"""Source selection, deleted-layer payloads and durable-release binding regressions."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

from apk_archive_layers import check_layers
from apk_archive_release import ARCHES, REQUESTS, expected_assets, stage_payload, validate_payload
from apk_durability import check as check_durability
import urllib.error

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('provision', ROOT / 'scripts/provision-apk-archives.py')
provision = importlib.util.module_from_spec(spec)
spec.loader.exec_module(provision)


def digest(data): return hashlib.sha256(data).hexdigest()


class SourceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.calls = self.root / 'calls'
        wget = self.root / 'wget'
        wget.write_text('''#!/bin/sh
set -eu
while [ "$#" -gt 1 ]; do
  case "$1" in -O) destination=$2; shift 2 ;; -T) shift 2 ;; *) shift ;; esac
done
printf '%s\\n' "$1" >> "$CALLS"
case "$1" in
  https://durable/*)
    if [ -n "$REDIRECT" ]; then printf "  Location: %s\\n" "$REDIRECT" >&2; fi
    if [ "$STATUS" != 200 ]; then
      if [ "$STATUS" != timeout ]; then printf '  HTTP/1.1 %s Error\\n' "$STATUS" >&2; fi
      exit 8
    fi
    if [ "$CORRUPT" = yes ]; then printf corrupt > "$destination"; else printf archive > "$destination"; fi ;;
  https://upstream/*)
    if [ "$CORRUPT" = upstream ]; then printf corrupt > "$destination"; else printf archive > "$destination"; fi ;;
  *) exit 99 ;;
esac
''')
        wget.chmod(0o755)

    def fetch(self, mode='prefer-durable', status='200', corrupt='no', redirect=''):
        result = subprocess.run(['sh', str(ROOT / 'scripts/apk-input-fetch.sh'),
                                 'https://durable/releases/download/build-inputs-apk-2026-10-19/pkg.apk', 'https://upstream/pkg.apk',
                                 str(self.root / 'pkg.apk'), mode, digest(b'archive')],
                                env={**os.environ, 'PATH': str(self.root) + os.pathsep + os.environ['PATH'],
                                     'CALLS': str(self.calls), 'STATUS': status, 'CORRUPT': corrupt, 'REDIRECT': redirect},
                                capture_output=True, text=True)
        calls = self.calls.read_text().splitlines() if self.calls.exists() else []
        return result, calls

    def test_durable_success(self):
        result, calls = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ['https://durable/releases/download/build-inputs-apk-2026-10-19/pkg.apk'])
        self.assertNotIn('FALLBACK', result.stderr)

    def test_missing_asset_fallback_warns_with_release_tag(self):
        result, calls = self.fetch('prefer-durable', '404')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ['https://durable/releases/download/build-inputs-apk-2026-10-19/pkg.apk', 'https://upstream/pkg.apk'])
        self.assertIn('::warning::APK durable archives missing from build-inputs-apk-2026-10-19 (HTTP 404)', result.stderr)

    def test_present_durable_asset_never_contacts_upstream(self):
        result, calls = self.fetch('prefer-durable')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ['https://durable/releases/download/build-inputs-apk-2026-10-19/pkg.apk'])

    def test_other_http_failures_never_fallback(self):
        for status in ['401', '403', '429', '500', '503', 'timeout']:
            with self.subTest(status=status):
                self.calls.unlink(missing_ok=True)
                result, calls = self.fetch('prefer-durable', status)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(len(calls), 1)
                self.assertNotIn('FALLBACK', result.stderr)

    def test_corrupt_durable_asset_never_fallback(self):
        result, calls = self.fetch('prefer-durable', corrupt='yes')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(calls), 1)
        self.assertIn('checksum mismatch', result.stderr)

    def test_corrupt_upstream_bytes_fail(self):
        result, calls = self.fetch('prefer-durable', '404', corrupt='upstream')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(calls), 2)
        self.assertIn('checksum mismatch', result.stderr)

    def test_off_origin_redirect_fails_even_with_404(self):
        for url in ['https://evil.example/pkg.apk', 'http://github.com/pkg.apk', '//evil.example/pkg.apk',
                    'https://github.com.evil/pkg.apk', 'https://github.com@evil.example/pkg.apk']:
            with self.subTest(url=url):
                self.calls.unlink(missing_ok=True)
                result, calls = self.fetch(status='404', redirect=url)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(len(calls), 1)
                self.assertIn('redirect off-origin', result.stderr)
                self.assertNotIn('::warning::', result.stderr)

    def test_expected_github_asset_cdn_redirect_accepted(self):
        result, calls = self.fetch(redirect='https://release-assets.githubusercontent.com/github-production-release-asset/id/pkg.apk?signature=public-fixture')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 1)

    def test_provision_is_explicit_and_logged(self):
        result, calls = self.fetch('provision-upstream')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ['https://upstream/pkg.apk'])
        self.assertIn('APK PROVISION', result.stderr)

    def test_unknown_mode_has_no_network(self):
        result, calls = self.fetch('automatic')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [])


def tar_bytes(files):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode='w') as archive:
        for name, data in files.items():
            entry = tarfile.TarInfo(name)
            entry.size = len(data)
            archive.addfile(entry, io.BytesIO(data))
    return output.getvalue()


class LayerTests(unittest.TestCase):
    def image(self, layers):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        files = {str(i) + '/layer.tar': tar_bytes(layer) for i, layer in enumerate(layers)}
        files['manifest.json'] = json.dumps([{'Layers': list(files)}]).encode()
        path = Path(tmp.name) / 'image.tar'
        path.write_bytes(tar_bytes(files))
        return path

    def test_clean_layers(self):
        self.assertEqual(check_layers(self.image([{'usr/bin/python': b'ELF'}, {'receipt.tsv': b'catalog'}])), 2)

    def test_archive_in_any_layer_fails_even_if_deleted(self):
        path = 'opt/nyxid-apk/archives/pkg.apk'
        with self.assertRaisesRegex(ValueError, 'payload retained'):
            check_layers(self.image([{path: b'archive'}, {'opt/nyxid-apk/.wh.archives': b''}]))

    def test_archive_elsewhere_fails(self):
        with self.assertRaises(ValueError): check_layers(self.image([{'tmp/pkg.apk': b'archive'}]))

    def test_non_apk_payload_in_archive_directory_fails(self):
        with self.assertRaises(ValueError): check_layers(self.image([{'./opt/nyxid-apk/archives/pkg': b'archive'}]))

    def test_empty_manifest_fails(self):
        with self.assertRaises(ValueError): check_layers(self.image([]))


class PayloadTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.catalogs = self.root / 'catalogs'
        self.catalogs.mkdir()
        self.payload = self.root / 'payload'
        self.archives = self.root / 'archives'
        self.archives.mkdir()
        self.archive = self.archives / 'python3-1-r0.apk'
        self.archive.write_bytes(b'reviewed signed archive fixture')
        checksum = digest(self.archive.read_bytes())
        for label in REQUESTS:
            for arch, native in ARCHES.items():
                durable = f'https://github.com/ChronoAIProject/NyxID/releases/download/build-inputs-apk-2026-10-19/{checksum}-{native}-python3-1-r0.apk'
                text = f'''# nyxid.apk-inputs.v2
# base-image alpine:3.22@sha256:{'a' * 64}
# arch {native}
# refreshed-on 2026-10-19
# packages python3
# before-sha256 {'b' * 64}
# after-sha256 {'c' * 64}
python3\t1-r0\t{native}\t{checksum}\t{durable}\thttps://dl-cdn.alpinelinux.org/alpine/v3.22/main/{native}/python3-1-r0.apk
'''
                path = self.catalogs / (label + '-' + arch + '.tsv')
                path.write_text(text)
                stage_payload(path, arch, self.archives, self.payload)

    def durability(self, missing_release=False, remove=None, change=None):
        tag, expected = expected_assets(self.catalogs, ARCHES)
        def fetch(path):
            if path.startswith('/releases/tags/'):
                if missing_release:
                    raise urllib.error.HTTPError('https://api.github.com', 404, 'missing', {}, None)
                return {'id': 42, 'draft': False, 'tag_name': tag}
            assets = [{'name': name, 'digest': 'sha256:' + checksum, 'size': 42, 'state': 'uploaded'}
                      for name, checksum in expected.items() if name != remove]
            if change:
                assets[0].update(change)
            return assets
        return check_durability(self.catalogs, fetch)

    def test_durability_complete(self): self.assertEqual(self.durability(), [])
    def test_durability_missing_release(self): self.assertIn('release is missing', self.durability(missing_release=True)[0])
    def test_durability_missing_archive(self):
        name = next(self.payload.glob('*.apk')).name
        self.assertIn('missing asset ' + name, self.durability(remove=name)[0])
    def test_durability_missing_catalog(self):
        name = next(self.payload.glob('*.tsv')).name
        self.assertIn('missing asset ' + name, self.durability(remove=name)[0])
    def test_durability_incomplete_or_corrupt(self):
        for change in [{'state': 'new'}, {'size': 0}, {'digest': None}, {'digest': 'sha256:' + '0' * 64}]:
            with self.subTest(change=change): self.assertTrue(self.durability(change=change))

    def test_complete_payload_and_deduplication(self):
        tag, files = validate_payload(self.catalogs, self.payload, ARCHES)
        self.assertEqual(tag, 'build-inputs-apk-2026-10-19')
        self.assertEqual(len(files), 8)  # six catalogs + two distinct archive names

    def test_extra_file_fails(self):
        (self.payload / 'unexpected.apk').write_bytes(b'poison')
        with self.assertRaises(ValueError): validate_payload(self.catalogs, self.payload, ARCHES)

    def test_missing_archive_fails(self):
        next(self.payload.glob('*.apk')).unlink()
        with self.assertRaises(ValueError): validate_payload(self.catalogs, self.payload, ARCHES)

    def test_modified_archive_fails(self):
        next(self.payload.glob('*.apk')).write_bytes(b'corrupt')
        with self.assertRaises(ValueError): validate_payload(self.catalogs, self.payload, ARCHES)

    def test_modified_catalog_fails(self):
        path = next(self.payload.glob('*.tsv'))
        path.write_text(path.read_text() + '# unreviewed\n')
        with self.assertRaises(ValueError): validate_payload(self.catalogs, self.payload, ARCHES)

    def test_symlink_fails(self):
        path = next(self.payload.glob('*.apk'))
        path.unlink()
        path.symlink_to(self.archive)
        with self.assertRaises(ValueError): validate_payload(self.catalogs, self.payload, ARCHES)

    def test_stage_rejects_corrupt_bytes(self):
        self.archive.write_bytes(b'corrupt')
        with self.assertRaises(ValueError): stage_payload(next(self.catalogs.glob('*amd64.tsv')), 'amd64', self.archives, self.payload)

    def test_mixed_dates_fail(self):
        path = next(self.catalogs.glob('*.tsv'))
        path.write_text(path.read_text().replace('2026-10-19', '2026-10-20'))
        shutil.copyfile(path, self.payload / path.name)
        with self.assertRaisesRegex(ValueError, 'one catalog refresh date'):
            validate_payload(self.catalogs, self.payload, ARCHES)

    def publish_fixture(self, corrupt_existing=False, missing_release=False, missing_assets=False):
        calls = []
        real_run = subprocess.run
        def run(command, **kwargs):
            if command[0] == 'awk':
                return real_run(command, **kwargs)
            calls.append(command)
            if command[:3] == ['gh', 'release', 'view']:
                return subprocess.CompletedProcess(command, int(missing_release))
            if command[:3] == ['gh', 'release', 'download']:
                if missing_assets:
                    return subprocess.CompletedProcess(command, 1)
                destination = Path(command[command.index('--dir') + 1])
                name = command[command.index('--pattern') + 1]
                (destination / name).write_bytes(b'corrupt' if corrupt_existing else (self.payload / name).read_bytes())
            return subprocess.CompletedProcess(command, 0)
        def show(command, **kwargs):
            self.assertEqual(command[:2], ['git', 'show'])
            return (self.catalogs / command[2].split('/')[-1]).read_bytes()
        with patch.object(provision.subprocess, 'run', side_effect=run), patch.object(provision.subprocess, 'check_output', side_effect=show):
            provision.publish('a' * 40, self.payload)
        return calls

    def test_existing_assets_are_verified_never_clobbered(self):
        calls = self.publish_fixture()
        self.assertFalse(any(c[:3] == ['gh', 'release', 'upload'] for c in calls))
        self.assertFalse(any('--clobber' in c for c in calls))

    def test_existing_corrupt_asset_fails(self):
        with self.assertRaisesRegex(ValueError, 'refusing replacement'): self.publish_fixture(corrupt_existing=True)

    def test_release_is_prerelease_nonlatest(self):
        calls = self.publish_fixture(missing_release=True)
        create = next(c for c in calls if c[:3] == ['gh', 'release', 'create'])
        self.assertIn('--prerelease', create)
        self.assertIn('--latest=false', create)
        self.assertEqual(create[create.index('--target') + 1], 'a' * 40)

    def test_uploads_only_validated_assets_without_clobber(self):
        calls = self.publish_fixture(missing_release=True, missing_assets=True)
        uploads = [c for c in calls if c[:3] == ['gh', 'release', 'upload']]
        self.assertEqual(len(uploads), 8)
        self.assertEqual({Path(c[4]).name for c in uploads}, {p.name for p in self.payload.iterdir()})
        self.assertTrue(all('--clobber' not in c for c in uploads))

    def test_unreviewed_commit_rejected_before_gh(self):
        with patch.object(provision.subprocess, 'run') as run:
            with self.assertRaises(ValueError): provision.publish('main', self.payload)
            run.assert_not_called()


if __name__ == '__main__':
    unittest.main()
