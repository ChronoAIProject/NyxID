"""Signing contracts derived from final-byte SBOM/scan evidence, never a glob.

The JSON manifest is a signed subject alongside every artifact it describes.
Receipts do not authorize publication: every publication invocation reruns W6.
"""
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

from publication_contract import file_digest, source_identity, validate_plan
from sbom_evidence import load_json, write

REPOSITORY = 'https://github.com/ChronoAIProject/NyxID'
POLICY = 'github-oidc-public-good-v1'
MANIFEST = 'official-release-manifest.json'
BUNDLE = 'official-release-manifest.sigstore.json'
WORKFLOWS = {'release': 'release.yml', 'platform': 'publish-images.yml',
             'index': 'publish-images.yml', 'images': 'publish-images.yml',
             'rehearsal': 'security-release-rehearsal.yml'}


def identity(scope, ref):
    if scope not in WORKFLOWS:
        raise ValueError('unknown signing scope')
    tag = re.fullmatch(r'refs/tags/v[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?', ref)
    if scope == 'release' and not tag:
        raise ValueError('official CLI releases require the existing production tag identity')
    if scope in ('platform', 'index', 'images') and not (tag or ref == 'refs/heads/main'):
        raise ValueError('official image signing requires main or a production version tag')
    if scope == 'rehearsal' and not (tag or ref == 'refs/heads/main' or
            re.fullmatch(r'refs/pull/[1-9][0-9]*/merge', ref) or re.fullmatch(r'refs/heads/[A-Za-z0-9_./-]+', ref)):
        raise ValueError('invalid rehearsal workflow ref')
    return REPOSITORY + '/.github/workflows/' + WORKFLOWS[scope] + '@' + ref


def record(name, digest):
    if not re.fullmatch(r'sha256:[0-9a-f]{64}', digest) or not isinstance(name, str) or \
            not name or any(c.isspace() for c in name) or '\\' in name or '..' in name.split('/'):
        raise ValueError('invalid signing subject name/digest')
    return dict(name=name, digest=dict(sha256=digest[7:]))


def evidence_files(directory, prefix):
    result = {}
    for path in sorted(directory.rglob('*')):
        if path.is_symlink(): raise ValueError('linked signing evidence')
        if path.is_file(): result[prefix + path.relative_to(directory).as_posix()] = file_digest(path)
    if not result: raise ValueError('empty signing evidence')
    return result


def manifest(scope, ref, *, plan=None, artifacts=None, evidence=None, stage=None, repository=None,
             archive_plan=None, archives=None, archive_evidence=None):
    """Recompute the signed contract from each independent mandatory C-6 gate."""
    signer = identity(scope, ref)
    if scope == 'release':
        from release_sboms import verify
        receipt = verify(plan, artifacts, evidence)
        if ref != 'refs/tags/' + plan['tag']: raise ValueError('release tag differs from source version')
        subjects = {n: d for n, d in receipt['subjects'].items()}
        subjects.update(receipt['outputs'])
        subjects['release-sboms.json'] = file_digest(evidence / 'release-sboms.json')
        credential = load_json(artifacts / 'releases.json')
        if credential.get('git_commit') != plan['source']['commit']:
            raise ValueError('credential manifest source differs from official release')
        extra = dict(credential_accept=credential['credential_accept'])
        source = plan['source']
    elif scope == 'platform':
        from image_sbom import verify
        from publish_sbom import repository_check
        repository_check(repository)
        receipt = verify(stage); source = receipt['source']
        if repository != 'ghcr.io/chronoaiproject/nyxid/' + receipt['component']:
            raise ValueError('image signing repository differs from component')
        subjects = {repository: receipt['subject']['digest']}
        subjects.update(evidence_files(stage / 'sbom', 'sbom/'))
        subjects['image-scan.json'] = file_digest(stage / 'image-scan.json')
        subjects['trivy.json'] = file_digest(stage / 'trivy.json')
        extra = dict(component=receipt['component'], platform=receipt['platform'])
    elif scope == 'index':
        from image_publication_sboms import verify_index
        receipt = verify_index(evidence); source = receipt['source']
        if repository != 'ghcr.io/chronoaiproject/nyxid/' + receipt['component']:
            raise ValueError('index signing repository differs from component')
        subjects = {repository: receipt['subject']['digest']}
        subjects.update(evidence_files(evidence, 'sbom/'))
        extra = dict(component=receipt['component'])
    else:
        from image_publication_sboms import closure
        receipt = closure(plan, evidence); source = plan['source']
        subjects = {s['name'] + '@' + receipt['subjects'][s['id']]: receipt['subjects'][s['id']] for s in plan['subjects']}
        subjects.update(evidence_files(evidence, 'sbom/'))
        extra = {}
        if scope == 'rehearsal':
            from release_sboms import verify_archives
            validate_plan(archive_plan)
            if archive_plan['source'] != source: raise ValueError('rehearsal archive/image sources differ')
            records = verify_archives(archive_plan, archives, archive_evidence)
            for identifier, (entry, _) in records.items():
                subjects[identifier.removeprefix('file:')] = entry['digest']
            subjects.update(evidence_files(archive_evidence, 'archive-sbom/'))
            extra['coverage'] = 'all planned images/indices and four actual CLI archives; no global installers'
    source_identity(**source)
    return dict(schema='nyxid.official-release.v1', scope=scope, source=source, ref=ref,
                identity=signer, issuer='https://token.actions.githubusercontent.com',
                subjects=[record(n, d) for n, d in sorted(subjects.items())], **extra)


def signing_subjects(document, manifest_path):
    result = document['subjects'] + [record(MANIFEST, file_digest(manifest_path))]
    pairs = [(s['name'], s['digest']['sha256']) for s in result]
    if len(pairs) != len(set(pairs)) or len(result) > 1024:
        raise ValueError('duplicate/excessive official signing subjects')
    return sorted(result, key=lambda s: (s['name'], s['digest']['sha256']))


def prepare(document, directory):
    directory.mkdir(exist_ok=False)
    write(directory / MANIFEST, document)
    subjects = signing_subjects(document, directory / MANIFEST)
    (directory / 'subjects.sha256').write_text(''.join(s['digest']['sha256'] + '  ' + s['name'] + '\n' for s in subjects))
    # The nonpublishing signer uses the same statement/subjects as the official
    # producer, without GitHub's attestation-upload API or registry credentials.
    predicate = dict(buildDefinition=dict(buildType='https://actions.github.io/buildtypes/workflow/v1',
        externalParameters=dict(workflow=dict(repository=REPOSITORY, path='.github/workflows/' + WORKFLOWS[document['scope']], ref=document['ref'])),
        resolvedDependencies=[dict(uri='git+' + REPOSITORY + '@' + document['ref'], digest=dict(gitCommit=document['source']['commit']))]),
        runDetails=dict(builder=dict(id=document['identity'])))
    write(directory / 'statement.json', dict(_type='https://in-toto.io/Statement/v1',
        predicateType='https://slsa.dev/provenance/v1', subject=subjects, predicate=predicate))


def verify(document, directory, verifier):
    if load_json(directory / MANIFEST) != document:
        raise ValueError('official manifest differs from current final-byte publication closure')
    bundle_path = directory / BUNDLE
    if bundle_path.stat().st_size > 512 * 1024: raise ValueError('oversized W6 bundle')
    bundle = load_json(bundle_path)
    raw = base64.b64decode(bundle['dsseEnvelope']['payload'], validate=True)
    if len(raw) > 256 * 1024: raise ValueError('oversized W6 statement')
    from publication_contract import unique_object
    payload = json.loads(raw, object_pairs_hook=unique_object)
    expected = signing_subjects(document, directory / MANIFEST)
    if payload.get('subject') != expected:
        # Producers may preserve checksum file order differently. Compare an
        # exact multiset, retaining duplicate/missing/extra/name checks.
        if sorted(payload.get('subject', []), key=lambda s: (s['name'], s['digest']['sha256'])) != expected:
            raise ValueError('signature subjects differ from exact official publication set')
    predicate = payload['predicate']
    workflow = predicate['buildDefinition']['externalParameters']['workflow']
    if workflow != dict(repository=REPOSITORY, path='.github/workflows/' + WORKFLOWS[document['scope']], ref=document['ref']) or \
            predicate['runDetails']['builder']['id'] != document['identity'] or \
            predicate['buildDefinition']['resolvedDependencies'] != [dict(
                uri='git+' + REPOSITORY + '@' + document['ref'],
                digest=dict(gitCommit=document['source']['commit']))]:
        raise ValueError('provenance source/workflow differs from official manifest')
    # Verify every subject in one TUF-authenticated session using the exact W6
    # code. A receipt or a successful external signer cannot replace this call.
    request = directory / 'verification-request.json'
    write(request, dict(bundle=str(bundle_path.resolve()), identity=document['identity'],
                        digests=sorted({s['digest']['sha256'] for s in expected})))
    subprocess.run([str(verifier), str(request.resolve())], check=True)
    return file_digest(directory / MANIFEST)


def authorize():
    if os.environ.get('OFFICIAL_RELEASE_AUTHORIZATION') != POLICY:
        raise ValueError('protected official-release environment must provision OFFICIAL_RELEASE_AUTHORIZATION=' + POLICY)


def check_upload(document, artifacts, proof):
    if document['scope'] != 'release': raise ValueError('final file upload set applies only to CLI releases')
    expected = {s['name'] for s in document['subjects']} | {MANIFEST, BUNDLE}
    paths = list(artifacts.iterdir())
    if {p.name for p in paths} != expected or any(p.is_symlink() or not p.is_file() for p in paths):
        raise ValueError('GitHub release upload has missing/extra/linked/unsigned files')
    for subject in document['subjects']:
        if file_digest(artifacts / subject['name']) != 'sha256:' + subject['digest']['sha256']:
            raise ValueError('uploaded artifact differs from signed digest')
    for name in (MANIFEST, BUNDLE):
        if (artifacts / name).read_bytes() != (proof / name).read_bytes():
            raise ValueError('uploaded signing evidence differs from verified evidence')


def attach(regctl, destination, directory):
    # Called only after verify(), and only in publication workflows. Raw bundle
    # plus manifest are consumable offline; W6 continues using the GitHub API.
    for name, media in ((MANIFEST, 'application/vnd.nyxid.official-release.v1+json'),
                        (BUNDLE, 'application/vnd.dev.sigstore.bundle.v0.3+json')):
        subprocess.run([str(regctl), 'artifact', 'put', '--subject', destination,
                        '--artifact-type', media, '--file', str(directory / name)], check=True)
