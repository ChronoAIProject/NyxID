"""Exact platform/index SBOM composition and complete image publication closure."""
import hashlib
import json
from pathlib import Path
import shutil

from image_sbom import metadata_capture, verify_bundle
from oci_subjects import OCI_INDEX, inspect
from publication_contract import validate_plan
from sbom_evidence import bind_outputs, document, include, load_json, validate, verify_outputs, write

PLATFORMS = ('linux/amd64', 'linux/arm64')


def platform_set(paths, component, source):
    result, blobs = {}, {}
    for path in paths:
        receipt = verify_bundle(path)
        platform = receipt['platform']
        if receipt['component'] != component or receipt['source'] != source or platform not in PLATFORMS or platform in result:
            raise ValueError('wrong/duplicate platform, component or source in image SBOM set')
        descriptor, records = metadata_capture(load_json(path / 'filesystem.syft.json'), receipt)
        descriptor['platform'] = dict(zip(('os', 'architecture'), platform.split('/')))
        result[platform] = (path, receipt, descriptor)
        for digest, data in records.items():
            if digest in blobs and blobs[digest] != data: raise ValueError('conflicting OCI metadata')
            blobs[digest] = data
    if set(result) != set(PLATFORMS): raise ValueError('missing planned image SBOM platform')
    return result, blobs


def composition(component, source, subject, platforms):
    result = document(component + '-multiarch', subject, source['commit'], 'container',
                      [dict(name='nyxid:inventory-source', value='exact runnable platform SBOM closure')])
    for platform, (path, _, _) in sorted(platforms.items()):
        include(result, load_json(path / 'image.cdx.json'), platform + ':')
    validate(result, subject, source['commit'])
    return result


def index(component, source, paths, output):
    platforms, blobs = platform_set(paths, component, source)
    manifest = dict(schemaVersion=2, mediaType=OCI_INDEX,
                    manifests=[platforms[p][2] for p in sorted(platforms)])
    raw = json.dumps(manifest, separators=(',', ':'), sort_keys=True).encode()
    digest = 'sha256:' + hashlib.sha256(raw).hexdigest()
    descriptor = dict(mediaType=OCI_INDEX, digest=digest, size=len(raw))
    blobs[digest] = raw
    subjects = inspect(descriptor, blobs.__getitem__, list(PLATFORMS), source['commit'])
    output.mkdir(exist_ok=False)
    (output / 'index.json').write_bytes(raw)
    write(output / 'index.cdx.json', composition(component, source, digest, platforms))
    for platform, (path, _, _) in platforms.items():
        shutil.copytree(path, output / platform.replace('/', '-'))
    receipt = dict(schema='nyxid.index-sbom.v1', component=component, source=source, subject=descriptor,
                   subjects=subjects, outputs=bind_outputs(output, ['index.json', 'index.cdx.json']))
    write(output / 'index-sbom.json', receipt)
    verify_index(output)
    return receipt


def verify_index(directory):
    receipt = load_json(directory / 'index-sbom.json')
    if receipt.get('schema') != 'nyxid.index-sbom.v1' or set(receipt['outputs']) != {'index.json', 'index.cdx.json'}:
        raise ValueError('missing exact image index SBOM')
    verify_outputs(directory, receipt['outputs'])
    if {p.name for p in directory.iterdir()} != {'index.json', 'index.cdx.json', 'index-sbom.json', 'linux-amd64', 'linux-arm64'}:
        raise ValueError('extra/missing image index evidence')
    platforms, blobs = platform_set([directory / p.replace('/', '-') for p in PLATFORMS], receipt['component'], receipt['source'])
    raw = (directory / 'index.json').read_bytes(); digest = 'sha256:' + hashlib.sha256(raw).hexdigest()
    if receipt['subject'] != dict(digest=digest, mediaType=OCI_INDEX, size=len(raw)):
        raise ValueError('image index subject digest changed')
    blobs[digest] = raw
    subjects = inspect(receipt['subject'], blobs.__getitem__, list(PLATFORMS), receipt['source']['commit'])
    if subjects != receipt['subjects'] or {p['digest'] for p in subjects['runnable']} != {r[1]['subject']['digest'] for r in platforms.values()}:
        raise ValueError('image index children differ from passing platform SBOM subjects')
    expected = dict(schemaVersion=2, mediaType=OCI_INDEX, manifests=[platforms[p][2] for p in sorted(platforms)])
    if load_json(directory / 'index.json') != expected:
        raise ValueError('image index descriptors differ from exact platform evidence')
    if load_json(directory / 'index.cdx.json') != composition(receipt['component'], receipt['source'], digest, platforms):
        raise ValueError('image index SBOM differs from platform composition')
    return receipt


def closure(plan, evidence):
    validate_plan(plan)
    if plan['producer'] != 'images': raise ValueError('image publication plan required')
    expected = {s['component']: s for s in plan['subjects'] if s['kind'] == 'image-index'}
    records, actual = {}, {}
    for path in sorted(evidence.rglob('index-sbom.json')):
        receipt = verify_index(path.parent); component = receipt['component']
        if component not in expected or component in records or receipt['source'] != plan['source']:
            raise ValueError('extra/duplicate/wrong-source image publication SBOM')
        records[component] = receipt
        actual['index:' + component] = receipt['subject']['digest']
        for platform in receipt['subjects']['runnable']:
            actual['image:' + component + ':' + platform['platform']] = platform['digest']
    if set(records) != set(expected) or set(actual) != {s['id'] for s in plan['subjects']}:
        raise ValueError('incomplete expected image publication SBOM/digest closure')
    return dict(schema='nyxid.image-sboms.v1', source=plan['source'], subjects=actual)


def generate_set(plan, evidence, output):
    validate_plan(plan)
    if plan['producer'] != 'images': raise ValueError('image publication plan required')
    grouped = {s['component']: [] for s in plan['subjects']}
    for path in evidence.rglob('image-sbom.json'):
        receipt = verify_bundle(path.parent)
        if receipt['component'] not in grouped:
            raise ValueError('extra component outside the planned publication SBOM set')
        grouped[receipt['component']].append(path.parent)
    output.mkdir(exist_ok=False)
    for component, paths in sorted(grouped.items()):
        index(component, plan['source'], paths, output / component)
    return closure(plan, output)
