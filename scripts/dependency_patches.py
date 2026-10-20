"""Exact, expiring dispositions after a clean install and executable patch proof.

A disposition never alters the raw registry audit. Every installed source file,
archive, patch, lock, snapshot and regression must match its reviewed binding.
"""
import base64
from datetime import date, timedelta
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import tempfile
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

POLICY = 'security/dependency-patches.json'
REGRESSION = 'mobile/scripts/test-security-patches.cjs'
COMPATIBILITY = 'scripts/test-mobile-yaml-compat.cjs'


def sha(data):
    return hashlib.sha256(data).hexdigest()


def normalized(value):
    if not isinstance(value, str) or not value or '\\' in value or \
            PurePosixPath(value).is_absolute() or str(PurePosixPath(value)) != value or \
            any(part in ('.', '..') for part in value.split('/')):
        raise ValueError('non-normalized disposition path')
    return value


def bound_file(root, name, digest):
    path = root / normalized(name)
    if any(parent.is_symlink() for parent in (path, *(root / p for p in path.relative_to(root).parents))) or \
            not path.is_file() or root.resolve() not in path.resolve().parents or \
            sha(path.read_bytes()) != digest:
        raise ValueError('missing/changed/moved bound file: ' + name)
    return path


def load_policy(root, today=None):
    path = root / POLICY
    if not path.exists():
        return []  # Absence permits no disposition; all registry findings block.
    data = json.loads(path.read_text())
    if set(data) != {'schema_version', 'patches'} or data['schema_version'] != 1 or not isinstance(data['patches'], list):
        raise ValueError('invalid dependency patch policy')
    seen = set()
    for record in data['patches']:
        required = {'package', 'version', 'advisories', 'lock', 'lock_sha256', 'patch', 'patch_sha256',
                    'integrity', 'source_files', 'regressions', 'regression_sha256', 'compatibility_sha256',
                    'owner', 'expires', 'rationale'}
        if set(record) != required or not all(isinstance(record[k], str) and record[k].strip()
                                            for k in required - {'advisories', 'source_files', 'regressions'}):
            raise ValueError('incomplete dependency patch disposition')
        now = today or date.today()
        expires = date.fromisoformat(record['expires'])
        if expires <= now or expires > now + timedelta(days=31):
            raise ValueError('expired/overlong dependency patch disposition: ' + record['package'])
        if record['lock'] != 'mobile/pnpm-lock.yaml' or \
                record['patch'] != 'mobile/patches/' + record['package'] + '@' + record['version'] + '.patch':
            raise ValueError('wrong lock or patch path')
        if not re.fullmatch(r'[a-z][a-z0-9-]*', record['package']) or \
                not re.fullmatch(r'\d+\.\d+\.\d+', record['version']):
            raise ValueError('invalid package identity')
        if not isinstance(record['source_files'], dict) or not record['source_files'] or \
                not isinstance(record['regressions'], dict) or not record['regressions'] or \
                not isinstance(record['advisories'], list) or set(record['advisories']) != set(record['regressions']):
            raise ValueError('missing source/regression binding')
        for name, digest in record['source_files'].items():
            normalized(name)
            if not re.fullmatch('[0-9a-f]{64}', digest): raise ValueError('invalid source digest')
        for advisory in record['advisories']:
            key = (record['lock'], record['package'], record['version'], advisory)
            if not re.fullmatch(r'GHSA-[a-z0-9]{4}-[a-z0-9]{4}-[a-z0-9]{4}', advisory) or key in seen:
                raise ValueError('invalid/duplicate advisory disposition')
            if record['regressions'][advisory] not in ('uri', 'image', 'icns', 'forge'):
                raise ValueError('unknown regression binding')
            seen.add(key)
        for field in ('lock_sha256', 'patch_sha256', 'regression_sha256', 'compatibility_sha256'):
            if not re.fullmatch('[0-9a-f]{64}', record[field]): raise ValueError('invalid bound digest')
    return data['patches']


def validate_inputs(root, records):
    from workflow_yaml import load_unique
    if not records: return
    lock = load_unique((root / 'mobile/pnpm-lock.yaml').read_text())
    manifest = json.loads((root / 'mobile/package.json').read_text())
    patches = manifest.get('pnpm', {}).get('patchedDependencies', {})
    expected = {r['package'] + '@' + r['version'] for r in records}
    if set(patches) != expected or set(lock.get('patchedDependencies', {})) != expected:
        raise ValueError('patch declarations differ from reviewed policy')
    for r in records:
        bound_file(root, r['lock'], r['lock_sha256'])
        bound_file(root, r['patch'], r['patch_sha256'])
        bound_file(root, REGRESSION, r['regression_sha256'])
        bound_file(root, COMPATIBILITY, r['compatibility_sha256'])
        identity = r['package'] + '@' + r['version']
        relative = str(PurePosixPath(r['patch']).relative_to('mobile'))
        if patches[identity] != relative or lock['patchedDependencies'][identity] != dict(hash=r['patch_sha256'], path=relative):
            raise ValueError('patch map identity mismatch')
        if lock['packages'][identity]['resolution'] != {'integrity': r['integrity']}:
            raise ValueError('original archive integrity changed')
        key = identity + '(patch_hash=' + r['patch_sha256'] + ')'
        snapshots = [name for name in lock['snapshots'] if name.startswith(r['package'] + '@')]
        if snapshots != [key]:
            raise ValueError('missing/unpatched/duplicate snapshot: ' + r['package'])
        for snapshot in lock['snapshots'].values():
            for group in ('dependencies', 'optionalDependencies'):
                version = snapshot.get(group, {}).get(r['package'])
                if version is not None and version != key[len(r['package']) + 1:]:
                    raise ValueError('unpatched dependency edge: ' + r['package'])


def inventory(directory):
    result = {}
    for path in directory.rglob('*'):
        relative = path.relative_to(directory)
        if 'node_modules' in relative.parts: continue
        if path.is_symlink(): raise ValueError('symlink in package source')
        if path.is_file(): result[relative.as_posix()] = sha(path.read_bytes())
        elif not path.is_dir(): raise ValueError('special package source file')
    return result


def archive_bytes(record):
    name, version = record['package'], record['version']
    url = 'https://registry.npmjs.org/' + name + '/-/' + name + '-' + version + '.tgz'
    with urlopen(Request(url, headers={'User-Agent': 'NyxID-patch-verifier'}), timeout=60) as response:
        if urlsplit(response.url).scheme != 'https' or urlsplit(response.url).netloc != 'registry.npmjs.org':
            raise ValueError('archive redirected off registry origin')
        data = response.read(20 * 1024 * 1024 + 1)
    if len(data) > 20 * 1024 * 1024: raise ValueError('archive too large')
    return data


def unpack_original(data, record, destination):
    integrity = 'sha512-' + base64.b64encode(hashlib.sha512(data).digest()).decode()
    if integrity != record['integrity']: raise ValueError('original archive integrity mismatch')
    destination.mkdir()
    total = 0
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as archive:
        for member in archive:
            name = normalized(member.name.rstrip('/'))
            if name == 'package' and member.isdir(): continue
            if not name.startswith('package/'): raise ValueError('archive outside package root')
            relative = PurePosixPath(name).relative_to('package')
            path = destination / relative
            if member.isdir(): path.mkdir(parents=True, exist_ok=True)
            elif member.isfile():
                total += member.size
                if total > 80 * 1024 * 1024 or path.exists(): raise ValueError('oversized/duplicate archive member')
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(archive.extractfile(member).read())
            else: raise ValueError('archive symlink or special file')
    package = json.loads((destination / 'package.json').read_text())
    if (package['name'], package['version']) != (record['package'], record['version']):
        raise ValueError('archive identity mismatch')


def installed_package(directory, record):
    store = directory / 'node_modules/.pnpm'
    candidates = list(store.glob(record['package'] + '@' + record['version'] + '*/node_modules/' + record['package']))
    if len(candidates) != 1 or not candidates[0].is_dir() or candidates[0].is_symlink():
        raise ValueError('missing/duplicate installed package: ' + record['package'])
    if inventory(candidates[0]) != record['source_files']:
        raise ValueError('installed patched source differs: ' + record['package'])
    return candidates[0]


def run_required(command, **kwargs):
    result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kwargs)
    if result.returncode:
        raise ValueError('required patch verification command failed: ' + ' '.join(map(str, command)) +
                         '\n' + result.stdout.decode(errors='replace') + result.stderr.decode(errors='replace'))
    return result


def negative_probe(script, probe, originals, environment):
    env = dict(environment, NYXID_SECURITY_PACKAGE_ROOT=str(originals))
    try:
        result = subprocess.run(['node', str(script), '--probe', probe], env=env,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
    except subprocess.TimeoutExpired:
        if probe in ('uri', 'image', 'icns'): return 'bounded-timeout'
        raise ValueError('unexpected original-probe timeout')
    if result.returncode == 1 and probe == 'uri' and b'RangeError: Maximum call stack size exceeded' in result.stderr and b'decodeComponents' in result.stderr:
        return 'vulnerable-stack-exhaustion'
    if result.returncode == 1 and b'ERR_ASSERTION' in result.stderr and b'MODULE_NOT_FOUND' not in result.stderr:
        return 'security-assertion-failed'
    raise ValueError('original did not demonstrate the targeted regression: ' + probe)


def verify_installed(root, directory, records, environment):
    """All proofs execute now; no externally supplied receipt can authorize a finding."""
    validate_inputs(root, records)
    script = directory / 'scripts/test-security-patches.cjs'
    if not script.is_file() or sha(script.read_bytes()) != records[0]['regression_sha256']:
        raise ValueError('missing regression evidence')
    modules = directory / 'node_modules'
    installed = {r['package']: installed_package(directory, r) for r in records}
    # Expose only the exact verified transitive modules to the existing regression
    # script, which originally ran with npm's hoisted layout. Consumer links in
    # the pnpm store remain intact and continue resolving their real dependencies.
    for name, source in installed.items():
        alias = modules / name
        if alias.exists() or alias.is_symlink():
            if alias.resolve() != source.resolve(): raise ValueError('unexpected test module alias')
        else: alias.symlink_to(source.resolve(), target_is_directory=True)
    query = list((modules / '.pnpm').glob('query-string@*/node_modules/query-string'))
    if len(query) != 1: raise ValueError('ambiguous actual query-string consumer')
    alias = modules / 'query-string'
    if alias.exists() or alias.is_symlink():
        if alias.resolve() != query[0].resolve(): raise ValueError('unexpected query consumer alias')
    else: alias.symlink_to(query[0].resolve(), target_is_directory=True)
    regressions = run_required(['node', str(script)], cwd=directory, env=environment, timeout=60)
    print(regressions.stdout.decode(), end='', flush=True)
    compatibility = run_required(['node', str(root / COMPATIBILITY), str(directory)], env=environment, timeout=60)
    print(compatibility.stdout.decode(), end='', flush=True)
    proofs = []
    with tempfile.TemporaryDirectory(prefix='nyxid-original-patch-proof-') as temporary:
        originals = Path(temporary)
        for r in records:
            original = originals / r['package']
            unpack_original(archive_bytes(r), r, original)
            patched = originals / (r['package'] + '-patched')
            shutil.copytree(original, patched)
            for extra in ('--check', '--apply'):
                run_required(['git', 'apply', '--no-index', extra, str(root / r['patch'])], cwd=patched,
                             env=environment, timeout=30)
            if inventory(patched) != r['source_files']:
                raise ValueError('reviewed patch does not reproduce installed source')
            # The original's dependencies use the exact same frozen installed
            # dependencies. Only the vulnerable package source is replaced.
            (original / 'node_modules').symlink_to(installed[r['package']].parent.resolve(), target_is_directory=True)
            for advisory, probe in r['regressions'].items():
                proof = negative_probe(script, probe, originals, environment)
                proofs.append(dict(advisory=advisory, package=r['package'], version=r['version'],
                                   lock=r['lock'], original_regression=proof))
    # Detect mutation by tests or verification tools before granting a disposition.
    validate_inputs(root, records)
    for r in records: installed_package(directory, r)
    print('Verified patched source inventory: ' + str(sum(len(r['source_files']) for r in records)) +
          ' files across ' + str(len(records)) + ' packages; vulnerable-original proofs: ' +
          json.dumps(proofs, sort_keys=True), flush=True)
    return proofs


def install_and_verify(root, directory, records, pnpm, environment):
    validate_inputs(root, records)
    package = json.loads((root / 'mobile/package.json').read_text())
    version = run_required([str(pnpm), '--version'], env=environment, timeout=30).stdout.decode().strip()
    if package.get('packageManager') != 'pnpm@' + version:
        raise ValueError('patch verification requires the locked pnpm version')
    for r in records:
        target = directory / Path(r['patch']).relative_to('mobile')
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(root / r['patch'], target)
    script = directory / 'scripts/test-security-patches.cjs'
    script.parent.mkdir(exist_ok=True)
    shutil.copyfile(root / REGRESSION, script)
    env = {key: value for key, value in environment.items()
           if key not in ('NODE_OPTIONS', 'NODE_PATH', 'NYXID_SECURITY_PACKAGE_ROOT')}
    env['CI'] = 'true'
    run_required([str(pnpm), 'install', '--frozen-lockfile', '--ignore-scripts', '--ignore-pnpmfile',
                  '--store-dir', str(directory.parent / 'patch-verification-store')],
                 cwd=directory, env=env, timeout=600)
    if sha((directory / 'pnpm-lock.yaml').read_bytes()) != records[0]['lock_sha256']:
        raise ValueError('frozen installation mutated the lock')
    return verify_installed(root, directory, records, env)


def classify(report, lock, records, proofs):
    """Return exact reviewed findings, retaining every unmatched finding as blocking."""
    if not records: return []
    allowed = {(r['lock'], r['package'], r['version'], advisory)
               for r in records for advisory in r['advisories']}
    verified = {(p['lock'], p['package'], p['version'], p['advisory']) for p in proofs
                if p.get('original_regression') in ('bounded-timeout', 'security-assertion-failed', 'vulnerable-stack-exhaustion')}
    reviewed = []
    for advisory in report.get('advisories', {}).values():
        findings = advisory.get('findings', [])
        if not findings: continue
        keys = [(lock, advisory.get('module_name'), f.get('version'), advisory.get('github_advisory_id')) for f in findings]
        if all(key in allowed and key in verified for key in keys):
            reviewed.append(dict(advisory=keys[0][3], package=keys[0][1], versions=sorted({key[2] for key in keys}),
                                 disposition='verified-patched-source'))
    return reviewed
