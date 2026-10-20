#!/usr/bin/env python3
"""Admit hosted patch execution and CUA archive identity bindings."""
from pathlib import Path
import json
import re

ROOT = Path(__file__).resolve().parents[1]


def check(root):
    errors = []
    text = (root / 'cli/Dockerfile.machine-hosted').read_text()
    required = ('COPY scripts/verify-hosted-context.py security/hosted-context.tsv /opt/nyxid-hosted-inputs/',
                'python3 /opt/nyxid-hosted-inputs/verify-hosted-context.py',
                '--context /tmp/cma-codex-source --pins /opt/nyxid-hosted-inputs/hosted-context.tsv --execute')
    if any(value not in text for value in required) or re.search(r'node\s+/tmp/cma-codex-source/', text):
        errors.append('hosted patches must all be authenticated before execution')
    release = json.loads((root / 'cli/resources/cua/release.json').read_text())
    tag = release.get('tag', '')
    if not re.fullmatch(r'cua-driver-rs-v[0-9]+\.[0-9]+\.[0-9]+', tag) or not re.fullmatch('[0-9a-f]{40}', release.get('source', '')):
        errors.append('CUA release requires an exact version tag and source commit')
    version = tag.removeprefix('cua-driver-rs-v')
    expected = {'checksums.txt', 'cua-driver-rs-' + version + '-darwin-universal.tar.gz',
                'cua-driver-rs-' + version + '-linux-arm64.tar.gz', 'cua-driver-rs-' + version + '-linux-x86_64.tar.gz'}
    assets = release.get('assets', [])
    if len(assets) != len(expected) or {a['name'] for a in assets} != expected:
        errors.append('CUA archive names must match the proposed release version')
    for asset in assets:
        if (asset.get('url') != 'https://github.com/trycua/cua/releases/download/' + tag + '/' + asset['name'] or
                not re.fullmatch('[0-9a-f]{64}', asset.get('sha256', '')) or
                type(asset.get('size')) is not int or asset['size'] <= 0):
            errors.append('CUA release archive URL/hash/size binding is invalid')
    return errors


if __name__ == '__main__':
    errors = check(ROOT)
    if errors:
        raise SystemExit('\n'.join(errors))
    print('Hosted patch execution and exact CUA archive bindings passed.')
