#!/usr/bin/env python3
"""Validate the isolated, hook-free producer lock before its CI installation."""
import base64
import json
from pathlib import Path
import re
import sys
from urllib.parse import urlsplit

DIRECT = {'@sigstore/bundle': '3.0.0', '@sigstore/sign': '3.0.0'}
OVERRIDES = {'@sigstore/core': '3.2.1', '@sigstore/protobuf-specs': '0.3.2'}


def check(directory):
    for name in ('package.json', 'package-lock.json'):
        if (directory / name).is_symlink(): raise ValueError('linked signing dependency input')
    package = json.loads((directory / 'package.json').read_text())
    lock = json.loads((directory / 'package-lock.json').read_text())
    if package != dict(name='@nyxid/release-signing-tools', version='0.0.0', private=True,
                       dependencies=DIRECT, overrides=OVERRIDES):
        raise ValueError('signing manifest must match the reviewed hook-free producer and security overrides')
    packages = lock['packages']
    if lock['lockfileVersion'] != 3 or packages[''].get('dependencies') != DIRECT:
        raise ValueError('signing manifest/lock disagreement')
    for name, value in packages.items():
        if not name: continue
        parsed = urlsplit(value.get('resolved', ''))
        integrity = value.get('integrity', '').split('-', 1)
        if (not re.fullmatch(r'node_modules/(?:@[a-z0-9._-]+/)?[a-z0-9._-]+(?:/node_modules/(?:@[a-z0-9._-]+/)?[a-z0-9._-]+)*', name) or
                any(part in ('.', '..') for part in name.split('/')) or
                value.get('link') or value.get('hasInstallScript') or not value.get('version') or
                parsed.scheme != 'https' or parsed.netloc != 'registry.npmjs.org' or
                parsed.query or parsed.fragment or not parsed.path.endswith('.tgz') or
                len(integrity) != 2 or integrity[0] != 'sha512' or
                len(base64.b64decode(integrity[1], validate=True)) != 64):
            raise ValueError('signing dependency requires a hook-free exact registry archive and SHA-512: ' + name)
        for dependency, version in dict(DIRECT, **OVERRIDES).items():
            if name.endswith('/' + dependency) and value['version'] != version:
                raise ValueError('nested unreviewed signing dependency: ' + name)
    for name, version in dict(DIRECT, **OVERRIDES).items():
        if packages.get('node_modules/' + name, {}).get('version') != version:
            raise ValueError('reviewed signing dependency version mismatch: ' + name)
    return len(packages) - 1


if __name__ == '__main__':
    directory = Path(sys.argv[1]) if len(sys.argv) == 2 else Path(__file__).resolve().parents[1] / 'security/signing-tools'
    print(f'Signing producer lock: {check(directory)} SHA-512 packages; zero lifecycle hooks.')
