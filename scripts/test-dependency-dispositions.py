#!/usr/bin/env python3
"""Exact use, parent, source, expiry and missing-proof negatives."""
import copy
from datetime import date
import io
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import dependency_dispositions as d
from workflow_yaml import load_unique

ROOT = Path(__file__).resolve().parents[1]


class UseDispositions(unittest.TestCase):
    def setUp(self):
        self.policy = json.loads((ROOT / d.POLICY).read_text())
        self.temporary = tempfile.TemporaryDirectory(prefix='nyxid-use-disposition-test-')
        self.addCleanup(self.temporary.cleanup); self.root = Path(self.temporary.name)
        for name in ('Cargo.lock', 'mobile/pnpm-lock.yaml'):
            destination = self.root / name; destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, destination)
        (self.root / 'security').mkdir(); self.save()

    def save(self):
        (self.root / d.POLICY).write_text(json.dumps(self.policy))

    def test_complete_current_policy_is_well_formed(self):
        self.assertEqual(len(d.load_policy(self.root, today=date(2026, 10, 20))['dispositions']), 3)

    def test_expired_or_overlong_record_even_unused_fails(self):
        for expiry in ('2026-10-20', '2028-01-01'):
            self.policy['dispositions'][0]['expires'] = expiry; self.save()
            with self.subTest(expiry=expiry), self.assertRaisesRegex(ValueError, 'expired/overlong'):
                d.load_policy(self.root, today=date(2026, 10, 20))

    def test_unrelated_lock_formatting_does_not_renew_or_invalidate_review(self):
        p = self.root / 'Cargo.lock'; original = p.read_bytes(); p.write_bytes(original + b'\n')
        self.assertEqual(d.load_policy(self.root, today=date(2026, 10, 20)), self.policy)
        p.write_bytes(original); self.policy['dispositions'][0]['uses'].pop('call_sites'); self.save()
        with self.assertRaises(ValueError): d.load_policy(self.root, today=date(2026, 10, 20))

    def test_affected_cargo_version_checksum_and_parent_changes_fail(self):
        p = self.root / 'Cargo.lock'; original = p.read_text()
        record = self.policy['dispositions'][0]
        for before, after in ((record['checksum'], '0' * 64),
                              ('name = "rsa"\nversion = "0.9.10"', 'name = "rsa"\nversion = "0.9.11"'),
                              ('"rsa 0.9.10",', '"rsa 0.10.0-rc.18",')):
            self.assertIn(before, original)
            p.write_text(original.replace(before, after))
            with self.subTest(change=before), self.assertRaises(ValueError):
                d.load_policy(self.root, today=date(2026, 10, 20))
        p.write_text(original)

    def test_unrelated_cargo_entry_and_workspace_release_version_are_not_review_bindings(self):
        p = self.root / 'Cargo.lock'; original = p.read_text()
        p.write_text(original.replace('version = "0.73.0"', 'version = "0.74.0"') +
                     '\n[[package]]\nname = "unrelated-fixture"\nversion = "1.0.0"\n')
        self.assertEqual(d.load_policy(self.root, today=date(2026, 10, 20)), self.policy)

    def test_manifest_release_number_is_independent_but_crypto_context_is_exact(self):
        name = 'backend/Cargo.toml'; path = self.root / name; path.parent.mkdir()
        original = (ROOT / name).read_text(); path.write_text(original)
        fingerprint = d.manifest_fingerprint(self.root, name)
        self.assertEqual(fingerprint, self.policy['dispositions'][0]['uses']['manifest_files'][name])
        path.write_text(original.replace('version = "0.73.0"', 'version = "0.74.0"', 1))
        self.assertEqual(d.manifest_fingerprint(self.root, name), fingerprint)
        for before, after in (('"aws_lc_rs"', '"rust_crypto"'),
                              ('rsa = { version = "0.9.10"', 'rsa = { version = "0.10.0"'),
                              ('name = "nyxid"', 'name = "different-app"')):
            self.assertIn(before, original)
            path.write_text(original.replace(before, after, 1))
            with self.subTest(change=before):
                if before.startswith('name'):
                    with self.assertRaises(ValueError): d.manifest_fingerprint(self.root, name)
                else:
                    self.assertNotEqual(d.manifest_fingerprint(self.root, name), fingerprint)

    def test_missing_moved_or_unbound_manifest_policy_fails(self):
        original = copy.deepcopy(self.policy['dispositions'][0]['uses']['manifest_files'])
        for mutation in ('missing', 'moved', 'unbound'):
            files = copy.deepcopy(original)
            if mutation == 'missing': files.pop('backend/Cargo.toml')
            elif mutation == 'moved': files['other/Cargo.toml'] = files.pop('backend/Cargo.toml')
            else: files['backend/Cargo.toml'] = '*'
            self.policy['dispositions'][0]['uses']['manifest_files'] = files; self.save()
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, 'exact RSA manifest binding'):
                d.load_policy(self.root, today=date(2026, 10, 20))

    def test_new_rsa_import_alias_or_private_operation_fails(self):
        source = self.root / 'backend/src/main.rs'; source.parent.mkdir(parents=True)
        source.write_text('fn existing() {}\n')
        policy = dict(rsa_guard=d.rsa_surface(self.root))
        for text in ('use rsa::RsaPrivateKey;\n', 'use rsa as alternate;\n',
                     'fn new_path(k: Key) { k.decrypt_blinded(rng, padding, ciphertext); }\n',
                     'fn new_path(k: Key) { k.sign(message); }\n',
                     'fn new_path(k: Key) { k.try_sign_digest_with_rng(rng, digest); }\n',
                     'fn new_path(k: Key) { k.sign_prehash_with_rng(rng, digest); }\n'):
            with self.subTest(text=text):
                source.write_text(text)
                with self.assertRaisesRegex(ValueError, 'surface changed'): d.validate_rsa_surface(self.root, policy)

    def test_duplicate_existing_private_operation_cannot_inherit_approval(self):
        source = self.root / 'cli/src/main.rs'; source.parent.mkdir(parents=True)
        source.write_text('key.sign(message);\n')
        policy = dict(rsa_guard=d.rsa_surface(self.root))
        source.write_text('key.sign(message);\nkey.sign(message);\n')
        with self.assertRaises(ValueError): d.validate_rsa_surface(self.root, policy)

    def test_reviewed_aes_integration_does_not_grant_new_private_operations(self):
        source = self.root / 'cli/src/node/secret_backend.rs'; source.parent.mkdir(parents=True)
        source.write_text('let value = file.decrypt(encoded)?;\n')
        policy = dict(rsa_guard=d.rsa_surface(self.root))
        d.validate_rsa_surface(self.root, policy)
        # A previously reviewed AES call is not an allowlist for another call,
        # even in the same file, nor for a new RSA import using an alias.
        for extra in ('let other = file.decrypt(unreviewed)?;\n', 'use rsa as another;\n'):
            source.write_text('let value = file.decrypt(encoded)?;\n' + extra)
            with self.subTest(extra=extra), self.assertRaisesRegex(
                    ValueError, 'surface changed.*cli/src/node/secret_backend.rs'):
                d.validate_rsa_surface(self.root, policy)

    def test_metadata_verification_preserves_the_callers_target_directory(self):
        environment = dict(CARGO_TARGET_DIR=str(self.root / 'caller-target'), CARGO_HOME='isolated-audit-home')
        with patch.object(d, 'run_required', return_value=subprocess.CompletedProcess([], 0, b'{}')) as run:
            self.assertTrue(d.verify_rsa(self.root, dict(rsa_guard=d.rsa_surface(self.root), dispositions=[]), environment))
        observed = run.call_args.kwargs['env']
        self.assertEqual(observed['CARGO_TARGET_DIR'], environment['CARGO_TARGET_DIR'])
        self.assertNotIn('CARGO_HOME', observed)

    def test_changed_reviewed_rsa_context_fails_even_if_operation_line_is_unchanged(self):
        source = self.root / 'cli/src/main.rs'; source.parent.mkdir(parents=True)
        source.write_text('use rsa::RsaPrivateKey;\nkey.sign(message);\n')
        policy = dict(rsa_guard=d.rsa_surface(self.root))
        source.write_text('use rsa::RsaPrivateKey;\n// changed context\nkey.sign(message);\n')
        with self.assertRaises(ValueError): d.validate_rsa_surface(self.root, policy)

    def test_test_fixture_binds_rsa_lines_and_every_private_operation_not_mongo_cleanup(self):
        source = self.root / 'backend/src/test_utils.rs'; source.parent.mkdir(parents=True)
        original = 'use rsa::RsaPrivateKey;\nkey.sign(message);\nfn cleanup() {}\n'
        source.write_text(original)
        policy = dict(rsa_guard=d.rsa_surface(self.root))
        source.write_text(original.replace('fn cleanup() {}', 'fn cleanup() { session.end(); }'))
        d.validate_rsa_surface(self.root, policy)
        for extra in ('use rsa::RsaPublicKey;\n', 'key.sign(message);\n', 'key.decrypt(input);\n'):
            source.write_text(original + extra)
            with self.subTest(extra=extra), self.assertRaisesRegex(ValueError, 'surface changed'):
                d.validate_rsa_surface(self.root, policy)

    def test_refresh_reads_head_objects_not_dirty_files_or_index_and_preserves_dispositions(self):
        def git(*args):
            return subprocess.check_output(['git', *args], cwd=self.root, stderr=subprocess.PIPE, text=True).strip()
        git('init', '-q'); git('config', 'user.name', 'Guard test'); git('config', 'user.email', 'guard@example.invalid')
        source = self.root / 'backend/src/main.rs'; source.parent.mkdir(parents=True)
        source.write_text('use rsa::RsaPublicKey;\nkey.sign(message);\n')
        git('add', 'backend/src/main.rs'); git('commit', '-qm', 'reviewed source')
        commit = git('rev-parse', 'HEAD'); expected = d.rsa_surface(self.root)
        source.write_text('use rsa::RsaPrivateKey;\nkey.decrypt(unreviewed);\n')
        git('add', 'backend/src/main.rs')
        source.write_text('key.try_sign_digest(unreviewed);\n')
        other = self.root / 'cli/src/new.rs'; other.parent.mkdir(parents=True); other.write_text('key.sign(new);\n')
        before = copy.deepcopy(self.policy)
        with patch('sys.stdout', new_callable=io.StringIO) as output:
            observed_commit, observed = d.refresh_rsa_guard(self.root)
        self.assertEqual(observed_commit, commit); self.assertEqual(observed, expected)
        updated = json.loads((self.root / d.POLICY).read_text())
        self.assertEqual(updated, {**before, 'rsa_guard': expected})
        self.assertIn(commit, output.getvalue()); self.assertIn('reference_files:backend/src/main.rs', output.getvalue())
        self.assertIn(d.sha(b'use rsa::RsaPublicKey;\nkey.sign(message);\n'), output.getvalue())
        self.assertIn('before=', output.getvalue()); self.assertIn('after=', output.getvalue())
        self.assertEqual(source.read_text(), 'key.try_sign_digest(unreviewed);\n')
        self.assertIn('unreviewed', git('show', ':backend/src/main.rs'))
        updated['rsa_guard'] = {}; (self.root / d.POLICY).write_text(json.dumps(updated))
        original = (self.root / d.POLICY).read_bytes()
        with patch('sys.stdout', new_callable=io.StringIO): d.refresh_rsa_guard(self.root, write=False)
        self.assertEqual((self.root / d.POLICY).read_bytes(), original)

    def metadata(self):
        return dict(packages=[dict(id='rsa', name='rsa', version='0.9.10'), dict(id='backend', name='nyxid', version='0.72.0')],
                    workspace_members=['backend'], resolve=dict(nodes=[dict(id='rsa', features=['pem', 'std'], deps=[]),
                    dict(id='backend', features=[], deps=[dict(pkg='rsa', dep_kinds=[dict(kind=None)])])]))

    def test_parent_chain_and_selected_features_are_observed_from_resolved_cargo_graph(self):
        metadata = self.metadata(); graph = d.rsa_graph(metadata, '0.9.10')
        self.assertEqual(graph, dict(features=['pem', 'std'], parents=['rsa@0.9.10 <- nyxid@workspace[normal]']))
        metadata['packages'][1]['version'] = '0.73.0'
        self.assertEqual(graph, d.rsa_graph(metadata, '0.9.10'))
        metadata['resolve']['nodes'][0]['features'].append('new-private-mode')
        self.assertNotEqual(graph, d.rsa_graph(metadata, '0.9.10'))
        metadata['resolve']['nodes'][1]['deps'][0]['dep_kinds'][0]['kind'] = 'dev'
        self.assertNotEqual(graph['parents'], d.rsa_graph(metadata, '0.9.10')['parents'])

    def test_unreviewed_braces_parent_or_version_is_rejected(self):
        lock = load_unique((ROOT / 'mobile/pnpm-lock.yaml').read_text())
        current = d.tooling_parents(lock)
        lock['snapshots']['runtime-consumer@1.0.0'] = dict(dependencies=dict(micromatch='4.0.8'))
        self.assertNotEqual(current, d.tooling_parents(lock))
        lock['snapshots']['runtime-consumer@1.0.0']['dependencies'] = dict(braces='3.0.3')
        with self.assertRaises(ValueError): d.tooling_parents(lock)
        lock['snapshots']['braces@3.0.4'] = {}
        with self.assertRaises(ValueError): d.tooling_parents(lock)

    def test_rsa_advisory_package_version_checksum_lock_and_executed_proof_are_exact(self):
        r = self.policy['dispositions'][0]
        report = dict(vulnerabilities=dict(list=[dict(advisory=dict(id=r['advisory']), package=dict(name='rsa', version=r['version'], checksum=r['checksum']))]))
        self.assertEqual(len(d.classify_cargo(report, 'Cargo.lock', self.policy, True)), 1)
        self.assertEqual(d.classify_cargo(report, 'Cargo.lock', self.policy, False), [])
        self.assertEqual(d.classify_cargo(report, 'other/Cargo.lock', self.policy, True), [])
        for field, value in [('version', '0.9.9'), ('name', 'new-package')]:
            changed = copy.deepcopy(report); changed['vulnerabilities']['list'][0]['package'][field] = value
            self.assertEqual(d.classify_cargo(changed, 'Cargo.lock', self.policy, True), [])
        changed = copy.deepcopy(report); changed['vulnerabilities']['list'][0]['advisory']['id'] = 'RUSTSEC-2099-0001'
        self.assertEqual(d.classify_cargo(changed, 'Cargo.lock', self.policy, True), [])
        changed = copy.deepcopy(report); changed['vulnerabilities']['list'][0]['package']['checksum'] = '0' * 64
        with self.assertRaises(ValueError): d.classify_cargo(changed, 'Cargo.lock', self.policy, True)

    def test_braces_requires_exact_advisory_and_live_bundle_proof(self):
        r = self.policy['dispositions'][-1]
        report = dict(advisories={'1': dict(module_name='braces', github_advisory_id=r['advisory'], findings=[dict(version='3.0.3')])})
        self.assertEqual(d.classify_tooling(report, r['lock'], self.policy, False), [])
        self.assertEqual(len(d.classify_tooling(report, r['lock'], self.policy, True)), 1)
        report['advisories']['1']['github_advisory_id'] = 'GHSA-aaaa-bbbb-cccc'
        self.assertEqual(d.classify_tooling(report, r['lock'], self.policy, True), [])

    def tooling_inputs(self):
        directory = self.root / 'clean-install'; directory.mkdir()
        shutil.copyfile(self.root / 'mobile/pnpm-lock.yaml', directory / 'pnpm-lock.yaml')
        checker = self.root / 'scripts/check-mobile-bundle.cjs'
        checker.parent.mkdir(); shutil.copyfile(ROOT / checker.relative_to(self.root), checker)
        return directory, self.policy['dispositions'][-1]

    def test_new_upstream_braces_release_requires_review_before_bundle_execution(self):
        directory, record = self.tooling_inputs()
        with patch('urllib.request.urlopen', return_value=io.BytesIO(b'{"version":"3.0.4"}')), \
                patch.object(d, 'run_required') as run, self.assertRaisesRegex(ValueError, 'new braces release'):
            d.verify_tooling(self.root, directory, record, {})
        run.assert_not_called()

    def test_missing_changed_or_failed_bundle_checker_cannot_grant_tooling_disposition(self):
        directory, record = self.tooling_inputs()
        def run(command, **kwargs):
            if command[0] == 'git': return subprocess.CompletedProcess(command, 0, b'', b'')
            raise ValueError('actual production export failed')
        with patch('urllib.request.urlopen', return_value=io.BytesIO(b'{"version":"3.0.3"}')), \
                patch.object(d, 'run_required', side_effect=run), self.assertRaisesRegex(ValueError, 'production export failed'):
            d.verify_tooling(self.root, directory, record, {})
        checker = self.root / 'scripts/check-mobile-bundle.cjs'
        checker.write_bytes(checker.read_bytes() + b'\n')
        with self.assertRaisesRegex(ValueError, 'bound file'): d.verify_tooling(self.root, directory, record, {})
        checker.unlink()
        with self.assertRaisesRegex(ValueError, 'bound file'): d.verify_tooling(self.root, directory, record, {})


if __name__ == '__main__': unittest.main()
