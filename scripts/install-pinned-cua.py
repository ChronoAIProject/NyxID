#!/usr/bin/env python3
"""Install the committed CUA archive into a fresh CI directory without hooks."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
MAX_EXPANDED = 256 * 1024 * 1024


def asset_for_host(release, system, machine):
    arch = {'x86_64': 'x86_64', 'aarch64': 'arm64'}.get(machine)
    if system != 'Linux' or arch is None:
        raise ValueError('native CUA acceptance requires supported Linux architecture')
    tag = release['tag']
    if not re.fullmatch(r'cua-driver-rs-v[0-9]+\.[0-9]+\.[0-9]+', tag):
        raise ValueError('CUA requires an exact version tag')
    name = 'cua-driver-rs-' + tag.removeprefix('cua-driver-rs-v') + '-linux-' + arch + '.tar.gz'
    assets = [asset for asset in release['assets'] if asset['name'] == name]
    if len(assets) != 1:
        raise ValueError('missing or duplicate CUA architecture asset')
    asset = assets[0]
    if (asset['url'] != 'https://github.com/trycua/cua/releases/download/' + tag + '/' + name or
            not re.fullmatch('[0-9a-f]{64}', asset['sha256']) or
            type(asset['size']) is not int or not 0 < asset['size'] <= MAX_EXPANDED):
        raise ValueError('invalid committed CUA URL/digest/size')
    return asset


def unpack(archive, asset, destination):
    if archive.stat().st_size != asset['size'] or hashlib.sha256(archive.read_bytes()).hexdigest() != asset['sha256']:
        raise ValueError('CUA archive size/checksum mismatch')
    if destination.exists() or destination.is_symlink():
        raise ValueError('CUA requires a fresh destination')
    prefix = asset['name'].removesuffix('.tar.gz')
    # Validate the complete archive before writing any extracted content. Keep
    # the SDK and other runtime files, but never run the bundled install script.
    with tarfile.open(archive) as package:
        members = package.getmembers(); seen = set(); expanded = 0
        for member in members:
            parts = PurePosixPath(member.name).parts
            if (not parts or parts[0] != prefix or '..' in parts or '\\' in member.name or
                    member.name in seen or not (member.isfile() or member.isdir())):
                raise ValueError('unsafe CUA archive member')
            seen.add(member.name); expanded += member.size
            if expanded > MAX_EXPANDED:
                raise ValueError('CUA expanded archive exceeds bound')
        if not any(m.name == prefix + '/cua-driver' and m.isfile() and m.mode & 0o111 for m in members):
            raise ValueError('CUA archive lacks executable driver')
        destination.mkdir(mode=0o755, parents=True)
        for member in members:
            target = destination.joinpath(*PurePosixPath(member.name).parts[1:])
            if member.isdir():
                target.mkdir(mode=0o755, parents=True, exist_ok=True)
            else:
                target.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
                with package.extractfile(member) as source, target.open('xb') as output:
                    shutil.copyfileobj(source, output)
                target.chmod(0o755 if member.mode & 0o111 else 0o644)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--destination', type=Path, required=True)
    args = parser.parse_args()
    asset = asset_for_host(json.loads((ROOT / 'cli/resources/cua/release.json').read_text()), platform.system(), platform.machine())
    args.destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='nyxid-native-cua-', dir=args.destination.parent) as directory:
        archive = Path(directory) / 'driver.tar.gz'; size = 0
        with urllib.request.urlopen(asset['url'], timeout=120) as response, archive.open('wb') as output:
            if not response.url.startswith('https://'):
                raise ValueError('CUA redirects require HTTPS')
            while chunk := response.read(65536):
                size += len(chunk)
                if size > asset['size']:
                    raise ValueError('CUA download exceeds committed size')
                output.write(chunk)
        unpack(archive, asset, args.destination)
    print('Verified pinned CUA archive: ' + asset['name'] + ' sha256=' + asset['sha256'])


if __name__ == '__main__':
    main()
