#!/usr/bin/env python3
"""Required actual-byte SBOM gates, with workflow mutation negatives."""
import copy
import fnmatch
from pathlib import Path
import unittest
from workflow_yaml import load_unique

ROOT = Path(__file__).resolve().parents[1]


def workflows():
    return {n:load_unique((ROOT/'.github/workflows'/ (n+'.yml')).read_text()) for n in ('ci','artifact-packaging','publish-images','release')}


def gate(job, command):
    matches=[(i,s) for i,s in enumerate(job['steps']) if command in s.get('run','')]
    if len(matches)!=1:raise ValueError('missing/duplicate mandatory SBOM command '+command)
    position,step=matches[0]
    if job.get('continue-on-error') or step.get('continue-on-error') or 'if' in step or any(v in step['run'] for v in ('||','set +e')):
        raise ValueError('SBOM gate cannot be conditional or warning-only')
    return position


def require(data):
    ci,pr,publish,release=[data[n] for n in ('ci','artifact-packaging','publish-images','release')]
    if pr['permissions']!={'contents':'read'} or 'artifact-packaging' not in ci['jobs']['ci-pass']['needs']:
        raise ValueError('PR SBOM jobs must be required and read-only')
    filters=load_unique(next(s['with']['filters'] for s in ci['jobs']['changes']['steps'] if s.get('uses','').startswith('dorny/paths-filter@')))['packaging']
    for path in ('scripts/image_sbom.py','scripts/sbom_evidence.py','scripts/check-release-sboms.py','scripts/check-image-publication-sboms.py',
                 'scripts/validate-cyclonedx.py','scripts/test_cyclonedx.py','scripts/check-dist-artifacts.py','scripts/shipped_composition_sbom.py','scripts/oci_subjects.py','scripts/setup-sbom-tools.sh',
                 'security/schemas/cyclonedx-1.6/bom-1.6.schema.json','security/python-sbom-requirements.txt'):
        if not any(fnmatch.fnmatchcase(path,p) for p in filters):raise ValueError('SBOM edit must select PR evidence')
    jobs=pr['jobs']
    if set(jobs['packaging-pass']['needs']) != set(jobs)-{'packaging-pass'}:raise ValueError('every PR SBOM job must gate CI')
    for job in jobs.values():
        if job.get('continue-on-error') or 'id-token' in str(job) or 'secrets' in job:raise ValueError('PR SBOM jobs cannot require publication permissions')
        for s in job.get('steps',[]):
            if any(c in s.get('run','') for c in ('publish-scanned-image.py','publish-index','promote-sbom-images.py','artifact put','gh release')):
                raise ValueError('PR jobs cannot publish SBOMs or image/release subjects')
    for job in ('credentials','images'):gate(jobs[job],'scripts/check-image-sbom.py')
    upload = next(s for s in jobs['images']['steps'] if s.get('name') == 'Preserve verified application SBOM evidence without publication')
    if "'pr-auxiliary-sbom'" not in upload['with']['name'] or \
            "fromJSON('[\"node-agent\",\"nyxid-node-machine\",\"nyxid-machine-updater\"]')" not in upload['with']['name']:
        raise ValueError('auxiliary image evidence must not enter the official publication set')
    gate(jobs['sbom-policy'],'scripts/test-sbom-evidence.py');gate(jobs['sbom-policy'],'scripts/test-sbom-workflows.py')
    gate(jobs['archive-sboms'],'scripts/check-dist-artifacts.py')
    if set(jobs['archive-sboms']['needs'])!={'archives','sbom-policy'}:raise ValueError('all actual archives must precede SBOM composition')
    if set(jobs['image-sbom-closure']['needs'])!={'credentials','images','sbom-policy'}:raise ValueError('all native image SBOMs must precede set closure')
    gate(jobs['image-sbom-closure'],'scripts/check-image-publication-sboms.py set')
    gate(jobs['image-sbom-closure'],'scripts/check-image-publication-sboms.py closure')
    build=publish['jobs']['build']
    if gate(build,'scripts/check-image-sbom.py')>=gate(build,'scripts/publish-scanned-image.py'):
        raise ValueError('SBOM verification must precede first registry copy')
    merge=publish['jobs']['merge'];gate(merge,'scripts/check-image-publication-sboms.py index')
    if gate(merge,'scripts/check-image-publication-sboms.py index')>=gate(merge,'scripts/check-image-publication-sboms.py publish-index'):
        raise ValueError('index composition must precede digest publication')
    if 'imagetools create' in str(merge) or 'promote-sbom-images.py' in str(merge):raise ValueError('index assembly cannot promote before complete set closure')
    gate(publish['jobs']['image-sbom-closure'],'scripts/check-image-publication-sboms.py closure')
    promote=publish['jobs']['promote']
    if set(promote['needs'])!={'merge','image-sbom-closure','credential-parity'}:raise ValueError('tag promotion requires complete image SBOM closure')
    gate(promote,'scripts/promote-sbom-images.py')
    gate(release['jobs']['release-integrity-manifest'],'scripts/check-asset-sbom.py')
    gate(release['jobs']['sbom-gate'],'scripts/check-dist-artifacts.py');gate(release['jobs']['sbom-gate'],'scripts/check-release-sboms.py')
    if set(release['jobs']['sbom-gate']['needs'])!={'plan','build-local-artifacts','build-global-artifacts','release-integrity-manifest'}:
        raise ValueError('complete actual release set must precede SBOM closure')
    host=release['jobs']['host']
    if 'sbom-gate' not in host['needs'] or "needs.sbom-gate.result == 'success'" not in host['if']:
        raise ValueError('release hosting requires successful complete SBOM closure')
    checks=[i for i,s in enumerate(host['steps']) if 'scripts/check-release-sboms.py' in s.get('run','')]
    writes=[i for i,s in enumerate(host['steps']) if any(c in s.get('run','') for c in ('dist host ','gh release create '))]
    if len(checks)!=2 or len(writes)!=2 or not checks[0]<writes[0]<checks[1]<writes[1]:raise ValueError('release bytes must be reverified before each publication path')
    signer = next(i for i,s in enumerate(host['steps']) if s.get('name') == 'Sign and publish release integrity manifest')
    if signer <= checks[0]: raise ValueError('manifest publication must follow complete SBOM closure')
    for i in checks:
        if any(k in host['steps'][i] for k in ('if','continue-on-error')) or '||' in host['steps'][i]['run']:raise ValueError('release SBOM replay must fail closed')


class SbomWorkflows(unittest.TestCase):
    def test_current_required_subject_and_composition_gates(self):require(workflows())

    def test_missing_conditional_warning_ignored_and_late_gates_fail(self):
        for workflow,job,command in (('artifact-packaging','credentials','scripts/check-image-sbom.py'),
                ('artifact-packaging','images','scripts/check-image-sbom.py'),('artifact-packaging','archive-sboms','scripts/check-dist-artifacts.py'),
                ('publish-images','build','scripts/check-image-sbom.py'),('publish-images','merge','scripts/check-image-publication-sboms.py index'),
                ('release','sbom-gate','scripts/check-release-sboms.py')):
            for mutation in ('missing','conditional','warning','ignored'):
                d=workflows();j=d[workflow]['jobs'][job];s=next(s for s in j['steps'] if command in s.get('run',''))
                if mutation=='missing':j['steps'].remove(s)
                elif mutation=='conditional':s['if']='false'
                elif mutation=='warning':s['continue-on-error']=True
                else:s['run']+=' || true'
                with self.subTest(workflow=workflow,job=job,mutation=mutation),self.assertRaises(ValueError):require(d)
        d=workflows();steps=d['publish-images']['jobs']['build']['steps'];s=next(s for s in steps if 'scripts/check-image-sbom.py' in s.get('run',''));steps.remove(s);steps.append(s)
        with self.assertRaises(ValueError):require(d)

    def test_partial_publication_dependencies_or_pr_bypass_rejected(self):
        for kind in ('images','release','pr','write','filter','early-tags','auxiliary'):
            d=workflows()
            if kind=='images':d['publish-images']['jobs']['promote']['needs'].remove('image-sbom-closure')
            elif kind=='release':d['release']['jobs']['host']['needs'].remove('sbom-gate')
            elif kind=='pr':d['artifact-packaging']['jobs']['packaging-pass']['needs'].remove('archive-sboms')
            elif kind=='write':d['artifact-packaging']['jobs']['image-sbom-closure']['steps'].append({'run':'regctl artifact put'})
            elif kind=='filter':
                step=next(s for s in d['ci']['jobs']['changes']['steps'] if s.get('uses','').startswith('dorny/paths-filter@'));step['with']['filters']='packaging: []'
            elif kind=='auxiliary':
                step=next(s for s in d['artifact-packaging']['jobs']['images']['steps'] if s.get('name')=='Preserve verified application SBOM evidence without publication')
                step['with']['name']='pr-image-sbom-${{ matrix.component }}-${{ matrix.arch }}'
            else:d['publish-images']['jobs']['merge']['steps'].append({'run':'docker buildx imagetools create -t latest'})
            with self.subTest(kind=kind),self.assertRaises(ValueError):require(d)


if __name__=='__main__':unittest.main()
