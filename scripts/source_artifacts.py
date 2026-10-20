"""Require actual final executable/asset identity against independent source inputs."""
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import subprocess

from publication_contract import load_json, require_build_identity, source_identity

spec = importlib.util.spec_from_file_location('identity_archives', Path(__file__).with_name('check-dist-artifacts.py'))
archives = importlib.util.module_from_spec(spec)
spec.loader.exec_module(archives)

# Includes public publication subjects and auxiliary application images. The
# private hosted target shares its public desktop producer, but is checked again
# after all private tool layers in its publication caller.
IMAGES = {
    'backend': ('backend/Dockerfile', '', 'nyxid', '/usr/local/bin/nyxid-server', 'gnu'),
    'frontend': ('frontend/Dockerfile', '', None, '/usr/share/nginx/html', None),
    'node-agent': ('cli/Dockerfile.node', '', 'nyxid-cli', '/usr/local/bin/nyxid', 'gnu'),
    'nyxid-node-machine': ('cli/Dockerfile.machine', '', 'nyxid-cli', '/usr/local/bin/nyxid', 'gnu'),
    'nyxid-machine-updater': ('cli/Dockerfile.machine-updater', 'production', 'nyxid-machine-updater', '/nyxid-machine-updater', 'musl'),
    'machine-node-only': ('cli/Dockerfile.machine-node-only', '', 'nyxid-cli', '/usr/local/bin/nyxid', 'gnu'),
    'machine-hosted-desktop': ('cli/Dockerfile.machine-hosted', 'desktop', 'nyxid-cli', '/usr/local/bin/nyxid', 'gnu'),
    'machine-hosted': ('cli/Dockerfile.machine-hosted', 'hosted', 'nyxid-cli', '/usr/local/bin/nyxid', 'gnu'),
    'mcp-demo': ('Dockerfile.glama', '', 'nyxid-mcp-demo', '/usr/local/bin/nyxid-mcp-demo', 'gnu'),
}


def versions(source):
    data = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--offline', '--no-deps',
                                             '--format-version=1'], cwd=source))
    return {package['name']: package['version'] for package in data['packages']}


def verify_labels(info, arch, commit, epoch):
    source_identity(commit, epoch)
    labels = info.get('Config', {}).get('Labels') or {}
    if info.get('Os') != 'linux' or info.get('Architecture') != arch or \
            labels.get('org.opencontainers.image.revision') != commit or \
            labels.get('io.nyxid.source-date-epoch') != str(epoch):
        raise ValueError('final image platform/revision/epoch differs from planned source')


def static_musl(binary, target):
    data = binary.read_bytes()
    machine = {'aarch64': 183, 'x86_64': 62}[target.split('-')[0]]
    if len(data) < 64 or data[:6] != b'\x7fELF\x02\x01' or struct.unpack_from('<H', data, 18)[0] != machine or \
            struct.unpack_from('<H', data, 16)[0] not in (2, 3):
        raise ValueError('static updater executable platform mismatch')
    offset = struct.unpack_from('<Q', data, 32)[0]
    size, count = struct.unpack_from('<HH', data, 54)
    if size != 56 or not 0 < count <= 128 or offset + size * count > len(data) or \
            any(struct.unpack_from('<I', data, offset + size * index)[0] == 3 for index in range(count)):
        raise ValueError('scratch updater requires a static executable without an interpreter')


def verify_binary(binary, extractor, package, version, target, commit, epoch):
    if target.endswith('-musl'):
        static_musl(binary, target)
    else:
        archives.check_binary_target(binary, target)
    receipt = json.loads(subprocess.check_output([str(extractor), '--identity-only', str(binary)]))
    require_build_identity(receipt, package, version, target, source_identity(commit, epoch))
    subject = receipt['subject']
    if subject.get('sha256') != hashlib.sha256(binary.read_bytes()).hexdigest() or \
            subject.get('size') != binary.stat().st_size or subject.get('name') != binary.name:
        raise ValueError('identity extractor subject differs from actual packaged executable')
    return receipt


def regular_inventory(root):
    files = {}
    for path in root.rglob('*'):
        if path.is_symlink() or not (path.is_dir() or path.is_file()):
            raise ValueError('nonregular frontend artifact')
        if path.is_file():
            files[path.relative_to(root).as_posix()] = path
    return files


def verify_frontend(root, source, commit, epoch):
    source_identity(commit, epoch)
    identity = load_json(root / 'build-identity.json')
    if identity != {'schema': 'nyxid.frontend-build-identity.v1', 'commit': commit,
                    'source_date_epoch': epoch, 'source_state': 'exported'}:
        raise ValueError('packaged frontend identity differs from exported source')
    version = load_json(root / 'build-version.json')
    if version.get('commit') != commit or version.get('buildId') != commit:
        raise ValueError('frontend update metadata differs from exported source')
    files = regular_inventory(root)
    assets = version.get('assets')
    if not isinstance(assets, list) or not assets or len(assets) != len(set(assets)) or \
            any(asset not in files or not asset.startswith('assets/') or not asset.endswith(('.js', '.css')) for asset in assets):
        raise ValueError('frontend initial assets are missing or invalid')
    seal = load_json(root / 'shipped-composition.json')
    locked = hashlib.sha256((source / 'frontend/package-lock.json').read_bytes()).hexdigest()
    if seal.get('schema') != 'nyxid.shipped-composition.v1' or seal.get('component') != 'spa' or \
            seal.get('source_lock_sha256') != locked:
        raise ValueError('frontend composition source lock mismatch')
    inventory = {item['path']: item for item in seal['artifacts']}
    if len(inventory) != len(seal['artifacts']) or set(inventory) != set(files) - {'shipped-composition.json'}:
        raise ValueError('frontend final composition file closure mismatch')
    for name, item in inventory.items():
        if item['size'] != files[name].stat().st_size or item['sha256'] != hashlib.sha256(files[name].read_bytes()).hexdigest():
            raise ValueError('frontend final composition bytes mismatch')
    return identity
