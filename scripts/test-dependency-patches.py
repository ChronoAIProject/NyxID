#!/usr/bin/env python3
"""Tamper and scope negatives for executable mobile-patch dispositions."""
import copy
from datetime import date
import hashlib
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

import dependency_patches as d
from workflow_yaml import load_unique
import yaml

ROOT = Path(__file__).resolve().parents[1]


class PatchDispositions(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='nyxid-patch-policy-test-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.records = json.loads((ROOT / d.POLICY).read_text())['patches']
        for name in {'mobile/package.json', d.REGRESSION, d.COMPATIBILITY,
                     *(r['lock'] for r in self.records), *(r['patch'] for r in self.records)}:
            destination = self.root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, destination)
        self.policy = self.root / d.POLICY
        self.policy.parent.mkdir(parents=True)
        self.save()

    def save(self):
        self.policy.write_text(json.dumps(dict(schema_version=1, patches=self.records)))

    def validate(self):
        records = d.load_policy(self.root, today=date(2026, 10, 20))
        d.validate_inputs(self.root, records)

    def test_actual_reviewed_policy_inputs_match(self):
        self.validate()
        self.assertEqual(sum(len(r['advisories']) for r in self.records), 4)

    def test_expired_unused_record_fails(self):
        for expiry in ('2026-10-20', '2027-01-01'):
            self.records[0]['expires'] = expiry
            self.save()
            with self.subTest(expiry=expiry), self.assertRaisesRegex(ValueError, 'expired/overlong'): self.validate()

    def test_bound_file_rejects_symlinked_file_or_parent_even_with_reviewed_bytes(self):
        original = self.root / self.records[0]['patch']
        digest = self.records[0]['patch_sha256']
        alias = original.with_suffix('.alias')
        alias.symlink_to(original)
        with self.assertRaisesRegex(ValueError, 'bound file'):
            d.bound_file(self.root, alias.relative_to(self.root).as_posix(), digest)
        directory = self.root / 'mobile/linked-patches'
        directory.symlink_to(original.parent, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'bound file'):
            d.bound_file(self.root, (directory / original.name).relative_to(self.root).as_posix(), digest)

    def test_missing_changed_and_moved_patch_fail(self):
        for change in ('missing', 'changed', 'moved'):
            with self.subTest(change=change):
                p = self.root / self.records[0]['patch']
                original = p.read_bytes()
                if change == 'changed': p.write_bytes(original + b'\n')
                elif change == 'moved': p.rename(p.with_suffix('.moved'))
                else: p.unlink()
                with self.assertRaisesRegex(ValueError, 'bound file'): self.validate()
                p.write_bytes(original)

    def test_moved_and_non_normalized_policy_path_fail(self):
        for value in ('mobile/other.patch', './mobile/patches/x.patch', 'mobile/../x.patch', 'mobile\\patches\\x.patch'):
            with self.subTest(value=value):
                records = copy.deepcopy(self.records)
                records[0]['patch'] = value
                self.policy.write_text(json.dumps(dict(schema_version=1, patches=records)))
                with self.assertRaises(ValueError): self.validate()

    def test_missing_owner_rationale_and_regression_bindings_fail(self):
        for field in ('owner', 'rationale', 'regressions', 'source_files', 'integrity'):
            with self.subTest(field=field):
                records = copy.deepcopy(self.records); records[0].pop(field)
                self.policy.write_text(json.dumps(dict(schema_version=1, patches=records)))
                with self.assertRaises(ValueError): self.validate()

    def test_changed_lock_requires_explicit_review(self):
        p = self.root / self.records[0]['lock']; p.write_bytes(p.read_bytes() + b'\n')
        with self.assertRaisesRegex(ValueError, 'bound file'): self.validate()

    def test_changed_or_missing_regression_script_fails(self):
        for name in (d.REGRESSION, d.COMPATIBILITY):
            p = self.root / name; original = p.read_bytes()
            p.write_bytes(original + b'\n')
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'bound file'): self.validate()
            p.unlink()
            with self.assertRaises(ValueError): self.validate()
            p.write_bytes(original)

    def rewrite_lock(self, transform):
        p = self.root / 'mobile/pnpm-lock.yaml'
        lock = load_unique(p.read_text()); transform(lock)
        p.write_text(yaml.safe_dump(lock, sort_keys=False))
        for r in self.records: r['lock_sha256'] = d.sha(p.read_bytes())
        self.save()

    def test_unpatched_duplicate_snapshot_fails_even_with_refreshed_lock_digest(self):
        r = self.records[0]
        self.rewrite_lock(lambda lock: lock['snapshots'].update({r['package'] + '@' + r['version']: {}}))
        with self.assertRaisesRegex(ValueError, 'snapshot'): self.validate()

    def test_unpatched_dependency_edge_fails(self):
        r = self.records[0]
        self.rewrite_lock(lambda lock: lock['snapshots'].update({'synthetic@1.0.0': {'dependencies': {r['package']: r['version']}}}))
        with self.assertRaisesRegex(ValueError, 'dependency edge'): self.validate()

    def test_changed_upstream_integrity_fails_even_with_refreshed_lock_digest(self):
        r = self.records[0]
        self.rewrite_lock(lambda lock: lock['packages'][r['package'] + '@' + r['version']]['resolution'].update(integrity='sha512-changed'))
        with self.assertRaisesRegex(ValueError, 'integrity'): self.validate()

    def test_changed_patch_map_fails(self):
        self.rewrite_lock(lambda lock: lock['patchedDependencies'][self.records[0]['package'] + '@' + self.records[0]['version']].update(hash='0' * 64))
        with self.assertRaisesRegex(ValueError, 'patch map'): self.validate()

    def test_wrong_package_version_fails(self):
        self.records[0]['version'] = '0.2.1'; self.save()
        with self.assertRaises(ValueError): self.validate()

    def test_archive_bytes_must_match_original_integrity(self):
        with self.assertRaisesRegex(ValueError, 'integrity'):
            d.unpack_original(b'corrupt original archive', self.records[0], self.root / 'original')

    def test_archive_traversal_and_symlink_fail(self):
        for name, kind in [('package/../escape', tarfile.REGTYPE), ('package/link', tarfile.SYMTYPE)]:
            data = io.BytesIO()
            with tarfile.open(fileobj=data, mode='w:gz') as archive:
                entry = tarfile.TarInfo(name); entry.type = kind; entry.linkname = '/tmp/escape'
                archive.addfile(entry)
            payload = data.getvalue()
            r = dict(self.records[0], integrity='sha512-' + d.base64.b64encode(hashlib.sha512(payload).digest()).decode())
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary, self.assertRaises(ValueError):
                d.unpack_original(payload, r, Path(temporary) / 'original')

    def test_changed_unpatched_duplicate_or_extra_installed_source_fails(self):
        r = dict(self.records[0], source_files={'index.js': d.sha(b'reviewed source')})
        package = self.root / 'node_modules/.pnpm' / (r['package'] + '@' + r['version'] + '_patch_hash=reviewed') / 'node_modules' / r['package']
        package.mkdir(parents=True); source = package / 'index.js'; source.write_bytes(b'reviewed source')
        self.assertEqual(d.installed_package(self.root, r), package)
        source.write_bytes(b'unpatched source')
        with self.assertRaisesRegex(ValueError, 'source differs'): d.installed_package(self.root, r)
        source.write_bytes(b'reviewed source'); (package / 'new.js').write_bytes(b'new code')
        with self.assertRaises(ValueError): d.installed_package(self.root, r)
        (package / 'new.js').unlink()
        duplicate = self.root / 'node_modules/.pnpm' / (r['package'] + '@' + r['version']) / 'node_modules' / r['package']
        duplicate.mkdir(parents=True)
        with self.assertRaisesRegex(ValueError, 'duplicate'): d.installed_package(self.root, r)

    def report(self, advisory=None, version=None):
        r = self.records[0]
        return {'advisories': {'1': {'module_name': r['package'], 'github_advisory_id': advisory or r['advisories'][0],
                                  'findings': [{'version': version or r['version'], 'paths': ['.>real>consumer']} ]}}}

    def proofs(self):
        return [dict(lock=r['lock'], package=r['package'], version=r['version'], advisory=a,
                     original_regression='security-assertion-failed') for r in self.records for a in r['advisories']]

    def test_only_exact_advisory_version_and_lock_with_executed_proof_is_reviewed(self):
        report = self.report(); before = copy.deepcopy(report)
        self.assertEqual(len(d.classify(report, 'mobile/pnpm-lock.yaml', self.records, self.proofs())), 1)
        self.assertEqual(report, before, 'raw registry report must remain complete')
        for value, lock, proofs in [(self.report(advisory='GHSA-aaaa-bbbb-cccc'), 'mobile/pnpm-lock.yaml', self.proofs()),
                                    (self.report(version='0.2.1'), 'mobile/pnpm-lock.yaml', self.proofs()),
                                    (report, 'moved/pnpm-lock.yaml', self.proofs()), (report, 'mobile/pnpm-lock.yaml', [])]:
            with self.subTest(value=value, lock=lock, proofs=proofs):
                self.assertEqual(d.classify(value, lock, self.records, proofs), [])

    def test_new_advisory_same_package_and_unreviewed_finding_remain_blocking(self):
        report = self.report(); report['advisories']['new'] = self.report(advisory='GHSA-aaaa-bbbb-cccc')['advisories']['1']
        self.assertEqual(len(d.classify(report, 'mobile/pnpm-lock.yaml', self.records, self.proofs())), 1)
        report['advisories']['1']['findings'].append({'version': '0.2.1', 'paths': ['.>other']})
        self.assertEqual(d.classify(report, 'mobile/pnpm-lock.yaml', self.records, self.proofs()), [])

    def test_original_probe_cannot_mistake_infrastructure_failure_or_success_for_vulnerability(self):
        for code, stderr in [(0, b''), (1, b'MODULE_NOT_FOUND'), (2, b'ERR_ASSERTION'), (1, b'other failure')]:
            with self.subTest(code=code, stderr=stderr), patch.object(d.subprocess, 'run', return_value=subprocess.CompletedProcess([], code, b'', stderr)), self.assertRaises(ValueError):
                d.negative_probe(Path('script'), 'forge', self.root, {})

    def test_original_timeout_is_only_accepted_for_known_loop_probes(self):
        with patch.object(d.subprocess, 'run', side_effect=subprocess.TimeoutExpired([], 5)):
            self.assertEqual(d.negative_probe(Path('script'), 'image', self.root, {}), 'bounded-timeout')
            with self.assertRaises(ValueError): d.negative_probe(Path('script'), 'forge', self.root, {})

    def test_frozen_install_requires_locked_manager_no_hooks_and_cannot_skip_proof(self):
        directory = self.root / 'isolated'; directory.mkdir()
        shutil.copyfile(self.root / 'mobile/pnpm-lock.yaml', directory / 'pnpm-lock.yaml')
        calls = []
        def run(command, **kwargs):
            calls.append((command, kwargs))
            return subprocess.CompletedProcess(command, 0, b'10.34.7\n', b'')
        with patch.object(d, 'run_required', side_effect=run), patch.object(d, 'verify_installed', return_value=['executed']) as proof:
            self.assertEqual(d.install_and_verify(self.root, directory, self.records, Path('pnpm'), dict(os.environ, NODE_OPTIONS='unsafe', NODE_PATH='unsafe')), ['executed'])
            proof.assert_called_once()
        command, options = calls[1]
        for flag in ('--frozen-lockfile', '--ignore-scripts', '--ignore-pnpmfile'): self.assertIn(flag, command)
        self.assertNotIn('NODE_OPTIONS', options['env']); self.assertNotIn('NODE_PATH', options['env'])
        self.assertEqual(options['env']['CI'], 'true')
        with patch.object(d, 'run_required', return_value=subprocess.CompletedProcess([], 0, b'10.17.1\n', b'')), self.assertRaisesRegex(ValueError, 'locked pnpm'):
            d.install_and_verify(self.root, directory, self.records, Path('pnpm'), {})


if __name__ == '__main__':
    unittest.main()
