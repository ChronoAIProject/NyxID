#!/usr/bin/env python3
"""Required packaging and publication wiring, including negative mutations."""
import copy
import fnmatch
import json
from pathlib import Path
import unittest

from workflow_yaml import load_unique

ROOT = Path(__file__).resolve().parents[1]


def workflows():
    return {name: load_unique((ROOT / '.github/workflows' / (name + '.yml')).read_text())
            for name in ('ci', 'artifact-packaging', 'publish-images', 'release')}


def require_gates(data):
    ci, packaging, publisher = (data[name] for name in ('ci', 'artifact-packaging', 'publish-images'))
    caller = ci['jobs']['artifact-packaging']
    if caller['uses'] != './.github/workflows/artifact-packaging.yml' or caller['permissions'] != {'contents': 'read'} or \
            'artifact-packaging' not in ci['jobs']['ci-pass']['needs'] or \
            'artifact-packaging=${{ needs.artifact-packaging.result }}' not in str(ci['jobs']['ci-pass']['steps']):
        raise ValueError('CI must require packaging')
    if "needs.changes.outputs.packaging == 'true'" not in caller['if'] or \
            ci['jobs']['changes']['outputs'].get('packaging') != '${{ steps.filter.outputs.packaging }}':
        raise ValueError('packaging verifier changes must select the PR jobs')
    filters = load_unique(next(step['with']['filters'] for step in ci['jobs']['changes']['steps']
                               if step.get('uses', '').startswith('dorny/paths-filter@')))['packaging']
    for path in ('scripts/source_artifacts.py', 'scripts/image_artifacts.py', 'scripts/credential_packaging.py',
                 'scripts/check-cli-archive-identity.py', 'scripts/test-packaging-workflows.py',
                 'scripts/publication_contract.py', 'scripts/export-build-source.py', 'docs/site/index.md'):
        if not any(fnmatch.fnmatchcase(path, pattern) for pattern in filters):
            raise ValueError('packaging verifier input missing from PR selection')
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
    image_job = jobs['images']
    if set(image_job['strategy']['matrix']['arch']) != {'amd64', 'arm64'} or \
            set(image_job['strategy']['matrix']['component']) != {'node-agent', 'nyxid-node-machine', 'nyxid-machine-updater',
                'machine-node-only', 'machine-hosted-desktop', 'mcp-demo'} or \
            'check-image-identity.py' not in str(image_job['steps']) or 'check-image-identity.py' not in text:
        raise ValueError('all actual public application image identities required')
    hosted = next(item for item in image_job['strategy']['matrix']['include'] if item.get('component') == 'machine-hosted-desktop')
    if hosted.get('target') != 'desktop' or '--build-context' in str(image_job):
        raise ValueError('PR hosted evidence must use the real public desktop target')
    operator = [step for step in image_job['steps'] if 'scripts/build-local-image.py' in step.get('run', '')]
    if len(operator) != 1 or operator[0].get('if') != "matrix.component == 'machine-hosted-desktop'" or \
            operator[0].get('continue-on-error') or any(text not in operator[0]['run'] for text in (
                '--file cli/Dockerfile.machine-hosted --target desktop', "['identity_check']['status'] == 'passed'",
                "['image']['tagged'] is True", "actual == receipt['image']['id']")):
        raise ValueError('PR hosted desktop must exercise mandatory operator verification before tagging')
    targets = {item['target'] for item in jobs['archives']['strategy']['matrix']['include']}
    policy = json.loads((ROOT / 'security/publication-policy.json').read_text())
    if targets != set(policy['cli']['targets']) or 'run-dist-build.py' not in str(jobs['archives']['steps']) or \
            'check-cli-archive-identity.py' not in str(jobs['archives']['steps']):
        raise ValueError('every actual stripped CLI archive identity required')
    for steps, checker, end in ((publisher['jobs']['build']['steps'], 'check-image-identity.py', 'Attest published image digest'),
                                (data['release']['jobs']['build-local-artifacts']['steps'], 'check-cli-archive-identity.py', 'Upload artifacts')):
        gates = [i for i, step in enumerate(steps) if checker in step.get('run', '')]
        stops = [i for i, step in enumerate(steps) if step.get('name') == end]
        if len(gates) != 1 or not stops or gates[0] >= stops[0] or steps[gates[0]].get('continue-on-error') or \
                'if' in steps[gates[0]] or any(bypass in steps[gates[0]]['run'] for bypass in ('||', 'set +e')):
            raise ValueError('final identity gate must precede required publication output')


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

    def test_missing_final_image_or_archive_identity_gate_fails(self):
        for workflow, job, checker in (('artifact-packaging', 'images', 'check-image-identity.py'),
                                       ('artifact-packaging', 'archives', 'check-cli-archive-identity.py'),
                                       ('publish-images', 'build', 'check-image-identity.py'),
                                       ('release', 'build-local-artifacts', 'check-cli-archive-identity.py')):
            data = workflows()
            value = data[workflow]['jobs'][job]
            value['steps'] = [step for step in value['steps'] if checker not in step.get('run', '')]
            with self.subTest(workflow=workflow, job=job), self.assertRaises(ValueError): require_gates(data)

    def test_private_hosted_inputs_cannot_enter_pr_job(self):
        data = workflows()
        matrix = data['artifact-packaging']['jobs']['images']['strategy']['matrix']
        next(item for item in matrix['include'] if item.get('component') == 'machine-hosted-desktop')['target'] = 'hosted'
        with self.assertRaises(ValueError): require_gates(data)

    def test_missing_cli_target_fails(self):
        data = workflows()
        data['artifact-packaging']['jobs']['archives']['strategy']['matrix']['include'].pop()
        with self.assertRaises(ValueError): require_gates(data)

    def test_operator_gate_must_have_real_pr_coverage(self):
        data = workflows()
        steps = data['artifact-packaging']['jobs']['images']['steps']
        steps[:] = [step for step in steps if 'scripts/build-local-image.py' not in step.get('run', '')]
        with self.assertRaisesRegex(ValueError, 'operator verification'): require_gates(data)

    def test_verifier_changes_cannot_skip_pr_jobs(self):
        data = workflows()
        data['ci']['jobs']['artifact-packaging']['if'] = "inputs.force-all || needs.changes.outputs.ci == 'true'"
        with self.assertRaisesRegex(ValueError, 'select the PR jobs'): require_gates(data)

    def test_publication_identity_cannot_be_conditional_warning_only_or_late(self):
        for mutation in ('conditional', 'continue', 'ignore', 'late'):
            data = workflows()
            steps = data['publish-images']['jobs']['build']['steps']
            gate = next(step for step in steps if 'check-image-identity.py' in step.get('run', ''))
            if mutation == 'conditional': gate['if'] = 'false'
            elif mutation == 'continue': gate['continue-on-error'] = True
            elif mutation == 'ignore': gate['run'] += ' || true'
            else:
                steps.remove(gate)
                steps.append(gate)
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, 'publication output'):
                require_gates(data)


if __name__ == '__main__':
    unittest.main()
