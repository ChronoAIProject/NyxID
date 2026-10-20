#!/usr/bin/env python3
"""Final-byte manifest, exact subject, mandatory verifier and write ordering tests."""
import base64
import copy
from contextlib import redirect_stdout
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import official_release as signing
import image_publication_sboms as images
from sbom_evidence import load_json, write
from test_publication_contract import images_plan

ROOT = Path(__file__).resolve().parents[1]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    result = importlib.util.module_from_spec(spec); spec.loader.exec_module(result)
    return result


fixtures = module('signing_sbom_fixtures', 'scripts/test-sbom-evidence.py')
publisher = module('signing_image_publisher', 'scripts/publish-scanned-image.py')
index_publisher = module('signing_index_publisher', 'scripts/check-image-publication-sboms.py')
promoter = module('signing_promoter', 'scripts/promote-sbom-images.py')
rejection = module('signing_rehearsal_rejection', 'scripts/check-rehearsal-rejection.py')


class Signing(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix='nyxid-signing-test-'); self.addCleanup(temp.cleanup)
        self.root = Path(temp.name); self.proof = self.root / 'proof'
        self.plan = images_plan(); self.evidence = self.root / 'evidence'; self.evidence.mkdir()
        for component in sorted({s['component'] for s in self.plan['subjects']}):
            paths = [self.root / (component + '-' + a) for a in ('amd64', 'arm64')]
            for architecture, path in zip(('amd64', 'arm64'), paths):
                fixtures.platform_bundle(path, component, architecture)
            images.index(component, self.plan['source'], paths, self.evidence / component)
        self.document = signing.manifest('images', 'refs/tags/v0.73.0', plan=self.plan, evidence=self.evidence)
        signing.prepare(self.document, self.proof)
        self.payload = load_json(self.proof / 'statement.json')
        self.bundle()
        self.verifier = self.root / 'verifier'; self.verifier.write_text('#!/bin/sh\nexit 0\n'); self.verifier.chmod(0o755)

    def bundle(self):
        write(self.proof / signing.BUNDLE, dict(dsseEnvelope=dict(payload=base64.b64encode(json.dumps(self.payload).encode()).decode())))

    def verify(self):
        return signing.verify(self.document, self.proof, self.verifier)

    def test_exact_complete_image_set_and_manifest_are_signed(self):
        subjects = self.document['subjects']
        self.assertEqual(sum(s['name'].startswith('ghcr.io/') for s in subjects), 15)
        self.assertEqual(len(self.payload['subject']), len(subjects) + 1)
        self.assertTrue(self.verify().startswith('sha256:'))

    def test_missing_bundle_manifest_and_verifier_fail(self):
        for path in (self.proof / signing.BUNDLE, self.proof / signing.MANIFEST, self.verifier):
            original = path.read_bytes(); path.unlink()
            with self.subTest(path=path.name), self.assertRaises((OSError, ValueError)): self.verify()
            path.write_bytes(original); path.chmod(0o755)

    def test_failed_or_nonexecutable_verifier_fails(self):
        self.verifier.write_text('#!/bin/sh\nexit 31\n')
        with self.assertRaises(subprocess.CalledProcessError): self.verify()
        self.verifier.chmod(0o644)
        with self.assertRaises(PermissionError): self.verify()

    def test_changed_manifest_cannot_be_replaced_by_a_receipt(self):
        original = load_json(self.proof / signing.MANIFEST)
        changed = copy.deepcopy(original); changed['source']['commit'] = '0' * 40
        write(self.proof / signing.MANIFEST, changed)
        with self.assertRaisesRegex(ValueError, 'manifest differs'): self.verify()

    def test_added_missing_duplicate_renamed_and_changed_digest_subject_fail(self):
        original = copy.deepcopy(self.payload)
        for mutation in ('added', 'missing', 'duplicate', 'renamed', 'digest'):
            self.payload = copy.deepcopy(original)
            if mutation == 'added': self.payload['subject'].append(signing.record('injected', 'sha256:' + 'e' * 64))
            elif mutation == 'missing': self.payload['subject'].pop()
            elif mutation == 'duplicate': self.payload['subject'].append(self.payload['subject'][0])
            elif mutation == 'renamed': self.payload['subject'][0]['name'] += '.other'
            else: self.payload['subject'][0]['digest']['sha256'] = '0' * 64
            self.bundle()
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, 'subjects differ'): self.verify()

    def test_changed_provenance_source_workflow_ref_and_builder_fail(self):
        original = copy.deepcopy(self.payload)
        for mutation in ('source', 'source-uri', 'extra-source', 'workflow', 'ref', 'builder'):
            self.payload = copy.deepcopy(original); p = self.payload['predicate']
            if mutation == 'source': p['buildDefinition']['resolvedDependencies'][0]['digest']['gitCommit'] = '0' * 40
            elif mutation == 'source-uri': p['buildDefinition']['resolvedDependencies'][0]['uri'] = 'git+https://github.com/attacker/other@refs/heads/main'
            elif mutation == 'extra-source': p['buildDefinition']['resolvedDependencies'].append(dict(uri='unplanned'))
            elif mutation == 'builder': p['runDetails']['builder']['id'] = signing.identity('rehearsal', 'refs/heads/main')
            else: p['buildDefinition']['externalParameters']['workflow']['path' if mutation == 'workflow' else 'ref'] = 'other'
            self.bundle()
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, 'provenance source/workflow'): self.verify()

    def test_rehashed_index_and_partial_sets_do_not_reach_signer(self):
        (self.evidence / 'backend/index.json').write_text('{}')
        with self.assertRaises(ValueError): signing.manifest('images', 'refs/heads/main', plan=self.plan, evidence=self.evidence)
        shutil.rmtree(self.evidence / 'backend')
        with self.assertRaises(ValueError): signing.manifest('images', 'refs/heads/main', plan=self.plan, evidence=self.evidence)

    def test_production_ref_and_repository_policy(self):
        for scope, ref in (('release', 'refs/heads/main'), ('images', 'refs/pull/1/merge'), ('platform', 'refs/heads/dev')):
            with self.subTest(scope=scope), self.assertRaises(ValueError): signing.identity(scope, ref)
        with self.assertRaisesRegex(ValueError, 'repository differs'):
            signing.manifest('index', 'refs/heads/main', evidence=self.evidence / 'frontend', repository='ghcr.io/attacker/nyxid/frontend')
        self.assertNotEqual(signing.identity('rehearsal', 'refs/heads/main'), signing.identity('images', 'refs/heads/main'))

    def test_authorization_is_required_not_a_disabled_default(self):
        for value in ('', 'false', 'true', 'other'):
            with patch.dict(os.environ, OFFICIAL_RELEASE_AUTHORIZATION=value), self.assertRaises(ValueError): signing.authorize()
        with patch.dict(os.environ, OFFICIAL_RELEASE_AUTHORIZATION=signing.POLICY): signing.authorize()

    def test_index_and_promotion_cannot_write_without_valid_signing(self):
        environment = dict(GITHUB_REF='refs/tags/v0.73.0', OFFICIAL_RELEASE_AUTHORIZATION=signing.POLICY)
        self.verifier.write_text('#!/bin/sh\nexit 31\n')
        with patch.dict(os.environ, environment), patch.object(signing.subprocess, 'run', wraps=subprocess.run) as run:
            with self.assertRaises(subprocess.CalledProcessError):
                promoter.promote(self.plan, self.evidence, 'frontend', 'ghcr.io/chronoaiproject/nyxid/frontend:latest', Path('/missing/regctl'), self.proof, self.verifier)
            self.assertTrue(all(c.args[0][0] == str(self.verifier) for c in run.call_args_list))
        # An images manifest cannot authorize a single-index write, even when
        # the verifier executable would have returned success.
        with patch.dict(os.environ, environment), patch.object(index_publisher.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'manifest differs'):
                index_publisher.publish(self.evidence / 'frontend', 'ghcr.io/chronoaiproject/nyxid/frontend', Path('regctl'), self.proof, self.verifier)
            run.assert_not_called()

    def test_subject_checksums_match_actual_action_input(self):
        rows = (self.proof / 'subjects.sha256').read_text().splitlines()
        self.assertEqual([dict(name=r.split('  ', 1)[1], digest=dict(sha256=r.split('  ', 1)[0])) for r in rows], self.payload['subject'])

    def test_platform_cannot_write_with_missing_or_failed_signature_verification(self):
        stage = self.root / 'platform'; stage.mkdir()
        receipt = fixtures.platform_bundle(stage / 'sbom', 'frontend', 'amd64')
        write(stage / 'image-scan.json', {'fixture': 'passing scan is independently tested'})
        write(stage / 'trivy.json', {'fixture': 'exact retained report'})
        proof = self.root / 'platform-proof'
        environment = dict(GITHUB_REF='refs/heads/main', OFFICIAL_RELEASE_AUTHORIZATION=signing.POLICY)
        # Scan/SBOM validity has its own real-OCI tests. Here the signer and
        # publisher both see the same passing evidence and must still stop
        # before any registry call when signature verification fails.
        with patch.dict(os.environ, environment), patch('image_sbom.verify', return_value=receipt), \
                patch.object(publisher, 'verify_stage', return_value=receipt), \
                patch.object(publisher, 'verify_sbom', return_value=receipt):
            document = signing.manifest('platform', 'refs/heads/main', stage=stage,
                                        repository='ghcr.io/chronoaiproject/nyxid/frontend')
            signing.prepare(document, proof)
            self.verifier.write_text('#!/bin/sh\nexit 31\n')
            for mutation in ('missing-bundle', 'failed-verifier', 'missing-verifier'):
                if mutation != 'missing-bundle':
                    payload = load_json(proof / 'statement.json')
                    write(proof / signing.BUNDLE, dict(dsseEnvelope=dict(payload=base64.b64encode(json.dumps(payload).encode()).decode())))
                verifier = self.verifier if mutation != 'missing-verifier' else self.root / 'absent-verifier'
                with self.subTest(mutation=mutation), patch.object(signing.subprocess, 'run', wraps=subprocess.run) as run:
                    with self.assertRaises((OSError, ValueError, subprocess.CalledProcessError)):
                        publisher.publish(stage, 'ghcr.io/chronoaiproject/nyxid/frontend', Path('regctl'), proof, verifier)
                    self.assertTrue(all(c.args[0][0] == str(verifier) for c in run.call_args_list))

    def test_rehearsal_separation_requires_workflow_identity_rejection(self):
        document = dict(scope='rehearsal', ref='refs/pull/1944/merge',
                        identity=signing.identity('rehearsal', 'refs/pull/1944/merge'))
        write(self.proof / signing.MANIFEST, document)
        for status, stderr in ((0, ''), (1, 'certificate chain failure'), (1, 'missing TUF root')):
            result = subprocess.CompletedProcess([], status, stderr=stderr, stdout='')
            with self.subTest(status=status, stderr=stderr), patch.object(rejection.subprocess, 'run', return_value=result), \
                    self.assertRaisesRegex(ValueError, 'specifically by the unchanged W6 workflow identity'):
                rejection.check(self.proof, self.verifier)
        result = subprocess.CompletedProcess([], 1, stderr='Attestation workflow identity mismatch', stdout='')
        with patch.object(rejection.subprocess, 'run', return_value=result) as run:
            with redirect_stdout(io.StringIO()): rejection.check(self.proof, self.verifier)
            self.assertEqual(run.call_count, 5)


class Upload(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix='nyxid-signed-upload-'); self.addCleanup(temp.cleanup)
        self.root = Path(temp.name); self.artifacts = self.root / 'artifacts'; self.artifacts.mkdir()
        self.proof = self.root / 'proof'; self.proof.mkdir()
        (self.artifacts / 'archive.tar.gz').write_bytes(b'actual immutable archive bytes')
        self.document = dict(scope='release', subjects=[signing.record('archive.tar.gz', signing.file_digest(self.artifacts / 'archive.tar.gz'))])
        for name in (signing.MANIFEST, signing.BUNDLE):
            (self.proof / name).write_bytes(b'verified signing evidence')
            shutil.copyfile(self.proof / name, self.artifacts / name)

    def check(self): signing.check_upload(self.document, self.artifacts, self.proof)

    def test_exact_upload_set(self): self.check()

    def test_extra_missing_linked_or_directory_upload_fails(self):
        for kind in ('extra', 'missing', 'link', 'directory'):
            archive = self.artifacts / 'archive.tar.gz'; original = archive.read_bytes()
            if kind == 'extra': (self.artifacts / 'unsigned').write_bytes(b'unsigned')
            else:
                archive.unlink()
                if kind == 'link': archive.symlink_to(self.proof / signing.MANIFEST)
                elif kind == 'directory': archive.mkdir()
            with self.subTest(kind=kind), self.assertRaises(ValueError): self.check()
            if kind == 'extra': (self.artifacts / 'unsigned').unlink()
            elif archive.is_dir(): archive.rmdir()
            elif archive.is_symlink(): archive.unlink()
            archive.write_bytes(original)

    def test_changed_archive_or_signing_bundle_fails(self):
        for path in (self.artifacts / 'archive.tar.gz', self.artifacts / signing.BUNDLE):
            original = path.read_bytes(); path.write_bytes(original + b'changed')
            with self.subTest(path=path.name), self.assertRaises(ValueError): self.check()
            path.write_bytes(original)


if __name__ == '__main__': unittest.main()
