#!/usr/bin/env python3
"""Prove dependency-root coverage and fail-closed audit/API handling."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from dependency_roots import discover

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('dependency_audits', ROOT / 'scripts/audit-dependencies.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class DependencyAudits(unittest.TestCase):
    def test_all_current_roots_and_workspace_members_are_covered(self):
        roots = discover(ROOT)
        self.assertEqual(len(roots), 13)
        self.assertEqual({item['lock'] for item in roots if item['manager'] == 'cargo'},
                         {'Cargo.lock', 'cli/tests/machine_context_spike/Cargo.lock', 'security/artifact-tools/Cargo.lock'})
        covered = {name for item in roots for name in item['manifests']}
        for name in ('backend', 'cli', 'machine', 'machine-updater', 'mcp-demo', 'cloud-auth'):
            self.assertIn(name + '/Cargo.toml', covered)
        for name in ('sdk/oauth-core', 'sdk/oauth-react', 'sdk/demo-react', 'security/hosted-tools', 'security/signing-tools', '.github/tools/claude'):
            self.assertIn(name + '/package.json', covered)
        self.assertEqual(next(item for item in roots if item['lock'] == 'security/signing-tools/package-lock.json')['manager'], 'npm')
        self.assertEqual(next(item for item in roots if item['manager'] == 'pnpm')['lock'], 'mobile/pnpm-lock.yaml')

    def test_new_unlocked_independent_manifest_and_orphan_lock_fail(self):
        paths = ['new/package.json']
        with patch('dependency_roots.subprocess.check_output', return_value='\0'.join(paths).encode()):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / 'new').mkdir()
                (root / paths[0]).write_text('{"dependencies":{"fixture":"1.0.0"}}')
                with self.assertRaisesRegex(ValueError, 'unlocked'): discover(root)
        with patch('dependency_roots.subprocess.check_output', return_value=b'new/package-lock.json\0'):
            with self.assertRaisesRegex(ValueError, 'orphan'): discover(ROOT)

    def test_child_package_outside_locked_npm_workspace_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'child').mkdir()
            (root / 'package.json').write_text('{}')
            (root / 'child/package.json').write_text('{}')
            paths = b'package.json\0package-lock.json\0child/package.json\0'
            with patch('dependency_roots.subprocess.check_output', return_value=paths), \
                    self.assertRaisesRegex(ValueError, 'not in its locked workspace'): discover(root)

    def exercise(self, report, code=0, mutate=False):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            lock = root / 'package-lock.json'
            lock.write_text('{}')
            (root / 'package.json').write_text('{}')
            def run(command, **kwargs):
                self.assertEqual(command, ['npm', 'audit', '--json', '--audit-level=low'])
                self.assertEqual(kwargs['env']['NPM_CONFIG_IGNORE_SCRIPTS'], 'true')
                if mutate: (kwargs['cwd'] / lock.name).write_text('changed')
                return subprocess.CompletedProcess(command, code, json.dumps(report).encode(), b'')
            with patch.object(audit, 'discover', return_value=[dict(lock=lock.name, manager='npm', manifests=['package.json'])]), \
                    patch.object(audit.subprocess, 'run', side_effect=run):
                return audit.audit(root, root / 'output', Path('cargo-audit'), Path('npm'), Path('pnpm'))

    def test_clean_audit_passes(self):
        self.assertEqual(self.exercise({'metadata': {'vulnerabilities': {'low': 0, 'total': 0}}}), 0)

    def test_even_low_vulnerability_fails(self):
        self.assertEqual(self.exercise({'metadata': {'vulnerabilities': {'low': 1, 'total': 1}}}, 1), 1)

    def test_api_errors_invalid_reports_or_mismatched_exit_fail(self):
        for report, code in [({'error': {'code': 'ENOTFOUND'}}, 1), ({}, 0),
                              ({'metadata': {'vulnerabilities': {'high': 1}}}, 0),
                              ({'metadata': {'vulnerabilities': {'high': 0}}}, 1)]:
            with self.subTest(report=report, code=code), self.assertRaises(ValueError): self.exercise(report, code)

    def test_lock_mutation_fails(self):
        with self.assertRaisesRegex(ValueError, 'mutated'): self.exercise({}, mutate=True)


if __name__ == '__main__':
    unittest.main()
