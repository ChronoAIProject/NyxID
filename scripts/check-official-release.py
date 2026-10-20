#!/usr/bin/env python3
"""Prepare or cryptographically reverify an exact official publication manifest."""
import argparse
from pathlib import Path
import os
from official_release import authorize, check_upload, manifest, prepare, verify
from sbom_evidence import load_json


def arguments():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('mode', choices=['prepare', 'verify'])
    p.add_argument('--scope', required=True, choices=['release', 'platform', 'index', 'images', 'rehearsal'])
    p.add_argument('--proof', type=Path, required=True)
    p.add_argument('--plan', type=Path); p.add_argument('--artifacts', type=Path)
    p.add_argument('--evidence', type=Path); p.add_argument('--stage', type=Path)
    p.add_argument('--repository'); p.add_argument('--verifier', type=Path)
    p.add_argument('--archive-plan', type=Path); p.add_argument('--archives', type=Path)
    p.add_argument('--archive-evidence', type=Path)
    p.add_argument('--final-upload', action='store_true')
    return p.parse_args()


if __name__ == '__main__':
    a = arguments()
    if a.scope != 'rehearsal': authorize()
    options = {k: getattr(a, k) for k in ('artifacts', 'evidence', 'stage', 'repository', 'archives', 'archive_evidence')}
    options['plan'] = load_json(a.plan) if a.plan else None
    options['archive_plan'] = load_json(a.archive_plan) if a.archive_plan else None
    document = manifest(a.scope, os.environ['GITHUB_REF'], **options)
    if a.mode == 'prepare': prepare(document, a.proof)
    else:
        if not a.verifier: raise SystemExit('a mandatory W6 verifier executable is required')
        digest = verify(document, a.proof, a.verifier.resolve())
        if a.final_upload:
            check_upload(document, a.artifacts, a.proof)
        print('W6 verified signed official publication manifest: ' + digest)
