#!/usr/bin/env python3
"""Promote tags only after replaying the whole planned platform/index SBOM set."""
import argparse
import os
from pathlib import Path
import re
import subprocess
from image_publication_sboms import closure
from sbom_evidence import load_json
import official_release


def promote(plan, evidence, component, tags, regctl, proof, verifier):
    receipt = closure(plan, evidence)
    official_release.authorize()
    document = official_release.manifest('images', os.environ['GITHUB_REF'], plan=plan, evidence=evidence)
    official_release.verify(document, proof, verifier)
    subject = next(s for s in plan['subjects'] if s['id'] == 'index:' + component)
    repository = subject['name']
    values = tags.splitlines()
    if not values or len(values) != len(set(values)) or any(not re.fullmatch(re.escape(repository) + r':[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}', t) for t in values):
        raise ValueError('only nonempty planned repository tags may be promoted')
    digest = receipt['subjects'][subject['id']]
    official_release.attach(regctl, repository + '@' + digest, proof)
    for tag in values:
        subprocess.run([str(regctl), 'image', 'copy', repository + '@' + digest, tag], check=True)
        if subprocess.check_output([str(regctl), 'manifest', 'digest', tag], text=True).strip() != digest:
            raise ValueError('promoted tag differs from validated SBOM index subject')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan', type=Path, required=True); p.add_argument('--evidence', type=Path, required=True)
    p.add_argument('--component', required=True); p.add_argument('--tags', required=True); p.add_argument('--regctl', type=Path, required=True)
    p.add_argument('--proof', type=Path, required=True); p.add_argument('--verifier', type=Path, required=True)
    a = p.parse_args(); promote(load_json(a.plan), a.evidence, a.component, a.tags, a.regctl, a.proof.resolve(), a.verifier.resolve())
