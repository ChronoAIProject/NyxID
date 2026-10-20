#!/usr/bin/env python3
"""Validate platform/index composition; optionally publish the verified index by digest."""
import argparse
import os
from pathlib import Path
import subprocess
from image_publication_sboms import closure, generate_set, index, verify_index
from publication_contract import source_identity
from sbom_evidence import load_json, write


def publish(directory, repository, regctl):
    from publish_sbom import attach, repository_check
    repository_check(repository)
    receipt = verify_index(directory)
    destination = repository + '@' + receipt['subject']['digest']
    subprocess.run([str(regctl), 'manifest', 'put', destination, '--content-type', receipt['subject']['mediaType'], '--by-digest'],
                   input=(directory / 'index.json').read_bytes(), check=True)
    if subprocess.check_output([str(regctl), 'manifest', 'digest', destination], text=True).strip() != receipt['subject']['digest']:
        raise ValueError('published index digest differs from complete SBOM composition')
    attach(regctl, destination, directory / 'index.cdx.json')
    return receipt['subject']['digest']


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='mode', required=True)
    p = sub.add_parser('index'); p.add_argument('--component', required=True); p.add_argument('--commit', required=True)
    p.add_argument('--epoch', type=int, required=True); p.add_argument('--evidence', type=Path, required=True); p.add_argument('--output', type=Path, required=True)
    p = sub.add_parser('set'); p.add_argument('--plan', type=Path, required=True); p.add_argument('--evidence', type=Path, required=True); p.add_argument('--output', type=Path, required=True)
    p = sub.add_parser('closure'); p.add_argument('--plan', type=Path, required=True); p.add_argument('--evidence', type=Path, required=True); p.add_argument('--output', type=Path)
    p = sub.add_parser('publish-index'); p.add_argument('--evidence', type=Path, required=True); p.add_argument('--repository', required=True); p.add_argument('--regctl', type=Path, required=True)
    args = parser.parse_args()
    if args.mode == 'index':
        index(args.component, source_identity(args.commit, args.epoch), [p.parent for p in args.evidence.rglob('image-sbom.json')], args.output)
    elif args.mode == 'set':
        generate_set(load_json(args.plan), args.evidence, args.output)
        print('Complete planned image platform/index SBOM set generated without publication')
    elif args.mode == 'closure':
        result = closure(load_json(args.plan), args.evidence)
        if args.output: write(args.output, result)
        print('Complete planned image platform/index SBOM and digest closure verified')
    else:
        digest = publish(args.evidence, args.repository, args.regctl)
        with open(os.environ['GITHUB_OUTPUT'], 'a') as output: output.write('digest=' + digest + '\n')
