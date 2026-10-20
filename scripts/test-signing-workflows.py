#!/usr/bin/env python3
"""Mutate the real workflow graph to prove signing precedes publication."""
import copy
from pathlib import Path
import unittest
from workflow_yaml import load_unique

ROOT = Path(__file__).resolve().parents[1]
ACTION = 'actions/attest-build-provenance@977bb373ede98d70efdf65b84cb5f73e068dcc2a'


def workflows():
    names = ('ci', 'release', 'publish-images', 'release-signing', 'security-release-rehearsal')
    result = {n: load_unique((ROOT / '.github/workflows' / (n + '.yml')).read_text()) for n in names}
    for workflow in result.values():
        if True in workflow: workflow['on'] = workflow.pop(True)
    return result


def mandatory(step):
    if 'if' in step or step.get('continue-on-error') or any(s in step.get('run', '') for s in ('||', 'set +e', '::warning::')):
        raise ValueError('signing gate cannot be conditional, ignored or warning-only')


def positions(job, text):
    return [i for i, s in enumerate(job['steps']) if text in s.get('run', '')]


def require(data):
    ci, release, images, policy, rehearsal = [data[n] for n in ('ci', 'release', 'publish-images', 'release-signing', 'security-release-rehearsal')]
    if 'RELEASE_INTEGRITY_SIGN' in str(release) or 'RELEASE_INTEGRITY_PUBLISH_CMD' in str(release):
        raise ValueError('optional/arbitrary signing commands are forbidden')
    for w in (release, images):
        if w.get('permissions') != {'contents': 'read'}: raise ValueError('publish scopes must be job scoped')
    def authority(job):
        if job.get('environment') != 'official-release' or job.get('continue-on-error') or \
                job.get('env', {}).get('OFFICIAL_RELEASE_AUTHORIZATION') != '${{ vars.OFFICIAL_RELEASE_AUTHORIZATION }}':
            raise ValueError('protected signing authority required')
    def producer(job, scope, write_command):
        authority(job)
        prepare = positions(job, 'check-official-release.py prepare --scope ' + scope)
        verify = positions(job, 'check-official-release.py verify --scope ' + scope)
        writes = positions(job, write_command)
        actions = [i for i, s in enumerate(job['steps']) if s.get('uses', '').startswith('actions/attest-build-provenance@')]
        if len(prepare) != 1 or len(actions) != 1 or not verify or len(writes) != 1 or not prepare[0] < actions[0] < verify[0] < writes[0]:
            raise ValueError('mandatory prepare/sign/verify must precede first publication')
        action = job['steps'][actions[0]]
        if action['uses'] != ACTION or action.get('id') != 'official-signature' or \
                action.get('with') != {'subject-checksums': '${{ runner.temp }}/official-release/subjects.sha256', 'push-to-registry': False}:
            raise ValueError('sign exact prepared subjects without early registry publication')
        for i in prepare + actions + verify + writes:
            mandatory(job['steps'][i])
        gate = job['steps'][verify[0]]
        if gate.get('env', {}).get('SIGNED_BUNDLE') != '${{ steps.official-signature.outputs.bundle-path }}' or \
                'cp "$SIGNED_BUNDLE" "$RUNNER_TEMP/official-release/official-release-manifest.sigstore.json"' not in gate['run']:
            raise ValueError('gate must consume this invocation\'s signer output')
        for i in verify:
            if '--verifier "$RUNNER_TEMP/verifier/release-verifier"' not in job['steps'][i]['run']:
                raise ValueError('exact W6 verifier is mandatory')
    host = release['jobs']['host']
    producer(host, 'release', 'dist host ')
    if 'release-signing' not in host['needs'] or "needs.release-signing.result == 'success'" not in host['if']:
        raise ValueError('authentic W6 fixture job must gate official release')
    final = positions(host, '--final-upload'); github = positions(host, 'gh release create ')
    if len(final) != 1 or len(github) != 1 or final[0] >= github[0]: raise ValueError('GitHub Release requires reverified exact signed upload set')
    mandatory(host['steps'][final[0]])
    producer(images['jobs']['build'], 'platform', 'scripts/publish-scanned-image.py')
    producer(images['jobs']['merge'], 'index', 'scripts/check-image-publication-sboms.py publish-index')
    closure = images['jobs']['image-sbom-closure']
    authority(closure)
    # Closure signs evidence, while registry publication is exclusively in the
    # later promoter, which downloads and re-verifies that exact evidence.
    steps = closure['steps']
    prepare = positions(closure, 'check-official-release.py prepare --scope images')
    verify = positions(closure, 'check-official-release.py verify --scope images')
    signatures = [i for i, s in enumerate(steps) if s.get('uses') == ACTION]
    if len(prepare) != 1 or len(verify) != 1 or len(signatures) != 1 or not prepare[0] < signatures[0] < verify[0]:
        raise ValueError('complete image release manifest must be signed and verified')
    for i in prepare + verify + signatures: mandatory(steps[i])
    promote = images['jobs']['promote']
    authority(promote)
    if 'image-sbom-closure' not in promote['needs'] or \
            not any(s.get('with', {}).get('name') == 'official-image-release' for s in promote['steps']):
        raise ValueError('promotion must consume complete signed image manifest')
    for command, job in (('scripts/publish-scanned-image.py', images['jobs']['build']),
                         ('scripts/check-image-publication-sboms.py publish-index', images['jobs']['merge']),
                         ('scripts/promote-sbom-images.py', promote)):
        step = job['steps'][positions(job, command)[0]]; mandatory(step)
        if '--proof "$RUNNER_TEMP/official-release" --verifier "$RUNNER_TEMP/verifier/release-verifier"' not in step['run']:
            raise ValueError('publication helper must reverify signed evidence itself')
    needed = ci['jobs']['ci-pass']['needs']
    if not {'release-signing', 'signing-rehearsal'}.issubset(needed) or \
            set(ci['jobs']['signing-rehearsal']['needs']) != {'artifact-packaging', 'release-signing'}:
        raise ValueError('PR CI must require policy, actual packaging, and OIDC rehearsal')
    if 'schedule' not in rehearsal['on'] or 'workflow_call' not in rehearsal['on']:
        raise ValueError('rehearsal requires PR-call and scheduled coverage')
    if rehearsal['permissions'] != {'contents': 'read'} or policy['permissions'] != {'contents': 'read'}:
        raise ValueError('PR workflows default to read-only')
    for name, job in rehearsal['jobs'].items():
        expected = {'contents': 'read', 'id-token': 'write'} if name == 'rehearsal' else {'contents': 'read'}
        if job.get('permissions') != expected: raise ValueError('OIDC is confined to rehearsal job')
        text = str(job)
        if any(s in text for s in ('secrets.', 'packages:', 'attestations:', 'gh release', 'docker login', 'push-to-registry', 'regctl artifact', 'publish-index', 'publish-scanned-image.py')):
            raise ValueError('rehearsal cannot publish or require secrets')
    job = rehearsal['jobs']['rehearsal']
    for text in ('node scripts/sign-rehearsal.cjs security/signing-tools "$RUNNER_TEMP/rehearsal"',
                 'check-official-release.py verify --scope rehearsal', 'scripts/check-rehearsal-rejection.py'):
        matches = positions(job, text)
        if len(matches) != 1: raise ValueError('missing exact rehearsal signer/verification/rejection')
        mandatory(job['steps'][matches[0]])
    sign = positions(job, 'node scripts/sign-rehearsal.cjs')[0]
    verify = positions(job, 'check-official-release.py verify --scope rehearsal')[0]
    reject = positions(job, 'scripts/check-rehearsal-rejection.py')[0]
    if not sign < verify <= reject or (verify == reject and
            job['steps'][verify]['run'].index('check-official-release.py verify') >
            job['steps'][reject]['run'].index('scripts/check-rehearsal-rejection.py')):
        raise ValueError('authentic rehearsal verification must precede production rejection')
    for w in (policy, rehearsal):
        job = w['jobs']['policy' if w is policy else 'rehearsal']
        checks = positions(job, 'python3 scripts/check-signing-tools.py')
        installs = positions(job, '"$RUNNER_TEMP/signing-npm/npm" ci --ignore-scripts --no-audit --no-fund')
        if len(checks) != 1 or len(installs) != 1 or checks[0] >= installs[0] or \
                job['steps'][installs[0]].get('working-directory') != 'security/signing-tools':
            raise ValueError('signing tool installation requires reviewed lock and no hooks')
        for i in checks + installs: mandatory(job['steps'][i])
        for step in job['steps']:
            if any(command in step.get('run', '') for command in ('npm install', 'npm i ', 'npx ', '--global', 'npm exec')):
                raise ValueError('unlocked signing tool installation is forbidden')
    if 'cargo test --locked -p nyxid-cli --example release-verifier' not in str(policy):
        raise ValueError('unchanged W6 fixtures must execute')
    if 'cargo clippy --locked -p nyxid-cli --example release-verifier -- -D warnings' not in str(policy):
        raise ValueError('exact W6 adapter requires warnings-denied Clippy')


class Workflows(unittest.TestCase):
    def test_required_production_and_pr_gates(self): require(workflows())

    def test_missing_late_ignored_conditional_or_substituted_signing_rejected(self):
        for workflow, job in (('release', 'host'), ('publish-images', 'build'), ('publish-images', 'merge')):
            for mutation in ('missing', 'late', 'conditional', 'ignored', 'warning', 'substituted', 'no-verifier'):
                d = workflows(); steps = d[workflow]['jobs'][job]['steps']
                step = next(s for s in steps if 'check-official-release.py verify' in s.get('run', ''))
                if mutation == 'missing': steps.remove(step)
                elif mutation == 'late': steps.remove(step); steps.append(step)
                elif mutation == 'conditional': step['if'] = 'false'
                elif mutation == 'ignored': step['continue-on-error'] = True
                elif mutation == 'warning': step['run'] += ' || true'
                elif mutation == 'substituted': step['env']['SIGNED_BUNDLE'] = '/unrelated/bundle'
                else: step['run'] = step['run'].replace('--verifier', '--optional-verifier')
                with self.subTest(workflow=workflow, job=job, mutation=mutation), self.assertRaises(ValueError): require(d)

    def test_rehearsal_writes_scope_expansion_missing_rejection_or_ci_bypass_fail(self):
        for mutation in ('write', 'oidc', 'reject', 'ci', 'schedule', 'promotion', 'hooks', 'unlocked', 'lock'):
            d = workflows(); rehearsal = d['security-release-rehearsal']; job = rehearsal['jobs']['rehearsal']
            if mutation == 'write': job['steps'].append(dict(run='gh release create'))
            elif mutation == 'oidc': rehearsal['jobs']['scheduled-verifier']['permissions']['id-token'] = 'write'
            elif mutation == 'reject': job['steps'] = [s for s in job['steps'] if 'check-rehearsal-rejection.py' not in s.get('run', '')]
            elif mutation == 'ci': d['ci']['jobs']['ci-pass']['needs'].remove('signing-rehearsal')
            elif mutation == 'schedule': del rehearsal['on']['schedule']
            elif mutation == 'promotion': d['publish-images']['jobs']['promote']['needs'].remove('image-sbom-closure')
            elif mutation == 'hooks':
                next(s for s in job['steps'] if ' ci --ignore-scripts' in s.get('run', ''))['run'] = 'npm ci'
            elif mutation == 'unlocked': job['steps'].append(dict(run='npm install --global @sigstore/sign'))
            else:
                job['steps'] = [s for s in job['steps'] if 'python3 scripts/check-signing-tools.py' not in s.get('run', '')]
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): require(d)

    def test_every_production_boundary_requires_protected_authority(self):
        for workflow, name in (('release', 'host'), ('publish-images', 'build'), ('publish-images', 'merge'),
                               ('publish-images', 'image-sbom-closure'), ('publish-images', 'promote')):
            for mutation in ('environment', 'authority', 'ignored'):
                d = workflows(); job = d[workflow]['jobs'][name]
                if mutation == 'environment': del job['environment']
                elif mutation == 'authority': job['env']['OFFICIAL_RELEASE_AUTHORIZATION'] = 'disabled'
                else: job['continue-on-error'] = True
                with self.subTest(workflow=workflow, job=name, mutation=mutation), self.assertRaises(ValueError): require(d)

    def test_production_rejection_cannot_precede_authentic_rehearsal_verification(self):
        d = workflows(); steps = d['security-release-rehearsal']['jobs']['rehearsal']['steps']
        verification = next(s for s in steps if 'check-official-release.py verify' in s.get('run', ''))
        rejection = verification['run'].splitlines()[-1]
        verification['run'] = rejection + '\n' + '\n'.join(verification['run'].splitlines()[:-1]) + '\n'
        with self.assertRaisesRegex(ValueError, 'verification must precede production rejection'): require(d)


if __name__ == '__main__': unittest.main()
