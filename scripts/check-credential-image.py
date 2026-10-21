#!/usr/bin/env python3
"""Verify image-extracted credentials; optionally require backend/frontend parity."""
import argparse
from pathlib import Path
import tempfile

from credential_packaging import parity, verify
from image_artifacts import copy, image_container


def check(image, component, source, commit, epoch):
    path = {'backend': '/app/frontend/dist', 'frontend': '/usr/share/nginx/html'}[component]
    with tempfile.TemporaryDirectory(prefix='nyxid-credential-image-') as temporary, image_container(image) as (container, info):
        root = Path(temporary)
        for directory in ('credential-accept', 'release-integrity'):
            copy(container, path + '/' + directory + '/.', root / directory)
        files = verify(root, source, commit, epoch)
        print(f'Credential packaging verified: {component} image {info["Id"]}, {len(files)} files, SOURCE_COMMIT={commit}')
        return files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True)
    parser.add_argument('--component', choices=('backend', 'frontend'), required=True)
    parser.add_argument('--compare-image')
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--epoch', type=int, required=True)
    args = parser.parse_args()
    files = check(args.image, args.component, args.source_root, args.commit, args.epoch)
    if args.compare_image:
        other = 'frontend' if args.component == 'backend' else 'backend'
        parity(files, check(args.compare_image, other, args.source_root, args.commit, args.epoch))
        print('Backend/frontend credential manifest, fingerprint and complete file parity verified.')


if __name__ == '__main__':
    main()
