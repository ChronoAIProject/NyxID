#!/usr/bin/env python3
"""Local smoke-consumer regression; no image, browser or hosted acceptance."""
import ast
import copy
import pathlib
import unittest

# Load the actual consumer without starting its PID-1 server/Node fixture.
source = pathlib.Path(__file__).with_name('full-machine-image-smoke.py')
tree = ast.parse(source.read_text())
helper = next(node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == 'assert_separation_report')
namespace = {}
exec(compile(ast.Module(body=[helper], type_ignores=[]), str(source), 'exec'), namespace)
assert_report = namespace['assert_separation_report']


class SeparationReportContract(unittest.TestCase):
    def setUp(self):
        self.storage = {'original_pvc_uid': 'fixture-pvc', 'allocation_id': 'fixture-allocation'}
        self.config = {'hosted_storage': self.storage, 'machine': {'separation_choice': 'shared_legacy'}}
        self.report = {
            'separation_choice': 'shared_legacy',
            'separated': {'available': True, 'landlock_abi': 6, 'reason': None},
            'separation_evidence': {
                'version': 4, 'os': 'linux', 'landlock_abi': 6,
                **{key: True for key in ['separate_users', 'credential_custody',
                    'managed_browser_configured', 'command_seccomp', 'landlock_enforced',
                    'managed_browser_sandbox', 'managed_browser_policy']},
                'probe_timed_out': False,
                'browser_sandbox_unobservable_in_container': False,
            },
        }

    def test_current_shared_choice_can_report_full_v4_support_after_store_grant(self):
        assert_report(self.report, self.config, self.storage)

    def test_installed_store_must_match_authenticated_grant(self):
        for invalid in [None, {}, dict(self.storage, original_pvc_uid='other-pvc')]:
            with self.subTest(storage=invalid), self.assertRaises(AssertionError):
                assert_report(self.report, dict(self.config, hosted_storage=invalid), self.storage)

    def test_complete_evidence_cannot_report_obsolete_storage_unavailable(self):
        self.report['separated'].update(available=False, reason='hosted_context_binding_required')
        with self.assertRaises(AssertionError):
            assert_report(self.report, self.config, self.storage)

    def test_each_missing_or_failed_v4_probe_requires_unavailable_support(self):
        variants = [(field, value) for field in ['separate_users', 'credential_custody',
                    'managed_browser_configured', 'command_seccomp', 'landlock_enforced',
                    'managed_browser_sandbox', 'managed_browser_policy'] for value in [None, False]]
        variants += [('version', 3), ('version', 5), ('os', 'macos'), ('landlock_abi', 5),
                     ('probe_timed_out', True), ('browser_sandbox_unobservable_in_container', True)]
        for field, value in variants:
            with self.subTest(field=field, value=value):
                report = copy.deepcopy(self.report)
                report['separation_evidence'][field] = value
                with self.assertRaises(AssertionError):
                    assert_report(report, self.config, self.storage)
                report['separated'].update(available=False, reason='fixture-probe-unavailable')
                report['separated']['landlock_abi'] = report['separation_evidence']['landlock_abi']
                assert_report(report, self.config, self.storage)
                report['separated']['reason'] = None
                with self.assertRaises(AssertionError):
                    assert_report(report, self.config, self.storage)


if __name__ == '__main__':
    unittest.main(verbosity=2)
