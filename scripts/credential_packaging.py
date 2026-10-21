"""Bind the actual packaged standalone page, manifest and composition to source."""
import base64
from datetime import datetime, timezone
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import struct

from publication_contract import load_json, source_identity

MANIFEST = 'release-integrity/releases.json'
SEAL = 'release-integrity/credential-composition.json'
GRAPH = 'credential-accept/composition-graph.json'
HTML = 'credential-accept/credential-accept.html'


def inventory(root):
    result = {}
    for directory in ('credential-accept', 'release-integrity'):
        base = root / directory
        if base.is_symlink() or not base.is_dir():
            raise ValueError('missing regular credential directory')
        for path in base.rglob('*'):
            if path.is_symlink() or not (path.is_file() or path.is_dir()):
                raise ValueError('nonregular credential asset')
            if path.is_file():
                if path.stat().st_size > 16 * 1024 * 1024:
                    raise ValueError('credential asset exceeds bound')
                result[path.relative_to(root).as_posix()] = path.read_bytes()
    return result


class Scripts(HTMLParser):
    def __init__(self):
        super().__init__()
        self.scripts, self.active = [], False

    def handle_starttag(self, tag, attrs):
        if tag == 'script':
            if self.active or len(dict(attrs)) != len(attrs):
                raise ValueError('duplicate HTML script attributes')
            self.scripts.append(dict(attrs))
            self.active = True

    def handle_endtag(self, tag):
        if tag == 'script':
            self.active = False

    def handle_data(self, data):
        if self.active and data.strip():
            raise ValueError('inline credential script')


def verify(root, source, commit, epoch):
    source_identity(commit, epoch)
    files = inventory(root)
    manifest = load_json(root / MANIFEST)
    version = load_json(source / 'frontend/package.json')['version']
    timestamp = datetime.fromtimestamp(epoch, timezone.utc).isoformat(timespec='milliseconds').replace('+00:00', 'Z')
    if set(manifest) != {'schema_version', 'app_version', 'git_commit', 'generated_at', 'credential_accept', 'artifacts'} or \
            manifest['schema_version'] != 'nyxid.release-integrity.v1' or manifest['app_version'] != version or \
            manifest['git_commit'] != commit or manifest['generated_at'] != timestamp:
        raise ValueError('credential manifest differs from source/version/epoch')
    expected, scripts = {MANIFEST, SEAL, GRAPH}, {}
    for artifact in manifest['artifacts']:
        if set(artifact) != {'role', 'path', 'content_type', 'size_bytes', 'sha384_sri', 'sha384_hex'}:
            raise ValueError('invalid credential artifact fields')
        name = artifact['path']
        role = artifact['role']
        if (role == 'credential_accept_html' and name == '/' + HTML):
            content_type = 'text/html; charset=utf-8'
        elif role == 'credential_accept_script' and re.fullmatch('/credential-accept/assets/credential-accept-[A-Za-z0-9_-]+\\.js', name):
            content_type = 'text/javascript; charset=utf-8'
        else:
            raise ValueError('invalid credential artifact role/path')
        relative = name[1:]
        if relative in expected or relative not in files:
            raise ValueError('missing or duplicate credential artifact')
        expected.add(relative)
        data = files[relative]
        digest = hashlib.sha384(data).digest()
        if type(artifact['size_bytes']) is not int or artifact['size_bytes'] != len(data) or \
                artifact['content_type'] != content_type or artifact['sha384_hex'] != digest.hex() or \
                artifact['sha384_sri'] != 'sha384-' + base64.b64encode(digest).decode():
            raise ValueError('credential artifact bytes differ from manifest')
        if role == 'credential_accept_script':
            scripts[name] = artifact
    if not scripts or HTML not in expected or set(files) != expected:
        raise ValueError('missing or extra packaged credential files')
    fingerprint = hashlib.sha384(b'nyxid:rci-accept:v1\0')
    for name in sorted(scripts, key=lambda value: value.encode('utf8')):
        encoded, data = name.encode('utf8'), files[name[1:]]
        fingerprint.update(struct.pack('>I', len(encoded)) + encoded + struct.pack('>Q', len(data)) + data)
    if manifest['credential_accept'] != {'fingerprint_sha384_hex': fingerprint.hexdigest()}:
        raise ValueError('credential fingerprint differs from shipped scripts')
    html = Scripts()
    html.feed(files[HTML].decode('utf8'))
    if html.active or len(html.scripts) != len(scripts) or {item.get('src') for item in html.scripts} != set(scripts):
        raise ValueError('credential HTML script set differs from manifest')
    for attrs in html.scripts:
        if attrs.get('integrity') != scripts[attrs['src']]['sha384_sri'] or attrs.get('crossorigin') != 'anonymous' or \
                attrs.get('data-nyx-integrity-role') != 'credential_accept_script' or attrs.get('type') != 'module':
            raise ValueError('credential HTML script SRI/metadata mismatch')
    seal = load_json(root / SEAL)
    graph = load_json(root / GRAPH)
    lock_hash = hashlib.sha256((source / 'frontend/package-lock.json').read_bytes()).hexdigest()
    if seal.get('schema') != 'nyxid.shipped-composition.v1' or seal.get('component') != 'credential-accept' or \
            seal.get('source_lock_sha256') != lock_hash or graph.get('schema') != 'nyxid.rendered-composition.v1' or \
            graph.get('component') != 'credential-accept' or graph.get('lock_sha256') != lock_hash:
        raise ValueError('credential composition differs from locked source')
    sealed = {item['path']: item for item in seal['artifacts']}
    if len(sealed) != len(seal['artifacts']) or set(sealed) != expected - {SEAL}:
        raise ValueError('credential composition inventory mismatch')
    for name, item in sealed.items():
        if item['size'] != len(files[name]) or item['sha256'] != hashlib.sha256(files[name]).hexdigest():
            raise ValueError('credential composition bytes mismatch')
    if len(seal['graphs']) != 1 or seal['graphs'][0].get('path') != GRAPH or \
            seal['graphs'][0].get('sha256') != hashlib.sha256(files[GRAPH]).hexdigest():
        raise ValueError('credential rendered graph mismatch')
    return {name: hashlib.sha256(data).hexdigest() for name, data in sorted(files.items())}


def parity(first, second):
    if first != second:
        raise ValueError('backend/frontend image credential bytes differ')
