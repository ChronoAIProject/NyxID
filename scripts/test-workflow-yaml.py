#!/usr/bin/env python3
from pathlib import Path
import unittest
import yaml
from workflow_yaml import RUST_ACTION, load_unique, validate_rust_steps


class WorkflowYaml(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
