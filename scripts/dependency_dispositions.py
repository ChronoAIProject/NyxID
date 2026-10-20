"""Reviewed per-use risk decisions, separate from verified source patches.

No global audit ignores: a changed graph, feature, call surface, lock, expiry or
missing executable production-bundle evidence leaves the finding blocking.
"""
from collections import Counter, defaultdict
from datetime import date, timedelta
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess

from dependency_patches import bound_file, regular_file, run_required, sha
import dependency_subgraph

POLICY = 'security/dependency-dispositions.json'
RSA_REFERENCE = re.compile(r'\b(?:rsa|russh|ssh_key)\s*::|\buse\s+rsa\b|\bRsa[A-Za-z0-9_]*')
PRIVATE_OPERATION = re.compile(r'\b(?:decrypt|decrypt_blinded|decrypt_session_key|sign|try_sign|sign_with_rng|'
                               r'try_sign_with_rng|sign_digest|try_sign_digest|sign_digest_with_rng|'
                               r'try_sign_digest_with_rng|sign_prehash|try_sign_prehash|sign_prehash_with_rng)\s*\(')


def load_policy(root, today=None):
    path = root / POLICY
    if not path.exists(): return None  # No record means no risk disposition.
    data = json.loads(path.read_text())
    if set(data) != {'schema_version', 'rsa_guard', 'dispositions'} or data['schema_version'] != 1:
        raise ValueError('invalid dependency disposition policy')
    seen = set()
    for r in data['dispositions']:
        common = {'kind', 'advisory', 'package', 'version', 'lock', 'lock_subgraph_sha256', 'owner', 'expires', 'rationale', 'parents'}
        extra = {'checksum', 'features', 'uses'} if r['package'] == 'rsa' else {'integrity', 'bundle_checker_sha256'}
        if set(r) != common | extra or not all(r[k] for k in common):
            raise ValueError('missing exact dependency disposition binding')
        days = 90 if r['package'] == 'rsa' else 31
        expires = date.fromisoformat(r['expires']); now = today or date.today()
        if expires <= now or expires > now + timedelta(days=days): raise ValueError('expired/overlong dependency disposition')
        if r['package'] == 'rsa':
            if r['kind'] != 'reviewed-rsa-uses' or r['advisory'] != 'RUSTSEC-2023-0071' or r['lock'] != 'Cargo.lock' or r['version'] not in ('0.9.10', '0.10.0-rc.18'):
                raise ValueError('unreviewed RSA disposition scope')
            if set(r['uses']) != {'scope', 'source_files', 'manifest_files', 'call_sites'} or \
                    not all(r['uses'][key] for key in ('scope', 'source_files', 'call_sites')) or \
                    not isinstance(r['uses']['manifest_files'], dict) or not r['parents'] or not r['features'] or \
                    not re.fullmatch('[0-9a-f]{64}', r['checksum']):
                raise ValueError('missing RSA source/use/graph evidence')
            manifests = r['uses']['manifest_files']
            expected = {'backend/Cargo.toml', 'machine/Cargo.toml'} if r['version'] == '0.9.10' else set()
            if set(manifests) != expected or any(not isinstance(value, str) or not re.fullmatch('[0-9a-f]{64}', value)
                                                 for value in manifests.values()):
                raise ValueError('missing exact RSA manifest binding')
        elif (r['package'], r['version'], r['advisory'], r['lock'], r['kind']) != \
                ('braces', '3.0.3', 'GHSA-vfj7-8cjw-p6xm', 'mobile/pnpm-lock.yaml', 'build-tooling-only'):
            raise ValueError('unreviewed tooling-only disposition scope')
        key = (r['lock'], r['package'], r['version'], r['advisory'])
        if key in seen: raise ValueError('duplicate dependency disposition')
        seen.add(key)
        lock = regular_file(root, r['lock']).read_text()
        if r['package'] == 'rsa':
            import tomllib
            graph = dependency_subgraph.cargo(tomllib.loads(lock), r['package'], r['version'])
        else:
            from workflow_yaml import load_unique
            graph = dependency_subgraph.pnpm(load_unique(lock), r['package'], r['version'])
        if dependency_subgraph.digest(graph) != r['lock_subgraph_sha256']:
            raise ValueError('affected package archive/consumer subgraph changed')
    return data


def manifest_fingerprint(root, name):
    """Bind the whole parsed manifest except the application's release number.

    Crypto providers/features, normal/dev/target dependencies, package identity,
    build configuration and every other TOML value remain review bindings.
    Workspace release version changes already have a separate subject identity.
    """
    import tomllib
    manifest = tomllib.loads(regular_file(root, name).read_text())
    packages = {'backend/Cargo.toml': 'nyxid', 'machine/Cargo.toml': 'nyxid-machine'}
    package = manifest.get('package', {})
    if name not in packages or package.get('name') != packages[name]:
        raise ValueError('unreviewed RSA manifest identity: ' + name)
    version = package.get('version')
    if isinstance(version, str) and re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?', version):
        package['version'] = '@application-release'
    elif version != {'workspace': True}:
        raise ValueError('invalid application release version: ' + name)
    return sha(json.dumps(manifest, sort_keys=True, separators=(',', ':')).encode())


def rsa_surface(root):
    """Conservative change guard, not a claim to infer arbitrary Rust data flow.

    Any RSA/SSH reference file needs exact source review. New private-operation
    spellings anywhere in workspace source also need review, even if the author
    believes they are unrelated to RSA. Counts prevent duplicate-line reuse.
    """
    references, operations = {}, {}
    # Workspace source only; independent/vendor Cargo roots get their own audit
    # and have no risk dispositions. Include newly created files, not just Git's
    # index, so a new private operation cannot evade local policy validation.
    for directory in sorted(root.glob('*/src')):
        for path in sorted(directory.rglob('*.rs')):
            text = path.read_text(); name = path.relative_to(root).as_posix()
            if RSA_REFERENCE.search(text): references[name] = sha(path.read_bytes())
            counts = Counter(sha(line.strip().encode()) for line in text.splitlines() if PRIVATE_OPERATION.search(line))
            if counts: operations[name] = dict(sorted(counts.items()))
    return dict(reference_files=references, private_operation_lines=operations)


def validate_rsa_surface(root, policy):
    observed = rsa_surface(root)
    reviewed = policy['rsa_guard']
    if observed != reviewed:
        changes = []
        for section in observed:
            before, after = reviewed.get(section, {}), observed[section]
            for name in sorted(set(before) | set(after)):
                if before.get(name) != after.get(name):
                    changes.append(section + ':' + name)
        raise ValueError('RSA reference/private-operation surface changed; explicit source review required: ' +
                         ', '.join(changes))


def rsa_graph(metadata, version):
    packages = {p['id']: p for p in metadata['packages']}
    nodes = {n['id']: n for n in metadata['resolve']['nodes']}
    workspace = set(metadata['workspace_members'])
    target = [p['id'] for p in packages.values() if p['name'] == 'rsa' and p['version'] == version]
    if len(target) != 1: raise ValueError('missing/duplicate RSA package')
    reverse = defaultdict(list)
    for node in nodes.values():
        for dep in node['deps']:
            kinds = ','.join(sorted({kind['kind'] or 'normal' for kind in dep['dep_kinds']}))
            reverse[dep['pkg']].append((node['id'], kinds))
    chains = []
    def visit(current, trail, seen):
        for parent, kinds in reverse[current]:
            if parent in seen: raise ValueError('cycle in reviewed RSA parent graph')
            p = packages[parent]
            version = 'workspace' if parent in workspace else p['version']
            chain = trail + [p['name'] + '@' + version + '[' + kinds + ']']
            if parent in workspace: chains.append(' <- '.join(chain))
            else: visit(parent, chain, seen | {parent})
    visit(target[0], ['rsa@' + version], {target[0]})
    return dict(features=sorted(nodes[target[0]]['features']), parents=sorted(chains))


def verify_rsa(root, policy, environment):
    validate_rsa_surface(root, policy)
    # Explicit target/features make this evidence independent of the audit host.
    env = {k: v for k, v in environment.items() if k != 'CARGO_HOME'}
    env.update(CARGO_TARGET_DIR='/tmp/nyxid-shared-target', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
               CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0')
    result = run_required(['cargo', 'metadata', '--locked', '--format-version=1', '--filter-platform',
                          'x86_64-unknown-linux-gnu', '--features', 'nyxid/aws-kms,nyxid/gcp-kms'],
                         cwd=root, env=env, timeout=600)
    metadata = json.loads(result.stdout)
    for r in policy['dispositions']:
        if r['package'] != 'rsa': continue
        graph = rsa_graph(metadata, r['version'])
        if graph != {key: r[key] for key in ('features', 'parents')}:
            raise ValueError('RSA parent chains or selected features changed')
        for name, digest in r['uses']['source_files'].items(): bound_file(root, name, digest)
        for name, digest in r['uses']['manifest_files'].items():
            if manifest_fingerprint(root, name) != digest:
                raise ValueError('reviewed RSA manifest configuration changed: ' + name)
    return True


def tooling_parents(lock):
    """Require the full reverse chain through pinned Metro/Jest parents."""
    snapshots = lock['snapshots']; reverse = defaultdict(set)
    for name, value in snapshots.items():
        for group in ('dependencies', 'optionalDependencies'):
            for package, version in value.get(group, {}).items():
                child = package + '@' + version
                reverse[child].add(name)
    if set(name for name in snapshots if name.startswith('braces@')) != {'braces@3.0.3'}:
        raise ValueError('unreviewed braces version/snapshot')
    if reverse['braces@3.0.3'] != {'micromatch@4.0.8'}:
        raise ValueError('braces has an unreviewed direct parent')
    return sorted('braces@3.0.3 <- micromatch@4.0.8 <- ' + parent for parent in reverse['micromatch@4.0.8'])


def verify_tooling(root, directory, record, environment):
    from workflow_yaml import load_unique
    lock = load_unique((root / record['lock']).read_text())
    if tooling_parents(lock) != record['parents'] or \
            lock['packages']['braces@3.0.3']['resolution'] != {'integrity': record['integrity']}:
        raise ValueError('build-only braces parent chain/archive changed')
    checker = bound_file(root, 'scripts/check-mobile-bundle.cjs', record['bundle_checker_sha256'])
    # Do not renew an avoidable risk if the registry publishes a fixed version.
    # The raw audit is also retained; a version becoming unaffected needs no
    # disposition, while a newly published candidate forces an explicit upgrade.
    from urllib.request import urlopen
    with urlopen('https://registry.npmjs.org/braces/latest', timeout=60) as response:
        latest = json.load(response)
    if latest.get('version') != record['version']:
        raise ValueError('new braces release requires upgrade/advisory review before renewing tooling disposition')
    # Copy complete tracked app input into the already clean/frozen install.
    paths = run_required(['git', 'ls-files', '-z', '--', 'mobile'], cwd=root, env=environment, timeout=30).stdout
    for name in paths.decode().split('\0'):
        if not name: continue
        source = root / name
        if source.is_symlink() or not source.is_file(): raise ValueError('non-file mobile bundle input')
        destination = directory / Path(name).relative_to('mobile')
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
    before = sha((directory / 'pnpm-lock.yaml').read_bytes())
    exported = run_required(['node', str(checker), str(directory)], env=environment, timeout=650)
    print(exported.stdout.decode(), end='', flush=True)
    if sha((directory / 'pnpm-lock.yaml').read_bytes()) != before:
        raise ValueError('bundle export mutated dependency lock')
    return True


def classify_cargo(report, lock, policy, verified):
    if not policy or not verified or lock != 'Cargo.lock': return []
    reviewed = []
    for finding in report['vulnerabilities']['list']:
        pkg = finding['package']; advisory = finding['advisory']['id']
        for r in policy['dispositions']:
            if (r['lock'], r['package'], r['version'], r['advisory']) != (lock, pkg['name'], pkg['version'], advisory): continue
            if pkg.get('checksum') != r.get('checksum'): raise ValueError('RSA audit archive identity differs')
            reviewed.append(dict(advisory=advisory, package=pkg['name'], version=pkg['version'],
                                 disposition='reviewed-rsa-uses', uses=r['uses']['scope']))
    return reviewed


def classify_tooling(report, lock, policy, verified):
    if not policy or not verified: return []
    reviewed = []
    for a in report.get('advisories', {}).values():
        for r in policy['dispositions']:
            if (r['lock'], r['package'], r['advisory']) != (lock, a['module_name'], a['github_advisory_id']): continue
            if a.get('findings') and all(f.get('version') == r['version'] for f in a['findings']):
                reviewed.append(dict(advisory=r['advisory'], package=r['package'], versions=[r['version']],
                                     disposition='build-tooling-only-production-bundle-verified'))
    return reviewed
