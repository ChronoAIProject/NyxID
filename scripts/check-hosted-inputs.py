#!/usr/bin/env python3
"""Admit hosted patch execution and CUA archive identity bindings."""
from pathlib import Path
import base64
import binascii
import json
import re
from urllib.parse import urlsplit

from apt_inputs import instructions

ROOT = Path(__file__).resolve().parents[1]


LOCKED_INSTALL = '/opt/nyxid-npm/npm ci --ignore-scripts --no-audit --no-fund --global=false'
NPM_CALL = re.compile(r'(?<![\w.-])(?:[A-Za-z0-9_./-]*/)?(?:npm|npx)\s+')


def check_npm(root, text):
    errors = []
    directory = root / 'security/hosted-tools'
    try:
        if any((directory / name).is_symlink() for name in ('package.json', 'package-lock.json', '.npmrc')):
            raise ValueError('symlinked npm input')
        package = json.loads((directory / 'package.json').read_text())
        lock = json.loads((directory / 'package-lock.json').read_text())
        if (directory / '.npmrc').read_text().strip() != 'ignore-scripts=true':
            errors.append('hosted dependency lifecycle execution must be disabled')
        if package.get('scripts') or package.get('allowScripts') != {}:
            errors.append('hosted root must have zero lifecycle hooks and zero script approvals')
        dependencies = package.get('dependencies', {})
        expected = {}
        for argument, name in (('CODEX_VERSION', '@openai/codex'), ('CODEX_ACP_VERSION', '@agentclientprotocol/codex-acp')):
            values = re.findall(r'^ARG ' + argument + r'=([0-9]+\.[0-9]+\.[0-9]+)$', text, re.M)
            if len(values) != 1:
                errors.append('hosted tool ARG must contain one exact version: ' + argument)
            else:
                expected[name] = values[0]
        if dependencies != expected:
            errors.append('hosted dependencies must exactly match reviewed tool versions')
        packages = lock.get('packages', {})
        if lock.get('lockfileVersion') != 3 or packages.get('', {}).get('dependencies') != dependencies:
            errors.append('hosted manifest and npm lock must agree')
        if not packages:
            errors.append('hosted npm lock is empty')
        for name, value in packages.items():
            if not name:
                continue
            parsed = urlsplit(value.get('resolved', ''))
            try:
                algorithm, digest = value.get('integrity', '').split('-', 1)
                valid_digest = algorithm == 'sha512' and len(base64.b64decode(digest, validate=True)) == 64
            except (ValueError, binascii.Error):
                valid_digest = False
            if (not re.fullmatch(r'node_modules/(?:@[a-z0-9._-]+/)?[a-z0-9._-]+', name) or
                    value.get('link') or not value.get('version') or not valid_digest or
                    parsed.scheme != 'https' or parsed.netloc != 'registry.npmjs.org' or
                    parsed.query or parsed.fragment or not parsed.path.endswith('.tgz')):
                errors.append('hosted npm package requires an exact registry archive with SHA-512 integrity: ' + name)
            if value.get('hasInstallScript'):
                errors.append('hosted npm dependencies must have zero install hooks: ' + name)
        for name, version in expected.items():
            if packages.get('node_modules/' + name, {}).get('version') != version:
                errors.append('hosted locked direct dependency differs from reviewed version: ' + name)
    except (OSError, ValueError, TypeError, AttributeError) as error:
        errors.append('invalid hosted npm inputs: ' + str(error))
    calls = []
    for raw in instructions(text):
        if not raw.startswith('RUN '):
            continue
        flat = raw.replace('\\\n', ' ')
        # These trusted entrypoints take 'npm' as a tool/package argument.
        flat = flat.replace('python3 /opt/nyxid-hosted-inputs/scripts/bootstrap-build-tool.py npm --bin-dir /opt/nyxid-npm', 'verified-npm-bootstrap')
        # Match only literal validated APT arguments, never shell substitutions.
        flat = re.sub(r'sh /opt/nyxid-apt/apt-snapshot-install\.sh /opt/nyxid-apt/apt-snapshots\.tsv\s+'
                      r'(?:(?:debian|ubuntu):[a-z]+\s+)+--\s+[a-z0-9+.,\s-]+(?=$|[;&|])',
                      'verified-apt-bootstrap', flat)
        for match in NPM_CALL.finditer(flat):
            calls.append(re.split(r'[;&|]', flat[match.start():], 1)[0].strip())
    if calls != [LOCKED_INSTALL]:
        errors.append('hosted npm must use only pinned npm ci --ignore-scripts; unlocked/global installs are forbidden')
    required = (
        'COPY security/hosted-tools/package.json security/hosted-tools/package-lock.json security/hosted-tools/.npmrc ./',
        'WORKDIR /usr/local/lib',
        'python3 /opt/nyxid-hosted-inputs/scripts/bootstrap-build-tool.py npm --bin-dir /opt/nyxid-npm',
        'COPY --from=hosted-tools /usr/local/lib/node_modules/ /usr/local/lib/node_modules/',
        'COPY --from=tools /usr/local/lib/node_modules/ /usr/local/lib/node_modules/',
    )
    if any(value not in text for value in required) or re.search(r'(?i)npm_config_[a-z_]+\s*=', text):
        errors.append('hosted build must consume the locked root and retain its complete runtime dependency tree')
    return errors


def check(root):
    errors = []
    text = (root / 'cli/Dockerfile.machine-hosted').read_text()
    errors.extend(check_npm(root, text))
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
    print('Hosted locked npm, hook denial, patch execution and exact CUA archive bindings passed.')
