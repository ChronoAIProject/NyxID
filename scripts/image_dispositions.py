"""Closed, exact image-finding eligibility. No generic ignore/VEX escape hatch.

Only the reviewed kernel-UAPI-header case is implemented. Every other blocked
finding remains fatal. An empty policy permits no dispositions.
"""
from datetime import date, timedelta
import hashlib
import io
import json
import lzma
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile

from build_catalogs import fetch, fetch_https
from image_artifacts import copy, image_container

ROOT = Path(__file__).resolve().parents[1]
KIND = 'linux-uapi-package-has-no-code-v1'
MACHINES = {'nyxid-node-machine', 'machine-node-only', 'machine-hosted-desktop', 'machine-hosted'}
FIELDS = {'cve', 'source_package', 'binary_package', 'version', 'component', 'architecture',
          'issued_on', 'expires_on', 'owner', 'rationale', 'affected_api', 'negative_test',
          'content_sha256', 'catalog_sha256', 'finding_sha256', 'source_version'}
HEX = re.compile('[0-9a-f]{64}')
HEADER_DOCUMENTS = frozenset({'/usr/share/doc/linux-libc-dev/changelog.Debian.gz',
                              '/usr/share/doc/linux-libc-dev/copyright'})
HEADER_DIRECTORIES = frozenset({'/usr', '/usr/include', '/usr/share', '/usr/share/doc',
                                '/usr/share/doc/linux-libc-dev'})


def canonical(value): return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()
def digest(value): return hashlib.sha256(value).hexdigest()


def load(root=ROOT, today=None):
    today = today or date.today()
    policy = json.loads((root / 'security/image-dispositions.json').read_text())
    if set(policy) != {'schema', 'declared_runtime', 'dispositions'} or policy['schema'] != 'nyxid.image-dispositions.v1':
        raise ValueError('invalid image disposition policy')
    seen = set()
    for item in policy['dispositions']:
        if set(item) != FIELDS:
            raise ValueError('missing/extra exact image disposition fields')
        issued, expires = date.fromisoformat(item['issued_on']), date.fromisoformat(item['expires_on'])
        if not issued <= today < expires <= issued + timedelta(days=30):
            raise ValueError('expired, future or over-30-day image disposition')
        if item['negative_test'] != KIND or item['component'] not in MACHINES or \
                item['architecture'] not in {'amd64', 'arm64'} or item['source_package'] != 'linux' or \
                item['binary_package'] != 'linux-libc-dev' or \
                not {'build-essential', 'linux-libc-dev'} <= set(policy['declared_runtime'].get(item['component'], [])):
            raise ValueError('finding outside declared minimal workstation runtime/closed negative test')
        if not re.fullmatch(r'CVE-\d{4}-\d{4,}', item['cve']) or \
                any(not isinstance(item[k], str) or not item[k].strip() for k in ('owner', 'version', 'source_version', 'rationale', 'affected_api')) or \
                any(not HEX.fullmatch(item[k]) for k in ('content_sha256', 'catalog_sha256', 'finding_sha256')):
            raise ValueError('incomplete exact image disposition identity/rationale')
        key = tuple(item[k] for k in ('cve', 'source_package', 'binary_package', 'version', 'component', 'architecture'))
        if key in seen: raise ValueError('duplicate image finding disposition')
        seen.add(key)
        if item['catalog_sha256'] != digest((root / 'security/apt-snapshots.tsv').read_bytes()):
            raise ValueError('snapshot catalog changed; recheck security-archive fixed versions before review')
    return policy


def header_member(name, mode, content):
    if name != PurePosixPath(name).as_posix() or '..' in PurePosixPath(name).parts or \
            not (name.startswith('/usr/include/') or name in HEADER_DOCUMENTS) or \
            mode & 0o111 or content.startswith(b'\x7fELF'):
        raise ValueError('linux-libc-dev negative proof found non-header path, executable or ELF: ' + name)
    return dict(path=name, mode=mode & 0o777, sha256=digest(content))


def header_directory(name):
    if name != PurePosixPath(name).as_posix() or '..' in PurePosixPath(name).parts or \
            not (name in HEADER_DIRECTORIES or name.startswith('/usr/include/')):
        raise ValueError('linux-libc-dev negative proof found non-header directory: ' + name)


def header_content(image_id, architecture, output):
    output.mkdir(parents=True, exist_ok=True)
    with image_container(image_id) as (container, _):
        destination = output / 'dpkg-list'
        copy(container, '/var/lib/dpkg/info/linux-libc-dev:' + architecture + '.list', destination)
        listing = (destination / ('linux-libc-dev:' + architecture + '.list')).read_text().splitlines()
        if len(listing) != len(set(listing)) or any(not p.startswith('/') or '..' in Path(p).parts for p in listing):
            raise ValueError('invalid final package file list')
        # Export the stopped container, inspect with the host parser, never run
        # a program supplied by the image to assert its own absence of code.
        expected = set(listing) - {'/.', '/'}
        files, found = [], set()
        with subprocess.Popen(['docker', 'export', container], stdout=subprocess.PIPE) as child:
            try:
                with tarfile.open(fileobj=child.stdout, mode='r|') as archive:
                    for member in archive:
                        name = '/' + member.name.removeprefix('./').lstrip('/').rstrip('/')
                        if name not in expected: continue
                        if name in found: raise ValueError('duplicate final package path')
                        found.add(name)
                        if member.isdir():
                            header_directory(name)
                            continue
                        if not member.isfile() or member.size > 8 * 1024 * 1024:
                            raise ValueError('linked, special or oversized package file')
                        content = archive.extractfile(member).read(member.size + 1)
                        files.append(header_member(name, member.mode, content))
                if child.wait() != 0: raise ValueError('final image filesystem export failed')
            except BaseException:
                child.terminate(); child.wait(); raise
            finally:
                child.stdout.close()
        if found != expected or not files:
            raise ValueError('missing final package path/content in negative proof')
    return digest(canonical(dict(file_list=sorted(listing), files=sorted(files, key=lambda f: f['path']))))


def security_candidate(root, architecture, directory, download):
    row = next(line.split('\t') for line in (root / 'security/apt-snapshots.tsv').read_text().splitlines()
               if line.startswith('ubuntu\tnoble\tnoble-security\t'))
    _, _, suite, _, url, release_hash = row
    directory.mkdir(parents=True, exist_ok=True)
    release_path, index_path = directory / 'InRelease', directory / 'Packages.xz'
    if download: release_path.write_bytes(fetch(url + '/dists/' + suite + '/InRelease'))
    release = release_path.read_bytes()
    if digest(release) != release_hash: raise ValueError('security archive differs from SHA-pinned signed InRelease')
    match = re.search(rb'^ ([0-9a-f]{64})\s+([0-9]+) (main/binary-' + architecture.encode() + rb'/Packages.xz)$', release, re.M)
    if not match: raise ValueError('missing native security package index binding')
    checksum, length, path = match.groups()
    if download: index_path.write_bytes(fetch_https(url + '/dists/' + suite + '/' + path.decode(), {'snapshot.ubuntu.com'}, 64*1024*1024))
    index = index_path.read_bytes()
    if digest(index) != checksum.decode() or len(index) != int(length):
        raise ValueError('security package index checksum/size mismatch')
    matches = []
    with io.TextIOWrapper(lzma.LZMAFile(io.BytesIO(index))) as stream:
        fields = {}
        for line in stream:
            if line.strip():
                if not line.startswith(' ') and ': ' in line:
                    key, value = line.rstrip('\n').split(': ', 1); fields[key] = value
            else:
                if fields.get('Package') == 'linux-libc-dev': matches.append(fields)
                fields = {}
        if fields.get('Package') == 'linux-libc-dev': matches.append(fields)
    if len(matches) != 1 or matches[0].get('Architecture') != architecture or \
            matches[0].get('Source', '').split(' ', 1)[0] != 'linux':
        raise ValueError('missing/ambiguous security archive candidate/source identity')
    source = matches[0]['Source']
    source_version = source.split('(', 1)[1].rstrip(')') if '(' in source else matches[0]['Version']
    return dict(version=matches[0]['Version'], source_version=source_version)


def finding_digest(vulnerability):
    # Omit only placement/cache identities. Keep every advisory/content field,
    # including descriptions, references and newly introduced semantic fields.
    return digest(canonical({k: v for k, v in vulnerability.items() if k not in
        {'Layer', 'PkgID', 'PkgIdentifier', 'Fingerprint'}}))


def apply(report, findings, image_id, component, platform, evidence, *, root=ROOT, download=False):
    policy = load(root)
    records = [item for item in policy['dispositions'] if item['component'] == component and
               platform == 'linux/' + item['architecture']]
    if not records: return findings
    if evidence is None: raise ValueError('missing executable final-image eligibility evidence')
    arch = platform.split('/')[1]
    candidate = security_candidate(root, arch, evidence / 'security-index', download)
    # Even publication replay re-inspects the immutable local image bytes.
    # A fabricated evidence JSON cannot replace the executable negative proof.
    import tempfile
    with tempfile.TemporaryDirectory(prefix='nyxid-header-proof-') as temporary:
        content = header_content(image_id, arch, Path(temporary))
    if any(item['version'] != candidate['version'] or item['source_version'] != candidate['source_version'] or
           item['content_sha256'] != content for item in records):
        raise ValueError('security archive candidate or final package content changed')
    reviewed = set()
    for result in report['Results']:
        packages = {p['Name']: p for p in result.get('Packages') or []}
        for vulnerability in result.get('Vulnerabilities') or []:
            for item in records:
                if (vulnerability['VulnerabilityID'], vulnerability['PkgName'], vulnerability['InstalledVersion']) != \
                        (item['cve'], item['binary_package'], item['version']): continue
                pkg = packages.get(item['binary_package'], {})
                compiler, development = packages.get('build-essential', {}), packages.get('libc6-dev', {})
                if not development.get('ID') or development['ID'] not in compiler.get('DependsOn', []) or \
                        not pkg.get('ID') or pkg['ID'] not in development.get('DependsOn', []):
                    raise ValueError('header package lacks its declared workstation dependency chain')
                source_version = pkg.get('SrcVersion', '') + ('-' + pkg['SrcRelease'] if pkg.get('SrcRelease') else '')
                if result.get('Class') != 'os-pkgs' or pkg.get('Arch') != arch or \
                        pkg.get('SrcName') != item['source_package'] or source_version != item['source_version'] or \
                        vulnerability.get('FixedVersion') or finding_digest(vulnerability) != item['finding_sha256']:
                    raise ValueError('source, available fix or exact reviewed finding content changed: ' +
                        json.dumps(dict(component=component, architecture=arch, cve=item['cve'],
                            package=item['binary_package'], version=item['version'],
                            fixed_version=vulnerability.get('FixedVersion'),
                            expected_finding_sha256=item['finding_sha256'],
                            actual_finding_sha256=finding_digest(vulnerability)), sort_keys=True))
                # The only supported case proves zero executable code in the
                # package, so no linked/reachable/network-input CRITICAL code
                # can qualify. No other package/kind can take this branch.
                reviewed.add((item['cve'], item['binary_package'], item['version']))
    if reviewed != {(item['cve'], item['binary_package'], item['version']) for item in records}:
        raise ValueError('stale/unmatched image disposition; remove resolved records instead of renewing')
    return [f for f in findings if (f['id'], f['package'], f['version']) not in reviewed]
