#!/usr/bin/env python3
"""Patch-source verification must precede every external Node execution."""
import hashlib
import json
import shutil
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('hosted_context', ROOT / 'scripts/verify-hosted-context.py')
hosted = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hosted)
spec = importlib.util.spec_from_file_location('hosted_policy', ROOT / 'scripts/check-hosted-inputs.py')
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)


class Context(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.context = self.root / 'context'
        self.context.mkdir()
        self.pins = self.root / 'pins.tsv'
        lines = ['# nyxid.hosted-context.v1', '# source-commit ' + 'a' * 40]
        self.bytes = {}
        for name in hosted.NAMES:
            data = ('reviewed fixture for ' + name).encode()
            (self.context / name).write_bytes(data)
            self.bytes[name] = data
            lines.append(name + '\t' + hashlib.sha256(data).hexdigest())
        self.pins.write_text('\n'.join(lines) + '\n')

    def test_verify_only_never_executes(self):
        with patch.object(hosted.subprocess, 'run') as run:
            hosted.verify(self.context, self.pins)
            run.assert_not_called()

    def test_exact_bytes_execute_in_reviewed_order_from_private_copies(self):
        seen = []
        def run(command, **kwargs):
            path = Path(command[1])
            self.assertEqual(command[0], 'node')
            self.assertNotEqual(path.parent, self.context)
            self.assertEqual(path.read_bytes(), self.bytes[path.name])
            self.assertEqual(set(p.name for p in path.parent.iterdir()), set(hosted.NAMES))
            self.assertEqual(kwargs, {'check': True, 'cwd': str(path.parent)})
            seen.append(path)
        with patch.object(hosted.subprocess, 'run', side_effect=run):
            hosted.verify(self.context, self.pins, True)
        self.assertEqual([p.name for p in seen], list(hosted.NAMES))
        self.assertTrue(all(not p.exists() for p in seen))

    def test_changed_third_patch_prevents_even_first_execution(self):
        (self.context / hosted.NAMES[-1]).write_bytes(b'changed source')
        with patch.object(hosted.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                hosted.verify(self.context, self.pins, True)
            run.assert_not_called()

    def test_missing_or_symlinked_script_prevents_execution(self):
        target = self.context / hosted.NAMES[-1]
        data = target.read_bytes()
        target.unlink()
        for symlink in (False, True):
            if symlink:
                original = self.root / 'original'
                original.write_bytes(data)
                target.symlink_to(original)
            with self.subTest(symlink=symlink), patch.object(hosted.subprocess, 'run') as run:
                with self.assertRaisesRegex(ValueError, 'missing, symlinked'):
                    hosted.verify(self.context, self.pins, True)
                run.assert_not_called()

    def test_names_order_traversal_duplicates_and_bad_hashes_reject(self):
        original = self.pins.read_text()
        for changed in (original.replace(hosted.NAMES[0], '../' + hosted.NAMES[0]),
                        original.replace(hosted.NAMES[-1], hosted.NAMES[0]),
                        original.replace(original.splitlines()[2], original.splitlines()[3]),
                        original + original.splitlines()[2] + '\n',
                        original.replace('source-commit ' + 'a' * 40, 'source-commit main')):
            self.pins.write_text(changed)
            with self.subTest(changed=changed), patch.object(hosted.subprocess, 'run') as run:
                with self.assertRaises(ValueError): hosted.verify(self.context, self.pins, True)
                run.assert_not_called()

    def test_source_changes_after_verification_cannot_change_executed_bytes(self):
        seen = []
        def run(command, **kwargs):
            (self.context / hosted.NAMES[-1]).write_bytes(b'changed during execution')
            target = Path(command[1])
            self.assertEqual(target.read_bytes(), self.bytes[target.name])
            seen.append(target.name)
        with patch.object(hosted.subprocess, 'run', side_effect=run):
            hosted.verify(self.context, self.pins, True)
        self.assertEqual(seen, list(hosted.NAMES))

    def test_execution_failure_stops_following_patches(self):
        with patch.object(hosted.subprocess, 'run', side_effect=subprocess.CalledProcessError(17, 'node')) as run:
            with self.assertRaises(subprocess.CalledProcessError): hosted.verify(self.context, self.pins, True)
            self.assertEqual(run.call_count, 1)


class Policy(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        for name in ('cli/Dockerfile.machine-hosted', 'cli/resources/cua/release.json',
                     'security/hosted-tools/package.json', 'security/hosted-tools/package-lock.json', 'security/hosted-tools/.npmrc'):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, path)

    def test_current_guard_and_archives_pass(self):
        self.assertEqual(policy.check(self.root), [])

    def test_unchecked_external_execution_rejected(self):
        path = self.root / 'cli/Dockerfile.machine-hosted'
        path.write_text(path.read_text() + '\nRUN node /tmp/cma-codex-source/persist_conversations.cjs\n')
        self.assertIn('authenticated before execution', '\n'.join(policy.check(self.root)))

    def test_version_only_cua_proposal_rejected_until_assets_are_refreshed(self):
        path = self.root / 'cli/resources/cua/release.json'
        data = json.loads(path.read_text())
        data['tag'] = 'cua-driver-rs-v99.0.0'
        path.write_text(json.dumps(data))
        self.assertTrue(policy.check(self.root))

    def test_cua_insecure_origin_or_unpinned_archive_rejected(self):
        path = self.root / 'cli/resources/cua/release.json'
        original = json.loads(path.read_text())
        for field, value in (('url', 'http://github.com/archive'), ('sha256', 'mutable'), ('size', 0)):
            data = json.loads(json.dumps(original))
            data['assets'][0][field] = value
            path.write_text(json.dumps(data))
            with self.subTest(field=field): self.assertTrue(policy.check(self.root))

    def test_unlocked_global_and_exec_variants_rejected(self):
        path = self.root / 'cli/Dockerfile.machine-hosted'
        original = path.read_text()
        for command in ('npm install --global @openai/codex@0.147.0', 'npm --global install @openai/codex@0.147.0',
                        'npm install -g @openai/codex@0.147.0', 'npm install --location=global @openai/codex@0.147.0',
                        'npx codex', '/usr/bin/npm ci --ignore-scripts', '/opt/nyxid-npm/npm rebuild',
                        'sh /opt/nyxid-apt/apt-snapshot-install.sh /opt/nyxid-apt/apt-snapshots.tsv ubuntu:noble -- nodejs $(npm install -g unreviewed)',
                        'sh /opt/nyxid-apt/apt-snapshot-install.sh /opt/nyxid-apt/apt-snapshots.tsv ubuntu:noble -- nodejs `npm install -g unreviewed`'):
            path.write_text(original + '\nRUN ' + command + '\n')
            with self.subTest(command=command):
                self.assertIn('unlocked/global installs', '\n'.join(policy.check(self.root)))

    def test_missing_or_overridden_ignore_scripts_rejected(self):
        path = self.root / 'cli/Dockerfile.machine-hosted'
        original = path.read_text()
        for changed in (original.replace('ci --ignore-scripts', 'ci'),
                        original.replace('ci --ignore-scripts', 'ci --ignore-scripts=false'),
                        original.replace('ci --ignore-scripts', 'ci --ignore-scripts --ignore-scripts=false'),
                        original + '\nENV NPM_CONFIG_IGNORE_SCRIPTS=false\n'):
            path.write_text(changed)
            with self.subTest(changed=changed[-60:]): self.assertTrue(policy.check(self.root))

    def test_root_lifecycle_hook_or_approval_rejected(self):
        path = self.root / 'security/hosted-tools/package.json'
        original = json.loads(path.read_text())
        for field, value in (('scripts', {'install': 'touch /tmp/hook'}), ('allowScripts', {'*': True})):
            data = {**original, field: value}
            path.write_text(json.dumps(data))
            with self.subTest(field=field):
                self.assertIn('zero lifecycle hooks', '\n'.join(policy.check(self.root)))

    def test_dependency_install_hook_rejected(self):
        path = self.root / 'security/hosted-tools/package-lock.json'
        data = json.loads(path.read_text())
        data['packages']['node_modules/@openai/codex']['hasInstallScript'] = True
        path.write_text(json.dumps(data))
        self.assertIn('zero install hooks', '\n'.join(policy.check(self.root)))

    def test_unbound_external_or_linked_dependency_rejected(self):
        path = self.root / 'security/hosted-tools/package-lock.json'
        original = json.loads(path.read_text())
        for field, value in (('integrity', ''), ('integrity', 'sha256-fixture'),
                             ('resolved', 'http://registry.npmjs.org/package.tgz'),
                             ('resolved', 'https://registry.npmjs.org.evil/package.tgz'), ('link', True)):
            data = json.loads(json.dumps(original))
            data['packages']['node_modules/zod'][field] = value
            path.write_text(json.dumps(data))
            with self.subTest(field=field, value=value):
                self.assertIn('exact registry archive', '\n'.join(policy.check(self.root)))

    def test_manifest_range_or_lock_drift_rejected(self):
        package = self.root / 'security/hosted-tools/package.json'
        data = json.loads(package.read_text())
        data['dependencies']['@openai/codex'] = '^0.147.0'
        package.write_text(json.dumps(data))
        errors = '\n'.join(policy.check(self.root))
        self.assertIn('exactly match reviewed tool versions', errors)
        self.assertIn('manifest and npm lock must agree', errors)

    def test_changed_locked_direct_dependency_rejected(self):
        path = self.root / 'security/hosted-tools/package-lock.json'
        data = json.loads(path.read_text())
        data['packages']['node_modules/@openai/codex']['version'] = '0.148.0'
        path.write_text(json.dumps(data))
        self.assertIn('locked direct dependency differs', '\n'.join(policy.check(self.root)))

    def test_missing_lock_or_disabled_config_rejected(self):
        path = self.root / 'security/hosted-tools/.npmrc'
        path.write_text('ignore-scripts=false\n')
        self.assertIn('lifecycle execution must be disabled', '\n'.join(policy.check(self.root)))
        (self.root / 'security/hosted-tools/package-lock.json').unlink()
        self.assertIn('invalid hosted npm inputs', '\n'.join(policy.check(self.root)))

    def test_runtime_omitting_hoisted_transitive_dependencies_rejected(self):
        path = self.root / 'cli/Dockerfile.machine-hosted'
        path.write_text(path.read_text().replace('COPY --from=tools /usr/local/lib/node_modules/ /usr/local/lib/node_modules/',
                                               'COPY --from=tools /usr/local/lib/node_modules/@openai/ /usr/local/lib/node_modules/@openai/'))
        self.assertIn('complete runtime dependency tree', '\n'.join(policy.check(self.root)))


if __name__ == '__main__':
    unittest.main()
