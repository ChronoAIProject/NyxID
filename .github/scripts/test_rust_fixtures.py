"""Keep tool discovery offline and Rust fixtures outside Detect Changes."""
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

from ci_gate_reuse import strict_load_workflow

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import rust_fixture


class PrerequisiteTests(unittest.TestCase):
    def setUp(self):
        env = patch.dict(os.environ, NYXID_SKIP_RUST_FIXTURES='0', NYXID_REQUIRE_RUST_FIXTURES='0')
        env.start()
        self.addCleanup(env.stop)

    def test_missing_binary_never_invokes_rustup(self):
        for absent in ('cargo', 'rustup', 'rustc', 'cargo-llvm-cov'):
            with self.subTest(absent=absent), \
                 patch('rust_fixture.shutil.which', side_effect=lambda name: None if name == absent else name), \
                 patch('rust_fixture.subprocess.check_output') as run:
                self.assertIn(absent, rust_fixture.missing_prerequisite(tools=('cargo-llvm-cov',)))
                run.assert_not_called()

    def test_missing_pin_never_queries_components_or_runs_a_proxy(self):
        with patch('rust_fixture.shutil.which', return_value='/fixture/tool'), \
             patch('rust_fixture.subprocess.check_output', return_value='stable-x86_64-unknown-linux-gnu\n') as run:
            self.assertIn('installed Rust 1.98.1', rust_fixture.missing_prerequisite())
            self.assertEqual([call.args[0] for call in run.call_args_list],
                             [['rustup', 'toolchain', 'list', '--quiet']])

    def test_requires_installed_components_even_when_proxies_exist(self):
        for component in ('clippy', 'llvm-tools'):
            with self.subTest(component=component), \
                 patch('rust_fixture.shutil.which', return_value='/fixture/tool'), \
                 patch('rust_fixture.subprocess.check_output', side_effect=[
                     '1.98.1-x86_64-unknown-linux-gnu\n',
                     'cargo-x86_64-unknown-linux-gnu\nrustc-x86_64-unknown-linux-gnu\nrust-std-x86_64-unknown-linux-gnu\n']) as run:
                self.assertIn(component, rust_fixture.missing_prerequisite(components=(component,)))
                self.assertEqual(run.call_args.args[0],
                                 ['rustup', 'component', 'list', '--toolchain', '1.98.1', '--installed'])

    def test_installed_prerequisites_and_noninstalling_execution(self):
        with patch('rust_fixture.shutil.which', return_value='/fixture/tool'), \
             patch('rust_fixture.subprocess.check_output', side_effect=[
                 'stable-aarch64-apple-darwin\n1.98.1-aarch64-apple-darwin\n',
                 'cargo-aarch64-apple-darwin\nrustc-aarch64-apple-darwin\n'
                 'rust-std-aarch64-apple-darwin\nclippy-aarch64-apple-darwin\nllvm-tools-aarch64-apple-darwin\n']):
            self.assertIsNone(rust_fixture.missing_prerequisite(
                components=('clippy', 'llvm-tools'), tools=('cargo-nextest', 'cargo-llvm-cov')))
        self.assertEqual(rust_fixture.rust_command('cargo', 'test'), ['rustup', 'run', '1.98.1', 'cargo', 'test'])
        self.assertEqual(rust_fixture.rust_environment()['RUSTUP_AUTO_INSTALL'], '0')
        self.assertEqual(rust_fixture.rust_environment()['CARGO_NET_OFFLINE'], 'true')

    def test_detection_opt_out_never_probes_tools(self):
        with patch.dict(os.environ, NYXID_SKIP_RUST_FIXTURES='1'), \
             patch('rust_fixture.shutil.which') as which, patch('rust_fixture.subprocess.check_output') as run:
            self.assertIn('disabled', rust_fixture.missing_prerequisite())
            which.assert_not_called()
            run.assert_not_called()

    def test_inventory_error_is_a_missing_prerequisite(self):
        for error in (OSError('missing'), subprocess.CalledProcessError(1, 'rustup')):
            with patch('rust_fixture.shutil.which', return_value='/fixture/tool'), \
                 patch('rust_fixture.subprocess.check_output', side_effect=error):
                self.assertIn('cannot inspect', rust_fixture.missing_prerequisite())

    def test_missing_prerequisites_skip_locally_but_fail_mandatory_jobs(self):
        class Fixture(unittest.TestCase):
            @rust_fixture.requires_rust(components=('clippy',))
            def runTest(self):
                raise AssertionError('fixture body must not run')
        for required in ('0', '1'):
            with patch.dict(os.environ, NYXID_REQUIRE_RUST_FIXTURES=required), \
                 patch('rust_fixture.missing_prerequisite', return_value='missing clippy'):
                result = unittest.TestResult()
                Fixture().run(result)
                self.assertEqual(len(result.skipped), int(required == '0'))
                self.assertEqual(len(result.failures), int(required == '1'))
                self.assertEqual(result.errors, [])


class WorkflowFixtureTests(unittest.TestCase):
    def test_changes_never_compiles_and_each_fixture_has_a_mandatory_caller(self):
        ci = strict_load_workflow((ROOT / '.github/workflows/ci.yml').read_text())
        backend = strict_load_workflow((ROOT / '.github/workflows/backend-tests.yml').read_text())
        changes = ci['jobs']['changes']['steps']
        helpers = next(step for step in changes if step.get('name') == 'Test CI helpers')
        self.assertEqual(helpers['env']['NYXID_SKIP_RUST_FIXTURES'], '1')
        self.assertEqual(helpers['run'], "python3 -m unittest discover -s .github/scripts -p 'test_*.py'")
        self.assertFalse(any('rust-toolchain@' in step.get('uses', '') for step in changes))
        self.assertIn("'scripts/rust_fixture.py'", next(step for step in changes if step.get('id') == 'filter')['with']['filters'])
        for job, filename, components in (
            (ci['jobs']['rust-clippy'], 'test_incremental_cache.py', 'clippy'),
            (ci['jobs']['rust-clippy'], 'test_codeql_cfg_test_files.py', 'clippy'),
            (ci['jobs']['rust-clippy'], 'test_git_build_scripts.py', 'clippy'),
            (ci['jobs']['backend-image-inputs'], 'test_backend_docker_embeds.py', ''),
            (backend['jobs']['build'], 'test_coverage_archive_cache.py', 'llvm-tools-preview'),
        ):
            with self.subTest(fixture=filename):
                steps = job['steps']
                test = next(step for step in steps if filename in step.get('run', ''))
                install = next(step for step in steps if step.get('uses') == f'dtolnay/rust-toolchain@{rust_fixture.TOOLCHAIN}')
                self.assertLess(steps.index(install), steps.index(test))
                self.assertIn(components, install.get('with', {}).get('components', ''))
                self.assertEqual(test['env']['NYXID_REQUIRE_RUST_FIXTURES'], '1')
                for owner in (ci, backend, job, test):
                    self.assertNotIn('NYXID_SKIP_RUST_FIXTURES', owner.get('env', {}))
                self.assertFalse(test.get('continue-on-error', False))
                if filename == 'test_coverage_archive_cache.py':
                    self.assertEqual(test['if'], 'inputs.coverage')
                    tools = next(step for step in steps if step.get('uses') == 'taiki-e/install-action@v2')
                    self.assertLess(steps.index(tools), steps.index(test))
                    self.assertIn('cargo-nextest@0.9.146', tools['with']['tool'])
                    self.assertIn('cargo-llvm-cov@0.9.1', tools['with']['tool'])
                else:
                    self.assertNotIn('if', test)


if __name__ == '__main__':
    unittest.main()
