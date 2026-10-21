#!/usr/bin/env python3
"""Require a real rehearsal bundle to fail W6's exact production identities."""
import argparse
import subprocess
from official_release import BUNDLE, REPOSITORY, identity
from sbom_evidence import file_digest, load_json, write


def check(proof, verifier):
    document = load_json(proof / 'official-release-manifest.json')
    if document['scope'] != 'rehearsal' or document['identity'] != identity('rehearsal', document['ref']):
        raise ValueError('distinct rehearsal scope/workflow required')
    digest = file_digest(proof / 'official-release-manifest.json')[7:]
    identities = [identity('release', 'refs/tags/v0.73.0'),
                  identity('images', 'refs/tags/v0.73.0'), identity('images', 'refs/heads/main')]
    # Also test both production workflows at the very same ref; rejection must
    # follow from the workflow identity, independently of branch/tag mismatch.
    identities += [REPOSITORY + '/.github/workflows/' + w + '@' + document['ref']
                   for w in ('release.yml', 'publish-images.yml')]
    for number, signer in enumerate(sorted(set(identities))):
        request = proof / ('production-rejection-' + str(number) + '.json')
        write(request, dict(bundle=str((proof / BUNDLE).resolve()), identity=signer, digests=[digest]))
        result = subprocess.run([str(verifier), str(request.resolve())], text=True, capture_output=True)
        if result.returncode == 0 or 'Attestation workflow identity mismatch' not in result.stderr:
            raise ValueError('rehearsal must be rejected specifically by the unchanged W6 workflow identity check: ' + signer)
    print(f'W6 rejected authentic rehearsal evidence under {len(set(identities))} exact production identities')


if __name__ == '__main__':
    from pathlib import Path
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--proof', required=True, type=Path); p.add_argument('--verifier', required=True, type=Path)
    a = p.parse_args(); check(a.proof.resolve(), a.verifier.resolve())
