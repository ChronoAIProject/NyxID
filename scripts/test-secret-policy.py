#!/usr/bin/env python3
"""Adversarial exception tests using the real pinned secret detector."""
from datetime import date, timedelta
import copy
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import tempfile
import unittest

from secret_policy import IDENTITY, disposition, finding_identity, load_exceptions, normalized_path

ROOT = Path(__file__).resolve().parents[1]


class SecretPolicy(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.policy_path = self.root / 'exceptions.json'
        self.entry = next(e for e in load_exceptions(ROOT / 'security/secret-exceptions.json')
                          if e['path'] == 'mobile/google-services.json')
        self.scanner = Path(os.environ['GITLEAKS_BIN']).resolve()

    def detect(self, content, name='mobile/google-services.json'):
        tree = self.root / 'tree'
        if tree.exists(): shutil.rmtree(tree)
        path = tree / name
        path.parent.mkdir(parents=True)
        path.write_bytes(content)
        report = self.root / 'raw-private.json'
        report.touch(mode=0o600)
        command = [str(self.scanner), 'dir', str(tree), '--config', str(ROOT / 'security/gitleaks.toml'),
                   '--gitleaks-ignore-path', str(self.root), '--ignore-gitleaks-allow', '--no-banner',
                   '--log-level', 'error', '--report-format', 'json', '--report-path', str(report)]
        result = subprocess.run(command, cwd=self.root, capture_output=True)
        self.assertIn(result.returncode, (0, 1))
        findings = json.loads(report.read_text())
        identities = []
        for item in findings:
            item['File'] = str(Path(item['File']).relative_to(tree))
            identities.append(finding_identity(item, content))
        return identities

    def firebase(self):
        return (ROOT / 'mobile/google-services.json').read_bytes()

    def test_reviewed_firebase_content_is_exactly_the_public_configuration(self):
        findings = self.detect(self.firebase())
        self.assertEqual(len(findings), 1)
        self.assertEqual(disposition(findings[0], [self.entry]), self.entry)

    def test_second_key_in_same_file_is_never_covered(self):
        content = json.loads(self.firebase())
        content['client'][0]['api_key'].append({'current_key': 'AIza' + secrets.token_urlsafe(27)[:35]})
        findings = self.detect((json.dumps(content, indent=2) + '\n').encode())
        self.assertEqual(len(findings), 2)
        self.assertTrue(any(disposition(item, [self.entry]) is None for item in findings))

    def test_changed_key_fails(self):
        content = json.loads(self.firebase())
        content['client'][0]['api_key'][0]['current_key'] = 'AIza' + secrets.token_urlsafe(27)[:35]
        findings = self.detect((json.dumps(content, indent=2) + '\n').encode())
        self.assertEqual(len(findings), 1)
        self.assertIsNone(disposition(findings[0], [self.entry]))

    def test_changed_surrounding_content_fails(self):
        findings = self.detect(self.firebase().replace(b'"current_key": ', b'"current_key" : '))
        self.assertEqual(len(findings), 1)
        self.assertIsNone(disposition(findings[0], [self.entry]))

    def test_moved_path_fails(self):
        findings = self.detect(self.firebase(), 'mobile/copied-google-services.json')
        self.assertEqual(len(findings), 1)
        self.assertIsNone(disposition(findings[0], [self.entry]))

    def test_wrong_detector_fails_even_for_identical_path_and_bytes(self):
        finding = self.detect(self.firebase())[0]
        finding['rule'] = 'generic-api-key'
        self.assertIsNone(disposition(finding, [self.entry]))

    def test_expired_unused_exception_fails_global_gate(self):
        entry = copy.deepcopy(self.entry)
        entry['expires'] = date.today().isoformat()
        self.policy_path.write_text(json.dumps(dict(schema_version=1, exceptions=[entry])))
        with self.assertRaisesRegex(ValueError, 'expired'): load_exceptions(self.policy_path)

    def test_owner_expiry_rationale_and_provenance_are_mandatory(self):
        for field in ('owner', 'expires', 'rationale', 'provenance'):
            entry = copy.deepcopy(self.entry)
            entry.pop(field)
            self.policy_path.write_text(json.dumps(dict(schema_version=1, exceptions=[entry])))
            with self.subTest(field=field), self.assertRaises(ValueError): load_exceptions(self.policy_path)

    def test_broad_path_rule_and_content_exceptions_are_impossible(self):
        for field, value in (('path', 'mobile/**'), ('rule', '*'), ('content_sha256', '.*')):
            entry = copy.deepcopy(self.entry)
            entry[field] = value
            self.policy_path.write_text(json.dumps(dict(schema_version=1, exceptions=[entry])))
            with self.subTest(field=field), self.assertRaises(ValueError): load_exceptions(self.policy_path)

    def test_every_reviewed_exception_requires_all_five_identity_fields(self):
        entries = load_exceptions(ROOT / 'security/secret-exceptions.json')
        for entry in entries:
            identity = {key: entry[key] for key in IDENTITY}
            context = dict(scope='history', commit=entry['history_commits'][0]) if 'history_commits' in entry else {}
            self.assertEqual(disposition(identity, entries, **context), entry)
            for field in IDENTITY:
                changed = dict(identity)
                changed[field] = ('unreviewed-rule' if field == 'rule' else
                                  'unreviewed/' + identity[field] if field == 'path' else '0' * 64)
                with self.subTest(path=entry['path'], field=field):
                    self.assertIsNone(disposition(changed, entries, **context))

    def test_inert_historical_kind_requires_all_reviewed_code_evidence(self):
        entries = load_exceptions(ROOT / 'security/secret-exceptions.json')
        inert = next(e for e in entries if e['kind'] == 'inert-historical-credential')
        for field in [None, *inert['code_evidence']]:
            entry = copy.deepcopy(inert)
            if field is None: entry.pop('code_evidence')
            else: entry['code_evidence'].pop(field)
            self.policy_path.write_text(json.dumps(dict(schema_version=1, exceptions=[entry])))
            with self.subTest(field=field), self.assertRaises(ValueError): load_exceptions(self.policy_path)
        for field, value in [('path', 'docs/API.md'), ('value_sha256', '0' * 64), ('kind', 'unknown-kind')]:
            entry = copy.deepcopy(inert); entry[field] = value
            self.policy_path.write_text(json.dumps(dict(schema_version=1, exceptions=[entry])))
            with self.subTest(field=field), self.assertRaises(ValueError): load_exceptions(self.policy_path)

    def test_annual_history_disposition_cannot_apply_to_current_tree_or_other_commit(self):
        entry = next(e for e in load_exceptions(ROOT / 'security/secret-exceptions.json') if 'history_commits' in e)
        identity = {key: entry[key] for key in IDENTITY}
        self.assertIsNone(disposition(identity, [entry]))
        self.assertIsNone(disposition(identity, [entry], scope='tree', commit=entry['history_commits'][0]))
        self.assertIsNone(disposition(identity, [entry], scope='history', commit='0' * 40))
        self.assertEqual(disposition(identity, [entry], scope='history', commit=entry['history_commits'][0]), entry)

    def test_expiry_cadence_is_monthly_for_tree_and_annual_for_immutable_history(self):
        entry = copy.deepcopy(self.entry); entry['expires'] = (date.today() + timedelta(days=32)).isoformat()
        self.policy_path.write_text(json.dumps(dict(schema_version=1, exceptions=[entry])))
        with self.assertRaisesRegex(ValueError, 'cadence'): load_exceptions(self.policy_path)
        entry['history_commits'] = ['1' * 40]
        entry['expires'] = (date.today() + timedelta(days=365)).isoformat()
        self.policy_path.write_text(json.dumps(dict(schema_version=1, exceptions=[entry])))
        self.assertEqual(load_exceptions(self.policy_path), [entry])
        entry['expires'] = (date.today() + timedelta(days=367)).isoformat()
        self.policy_path.write_text(json.dumps(dict(schema_version=1, exceptions=[entry])))
        with self.assertRaisesRegex(ValueError, 'cadence'): load_exceptions(self.policy_path)

    def test_path_normalization_cannot_escape_or_reopen_absolute_paths(self):
        self.assertEqual(normalized_path('./mobile//google-services.json'), 'mobile/google-services.json')
        for path in ('/mobile/google-services.json', '../mobile/google-services.json', 'mobile/../google-services.json',
                     'mobile\\google-services.json', ''):
            with self.subTest(path=path), self.assertRaises(ValueError): normalized_path(path)

    def test_same_file_generic_secret_and_inline_allow_are_discovered(self):
        content = b'provider_api_key = "' + secrets.token_hex(24).encode() + b'" # gitleaks:allow\n'
        findings = self.detect(content, 'docs/API.md')
        self.assertTrue(findings)
        self.assertTrue(all(disposition(item, [self.entry]) is None for item in findings))

    def test_inventory_progress_prose_is_exact_and_cannot_cover_a_new_key(self):
        name = 'docs/SECURITY_W8_INVENTORY.md'
        content = (ROOT / name).read_bytes()
        entries = load_exceptions(ROOT / 'security/secret-exceptions.json')
        findings = self.detect(content, name)
        self.assertEqual(len(findings), 1)
        self.assertEqual(disposition(findings[0], entries)['kind'], 'non-secret-identifier')
        # Include the new key on the reviewed line: its full-content binding
        # must refuse both the changed prose finding and the new credential.
        content = content.replace(b'undeclared/missing/mixed/stale',
                                  b'undeclared/missing/mixed/stale provider_api_key = "' +
                                  secrets.token_hex(24).encode() + b'"')
        changed = self.detect(content, name)
        self.assertGreaterEqual(len(changed), 2)
        self.assertTrue(all(disposition(item, entries) is None for item in changed))

    def test_mock_keychain_fixture_is_exact_and_never_covers_a_new_secret(self):
        name = 'cli/src/node/agent.rs'
        source = subprocess.check_output(['git', 'show', 'HEAD:' + name], cwd=ROOT)
        content = next(line for line in source.splitlines(keepends=True)
                       if b'.set("signing_secret", ' in line)
        entries = load_exceptions(ROOT / 'security/secret-exceptions.json')
        findings = self.detect(content, name)
        self.assertEqual(len(findings), 1)
        self.assertEqual(disposition(findings[0], entries)['kind'], 'synthetic-test-fixture')
        changed = content.rstrip(b'\n') + b' provider_api_key = "' + secrets.token_hex(24).encode() + b'"\n'
        changed_findings = self.detect(changed, name)
        self.assertGreaterEqual(len(changed_findings), 2)
        self.assertTrue(all(disposition(item, entries) is None for item in changed_findings))
        changed_value = content.replace(content.split(b'"')[3], secrets.token_hex(16).encode())
        for data, path in ((content, 'cli/src/node/other.rs'), (changed_value, name)):
            with self.subTest(path=path):
                findings = self.detect(data, path)
                self.assertTrue(findings)
                self.assertTrue(all(disposition(item, entries) is None for item in findings))

    def test_hash_only_policy_metadata_is_not_a_credential_but_new_policy_file_key_is_detected(self):
        content = (ROOT / 'security/secret-exceptions.json').read_bytes()
        self.assertEqual(self.detect(content, 'security/secret-exceptions.json'), [])
        content += b'\nprovider_api_key = "' + secrets.token_hex(24).encode() + b'"\n'
        findings = self.detect(content, 'security/secret-exceptions.json')
        self.assertTrue(findings)
        entries = load_exceptions(ROOT / 'security/secret-exceptions.json')
        self.assertTrue(all(disposition(item, entries) is None for item in findings))

    def test_deleted_secret_on_another_branch_still_blocks_and_is_not_printed(self):
        repository = self.root / 'repository'
        repository.mkdir()
        def git(*args):
            return subprocess.check_output(['git', *args], cwd=repository, stderr=subprocess.PIPE)
        git('init', '-b', 'main')
        git('config', 'user.email', 'scanner-fixture@example.invalid')
        git('config', 'user.name', 'Scanner fixture')
        git('config', 'commit.gpgsign', 'false')
        (repository / 'security').mkdir()
        shutil.copyfile(ROOT / 'security/gitleaks.toml', repository / 'security/gitleaks.toml')
        (repository / 'security/secret-exceptions.json').write_text('{"schema_version":1,"exceptions":[]}\n')
        git('add', 'security')
        git('commit', '-m', 'scanner fixture')
        git('checkout', '-b', 'removed-fixture')
        dummy = secrets.token_hex(24)
        (repository / 'removed.txt').write_text('provider_api_key = "' + dummy + '" # gitleaks:allow\n')
        git('add', 'removed.txt')
        git('commit', '-m', 'temporary synthetic credential')
        git('rm', 'removed.txt')
        git('commit', '-m', 'remove fixture')
        git('checkout', 'main')
        output = self.root / 'safe-report'
        result = subprocess.run(['python3', str(ROOT / 'scripts/scan-secrets.py'), '--root', str(repository),
                                 '--scanner', str(self.scanner), '--output', str(output)], capture_output=True, text=True,
                                env={**os.environ, 'GIT_DIR': str(self.root / 'absent.git'),
                                     'GIT_CONFIG_COUNT': '1', 'GIT_CONFIG_KEY_0': 'safe.directory',
                                     'GIT_CONFIG_VALUE_0': '*', 'GITLEAKS_CONFIG': str(self.root / 'absent.toml')})
        self.assertEqual(result.returncode, 1, result.stderr)
        report = (output / 'secret-scan.json').read_text()
        self.assertNotIn(dummy, result.stdout + result.stderr + report)
        data = json.loads(report)
        self.assertEqual(data['counts']['tree'], 0)
        self.assertGreater(data['counts']['unresolved'], 0)
        self.assertTrue(all(item['scope'] == 'history' and item['disposition'] == 'UNRESOLVED' for item in data['findings']))


if __name__ == '__main__':
    unittest.main()
