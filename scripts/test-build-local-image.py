#!/usr/bin/env python3
"""Exercise the operator build/check/tag contract with disposable command fixtures."""
import importlib.util
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
IMAGE = 'sha256:' + 'a' * 64


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


fixtures = module('local_image_fixtures', 'test-export-build-source.py')
builder = module('local_image_builder', 'build-local-image.py')


class LocalImage(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.SourceExport('test_exact_commit_export_is_no_git_and_repeatable')
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.dockerfile = 'cli/Dockerfile.machine-hosted'
        file = self.fixture.root / self.dockerfile
        file.parent.mkdir()
        file.write_text('FROM scratch AS desktop\nFROM desktop AS hosted\n')
        scripts = self.fixture.root / 'scripts'
        scripts.mkdir()
        checker = scripts / 'check-image-identity.py'
        checker.write_text('''import json,os,sys
from pathlib import Path
with Path(os.environ['TEST_COMMANDS']).open('a') as output:
    output.write(json.dumps({'tool':'checker','argv':sys.argv[1:]})+'\\n')
if os.environ.get('TEST_MUTATE_SOURCE'):
    (Path(sys.argv[sys.argv.index('--source-root')+1])/'source.txt').write_text('changed by checker')
raise SystemExit(int(os.environ.get('TEST_CHECK_EXIT','0')))
''')
        (scripts / 'bootstrap-build-tool.py').write_text("import json,os,sys\nfrom pathlib import Path\nwith Path(os.environ['TEST_COMMANDS']).open('a') as output:\n    output.write(json.dumps({'tool':'bootstrap','argv':sys.argv[1:]})+'\\n')\nraise SystemExit(int(os.environ.get('TEST_SCAN_BOOTSTRAP_EXIT','0')))\n")
        (scripts / 'check-image-vulnerabilities.py').write_text("import json,os,sys\nfrom pathlib import Path\nwith Path(os.environ['TEST_COMMANDS']).open('a') as output:\n    output.write(json.dumps({'tool':'vulnerability','argv':sys.argv[1:]})+'\\n')\nraise SystemExit(int(os.environ.get('TEST_SCAN_EXIT','0')))\n")
        (scripts / 'fixture_metadata.py').write_text('VALUE = 1\n')
        (scripts / 'check-image-sbom.py').write_text("import fixture_metadata\nimport json,os,sys\nfrom pathlib import Path\nwith Path(os.environ['TEST_COMMANDS']).open('a') as output:\n    output.write(json.dumps({'tool':'sbom','argv':sys.argv[1:]})+'\\n')\nraise SystemExit(int(os.environ.get('TEST_SBOM_EXIT','0')))\n")
        manifest = self.fixture.root / 'security/artifact-tools/Cargo.toml'
        manifest.parent.mkdir(parents=True)
        (self.fixture.root / 'security/python-sbom-requirements.txt').write_text('# fixture lock\n')
        manifest.write_text('[package]\nname="fixture-inspector"\nversion="0.1.0"\n')
        self.fixture.commit()
        self.destination = self.fixture.directory / 'image-export'
        self.record = self.fixture.directory / 'commands.jsonl'
        binaries = self.fixture.directory / 'bin'
        binaries.mkdir()
        common = '''import json,sys,os
from pathlib import Path
args=sys.argv[1:]
with Path(os.environ['TEST_COMMANDS']).open('a') as output:
    output.write(json.dumps({'tool':Path(sys.argv[0]).name,'argv':args,
        'commit':os.environ.get('NYXID_GIT_HASH'),'epoch':os.environ.get('SOURCE_DATE_EPOCH'),
        'jobs':os.environ.get('CARGO_BUILD_JOBS')})+'\\n')
'''
        docker = binaries / 'docker'
        docker.write_text('#!' + sys.executable + '\n' + common + '''
if args[0]=='version': print('linux/arm64')
elif args[0]=='build':
    assert '--tag' not in args and '-t' not in args, 'unverified build was tagged'
    Path(args[args.index('--iidfile')+1]).write_text(os.environ.get('TEST_IMAGE_ID', ''' + repr(IMAGE) + '''))
    raise SystemExit(int(os.environ.get('TEST_BUILD_EXIT','0')))
elif args[0]=='tag':
    assert args[1:3]==['--', ''' + repr(IMAGE) + ''']
    raise SystemExit(int(os.environ.get('TEST_TAG_EXIT','0')))
else: raise SystemExit('unexpected Docker operation')
''')
        cargo = binaries / 'cargo'
        cargo.write_text('#!' + sys.executable + '\n' + common + '''
assert args[:4]==['build','--locked','--release','--manifest-path']
assert os.environ['CARGO_BUILD_JOBS']=='2'
executable=Path(os.environ['CARGO_TARGET_DIR'])/'release/nyxid-artifact-info'
executable.parent.mkdir(parents=True)
executable.write_text('fixture inspector')
raise SystemExit(int(os.environ.get('TEST_INSPECTOR_EXIT','0')))
''')
        docker.chmod(0o755)
        cargo.chmod(0o755)
        self.environment = patch.dict(os.environ, {'PATH': str(binaries) + os.pathsep + os.environ['PATH'],
                                                  'TEST_COMMANDS': str(self.record)})
        self.environment.start()
        self.addCleanup(self.environment.stop)
        real_run = subprocess.run
        def run(command, *args, **kwargs):
            if command[:3] == [sys.executable, '-m', 'venv']:
                environment = Path(command[3]); (environment / 'bin').mkdir(parents=True)
                (environment / 'bin/python').symlink_to(sys.executable)
                tool = 'sbom-venv'
            elif len(command) > 3 and command[1:4] == ['-m', 'pip', 'install']:
                self.assertIn('--require-hashes', command); self.assertIn('--only-binary=:all:', command)
                tool = 'sbom-pip'
            else:
                if not kwargs.get('capture_output'):
                    kwargs.setdefault('stdout', subprocess.PIPE)
                    kwargs.setdefault('stderr', subprocess.PIPE)
                return real_run(command, *args, **kwargs)
            with self.record.open('a') as output: output.write(json.dumps(dict(tool=tool, argv=command[1:])) + '\n')
            return subprocess.CompletedProcess(command, int(os.environ.get('TEST_SBOM_SETUP_EXIT', '0')))
        self.runner = patch.object(builder.subprocess, 'run', side_effect=run)
        self.runner.start(); self.addCleanup(self.runner.stop)

    def commands(self):
        return [json.loads(line) for line in self.record.read_text().splitlines()] if self.record.exists() else []

    def run_build(self, **kwargs):
        # Failure cases deliberately exercise mandatory gates. Keep expected
        # diagnostics available for assertions without CI error annotations.
        self.diagnostics = io.StringIO()
        with contextlib.redirect_stdout(self.diagnostics), contextlib.redirect_stderr(self.diagnostics):
            return builder.build(self.fixture.root, self.destination, self.dockerfile, 'fixture:local', **kwargs)

    def receipt(self):
        return json.loads((self.destination / 'image-build-execution.json').read_text())

    def assert_untagged(self):
        self.assertFalse(any(item['tool'] == 'docker' and item['argv'][0] == 'tag' for item in self.commands()))
        self.assertFalse(self.receipt()['image']['tagged'])
        self.assertNotEqual(self.receipt()['exit_code'], 0)

    def test_exact_export_build_id_gate_then_tag_and_bound_receipt(self):
        code, exported = self.run_build(target='hosted', platform='linux/arm64',
                                       contexts=['cma=/separately-reviewed/context'])
        self.assertEqual(code, 0)
        commands = self.commands()
        self.assertEqual([item['tool'] for item in commands], ['docker', 'cargo', 'checker', 'bootstrap', 'bootstrap', 'vulnerability', 'bootstrap', 'sbom-venv', 'sbom-pip', 'sbom', 'docker'])
        commit, epoch = self.fixture.commit_id, str(exported['source']['epoch'])
        self.assertEqual(commands[0]['argv'], ['build', '--load', '--build-arg', 'NYXID_GIT_HASH=' + commit,
            '--build-arg', 'SOURCE_DATE_EPOCH=' + epoch, '--file', str(self.destination / 'source' / self.dockerfile),
            '--iidfile', str(self.destination / 'image-id'), '--target', 'hosted', '--platform', 'linux/arm64',
            '--build-context', 'cma=/separately-reviewed/context', str(self.destination / 'source')])
        self.assertEqual((commands[0]['commit'], commands[0]['epoch']), (commit, epoch))
        self.assertEqual(commands[2]['argv'], ['--image', IMAGE, '--component', 'machine-hosted', '--arch', 'arm64',
            '--source-root', str(self.destination / 'source'), '--commit', commit, '--epoch', epoch,
            '--extractor', str(self.destination / 'artifact-target/release/nyxid-artifact-info')])
        self.assertEqual(commands[-1]['argv'], ['tag', '--', IMAGE, 'fixture:local'])
        receipt = self.receipt()
        self.assertTrue(receipt['post_build_source_verified'])
        self.assertEqual(receipt['source']['commit'], commit)
        self.assertEqual(receipt['identity_check']['status'], 'passed')
        self.assertEqual(receipt['identity_check']['exit_code'], 0)
        self.assertEqual(receipt['vulnerability_check']['status'], 'passed')
        self.assertEqual(receipt['vulnerability_check']['exit_code'], 0)
        self.assertEqual(receipt['sbom_check']['status'], 'passed')
        self.assertEqual(receipt['sbom_check']['exit_code'], 0)
        self.assertEqual(commands[-2]['argv'][:2], ['--stage', str(self.destination / 'scanned-image')])
        self.assertEqual(commands[5]['argv'][:6], ['--image', IMAGE, '--component', 'machine-hosted', '--platform', 'linux/arm64'])
        self.assertEqual(receipt['image'], dict(id=IMAGE, component='machine-hosted', platform='linux/arm64',
                                               requested_tag='fixture:local', tagged=True))
        self.assertEqual(receipt['exit_code'], 0)

    def test_default_hosted_target_and_native_daemon_platform_are_checked(self):
        code, _ = self.run_build()
        self.assertEqual(code, 0)
        self.assertEqual(self.commands()[0]['argv'][0], 'version')
        self.assertEqual(self.receipt()['image']['component'], 'machine-hosted')
        self.assertEqual(self.receipt()['image']['platform'], 'linux/arm64')
        self.assertEqual(self.receipt()['identity_check']['status'], 'passed')

    def test_failed_checker_exits_nonzero_without_tag(self):
        with patch.dict(os.environ, TEST_CHECK_EXIT='19'):
            code, _ = self.run_build()
        self.assertNotEqual(code, 0)
        self.assert_untagged()
        self.assertEqual(self.receipt()['identity_check']['exit_code'], 19)
        self.assertEqual(self.receipt()['identity_check']['status'], 'failed')

    def test_vulnerability_failure_leaves_no_tag(self):
        with patch.dict(os.environ, TEST_SCAN_EXIT='31'):
            code, _ = self.run_build()
        self.assertNotEqual(code, 0)
        self.assert_untagged()
        self.assertEqual(self.receipt()['vulnerability_check']['status'], 'failed')
        self.assertEqual(self.receipt()['vulnerability_check']['exit_code'], 31)

    def test_failed_or_missing_sbom_leaves_no_tag(self):
        for failure in ('missing', 'checker', 'bootstrap'):
            with self.subTest(failure=failure):
                # Each build needs a new immutable export directory.
                self.destination = self.fixture.directory / ('sbom-failure-' + failure)
                if failure == 'missing':
                    (self.fixture.root / 'scripts/check-image-sbom.py').rename(self.fixture.root / 'scripts/check-image-sbom.hidden')
                    self.fixture.commit()
                with patch.dict(os.environ, TEST_SBOM_EXIT='43' if failure == 'checker' else '0',
                                TEST_SBOM_SETUP_EXIT='47' if failure == 'bootstrap' else '0'):
                    code, _ = self.run_build()
                self.assertNotEqual(code, 0); self.assert_untagged()
                self.assertEqual(self.receipt()['sbom_check']['status'], 'failed')
                if failure == 'missing':
                    (self.fixture.root / 'scripts/check-image-sbom.hidden').rename(self.fixture.root / 'scripts/check-image-sbom.py')
                    self.fixture.commit()

    def test_missing_vulnerability_checker_leaves_no_tag(self):
        (self.fixture.root / 'scripts/check-image-vulnerabilities.py').unlink()
        self.fixture.commit()
        code, _ = self.run_build()
        self.assertNotEqual(code, 0)
        self.assert_untagged()

    def test_failed_scanner_bootstrap_leaves_no_tag(self):
        with patch.dict(os.environ, TEST_SCAN_BOOTSTRAP_EXIT='37'):
            code, _ = self.run_build()
        self.assertNotEqual(code, 0)
        self.assert_untagged()
        self.assertEqual(self.receipt()['vulnerability_check']['trivy_bootstrap_exit_code'], 37)
        self.assertFalse(any(item['tool']=='vulnerability' for item in self.commands()))

    def test_missing_checker_exits_nonzero_without_tag(self):
        (self.fixture.root / 'scripts/check-image-identity.py').unlink()
        self.fixture.commit()
        code, _ = self.run_build()
        self.assertNotEqual(code, 0)
        self.assert_untagged()
        self.assertIn('checker is missing', self.receipt()['identity_check']['error'])

    def test_failed_inspector_build_exits_nonzero_without_checker_or_tag(self):
        with patch.dict(os.environ, TEST_INSPECTOR_EXIT='23'):
            code, _ = self.run_build()
        self.assertNotEqual(code, 0)
        self.assert_untagged()
        self.assertFalse(any(item['tool'] == 'checker' for item in self.commands()))
        self.assertEqual(self.receipt()['identity_check']['inspector_build_exit_code'], 23)

    def test_failed_docker_build_cannot_check_or_tag(self):
        with patch.dict(os.environ, TEST_BUILD_EXIT='17'):
            code, _ = self.run_build()
        self.assertEqual(code, 17)
        self.assert_untagged()
        self.assertEqual(self.receipt()['identity_check']['status'], 'not-run')

    def test_invalid_image_id_cannot_check_or_tag(self):
        with patch.dict(os.environ, TEST_IMAGE_ID='mutable:tag'):
            code, _ = self.run_build()
        self.assertNotEqual(code, 0)
        self.assert_untagged()

    def test_failed_tag_cannot_return_success(self):
        with patch.dict(os.environ, TEST_TAG_EXIT='29'):
            code, _ = self.run_build()
        self.assertEqual(code, 29)
        self.assertEqual(self.receipt()['identity_check']['status'], 'passed')
        self.assertFalse(self.receipt()['image']['tagged'])

    def test_checker_cannot_change_source_and_still_tag(self):
        with patch.dict(os.environ, TEST_MUTATE_SOURCE='1'):
            code, _ = self.run_build()
        self.assertNotEqual(code, 0)
        self.assert_untagged()
        self.assertFalse(self.receipt()['post_build_source_verified'])
        self.assertEqual(self.receipt()['identity_check']['status'], 'failed')

    def test_dirty_checkout_cannot_invoke_docker(self):
        (self.fixture.root / 'source.txt').write_text('uncommitted source')
        with self.assertRaisesRegex(ValueError, 'clean checkout'): self.run_build()
        self.assertFalse(self.record.exists())

    def test_uncommitted_or_escaping_dockerfile_cannot_be_used(self):
        with self.assertRaises(ValueError):
            builder.build(self.fixture.root, self.destination, '../Dockerfile', 'fixture:local')
        self.assertFalse(self.record.exists())

    def test_unknown_file_stage_or_multiplatform_cannot_bypass_gate(self):
        for file, target, platform in (('Dockerfile', None, None), (self.dockerfile, 'hosted-tools', None),
                                        (self.dockerfile, 'builder', None), (self.dockerfile, None, 'linux/amd64,linux/arm64')):
            with self.subTest(file=file, target=target, platform=platform), self.assertRaises(ValueError):
                builder.build(self.fixture.root, self.destination, file, 'fixture:local', target, platform)
        self.assertFalse(self.record.exists())

    def test_cli_exposes_no_verification_optout_or_alternate_checker(self):
        for option in ('--skip-sbom-check', '--sbom-checker', '--skip-identity-check', '--skip-vulnerability-check', '--no-scan', '--no-verify', '--checker', '--extractor'):
            result = subprocess.run([sys.executable, str(ROOT / 'scripts/build-local-image.py'), '--file', self.dockerfile,
                                     '--tag', 'fixture:local', option], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('unrecognized arguments', result.stderr)
        self.assertFalse(self.record.exists())

    def test_every_shipped_definition_and_final_stage_has_mandatory_component(self):
        from source_artifacts import IMAGES
        for component, (file, target, *_) in IMAGES.items():
            with self.subTest(component=component): self.assertEqual(builder.component_for(file, target), component)
        self.assertEqual(builder.component_for('cli/Dockerfile.machine-updater', 'final'), 'nyxid-machine-updater')
        self.assertEqual(builder.component_for('frontend/Dockerfile', 'runtime'), 'frontend')

    def test_explicit_updater_e2e_stage_retains_test_only_receipt(self):
        self.dockerfile = 'cli/Dockerfile.machine-updater'
        (self.fixture.root / self.dockerfile).write_text('FROM scratch AS e2e\n')
        self.fixture.commit()
        code, _ = self.run_build(target='e2e')
        self.assertEqual(code, 0)
        self.assertEqual(self.receipt()['identity_check']['status'], 'not-applicable-test-target')
        self.assertFalse(any(item['tool'] in ('cargo', 'checker') for item in self.commands()))


if __name__ == '__main__':
    unittest.main()
