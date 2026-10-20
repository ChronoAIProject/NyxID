#!/usr/bin/env python3
"""Check the two actual digest-bound publication images before tag promotion."""
import argparse
import importlib.util
from pathlib import Path
import re
import subprocess

from credential_packaging import parity

spec = importlib.util.spec_from_file_location('credential_image', Path(__file__).with_name('check-credential-image.py'))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


def reference(directory, repository, component):
    files = list(directory.iterdir())
    if len(files) != 1 or files[0].is_symlink() or not files[0].is_file() or \
            files[0].stat().st_size != 0 or not re.fullmatch('[0-9a-f]{64}', files[0].name):
        raise ValueError('missing/extra/invalid published credential digest')
    return repository + '/' + component + '@sha256:' + files[0].name


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--digests', type=Path, required=True)
    parser.add_argument('--repository', required=True)
    parser.add_argument('--arch', choices=('amd64', 'arm64'), required=True)
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--epoch', type=int, required=True)
    args = parser.parse_args()
    if not re.fullmatch('ghcr.io/[a-z0-9-]+/nyxid', args.repository):
        parser.error('expected official GHCR image namespace')
    inventories = []
    for component in ('backend', 'frontend'):
        image = reference(args.digests / component, args.repository, component)
        subprocess.run(['docker', 'pull', '--platform=linux/' + args.arch, image], check=True)
        inventories.append(checker.check(image, component, args.source_root, args.commit, args.epoch))
    parity(*inventories)
    print('Actual published backend/frontend credential bytes match before tag promotion.')


if __name__ == '__main__':
    main()
