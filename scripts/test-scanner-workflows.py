#!/usr/bin/env python3
"""Negative tests for mandatory PR/scheduled and publication scan wiring."""
import copy
from pathlib import Path
import unittest

from workflow_yaml import load_unique

ROOT = Path(__file__).resolve().parents[1]


def workflows():
    return {name: load_unique((ROOT / '.github/workflows' / (name + '.yml')).read_text())
            for name in ('security-scans', 'ci', 'artifact-packaging', 'publish-images', 'release')}


def required_step(steps, script):
    indexes = [index for index, step in enumerate(steps) if script in step.get('run', '')]
    if len(indexes) != 1:
        raise ValueError('missing/duplicated scanner gate: ' + script)
    step = steps[indexes[0]]
    if 'if' in step or step.get('continue-on-error') or any(value in step['run'] for value in ('|| true', 'set +e', 'exit 0')):
        raise ValueError('conditional or warning-only scanner gate: ' + script)
    return indexes[0]


def require(data):
    scans = data['security-scans']
    # PyYAML's YAML 1.1 resolver treats GitHub's unquoted `on` as True.
    if scans['permissions'] != {'contents': 'read'} or 'schedule' not in scans.get('on', scans.get(True, {})):
        raise ValueError('read-only scheduled scanners required')
    ci = data['ci']
    caller = ci['jobs']['security-scans']
    if caller.get('if') or caller['uses'] != './.github/workflows/security-scans.yml' or \
            caller['permissions'] != {'contents': 'read'} or 'security-scans' not in ci['jobs']['ci-pass']['needs'] or \
            'security-scans=${{ needs.security-scans.result }}' not in str(ci['jobs']['ci-pass']['steps']):
        raise ValueError('all PRs must require source security scanning through CI Pipeline')
    jobs = scans['jobs']
    if set(jobs['passed']['needs']) != set(jobs) - {'passed'}:
        raise ValueError('every scanner must feed the required aggregator')
    for job in jobs.values():
        if job.get('continue-on-error') or 'secrets' in job or 'id-token' in str(job):
            raise ValueError('PR scanners must fail closed without secrets/OIDC')
    history = jobs['history']['steps']
    checkout = next(step for step in history if step.get('uses', '').startswith('actions/checkout@'))
    if checkout['with'].get('fetch-depth') != 0:
        raise ValueError('full-history secret checkout required')
    required_step(history, 'scripts/scan-secrets.py')
    required_step(jobs['dependencies']['steps'], 'scripts/audit-dependencies.py')
    for script in ('scripts/test-dependency-patches.py', 'scripts/test-dependency-dispositions.py', 'scripts/test-mobile-bundle-policy.cjs'):
        required_step(jobs['policy']['steps'], script)
    required_step(ci['jobs']['mobile']['steps'], 'scripts/check-mobile-bundle.cjs')
    required_step(ci['jobs']['mobile']['steps'], 'scripts/test-mobile-yaml-compat.cjs')
    required_step(jobs['freshness']['steps'], 'scripts/check-build-input-freshness.py')
    packaging = data['artifact-packaging']
    if 'schedule' not in packaging.get('on', packaging.get(True, {})) or packaging['permissions'] != {'contents': 'read'}:
        raise ValueError('actual images must be scanned on schedule without publication permissions')
    for name in ('credentials', 'images'):
        steps = packaging['jobs'][name]['steps']
        required_step(steps, 'scripts/check-image-vulnerabilities.py')
        if 'arm64' not in str(packaging['jobs'][name]['strategy']) or 'amd64' not in str(packaging['jobs'][name]['strategy']):
            raise ValueError('both native architectures must be scanned')
    if 'scan-freshness' not in packaging['jobs']['packaging-pass']['needs'] or \
            'test "$FRESHNESS" = success' not in str(packaging['jobs']['packaging-pass']['steps']):
        raise ValueError('scheduled image scans require fresh catalogs')
    required_step(packaging['jobs']['scan-freshness']['steps'], 'scripts/check-build-input-freshness.py')
    steps = data['publish-images']['jobs']['build']['steps']
    if data['publish-images']['jobs']['build'].get('needs') != 'ci-gate':
        raise ValueError('dependency/history gates must pass before the first registry write')
    builds = [step for step in steps if step.get('uses', '').startswith('docker/build-push-action@')]
    if len(builds) != 1 or builds[0]['with'].get('push') is not False or builds[0]['with'].get('load') is not True or \
            'push=true' in str(builds[0]) or 'outputs' in builds[0]['with']:
        raise ValueError('first publication write must follow the final staged scan')
    scan = required_step(steps, 'scripts/check-image-vulnerabilities.py')
    publish = required_step(steps, 'scripts/publish-scanned-image.py')
    if scan >= publish or steps[publish].get('id') != 'publish':
        raise ValueError('publication requires the passing exact staged subject')
    attest = next(step for step in steps if step.get('name') == 'Attest published image digest')
    if attest['with']['subject-digest'] != '${{ steps.publish.outputs.digest }}':
        raise ValueError('attestation must name the scanned publication digest')
    release = data['release']['jobs']
    if release['security-gate']['uses'] != './.github/workflows/security-scans.yml' or \
            'security-gate' not in release['host']['needs'] or "needs.security-gate.result == 'success'" not in release['host']['if']:
        raise ValueError('release publication must require dependency/history scans')


class ScannerWorkflows(unittest.TestCase):
    def test_current_workflows_enforce_scanners(self):
        require(workflows())

    def test_no_history_truncation_or_optional_secret_scan(self):
        for mutation in ('shallow', 'missing', 'conditional', 'warning'):
            data = workflows()
            steps = data['security-scans']['jobs']['history']['steps']
            gate = next(step for step in steps if 'scripts/scan-secrets.py' in step.get('run', ''))
            if mutation == 'shallow': steps[0]['with']['fetch-depth'] = 1
            elif mutation == 'missing': steps.remove(gate)
            elif mutation == 'conditional': gate['if'] = 'false'
            else: gate['continue-on-error'] = True
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): require(data)

    def test_pr_aggregator_dependency_or_freshness_omission_fails(self):
        for name in ('security-scans', 'dependency', 'freshness'):
            data = workflows()
            if name == 'security-scans': data['ci']['jobs']['ci-pass']['needs'].remove(name)
            elif name == 'dependency':
                steps = data['security-scans']['jobs']['dependencies']['steps']
                steps.remove(next(step for step in steps if 'scripts/audit-dependencies.py' in step.get('run', '')))
            else: data['artifact-packaging']['jobs']['packaging-pass']['needs'].remove('scan-freshness')
            with self.subTest(name=name), self.assertRaises(ValueError): require(data)

    def test_publishing_before_scan_or_ignoring_failure_is_rejected(self):
        for mutation in ('early-push', 'scan-missing', 'scan-warning', 'copy-first', 'wrong-subject', 'no-source-gate'):
            data = workflows()
            steps = data['publish-images']['jobs']['build']['steps']
            scan = next(step for step in steps if 'scripts/check-image-vulnerabilities.py' in step.get('run', ''))
            publish = next(step for step in steps if 'scripts/publish-scanned-image.py' in step.get('run', ''))
            if mutation == 'early-push':
                next(step for step in steps if step.get('uses', '').startswith('docker/build-push-action@'))['with']['push'] = True
            elif mutation == 'scan-missing': steps.remove(scan)
            elif mutation == 'scan-warning': scan['run'] += ' || true'
            elif mutation == 'copy-first': steps.remove(publish); steps.insert(0, publish)
            elif mutation == 'no-source-gate': data['publish-images']['jobs']['build'].pop('needs')
            else:
                next(step for step in steps if step.get('name') == 'Attest published image digest')['with']['subject-digest'] = '${{ steps.build.outputs.digest }}'
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): require(data)

    def test_each_final_image_scan_is_unconditional(self):
        for name in ('credentials', 'images'):
            data = workflows()
            step = next(step for step in data['artifact-packaging']['jobs'][name]['steps']
                        if 'scripts/check-image-vulnerabilities.py' in step.get('run', ''))
            step['if'] = 'false'
            with self.subTest(name=name), self.assertRaises(ValueError): require(data)

    def test_mobile_bundle_or_exact_disposition_negatives_cannot_be_optional(self):
        for job, script in [('mobile', 'scripts/check-mobile-bundle.cjs'),
                            ('policy', 'scripts/test-dependency-dispositions.py'),
                            ('policy', 'scripts/test-dependency-patches.py')]:
            data = workflows()
            steps = data['ci']['jobs']['mobile']['steps'] if job == 'mobile' else data['security-scans']['jobs'][job]['steps']
            step = next(step for step in steps if script in step.get('run', ''))
            step['continue-on-error'] = True
            with self.subTest(job=job, script=script), self.assertRaises(ValueError): require(data)

    def test_release_gate_cannot_be_omitted(self):
        data = workflows()
        data['release']['jobs']['host']['needs'].remove('security-gate')
        with self.assertRaises(ValueError): require(data)


if __name__ == '__main__':
    unittest.main()
