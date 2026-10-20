#!/usr/bin/env python3
"""Exercise immutable scan/publish binding using real regctl OCI import."""
import copy
import hashlib
import importlib.util
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

import image_vulnerabilities as policy

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('publish_scanned', ROOT / 'scripts/publish-scanned-image.py')
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)


class ImageVulnerabilities(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.stage = Path(self.temporary.name)
        self.regctl = Path(os.environ['REGCTL_BIN']).resolve()
        self.commit = 'a' * 40
        layer = io.BytesIO()
        with tarfile.open(fileobj=layer, mode='w') as stream:
            data = b'synthetic package fixture\n'
            item = tarfile.TarInfo('fixture.txt')
            item.size = len(data)
            stream.addfile(item, io.BytesIO(data))
        config = dict(architecture='arm64', os='linux',
                      config={'Labels': {'org.opencontainers.image.revision': self.commit}},
                      rootfs={'type': 'layers', 'diff_ids': ['sha256:' + hashlib.sha256(layer.getvalue()).hexdigest()]})
        data = json.dumps(config).encode()
        self.image_id = 'sha256:' + hashlib.sha256(data).hexdigest()
        config_name = self.image_id[7:] + '.json'
        manifest = [dict(Config=config_name, RepoTags=['fixture:scan'], Layers=['layer/layer.tar'])]
        archive = self.stage / 'image.tar'
        with tarfile.open(archive, 'w') as stream:
            for name, content in [(config_name, data), ('layer/layer.tar', layer.getvalue()),
                                   ('manifest.json', json.dumps(manifest).encode())]:
                item = tarfile.TarInfo(name)
                item.size = len(content)
                stream.addfile(item, io.BytesIO(content))
        local = 'ocidir://' + str(self.stage / 'oci') + ':staged'
        subprocess.run([str(self.regctl), 'image', 'import', local, str(archive)], check=True, capture_output=True)
        self.digest = subprocess.check_output([str(self.regctl), 'manifest', 'digest', local], text=True).strip()
        subject = policy.layout_subject(self.stage / 'oci', self.digest, 'linux/arm64', self.commit)
        self.assertEqual(subject['runnable'][0]['config'], self.image_id)
        self.report = dict(SchemaVersion=2, ArtifactType='container_image', Metadata={'ImageID': self.image_id},
                           Results=[dict(Target='fixture distro', Class='os-pkgs', Type='debian',
                                         Packages=[{'Name': 'fixture', 'Version': '1.0'}]),
                                    dict(Target='usr/local/bin/nyxid-server', Class='lang-pkgs', Type='rustbinary',
                                         Packages=[{'Name': 'nyxid', 'Version': '1.0'}])])
        self.report_path = self.stage / 'trivy.json'
        self.report_path.write_text(json.dumps(self.report))
        self.receipt = dict(schema_version=1, status='passed', component='backend', platform='linux/arm64',
                            source_commit=self.commit, image_id=self.image_id, subject=subject['subject'],
                            archive_sha256=policy.sha(archive), report_sha256=policy.sha(self.report_path),
                            blocking=[], threshold=sorted(policy.BLOCKED))
        self.write_receipt()

    def write_receipt(self):
        (self.stage / 'image-scan.json').write_text(json.dumps(self.receipt))

    def test_real_import_binds_exact_config_layers_and_native_platform(self):
        self.assertEqual(policy.verify_stage(self.stage), self.receipt)

    def exercise_scan(self, corrupt_report=False):
        run = subprocess.run
        check_output = subprocess.check_output
        def output(command, **kwargs):
            if command[:3] == ['docker', 'image', 'inspect']:
                return json.dumps([dict(Id=self.image_id, Os='linux', Architecture='arm64')]).encode()
            return check_output(command, **kwargs)
        def execute(command, **kwargs):
            if command[:3] == ['docker', 'image', 'save']:
                self.assertEqual(command[-1], self.image_id)
                shutil.copyfile(self.stage / 'image.tar', command[command.index('--output') + 1])
                return subprocess.CompletedProcess(command, 0)
            if command[0] == 'trivy-fixture':
                self.assertNotIn('TRIVY_IGNORE_UNFIXED', kwargs['env'])
                self.assertIn('--ignorefile=', command)
                self.assertEqual(command[command.index('--scanners') + 1], 'vuln')
                self.assertEqual(command[command.index('--format') + 1], 'json')
                self.assertEqual(Path(command[command.index('--config') + 1]).read_text(), '{}\n')
                self.assertEqual(Path(command[-1]).read_bytes(), (self.stage / 'image.tar').read_bytes())
                report = copy.deepcopy(self.report)
                if corrupt_report: report['Metadata']['ImageID'] = 'sha256:' + '0' * 64
                Path(command[command.index('--output') + 1]).write_text(json.dumps(report))
                return subprocess.CompletedProcess(command, 0)
            return run(command, **kwargs)
        with patch.object(policy.subprocess, 'check_output', side_effect=output), \
                patch.object(policy.subprocess, 'run', side_effect=execute), \
                patch.dict(os.environ, {'TRIVY_IGNORE_UNFIXED': 'true'}):
            return policy.scan('mutable:input', 'backend', 'linux/arm64', self.commit,
                               Path('trivy-fixture'), self.regctl, self.stage / 'new-scan')

    def test_scanner_uses_one_resolved_image_archive_and_no_environment_suppressions(self):
        self.assertEqual(self.exercise_scan(), 0)
        self.assertEqual(policy.verify_stage(self.stage / 'new-scan')['image_id'], self.image_id)

    def test_unrelated_scanner_subject_cannot_create_a_passing_receipt(self):
        with self.assertRaisesRegex(ValueError, 'actual final image config'): self.exercise_scan(corrupt_report=True)
        self.assertFalse((self.stage / 'new-scan/image-scan.json').exists())

    def test_machine_runtime_rejects_build_headers_even_without_reported_vulnerabilities(self):
        for component in ('machine-node-only', 'nyxid-node-machine', 'machine-hosted-desktop', 'machine-hosted'):
            for package in ('linux-libc-dev', 'build-essential'):
                report = copy.deepcopy(self.report)
                report['Results'][0]['Packages'].append({'Name': package, 'Version': '1'})
                with self.subTest(component=component, package=package), self.assertRaisesRegex(ValueError, 'build-only packages'):
                    policy.evaluate(report, self.image_id, component)

    def test_all_blocked_severities_and_unfixed_findings_fail(self):
        for severity in policy.BLOCKED:
            report = copy.deepcopy(self.report)
            report['Results'][0]['Vulnerabilities'] = [dict(VulnerabilityID='CVE-synthetic', PkgName='fixture',
                InstalledVersion='1.0', Severity=severity)]
            with self.subTest(severity=severity): self.assertEqual(len(policy.evaluate(report, self.image_id, 'backend')), 1)

    def test_wrong_subject_missing_inventory_and_eol_fail(self):
        for mutation in ('config', 'empty', 'packages', 'eol'):
            report = copy.deepcopy(self.report)
            if mutation == 'config': report['Metadata']['ImageID'] = 'sha256:' + '0' * 64
            elif mutation == 'empty': report['Results'] = []
            elif mutation == 'packages':
                for item in report['Results']: item['Packages'] = []
            else: report['Metadata']['OS'] = {'EOSL': True}
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): policy.evaluate(report, self.image_id, 'backend')

    def test_os_only_or_wrong_application_coverage_cannot_pass(self):
        for mutation in ('missing-binary', 'wrong-target', 'wrong-package', 'missing-distro'):
            report = copy.deepcopy(self.report)
            if mutation == 'missing-binary': report['Results'].pop()
            elif mutation == 'wrong-target': report['Results'][1]['Target'] = 'other/nyxid-server'
            elif mutation == 'wrong-package': report['Results'][1]['Packages'][0]['Name'] = 'unrelated'
            else: report['Results'].pop(0)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): policy.evaluate(report, self.image_id, 'backend')

    def test_scratch_updater_requires_its_real_binary_inventory_without_distro(self):
        report = copy.deepcopy(self.report)
        report['Results'] = [dict(Target='nyxid-machine-updater', Class='lang-pkgs', Type='rustbinary',
                                  Packages=[{'Name': 'nyxid-machine-updater', 'Version': '1.0'}])]
        self.assertEqual(policy.evaluate(report, self.image_id, 'nyxid-machine-updater'), [])
        report['Results'][0]['Type'] = 'cargo'
        with self.assertRaises(ValueError): policy.evaluate(report, self.image_id, 'nyxid-machine-updater')

    def test_changed_image_archive_blocks_any_registry_operation(self):
        with (self.stage / 'image.tar').open('ab') as stream: stream.write(b'changed')
        with patch.object(publisher.subprocess, 'run') as run, self.assertRaisesRegex(ValueError, 'archive/report changed'):
            publisher.publish(self.stage, 'ghcr.io/fixture/nyxid/backend', self.regctl, self.stage / 'proof', Path('/verifier'))
        run.assert_not_called()

    def test_changed_scanner_report_blocks_publication(self):
        self.report_path.write_text('{}')
        with self.assertRaisesRegex(ValueError, 'archive/report changed'): policy.verify_stage(self.stage)

    def test_missing_scan_or_failed_status_blocks_publication(self):
        (self.stage / 'image-scan.json').unlink()
        with self.assertRaises(OSError): policy.verify_stage(self.stage)
        self.receipt['status'] = 'failed'
        self.write_receipt()
        with self.assertRaisesRegex(ValueError, 'passing final'): policy.verify_stage(self.stage)

    def test_weakened_threshold_or_forged_clean_summary_blocks_publication(self):
        self.receipt['threshold'] = ['CRITICAL']
        self.write_receipt()
        with self.assertRaises(ValueError): policy.verify_stage(self.stage)
        self.receipt['threshold'] = sorted(policy.BLOCKED)
        self.report['Results'][0]['Vulnerabilities'] = [dict(VulnerabilityID='CVE-synthetic', PkgName='fixture',
            InstalledVersion='1', Severity='HIGH')]
        self.report_path.write_text(json.dumps(self.report))
        self.receipt['report_sha256'] = policy.sha(self.report_path)
        self.write_receipt()
        with self.assertRaisesRegex(ValueError, 'exact staged subject'): policy.verify_stage(self.stage)

    def test_changed_staged_blob_is_rejected(self):
        blob = next((self.stage / 'oci/blobs/sha256').iterdir())
        blob.write_bytes(b'corrupted')
        with self.assertRaisesRegex(ValueError, 'blob bytes'): policy.verify_stage(self.stage)

    def test_symlink_or_wrong_platform_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'platform'):
            policy.layout_subject(self.stage / 'oci', self.digest, 'linux/amd64', self.commit)
        (self.stage / 'oci/extra').symlink_to('/etc/passwd')
        with self.assertRaisesRegex(ValueError, 'nonregular'): policy.verify_stage(self.stage)

    def test_publisher_copies_only_passing_digest_without_rebuilding_or_tags(self):
        with patch.dict(os.environ, GITHUB_REF='refs/heads/main'), patch.object(publisher, 'verify_sbom', return_value={}), \
                patch.object(publisher.official_release, 'authorize'), patch.object(publisher.official_release, 'manifest'), \
                patch.object(publisher.official_release, 'verify'), patch.object(publisher.official_release, 'attach'), \
                patch.object(publisher.subprocess, 'run') as run, \
                patch.object(publisher.subprocess, 'check_output', return_value=self.digest + '\n'):
            self.assertEqual(publisher.publish(self.stage, 'ghcr.io/fixture/nyxid/backend', self.regctl, self.stage / 'proof', Path('/verifier')), self.digest)
        command = run.call_args_list[0].args[0]
        self.assertEqual(command, [str(self.regctl), 'image', 'copy',
            'ocidir://' + str(self.stage / 'oci') + '@' + self.digest, 'ghcr.io/fixture/nyxid/backend@' + self.digest])
        self.assertEqual(run.call_args.args[0][1:3], ['artifact', 'put'])
        self.assertIn('ghcr.io/fixture/nyxid/backend@' + self.digest, run.call_args.args[0])


if __name__ == '__main__':
    unittest.main()
