#!/usr/bin/env python3
from pathlib import Path
import itertools
import importlib.util
import json
import os
import subprocess
import unittest
import yaml
from workflow_yaml import RUST_ACTION, load_unique, validate_rust_steps

spec = importlib.util.spec_from_file_location('workflow_permissions',
    Path(__file__).with_name('check-reusable-workflow-permissions.py'))
workflow_permissions = importlib.util.module_from_spec(spec)
spec.loader.exec_module(workflow_permissions)


class WorkflowYaml(unittest.TestCase):
    def test_pr_path_filter_retains_only_its_required_read_scope(self):
        workflow = {'permissions': {'contents': 'read'}, 'jobs': {'changes': {
            'steps': [{'uses': 'dorny/paths-filter@' + 'a' * 40}]}}}
        self.assertEqual(workflow_permissions.validate_permission_scopes(workflow),
                         ['changes: paths-filter requires pull-requests: read for PR file lists'])
        workflow['jobs']['changes']['permissions'] = {'contents': 'read', 'pull-requests': 'read'}
        self.assertEqual(workflow_permissions.validate_permission_scopes(workflow), [])

    def test_job_without_explicit_permissions_is_rejected(self):
        workflow = {'jobs': {'added-job': {'runs-on': 'ubuntu-latest', 'steps': []}}}
        self.assertEqual(workflow_permissions.validate_permission_scopes(workflow),
                         ['added-job: missing explicit permissions at job or workflow level'])
        workflow['jobs']['added-job']['permissions'] = {}
        self.assertEqual(workflow_permissions.validate_permission_scopes(workflow), [])
        del workflow['jobs']['added-job']['permissions']
        workflow['permissions'] = {'contents': 'read'}
        self.assertEqual(workflow_permissions.validate_permission_scopes(workflow), [])

    def test_conditional_step_duplicate_with_rejected(self):
        text = """jobs:
  coverage:
    steps:
      - uses: dtolnay/rust-toolchain@18f0487477426dce244608110aa2660c2893deb7
        with:
          toolchain: 1.98.1
        if: steps.base.outputs.exists == 'true'
        with:
          components: llvm-tools-preview
"""
        with self.assertRaisesRegex(yaml.constructor.ConstructorError, "duplicate mapping key: with"):
            load_unique(text)

    def test_nested_duplicate_permission_rejected(self):
        with self.assertRaisesRegex(yaml.constructor.ConstructorError, "duplicate mapping key: contents"):
            load_unique("jobs: {job: {permissions: {contents: read, contents: write}}}")

    def rust_step(self, inputs, action=RUST_ACTION):
        return {"jobs": {"job": {"steps": [{"uses": action, "with": inputs, "if": "true"}]}}}

    def test_unsupported_toolchain_input_rejected(self):
        errors = validate_rust_steps(self.rust_step({"toolchain": "1.98.1", "components": "rustfmt, clippy"}))
        self.assertTrue(any("unsupported input" in error for error in errors))

    def test_unknown_version_implementation_rejected(self):
        errors = validate_rust_steps(self.rust_step({"components": "rustfmt, clippy"},
                                                  "dtolnay/rust-toolchain@" + "a" * 40))
        self.assertTrue(any("reviewed 1.98.1" in error for error in errors))

    def test_missing_repository_component_rejected(self):
        for components in ("", "clippy", "rustfmt", "${{ matrix.components }}", ["rustfmt", "clippy"]):
            with self.subTest(components=components):
                errors = validate_rust_steps(self.rust_step({"components": components}))
                self.assertTrue(any("during setup" in error for error in errors))

    def test_conditional_single_mapping_accepted(self):
        workflow = self.rust_step({"components": "llvm-tools-preview, clippy, rustfmt",
                                   "targets": "aarch64-unknown-linux-gnu"})
        self.assertEqual(validate_rust_steps(workflow), [])


class CatalogWorkflowBoundary(unittest.TestCase):
    def load(self, name):
        root = Path(__file__).resolve().parents[1]
        return load_unique((root / '.github/workflows' / name).read_text())

    def test_native_dbus_supplement_is_restricted_to_base_coverage(self):
        jobs = self.load('ci.yml')['jobs']
        callers = []
        for name, job in jobs.items():
            for step in job.get('steps', []):
                command = step.get('run', '')
                if '--base-coverage' in command:
                    callers.append(name)
                    self.assertEqual(command, 'bash .github/scripts/install-linux-build-deps.sh --base-coverage')
        self.assertEqual(callers, ['coverage-backend-base'])
        base = jobs['coverage-backend-base']
        checkout = next(step for step in base['steps'] if step.get('name') == 'Checkout base tree')
        self.assertEqual(checkout['with']['ref'], '${{ github.event.pull_request.base.sha }}')
        self.assertEqual(checkout['with']['path'], 'base-coverage')
        install = next(step for step in base['steps'] if step.get('name') == 'Install system dependencies')
        self.assertEqual(install['if'], "steps.base-cache.outputs.cache-hit != 'true'")
        # The trusted PR snapshot helper runs from the root, never the base's
        # older mutable apt-get wrapper. Only the measured source comes from base.
        self.assertNotIn('working-directory', install)

    def test_apk_provisioning_never_checks_out_dispatch_input_or_shares_caches(self):
        workflow = self.load('provision-apk-archives.yml')
        self.assertEqual(set(workflow.get('on', workflow.get(True))), {'workflow_dispatch'})
        prepare, publish = (workflow['jobs'][name] for name in ('prepare', 'publish'))
        self.assertEqual(prepare['permissions'], {'contents': 'read'})
        self.assertEqual(publish['permissions'], {'contents': 'write'})
        self.assertEqual(publish['needs'], 'prepare')
        for job in (prepare, publish):
            self.assertEqual(job['if'], "github.ref == 'refs/heads/main'")
            checkouts = [step for step in job['steps'] if step.get('uses', '').startswith('actions/checkout@')]
            self.assertEqual(len(checkouts), 1)
            self.assertEqual(checkouts[0]['with']['ref'], '${{ github.workflow_sha }}')
            self.assertEqual(checkouts[0]['with']['fetch-depth'], 0)
            self.assertFalse(checkouts[0]['with']['persist-credentials'])
            self.assertNotIn('cache', str(job['steps']).lower())
        command = next(step['run'] for step in prepare['steps'] if step.get('name') ==
                       'Extract catalogs from a reviewed ancestor of main')
        self.assertIn('trusted/scripts/provision-apk-archives.py extract', command)
        self.assertIn('--catalog-commit "$CATALOG_COMMIT"', command)

    def test_actionlint_is_required_in_both_early_workflow_policy_jobs(self):
        ci = self.load('ci.yml')
        for job_name in ('changes', 'workflow-permissions'):
            with self.subTest(job=job_name):
                job = ci['jobs'][job_name]
                self.assertNotIn('if', job)
                steps = [step for step in job['steps'] if 'scripts/check-actionlint.py' in step.get('run', '')]
                self.assertEqual(len(steps), 1)
                step = steps[0]
                self.assertIn('python3 scripts/check-actionlint.py --bin-dir "$RUNNER_TEMP/actionlint"', step['run'])
                self.assertNotIn('if', step)
                self.assertNotIn('continue-on-error', step)
                self.assertNotIn('||', step['run'])
        native = self.load('machine-native-acceptance.yml')['jobs']['native']
        self.assertNotIn('CARGO_TARGET_DIR', native['env'])
        self.assertEqual(native['steps'][0]['run'], 'echo "CARGO_TARGET_DIR=$RUNNER_TEMP/machine-native-target" >> "$GITHUB_ENV"')

    def test_pr_candidate_workflow_is_required_and_read_only(self):
        ci = self.load('ci.yml')
        job = ci['jobs']['catalog-maintenance']
        self.assertEqual(job['uses'], './.github/workflows/build-input-maintenance.yml')
        self.assertEqual(job['permissions'], {'contents': 'read'})
        pipeline = ci['jobs']['ci-pass']
        self.assertIn('catalog-maintenance', pipeline['needs'])
        self.assertIn('catalog-maintenance=${{ needs.catalog-maintenance.result }}',
                      str(pipeline['steps']))
        workflow = self.load('build-input-maintenance.yml')
        self.assertEqual(workflow['permissions'], {'contents': 'read'})
        for name, candidate in workflow['jobs'].items():
            with self.subTest(job=name):
                self.assertNotIn('secrets', candidate)
                self.assertNotIn('continue-on-error', candidate)
                self.assertEqual(candidate.get('permissions', {'contents': 'read'}), {'contents': 'read'})
                self.assertNotIn('gh pr create', str(candidate))
                self.assertNotIn('gh release', str(candidate))
        self.assertEqual(set(workflow['jobs']['apt-replay']['strategy']['matrix']['base']),
                         {'bookworm', 'trixie', 'focal', 'noble'})
        self.assertEqual(set(workflow['jobs']['apt-replay']['strategy']['matrix']['arch']), {'amd64', 'arm64'})
        self.assertEqual({item['arch'] for item in workflow['jobs']['apk-candidate']['strategy']['matrix']['include']},
                         {'amd64', 'arm64'})

    def test_hosted_native_jobs_build_actual_locked_target_and_run_offline(self):
        workflow = self.load('build-input-maintenance.yml')
        job = workflow['jobs']['hosted-tools']
        self.assertEqual({item['arch'] for item in job['strategy']['matrix']['include']}, {'amd64', 'arm64'})
        self.assertEqual(job['needs'], 'policy')
        text = str(job['steps'])
        self.assertIn('--file cli/Dockerfile.machine-hosted --target hosted-tools', text)
        self.assertIn('--network=none', text)
        self.assertIn('input=b""', text)
        self.assertNotIn('--push', text)
        self.assertNotIn('--build-context', text)

    def test_apt_candidate_resolves_settled_runs_from_one_utc_cutoff(self):
        job = self.load('build-input-maintenance.yml')['jobs']['apt-candidate']
        commands = '\n'.join(step.get('run', '') for step in job['steps'])
        self.assertIn('python3 scripts/refresh-apt-catalog.py --output "$RUNNER_TEMP/apt-candidate/security/apt-snapshots.tsv" --before "$SNAPSHOT_BEFORE"', commands)
        self.assertIn("'before=' + (now - timedelta(days=2)).strftime('%Y%m%dT000000Z')", commands)
        fetch = next(step for step in job['steps'] if 'SNAPSHOT_BEFORE' in step.get('env', {}))
        self.assertEqual(fetch['env']['SNAPSHOT_BEFORE'], '${{ steps.date.outputs.before }}')
        self.assertNotIn('--stamp', commands)
        self.assertNotIn('stamp=', commands)

    def test_only_schedule_publisher_can_write_after_native_verification(self):
        workflow = self.load('refresh-build-inputs.yml')
        events = workflow.get('on', workflow.get(True))
        self.assertEqual(set(events), {'schedule'})
        self.assertEqual(events['schedule'], [{'cron': '7 1 * * 1'}])
        self.assertEqual(workflow['permissions'], {'contents': 'read'})
        jobs = workflow['jobs']
        publisher = jobs['publish-candidate']
        self.assertEqual(publisher['needs'], 'candidates')
        self.assertEqual(publisher['permissions'], {'contents': 'write', 'pull-requests': 'write'})
        self.assertIn("github.event_name == 'schedule'", publisher['if'])
        self.assertIn("github.ref == 'refs/heads/main'", publisher['if'])
        for name in ('freshness', 'candidates'):
            self.assertEqual(jobs[name].get('permissions', {'contents': 'read'}), {'contents': 'read'})
            self.assertNotIn('needs', jobs[name], 'staleness must not block recovery candidate generation')
        self.assertEqual(jobs['candidates']['uses'], './.github/workflows/build-input-maintenance.yml')
        self.assertNotIn('id-token', str(workflow))
        checkout = next(step for step in publisher['steps'] if step.get('uses', '').startswith('actions/checkout@'))
        self.assertEqual(checkout['with']['ref'], '${{ github.workflow_sha }}')
        self.assertFalse(checkout['with']['persist-credentials'])



class BackendTestPartitions(unittest.TestCase):
    def setUp(self):
        self.root = Path(__file__).resolve().parents[1]
        self.workflow = load_unique((self.root / '.github/workflows/ci.yml').read_text())
        self.jobs = self.workflow['jobs']

    def test_backend_archive_jobs_have_explicit_least_privilege(self):
        for name in ('backend-test-build', 'backend-test-partition'):
            self.assertEqual(self.jobs[name]['permissions'], {'contents': 'read'})
        self.assertEqual(self.jobs['backend-test']['permissions'], {})

    def test_one_locked_archive_build_preserves_packages_features_and_toolchain(self):
        build = self.jobs['backend-test-build']
        self.assertEqual(build['needs'], 'changes')
        # Main raised its backend build/test budget before the archive split.
        self.assertEqual(build['timeout-minutes'], 120)
        self.assertEqual(build['env']['CARGO_PROFILE_TEST_DEBUG'], 'line-tables-only')
        commands = [step['run'] for step in build['steps'] if 'run' in step]
        archives = [cmd for cmd in commands if 'cargo nextest archive' in cmd]
        self.assertEqual(len(archives), 1)
        self.assertIn('cargo nextest archive --locked -p nyxid -p nyxid-service-adapters --features nyxid/aws-kms,nyxid/gcp-kms --profile ci', archives[0])
        self.assertIn('--archive-file "$RUNNER_TEMP/backend-tests.tar.zst"', archives[0])
        self.assertNotIn('cargo nextest run', '\n'.join(commands))
        self.assertNotIn('start-test-mongodb.sh', '\n'.join(commands))
        upload = next(step for step in build['steps'] if step.get('name') == 'Upload backend test archive')
        self.assertEqual(upload['with']['name'], 'backend-test-archive')
        self.assertEqual(upload['with']['path'], '${{ runner.temp }}/backend-tests.tar.zst')
        self.assertEqual(upload['with']['if-no-files-found'], 'error')
        self.assertNotIn('if', upload)

    def test_two_partitions_replay_same_archive_with_independent_mongo_and_reports(self):
        run = self.jobs['backend-test-partition']
        self.assertEqual(run['needs'], 'backend-test-build')
        self.assertEqual(run['strategy'], {'fail-fast': False, 'matrix': {'partition': [1, 2]}})
        self.assertEqual(run['timeout-minutes'], 90)
        self.assertEqual(run['runs-on'], self.jobs['backend-test-build']['runs-on'])
        mongo = next(step for step in run['steps'] if step.get('name') == 'Start transaction-capable MongoDB')
        self.assertEqual(mongo['run'], 'bash .github/scripts/start-test-mongodb.sh')
        self.assertNotIn('if', mongo)
        download = next(step for step in run['steps'] if step.get('name') == 'Download backend test archive')
        self.assertEqual(download['with']['name'], 'backend-test-archive')
        command = next(step['run'] for step in run['steps'] if step.get('name') == 'Run backend test partition')
        for arg in ('--archive-file "$RUNNER_TEMP/backend-test-archive/backend-tests.tar.zst"',
                    '--workspace-remap "$GITHUB_WORKSPACE"', '--extract-to "$RUNNER_TEMP/backend-test-run"',
                    '--partition hash:${{ matrix.partition }}/2', '--profile ci'):
            self.assertIn(arg, command)
        for compile_option in ('cargo nextest archive', 'cargo build', 'cargo test', '--features', '--package'):
            self.assertNotIn(compile_option, command)
        junit = '${{ runner.temp }}/backend-test-run/target/nextest/ci/junit.xml'
        summary = next(step for step in run['steps'] if step.get('name') == 'Publish test summary')
        results = next(step for step in run['steps'] if step.get('name') == 'Upload backend test results')
        self.assertEqual(summary['with']['paths'], junit)
        self.assertEqual(results['with']['path'], junit)
        self.assertEqual(results['with']['name'], 'test-results-backend-${{ matrix.partition }}')
        for step in (summary, results):
            self.assertEqual(step['if'], 'always()')
        self.assertNotIn('continue-on-error', run)

    def test_both_archive_jobs_use_the_same_integrity_locked_nextest(self):
        catalog = json.loads((self.root / 'security/build-tools.json').read_text())
        self.assertEqual(catalog['cargo-nextest']['version'], '0.9.146')
        for name in ('backend-test-build', 'backend-test-partition'):
            with self.subTest(job=name):
                steps = self.jobs[name]['steps']
                install = next(step for step in steps if step.get('name') == 'Install checksum-locked nextest 0.9.146')
                self.assertIn('python3 scripts/bootstrap-build-tool.py cargo-nextest', install['run'])
                self.assertNotIn('if', install)
                self.assertNotIn('continue-on-error', install)
                rust = next(step for step in steps if step.get('uses', '').startswith('dtolnay/rust-toolchain@'))
                self.assertEqual(rust['uses'], RUST_ACTION)

    def test_required_name_and_pipeline_keep_the_fail_closed_aggregate(self):
        aggregate = self.jobs['backend-test']
        self.assertEqual(aggregate['name'], 'Backend Test')
        self.assertEqual(aggregate['if'], 'always()')
        self.assertEqual(set(aggregate['needs']), {'changes', 'backend-test-build', 'backend-test-partition'})
        self.assertIn('backend-test', self.jobs['ci-pass']['needs'])
        self.assertIn('backend-test=${{ needs.backend-test.result }}', str(self.jobs['ci-pass']['steps']))
        step = aggregate['steps'][0]
        self.assertNotIn('continue-on-error', step)
        self.assertEqual(step['env']['EXPECTED'], '${{ ' + self.jobs['backend-test-build']['if'] + ' }}')
        # Execute the real gate for every upstream status combination: cancelled,
        # failed or unexpectedly skipped partitions must never turn green.
        for expected in ('true', 'false'):
            for changes, build, partitions in itertools.product(('success', 'failure', 'cancelled', 'skipped'), repeat=3):
                should_pass = changes == 'success' and ((expected == 'true' and build == partitions == 'success') or
                                                       (expected == 'false' and build == partitions == 'skipped'))
                env = dict(os.environ, EXPECTED=expected, CHANGES_RESULT=changes,
                           BUILD_RESULT=build, PARTITIONS_RESULT=partitions)
                with self.subTest(expected=expected, changes=changes, build=build, partitions=partitions):
                    result = subprocess.run(['bash', '-c', step['run']], env=env, text=True, capture_output=True)
                    self.assertEqual(result.returncode == 0, should_pass, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
