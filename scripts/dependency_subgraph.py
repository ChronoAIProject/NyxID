"""Canonical affected-package/consumer graphs for reviewed advisory bindings.

Only ancestors of the affected package enter the hash. Unrelated lock entries,
formatting and workspace release versions cannot renew or invalidate a review.
Registry names, versions, sources, checksums and every consumer edge remain exact.
"""
from collections import defaultdict
import hashlib
import json
import re


def closure(packages, edges, target):
    if target not in packages:
        raise ValueError('affected dependency package missing from lock')
    reverse = defaultdict(set)
    for parent, kind, alias, child in edges:
        if parent not in packages or child not in packages:
            raise ValueError('unresolved dependency lock edge')
        reverse[child].add(parent)
    selected, pending = set(), [target]
    while pending:
        current = pending.pop()
        if current in selected:
            continue
        selected.add(current)
        pending.extend(reverse[current] - selected)
    return dict(packages={key: packages[key] for key in sorted(selected)},
                edges=sorted([list(e) for e in edges if e[0] in selected and e[3] in selected]))


def cargo(lock, package, version):
    records = lock['package']
    keys = {}
    packages = {}
    for item in records:
        key = item['name'] + '@' + (item['version'] if 'source' in item else 'workspace')
        if 'source' in item:
            key += '#' + item['source']
            value = {k: item.get(k) for k in ('name', 'version', 'source', 'checksum')}
        else:
            value = dict(name=item['name'], workspace=True)
        if key in packages:
            raise ValueError('duplicate dependency lock identity')
        packages[key] = value
        keys[(item['name'], item['version'], item.get('source'))] = key
    edges = set()
    for item in records:
        parent = keys[(item['name'], item['version'], item.get('source'))]
        for dependency in item.get('dependencies', []):
            match = re.fullmatch(r'(\S+)(?: ([^ ()]+))?(?: \((.+)\))?', dependency)
            if not match:
                raise ValueError('invalid Cargo lock edge')
            name, selected_version, source = match.groups()
            candidates = [key for (n, v, s), key in keys.items() if n == name and
                          (selected_version is None or v == selected_version) and (source is None or s == source)]
            if len(candidates) != 1:
                raise ValueError('missing/ambiguous Cargo lock edge')
            edges.add((parent, 'dependency', name, candidates[0]))
    targets = [key for (n, v, _), key in keys.items() if (n, v) == (package, version)]
    if len(targets) != 1:
        raise ValueError('affected dependency package missing/ambiguous in lock')
    return closure(packages, edges, targets[0])


def pnpm(lock, package, version):
    snapshots = lock['snapshots']
    packages = {}
    for key in snapshots:
        identity = key.split('(', 1)[0]
        if identity not in lock['packages']:
            raise ValueError('pnpm snapshot archive missing from lock')
        packages[key] = dict(identity=identity, archive=lock['packages'][identity])
    edges = set()
    def dependencies(parent, record):
        for kind in ('dependencies', 'devDependencies', 'optionalDependencies'):
            for alias, value in record.get(kind, {}).items():
                selected = value['version'] if isinstance(value, dict) else value
                if selected.startswith(('link:', 'workspace:')):
                    continue
                child = alias + '@' + selected
                # pnpm aliases record the real package identity in the version.
                if child not in packages and selected in packages:
                    child = selected
                if child not in packages:
                    raise ValueError('unresolved pnpm lock edge: ' + child)
                edges.add((parent, kind, alias, child))
    for key, record in snapshots.items():
        dependencies(key, record or {})
    for name, record in lock['importers'].items():
        key = 'importer:' + name
        packages[key] = dict(importer=name)
        dependencies(key, record)
    targets = [key for key in snapshots if key.split('(', 1)[0] == package + '@' + version]
    if not targets:
        raise ValueError('affected dependency package missing from lock')
    graphs = [closure(packages, edges, target) for target in sorted(targets)]
    return graphs


def digest(graph):
    return hashlib.sha256(json.dumps(graph, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
