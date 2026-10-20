#!/usr/bin/env python3
"""Check final image labels and extracted executable/assets plus safe executable/entrypoint startup."""
import argparse
from pathlib import Path
import tempfile

from credential_packaging import verify as verify_credentials
from image_artifacts import copy, image_container
from image_runtime import check as check_runtime
from source_artifacts import IMAGES, verify_binary, verify_frontend, verify_labels, versions


def check(image, component, arch, source, commit, epoch, extractor):
    _, _, package, path, abi = IMAGES[component]
    with tempfile.TemporaryDirectory(prefix='nyxid-image-identity-') as temporary, image_container(image) as (container, info):
        verify_labels(info, arch, commit, epoch)
        directory = Path(temporary) / 'extracted'
        if package is None:
            copy(container, path + '/.', directory)
            verify_frontend(directory, source, commit, epoch)
            verify_credentials(directory, source, commit, epoch)
            check_runtime(info["Id"], component)
        else:
            extracted = copy(container, path, directory)
            name = Path(path).name
            if extracted != [name]:
                raise ValueError('unexpected executable extraction closure')
            target = {'amd64': 'x86_64', 'arm64': 'aarch64'}[arch] + '-unknown-linux-' + abi
            verify_binary(directory / name, extractor, package, versions(source)[package], target, commit, epoch)
            if component == 'backend':
                credential_root = Path(temporary) / 'credentials'
                for name in ('credential-accept', 'release-integrity'):
                    copy(container, '/app/frontend/dist/' + name + '/.', credential_root / name)
                verify_credentials(credential_root, source, commit, epoch)
            check_runtime(info['Id'], component, directory / Path(path).name)
        print(f'Final source identity verified: {component} {arch}, image {info["Id"]}, SOURCE_COMMIT={commit}, epoch={epoch}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True)
    parser.add_argument('--component', choices=IMAGES, required=True)
    parser.add_argument('--arch', choices=('amd64', 'arm64'), required=True)
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--epoch', type=int, required=True)
    parser.add_argument('--extractor', type=Path, required=True)
    args = parser.parse_args()
    check(args.image, args.component, args.arch, args.source_root, args.commit, args.epoch, args.extractor.resolve())


if __name__ == '__main__':
    main()
