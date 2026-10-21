"""Final filesystem, executable and retained JS/CSS composition SBOMs."""
import base64
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

from credential_packaging import MANIFEST, SEAL, verify as verify_credentials
from image_artifacts import copy, image_container
from image_vulnerabilities import SCRATCH_COMPONENTS, verify_stage
from publication_contract import file_digest, require_build_identity, source_identity
from sbom_evidence import (bind_outputs, composition_source, document, include,
                           validate, verify_outputs, verify_seal, write, load_json)
from shipped_composition_sbom import components
from source_artifacts import IMAGES, archives, verify_binary, verify_frontend, verify_labels, versions


def run_syft(syft, arguments, output):
    with tempfile.TemporaryDirectory(prefix='nyxid-syft-') as temporary:
        config = Path(temporary) / 'syft.yaml'; config.write_text('{}\n')
        environment = {k: v for k, v in os.environ.items() if not k.startswith('SYFT_')}
        with output.open('wb') as stream:
            subprocess.run([str(syft), '--config', str(config), *arguments], cwd=temporary,
                           env=environment, check=True, stdout=stream)


def asset_document(root, component, source, commit, epoch):
    if component == 'spa':
        verify_frontend(root, source, commit, epoch)
        verify_credentials(root, source, commit, epoch)
    elif component == 'credential-accept':
        verify_credentials(root, source, commit, epoch)
    else:
        raise ValueError('unreviewed standalone asset composition')
    seal = verify_seal(source, root, component)
    subject = root / ('shipped-composition.json' if component == 'spa' else MANIFEST)
    result = document('nyxid-' + component, file_digest(subject), commit,
                      extra=[dict(name='nyxid:inventory-source', value='verified final-byte JS/CSS membership')])
    result['components'], refs = components(seal, component + ':')
    result['dependencies'][0]['dependsOn'] = refs
    # The credential seal is outside its own artifact inventory. Bind those
    # bytes too; its manifest is already a file component inside the seal.
    if component == 'credential-accept':
        result['components'].append(dict(type='file', name=SEAL, **{'bom-ref': 'credential:seal'},
            hashes=[dict(alg='SHA-256', content=file_digest(root / SEAL)[7:])]))
        result['dependencies'][0]['dependsOn'].append('credential:seal')
    validate(result, file_digest(subject), commit)
    return result


def verify_catalog(catalog, receipt):
    metadata = catalog.get('source', {}).get('metadata', {})
    manifest = base64.b64decode(metadata.get('manifest', ''), validate=True)
    config = base64.b64decode(metadata.get('config', ''), validate=True)
    if catalog.get('descriptor', {}).get('name') != 'syft' or catalog['descriptor'].get('version') != '1.54.1' or \
            catalog.get('source', {}).get('type') != 'image' or metadata.get('imageID') != receipt['image_id'] or \
            'sha256:' + hashlib.sha256(config).hexdigest() != receipt['image_id'] or \
            'sha256:' + hashlib.sha256(manifest).hexdigest() != receipt['subject']['digest'] or \
            metadata.get('manifestDigest') != receipt['subject']['digest'] or \
            metadata.get('os', '') + '/' + metadata.get('architecture', '') != receipt['platform'] or \
            catalog.get('descriptor', {}).get('configuration', {}).get('search', {}).get('scope') != 'squashed':
        raise ValueError('Syft filesystem catalog differs from the exact staged manifest/config/platform/tool')
    packages = catalog.get('artifacts', [])
    if not packages or (receipt['component'] not in SCRATCH_COMPONENTS and
                        not any(item.get('type') in ('deb', 'apk', 'rpm') for item in packages)):
        raise ValueError('missing actual filesystem distribution package inventory')


def metadata_capture(catalog, receipt):
    verify_catalog(catalog, receipt)
    metadata = catalog['source']['metadata']
    records = {}
    for name in ('manifest', 'config'):
        content = base64.b64decode(metadata[name], validate=True)
        records['sha256:' + hashlib.sha256(content).hexdigest()] = content
    return dict(digest=receipt['subject']['digest'], mediaType=receipt['subject']['mediaType'],
                size=len(records[receipt['subject']['digest']])), records


def complete_file_inventory(catalog, converted):
    # Syft 1.54.1's CycloneDX encoder omits file coordinates without metadata,
    # directories, symlinks and sockets (cyclonedxhelpers.ToFormatModel). ELF
    # references in the minimal Debian rootfs have coordinates but no metadata.
    # Preserve those exact raw file identities; never repair package conversion
    # or a missing regular file that Syft's encoder is expected to retain.
    components = converted.setdefault('components', [])
    present = {item['bom-ref'] for item in components}
    for item in catalog.get('files', []):
        if item['id'] in present:
            continue
        metadata = item.get('metadata')
        if metadata is not None and metadata.get('type') not in ('Directory', 'SymbolicLink', 'Socket'):
            continue  # The independent exact comparison below must reject this.
        component = {'type': 'file', 'name': item['location']['path'], 'bom-ref': item['id'],
                     'properties': [{'name': 'syft:file:location', 'value': json.dumps(item['location'], sort_keys=True)}]}
        if metadata is not None:
            component['properties'].append({'name': 'syft:file:metadata', 'value': json.dumps(metadata, sort_keys=True)})
        hashes = [{'alg': {'sha256': 'SHA-256', 'sha1': 'SHA-1', 'md5': 'MD5',
                           'sha384': 'SHA-384', 'sha512': 'SHA-512'}[digest['algorithm']],
                   'content': digest['value']} for digest in item.get('digests', [])]
        if hashes:
            component['hashes'] = hashes
        components.append(component)
        present.add(item['id'])


def verify_conversion(catalog, converted):
    # Conversion must retain every package detected in the final filesystem,
    # including identities with no purl. This is independent of graph merging.
    raw = Counter((a['name'], a.get('version'), a.get('purl')) for a in catalog['artifacts'])
    for item in catalog.get('files', []):
        raw[(item['location']['path'], None, None)] += 1
    distro = catalog.get('distro') or {}
    if distro.get('id'):
        raw[(distro['id'], distro.get('versionID'), None)] += 1
    actual = Counter((a['name'], a.get('version'), a.get('purl')) for a in converted.get('components', []))
    if raw != actual:
        delta = lambda values: [dict(name=k[0], version=k[1], purl=k[2], count=v) for k, v in values.items()]
        raise ValueError('converted filesystem SBOM omits or changes actual package identities: ' +
                         json.dumps(dict(missing=delta(raw - actual), unexpected=delta(actual - raw)), sort_keys=True))


def runtime_inventory(catalog, receipt, path):
    expected = {(p['name'], p['version'], p['source']) for p in receipt['audit']['packages']
                if p.get('kind', 'runtime') == 'runtime'}
    actual = {(p['metadata']['name'], p['metadata']['version'], p['metadata']['source'])
              for p in catalog['artifacts'] if p.get('foundBy') == 'cargo-auditable-binary-cataloger'
              and any(location.get('path', '').lstrip('/') == path.lstrip('/') for location in p.get('locations', []))}
    if actual != expected:
        raise ValueError('image runtime inventory differs from the actual final executable')


def assembled(receipt, filesystem, children):
    result = document(receipt['component'], receipt['subject']['digest'], receipt['source_commit'], 'container',
                      [dict(name='nyxid:platform', value=receipt['platform']),
                       dict(name='nyxid:image-config', value=receipt['image_id']),
                       dict(name='nyxid:inventory-source', value='final squashed image filesystem and extracted final artifacts')])
    # The converted Syft document's root is a config identity. Keep its complete
    # package graph under the actual manifest subject instead of using that
    # config identity as the image's published subject.
    filesystem = json.loads(json.dumps(filesystem))
    old_root = filesystem['metadata']['component']
    refs = [c['bom-ref'] for c in filesystem.get('components', [])]
    if not refs: raise ValueError('empty converted final filesystem inventory')
    filesystem['dependencies'] = [d for d in filesystem.get('dependencies', []) if d['ref'] != old_root['bom-ref']]
    filesystem['dependencies'].insert(0, dict(ref=old_root['bom-ref'], dependsOn=refs))
    include(result, filesystem, 'filesystem:')
    for name, child in sorted(children.items()): include(result, child, name + ':')
    validate(result, receipt['subject']['digest'], receipt['source_commit'])
    return result


def generate(stage, source, epoch, extractor, syft):
    scan = verify_stage(stage)
    output = stage / 'sbom'; output.mkdir(exist_ok=False)
    names, children = [], {}
    catalog_path = output / 'filesystem.syft.json'
    run_syft(syft, ['scan', 'oci-dir:' + str(stage / 'oci'), '--scope', 'squashed', '-o', 'syft-json'], catalog_path)
    catalog = load_json(catalog_path); verify_catalog(catalog, scan)
    run_syft(syft, ['convert', str(catalog_path), '-o', 'cyclonedx-json@1.6'], output / 'filesystem.cdx.json')
    filesystem = load_json(output / 'filesystem.cdx.json')
    complete_file_inventory(catalog, filesystem)
    verify_conversion(catalog, filesystem)
    write(output / 'filesystem.cdx.json', filesystem)
    names += ['filesystem.syft.json', 'filesystem.cdx.json']
    component = scan['component']; _, _, package, path, abi = IMAGES[component]
    arch = scan['platform'].split('/')[1]; commit = scan['source_commit']
    # Resolve and extract the same immutable config already saved/scanned. No
    # foreign image executable is run. Every temporary container removes volumes.
    with tempfile.TemporaryDirectory(prefix='nyxid-sbom-extract-') as temporary, \
            image_container(scan['image_id']) as (container, info):
        verify_labels(info, arch, commit, epoch)
        root = Path(temporary)
        if package:
            binary_root = root / 'binary'; found = copy(container, path, binary_root)
            binary = binary_root / Path(path).name
            if found != [binary.name]: raise ValueError('unexpected final executable extraction closure')
            target = {'amd64': 'x86_64', 'arm64': 'aarch64'}[arch] + '-unknown-linux-' + abi
            verify_binary(binary, extractor, package, versions(source)[package], target, commit, epoch)
            receipt = json.loads(subprocess.check_output([str(extractor), str(binary)]))
            require_build_identity(receipt, package, versions(source)[package], target, source_identity(commit, epoch))
            write(output / 'binary-info.json', receipt)
            run_syft(syft, ['scan', 'file:' + str(binary), '-o', 'syft-json'], output / 'binary.syft.json')
            binary_sbom = archives.binary_sbom
            binary_sbom.verify_syft(load_json(output / 'binary.syft.json'), receipt)
            runtime_inventory(catalog, receipt, path)
            child = binary_sbom.sbom(binary, receipt, package, commit)
            if package == 'nyxid-cli':
                with composition_source(source) as provenance:
                    assets = provenance / 'cli/src/wizard/assets'
                    verify_seal(provenance, assets, 'wizard')
                    import importlib.util
                    spec = importlib.util.spec_from_file_location('sbom_wizard', Path(__file__).with_name('verify-embedded-wizard.py'))
                    wizard = importlib.util.module_from_spec(spec); spec.loader.exec_module(wizard)
                    embedded = wizard.verify(binary, assets)
                    write(output / 'embedded-wizard.json', embedded)
                    (output / 'wizard-seal.json').write_bytes((assets / 'shipped-composition.json').read_bytes())
                    names += ['embedded-wizard.json', 'wizard-seal.json']
                    binary_sbom.attach_wizard(child, (assets / 'shipped-composition.json').read_bytes(), embedded)
            validate(child, file_digest(binary), commit)
            children['binary'] = child; write(output / 'binary.cdx.json', child)
            names += ['binary-info.json', 'binary.syft.json', 'binary.cdx.json']
        if component in ('backend', 'frontend'):
            assets = root / 'assets'
            if component == 'frontend': copy(container, path + '/.', assets)
            else:
                for directory in ('credential-accept', 'release-integrity'):
                    copy(container, '/app/frontend/dist/' + directory + '/.', assets / directory)
            with composition_source(source) as provenance:
                kind = 'spa' if component == 'frontend' else 'credential-accept'
                child = asset_document(assets, kind, provenance, commit, epoch)
            children['assets'] = child; write(output / 'assets.cdx.json', child); names.append('assets.cdx.json')
    filesystem = load_json(output / 'filesystem.cdx.json')
    verify_conversion(catalog, filesystem)
    result = assembled(scan, filesystem, children)
    write(output / 'image.cdx.json', result); names.append('image.cdx.json')
    receipt = dict(schema='nyxid.image-sbom.v1', component=component, platform=scan['platform'],
                   source=source_identity(commit, epoch), subject=scan['subject'], image_id=scan['image_id'],
                   archive_sha256=scan['archive_sha256'], children=sorted(children), outputs=bind_outputs(output, names))
    write(output / 'image-sbom.json', receipt)
    verify(stage)
    print('Final filesystem/composition SBOM verified: ' + component + ' ' + scan['platform'] + ' ' + scan['subject']['digest'])
    return receipt


def verify_bundle(directory):
    receipt = load_json(directory / 'image-sbom.json')
    if receipt.get('schema') != 'nyxid.image-sbom.v1' or receipt.get('component') not in IMAGES:
        raise ValueError('missing reviewed final image SBOM')
    verify_outputs(directory, receipt['outputs'])
    component = receipt['component']; package = IMAGES[component][2]
    expected = {'filesystem.syft.json', 'filesystem.cdx.json', 'image.cdx.json'}
    children = {}
    if package:
        expected.update(('binary-info.json', 'binary.syft.json', 'binary.cdx.json'))
        children['binary'] = load_json(directory / 'binary.cdx.json')
        if package == 'nyxid-cli': expected.update(('embedded-wizard.json', 'wizard-seal.json'))
    if component in ('backend', 'frontend'):
        expected.add('assets.cdx.json'); children['assets'] = load_json(directory / 'assets.cdx.json')
    if set(receipt['outputs']) != expected or receipt.get('children') != sorted(children):
        raise ValueError('missing/extra actual binary, wizard or asset SBOM composition evidence')
    scan = dict(component=component, subject=receipt['subject'], source_commit=receipt['source']['commit'],
                platform=receipt['platform'], image_id=receipt['image_id'])
    catalog = load_json(directory / 'filesystem.syft.json')
    verify_catalog(catalog, scan)
    filesystem = load_json(directory / 'filesystem.cdx.json')
    verify_conversion(catalog, filesystem)
    if package:
        info = load_json(directory / 'binary-info.json')
        target = {'amd64': 'x86_64', 'arm64': 'aarch64'}[receipt['platform'].split('/')[1]] + '-unknown-linux-' + IMAGES[component][4]
        require_build_identity(info, package, next(p['version'] for p in info['audit']['packages'] if p.get('root')), target, receipt['source'])
        archives.binary_sbom.verify_syft(load_json(directory / 'binary.syft.json'), info)
        runtime_inventory(catalog, info, IMAGES[component][3])
        compiled = archives.binary_sbom.inventory_document(info, package, receipt['source']['commit'])
        if package == 'nyxid-cli':
            archives.binary_sbom.attach_wizard(compiled, (directory / 'wizard-seal.json').read_bytes(),
                                                load_json(directory / 'embedded-wizard.json'))
        if children['binary'] != compiled:
            raise ValueError('binary SBOM differs from compiled/wizard inventory of extracted final executable')
    for child in children.values():
        validate(child, child['metadata']['component']['bom-ref'], receipt['source']['commit'])
    actual = load_json(directory / 'image.cdx.json')
    if actual != assembled(scan, filesystem, children):
        raise ValueError('final image SBOM composition differs from retained filesystem/binary/asset evidence')
    return receipt


def verify(stage):
    scan = verify_stage(stage); receipt = verify_bundle(stage / 'sbom')
    if receipt['source']['commit'] != scan['source_commit'] or any(receipt[key] != scan[key] for key in
            ('component', 'platform', 'subject', 'image_id', 'archive_sha256')):
        raise ValueError('SBOM does not bind the exact passing staged image')
    return receipt
