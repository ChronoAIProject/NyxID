#!/usr/bin/env python3
import argparse
from pathlib import Path
from publication_contract import load_json
from release_sboms import generate, verify

if __name__ == '__main__':
    p = argparse.ArgumentParser(description='Require final SBOM digest/composition closure for every planned release subject')
    p.add_argument('--plan', type=Path, required=True)
    p.add_argument('--artifacts', type=Path, required=True)
    p.add_argument('--evidence', type=Path, required=True)
    p.add_argument('--credential-sbom', type=Path)
    p.add_argument('--output', type=Path)
    a = p.parse_args()
    if bool(a.output) != bool(a.credential_sbom): p.error('generation requires --output and --credential-sbom together')
    if a.output: generate(load_json(a.plan), a.artifacts, a.evidence, a.credential_sbom, a.output)
    else: verify(load_json(a.plan), a.artifacts, a.evidence)
    print('Complete expected release SBOM subject/composition closure verified')
