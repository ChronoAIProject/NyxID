#!/usr/bin/env python3
"""Maintainer-only prepare/publish entrypoint. Publication is never called by PR CI."""
import argparse
from pathlib import Path
import re
import subprocess
import tempfile

from apk_archive_release import ROOT, prepare, sha, validate_payload
from apk_inputs import ARCHES
from resolve_apk_requests import REQUESTS

REPOSITORY = 'ChronoAIProject/NyxID'


def publish(commit, payload):
    if not re.fullmatch('[0-9a-f]{40}', commit):
        raise ValueError('catalog_commit must be a full reviewed commit SHA')
    # Read candidate data with git; execute only the trusted workflow's scripts.
    with tempfile.TemporaryDirectory(prefix='nyxid-apk-catalogs-') as temporary:
        catalogs = Path(temporary)
        for label in REQUESTS:
            for arch in ARCHES:
                name = label + '-' + arch + '.tsv'
                data = subprocess.check_output(['git', 'show', commit + ':security/apk/' + name], cwd=ROOT)
                (catalogs / name).write_bytes(data)
        tag, files = validate_payload(catalogs, payload, ARCHES)
        # A release with the same date may already serve other unchanged catalogs.
        # Never replace assets: matching bytes are idempotent, mismatches stop.
        result = subprocess.run(['gh', 'release', 'view', tag, '--repo', REPOSITORY], capture_output=True)
        if result.returncode:
            subprocess.run(['gh', 'release', 'create', tag, '--repo', REPOSITORY, '--target', commit,
                            '--prerelease', '--latest=false', '--title', tag,
                            '--notes', 'Verified distro build inputs from reviewed catalog commit ' + commit + '. SHA-256 and Alpine signatures are required by every consuming build.'], check=True)
        for name in files:
            with tempfile.TemporaryDirectory(prefix='nyxid-apk-existing-') as download:
                existing = subprocess.run(['gh', 'release', 'download', tag, '--repo', REPOSITORY,
                                           '--pattern', name, '--dir', download], capture_output=True)
                if existing.returncode == 0:
                    path = Path(download) / name
                    if not path.is_file() or path.is_symlink() or sha(path) != sha(payload / name):
                        raise ValueError('existing durable asset differs; refusing replacement: ' + name)
                    print('Verified existing durable asset: ' + name, flush=True)
                    continue
            subprocess.run(['gh', 'release', 'upload', tag, str(payload / name), '--repo', REPOSITORY], check=True)
        print(f'Durable APK archive release verified: {tag}, {len(files)} files', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    prep = commands.add_parser('prepare')
    prep.add_argument('--root', type=Path, required=True)
    prep.add_argument('--arch', choices=ARCHES, required=True)
    prep.add_argument('--output', type=Path, required=True)
    upload = commands.add_parser('publish')
    upload.add_argument('--catalog-commit', required=True)
    upload.add_argument('--payload', type=Path, required=True)
    args = parser.parse_args()
    if args.command == 'prepare':
        prepare(args.root.resolve(), args.arch, args.output.resolve())
    else:
        publish(args.catalog_commit, args.payload.resolve())
