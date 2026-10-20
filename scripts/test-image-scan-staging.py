#!/usr/bin/env python3
"""Real regctl replay of containerd's dual OCI/Docker save views and PR tags."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import tarfile
import unittest

import image_vulnerabilities as policy
from workflow_yaml import load_unique

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('image_fixtures', ROOT / 'scripts/test-image-vulnerabilities.py')
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


def workflow_contract(workflow):
    job = workflow['jobs']['images']
    builds = [step for step in job['steps'] if step.get('uses', '').startswith('docker/build-push-action@')]
    if len(builds) != 1 or builds[0]['with'].get('tags') != 'nyxid-identity:ci' or \
            builds[0]['with'].get('load') is not True or builds[0]['with'].get('push') is not False:
        raise ValueError('identity image must be staged locally under the scanned tag')
    scans = [s for s in job['steps'] if 'scripts/check-image-vulnerabilities.py' in s.get('run', '')]
    if len(scans) != 1 or '--image nyxid-identity:ci' not in scans[0]['run'] or \
            scans[0].get('if') or scans[0].get('continue-on-error'):
        raise ValueError('required scan must consume the actual loaded identity image')


class Staging(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.ImageVulnerabilities()
        self.addCleanup(self.fixture.doCleanups)
        self.fixture.setUp()

    def test_containerd_dual_view_import_preserves_one_tagged_subject_and_untagged_alias(self):
        f = self.fixture
        source = f.stage / 'oci'
        index = json.loads((source / 'index.json').read_text())
        index['manifests'][0]['annotations'] = {
            'io.containerd.image.name': 'docker.io/library/nyxid-identity:ci',
            'org.opencontainers.image.ref.name': f.image_id}
        (source / 'index.json').write_text(json.dumps(index))
        archive = f.stage / 'containerd.tar'
        with tarfile.open(archive, 'w') as output:
            for path in source.rglob('*'):
                if path.is_file(): output.add(path, arcname=str(path.relative_to(source)))
            with tarfile.open(f.stage / 'image.tar') as compatibility:
                for member in compatibility:
                    output.addfile(member, compatibility.extractfile(member))
        layout = f.stage / 'imported'
        local = 'ocidir://' + str(layout) + ':staged'
        subprocess.run([str(f.regctl), 'image', 'import', local, str(archive)], check=True, capture_output=True)
        digest = subprocess.check_output([str(f.regctl), 'manifest', 'digest', local], text=True).strip()
        descriptors = json.loads((layout / 'index.json').read_text())['manifests']
        self.assertEqual(sum(d['digest'] == digest for d in descriptors), 2)
        observed = policy.layout_subject(layout, digest, 'linux/arm64', f.commit)
        self.assertEqual(observed['subject']['digest'], digest)
        self.assertEqual(observed['runnable'][0]['config'], f.image_id)

    def test_missing_duplicate_or_substituted_staging_alias_fails(self):
        f = self.fixture; path = f.stage / 'oci/index.json'
        original = json.loads(path.read_text())
        for mutation in ('missing', 'duplicate', 'substituted'):
            index = copy.deepcopy(original)
            if mutation == 'missing': index['manifests'][0]['annotations'].clear()
            elif mutation == 'duplicate': index['manifests'].append(index['manifests'][0])
            else: index['manifests'][0]['digest'] = 'sha256:' + '0' * 64
            path.write_text(json.dumps(index))
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                policy.layout_subject(f.stage / 'oci', f.digest, 'linux/arm64', f.commit)

    def test_pr_scan_uses_exact_loaded_tag_and_cannot_be_optional(self):
        workflow = load_unique((ROOT / '.github/workflows/artifact-packaging.yml').read_text())
        workflow_contract(workflow)
        for mutation in ('tag', 'scan', 'load', 'optional'):
            changed = copy.deepcopy(workflow); steps = changed['jobs']['images']['steps']
            build = next(s for s in steps if s.get('uses', '').startswith('docker/build-push-action@'))
            scan = next(s for s in steps if 'scripts/check-image-vulnerabilities.py' in s.get('run', ''))
            if mutation == 'tag': build['with']['tags'] = 'unrelated:ci'
            elif mutation == 'load': build['with']['load'] = False
            elif mutation == 'optional': scan['continue-on-error'] = True
            else: scan['run'] = scan['run'].replace('--image nyxid-identity:ci', '--image unrelated:ci')
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): workflow_contract(changed)


if __name__ == '__main__': unittest.main()
