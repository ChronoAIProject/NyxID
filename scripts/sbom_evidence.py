"""Offline schema, subject and dependency-closure checks for shipped SBOMs."""
from contextlib import contextmanager
from functools import lru_cache
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import uuid

from publication_contract import DIGEST, file_digest, unique_object

ROOT = Path(__file__).resolve().parents[1]


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def load_json(path):
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 256 * 1024 * 1024:
        raise ValueError("missing, linked or oversized SBOM evidence")
    return json.loads(path.read_bytes(), object_pairs_hook=unique_object)


@lru_cache(maxsize=1)
def schema_validator():
    import importlib.util
    spec = importlib.util.spec_from_file_location('nyxid_cyclonedx', ROOT / 'scripts/validate-cyclonedx.py')
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module.validator()


def properties(component):
    pairs = component.get('properties', [])
    values = {p['name']: p['value'] for p in pairs}
    if len(values) != len(pairs):
        raise ValueError('duplicate SBOM subject property')
    return values


def validate(document, digest, commit):
    if not DIGEST.fullmatch(digest) or not re.fullmatch('[0-9a-f]{40}', commit):
        raise ValueError('exact SBOM subject/source identity required')
    from jsonschema.exceptions import ValidationError
    try:
        schema_validator().validate(document)
    except ValidationError as error:
        raise ValueError('CycloneDX 1.6 schema: ' + error.message) from error
    root = document['metadata']['component']
    if document.get('specVersion') != '1.6' or root.get('bom-ref') != digest or \
            root.get('hashes') != [{'alg': 'SHA-256', 'content': digest[7:]}] or \
            properties(root).get('nyxid:source-commit') != commit:
        raise ValueError('SBOM does not bind the expected final subject and source')
    components = document.get('components', [])
    refs = [root['bom-ref']] + [c.get('bom-ref') for c in components]
    if not components or any(not isinstance(ref, str) or not ref for ref in refs) or len(set(refs)) != len(refs):
        raise ValueError('empty or duplicate SBOM component inventory')
    ref_set = set(refs)
    graph = {}
    for node in document.get('dependencies', []):
        children = []
        for relation in ('dependsOn', 'provides'):
            edges = node.get(relation, [])
            if len(edges) != len(set(edges)) or not set(edges) <= ref_set:
                raise ValueError('SBOM dependency references do not close')
            children.extend(edges)
        if node['ref'] in graph or node['ref'] not in ref_set:
            raise ValueError('SBOM dependency references do not close')
        graph[node['ref']] = children
    reached, pending = set(), [root['bom-ref']]
    while pending:
        ref = pending.pop()
        if ref in reached: continue
        reached.add(ref); pending.extend(graph.get(ref, []))
    if reached != ref_set:
        raise ValueError('SBOM contains components outside its subject dependency closure')
    return document


def document(name, digest, commit, kind='application', extra=()):
    return dict(bomFormat='CycloneDX', specVersion='1.6', version=1,
                serialNumber='urn:uuid:' + str(uuid.uuid5(uuid.NAMESPACE_URL, digest)),
                metadata={'component': dict(type=kind, name=name, **{'bom-ref': digest},
                    hashes=[dict(alg='SHA-256', content=digest[7:])],
                    properties=[dict(name='nyxid:source-commit', value=commit), *extra])},
                components=[], dependencies=[dict(ref=digest, dependsOn=[])])


def include(parent, child, prefix):
    """Copy a validated child graph with distinct references, preserving purls."""
    child = json.loads(json.dumps(child))
    root = child['metadata']['component']
    components = [root, *child.get('components', [])]
    remap = {c['bom-ref']: prefix + c['bom-ref'] for c in components}
    for component in components:
        component['bom-ref'] = remap[component['bom-ref']]
    parent['components'].extend(components)
    for dependency in child.get('dependencies', []):
        copied = dict(dependency, ref=remap[dependency['ref']])
        for key in ('dependsOn', 'provides'):
            if key in copied: copied[key] = [remap[r] for r in dependency[key]]
        parent['dependencies'].append(copied)
    parent['dependencies'][0]['dependsOn'].append(root['bom-ref'])


def bind_outputs(directory, names):
    return {name: file_digest(directory / name, 256 * 1024 * 1024) for name in sorted(names)}


def verify_outputs(directory, records):
    if not records or any(not re.fullmatch('[A-Za-z0-9_.-]+', name) for name in records):
        raise ValueError('invalid SBOM evidence output closure')
    if bind_outputs(directory, records) != records:
        raise ValueError('SBOM evidence bytes changed')


@contextmanager
def composition_source(source):
    """Verify provenance using a fresh integrity-locked, hook-free npm install.

    Work in a copy: the authenticated producer export and the operator checkout
    must remain unchanged. This also avoids trusting arbitrary node_modules.
    """
    with tempfile.TemporaryDirectory(prefix='nyxid-sbom-source-') as temporary:
        root = Path(temporary).resolve()
        tree = root / 'source'
        shutil.copytree(source, tree, ignore=shutil.ignore_patterns('.git', 'node_modules', 'target', 'dist', '__pycache__', '.venv', '.env', '.env.*'))
        tools = root / 'tools'
        subprocess.run([sys.executable, str(tree / 'scripts/bootstrap-build-tool.py'), 'npm', '--bin-dir', str(tools)], check=True)
        environment = {k: v for k, v in os.environ.items() if not k.upper().startswith('NPM_CONFIG_')}
        subprocess.run([str(tools / 'npm'), 'ci', '--ignore-scripts', '--no-audit', '--no-fund'],
                       cwd=tree / 'frontend', env=environment, check=True)
        yield tree


def verify_seal(source, root, component):
    script = source / 'frontend/scripts/seal-shipped-composition.mjs'
    output = root / ('shipped-composition.json' if component in ('spa', 'wizard') else 'release-integrity/credential-composition.json')
    options = dict(root=str(root), component=component, output=str(output), verify=True,
                   wizardGraph=str(source / 'cli/src/wizard/bundle-meta/rendered-graph.json'))
    code = 'import {seal} from ' + json.dumps(script.as_uri()) + '; seal(JSON.parse(process.argv[1]));'
    subprocess.run(['node', '--input-type=module', '-e', code, json.dumps(options)], cwd=source, check=True)
    return load_json(output)
