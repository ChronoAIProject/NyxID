#!/usr/bin/env python3
"""Required packaging and publication wiring, including negative mutations."""
import copy
from pathlib import Path
import unittest

from workflow_yaml import load_unique

ROOT = Path(__file__).resolve().parents[1]


def workflows():
    return {name: load_unique((ROOT / '.github/workflows' / (name + '.yml')).read_text())
            for name in ('ci', 'artifact-packaging', 'publish-images')}


def require_gates(data):
    ci, packaging, publisher = (data[name] for name in ('ci', 'artifact-packaging', 'publish-images'))
    caller = ci['jobs']['artifact-packaging']
    if caller['uses'] != './.github/workflows/artifact-packaging.yml' or caller['permissions'] != {'contents': 'read'} or \
            'artifact-packaging' not in ci['jobs']['ci-pass']['needs'] or \
            'artifact-packaging=${{ needs.artifact-packaging.result }}' not in str(ci['jobs']['ci-pass']['steps']):
        raise ValueError('CI must require packaging')
    if packaging['permissions'] != {'contents': 'read'}:
        raise ValueError('PR packaging permissions')
    jobs = packaging['jobs']
    if {item['arch'] for item in jobs['credentials']['strategy']['matrix']['include']} != {'amd64', 'arm64'}:
        raise ValueError('both native credential architectures required')
    for job in jobs.values():
        if job.get('continue-on-error') or 'secrets' in job or 'id-token' in str(job):
            raise ValueError('PR packaging cannot bypass checks or need secrets/OIDC')
        for step in job.get('steps', []):
            if step.get('continue-on-error'):
                raise ValueError('packaging step cannot be warning-only')
    text = str(jobs['credentials']['steps'])
    for required in ('backend/Dockerfile', 'frontend/Dockerfile', 'check-credential-image.py',
                     '--compare-image nyxid-credential-frontend:ci', '--source-root', '--commit', '--epoch'):
        if required not in text:
            raise ValueError('actual image credential extraction/parity required')
    if set(jobs['packaging-pass']['needs']) != set(jobs) - {'packaging-pass'}:
        raise ValueError('packaging aggregator must require every job')
    if 'credential-parity' not in publisher['jobs']['merge']['needs'] or \
            publisher['jobs']['credential-parity']['needs'] != 'build' or \
            'check-published-credentials.py' not in str(publisher['jobs']['credential-parity']['steps']) or \
            publisher['jobs']['credential-parity'].get('continue-on-error'):
        raise ValueError('publication promotion must require actual credential subjects')


class PackagingWorkflows(unittest.TestCase):
    def test_current_workflows_require_actual_packaging(self):
        require_gates(workflows())

    def test_removing_pr_gate_fails(self):
        data = workflows()
        data['ci']['jobs']['ci-pass']['needs'].remove('artifact-packaging')
        with self.assertRaises(ValueError): require_gates(data)

    def test_removing_publication_gate_fails(self):
        data = workflows()
        data['publish-images']['jobs']['merge']['needs'].remove('credential-parity')
        with self.assertRaises(ValueError): require_gates(data)

    def test_dropping_native_architecture_fails(self):
        data = workflows()
        data['artifact-packaging']['jobs']['credentials']['strategy']['matrix']['include'].pop()
        with self.assertRaises(ValueError): require_gates(data)

    def test_warning_only_or_missing_extraction_fails(self):
        for mutation in ('continue', 'extract'):
            data = copy.deepcopy(workflows())
            job = data['artifact-packaging']['jobs']['credentials']
            if mutation == 'continue': job['continue-on-error'] = True
            else: job['steps'] = [step for step in job['steps'] if 'check-credential-image.py' not in str(step)]
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): require_gates(data)


if __name__ == '__main__':
    unittest.main()
