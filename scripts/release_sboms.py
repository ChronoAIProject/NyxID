"""Replay final archive evidence and require the entire planned release set."""
from pathlib import Path
import shutil
import tempfile

from publication_contract import file_digest, validate_plan
from sbom_evidence import bind_outputs, validate, verify_outputs, write, load_json
from source_artifacts import archives


def verify_archives(plan, artifacts, evidence):
    expected = {s['id']: s for s in plan['subjects'] if s.get('artifact_kind') == 'executable-zip'}
    records = {}
    for path in sorted(evidence.rglob('archive-inspection.json')):
        receipt = load_json(path)
        if receipt.get('schema') != 'nyxid.archive-inspection.v1' or receipt.get('source') != plan['source']:
            raise ValueError('archive SBOM evidence source differs from publication plan')
        for record in receipt['subjects']:
            identifier = record['id']
            if identifier not in expected or identifier in records or record['source'] != plan['source']:
                raise ValueError('missing/duplicate/extra archive SBOM subject')
            subject = expected[identifier]; directory = path.parent
            names = {subject['name'] + suffix for suffix in
                     ('.binary-info.json', '.syft.json', '.embedded-wizard.json', '.binary.cdx.json', '.cdx.json')}
            if set(record['outputs']) != names: raise ValueError('incomplete final archive SBOM output closure')
            verify_outputs(directory, record['outputs'])
            with tempfile.TemporaryDirectory(prefix='nyxid-sbom-replay-') as temporary:
                root = Path(temporary)
                inspection = archives.inspect_archive(artifacts / subject['name'], subject, root)
                binary = root / subject['binary']
                info = load_json(directory / (subject['name'] + '.binary-info.json'))
                identity = archives.require_build_identity(info, subject['package'], plan['version'], subject['target'], plan['source'])
                if record['digest'] != inspection['digest'] or record['binary'] != info['subject'] or record['build_identity'] != identity:
                    raise ValueError('archive/binary digest identity differs from actual final bytes')
                archives.binary_sbom.verify_syft(load_json(directory / (subject['name'] + '.syft.json')), info)
                binary_document = archives.binary_sbom.sbom(binary, info, subject['package'], plan['source']['commit'])
                # Recheck all final embedded bytes independently of a stored
                # receipt, without executing the foreign release executable.
                import importlib.util
                spec = importlib.util.spec_from_file_location('replay_wizard', Path(__file__).with_name('verify-embedded-wizard.py'))
                wizard = importlib.util.module_from_spec(spec); spec.loader.exec_module(wizard)
                assets = archives.binary_sbom.ROOT / 'cli/src/wizard/assets'
                embedded = wizard.verify(binary, assets)
                if embedded != load_json(directory / (subject['name'] + '.embedded-wizard.json')):
                    raise ValueError('embedded wizard evidence differs from actual final archive binary')
                archives.binary_sbom.attach_wizard(binary_document, (assets / 'shipped-composition.json').read_bytes(), embedded)
                archive_document = archives.archive_sbom(binary_document, subject, inspection)
                for suffix, document, digest in (('.binary.cdx.json', binary_document, 'sha256:' + info['subject']['sha256']),
                                                 ('.cdx.json', archive_document, inspection['digest'])):
                    validate(document, digest, plan['source']['commit'])
                    if load_json(directory / (subject['name'] + suffix)) != document:
                        raise ValueError('archive SBOM differs from compiled/wizard/final-member composition')
            records[identifier] = (record, directory)
    if set(records) != set(expected): raise ValueError('missing planned target archive SBOM evidence')
    return records


def generate(plan, artifacts, evidence, credential_sbom, output):
    validate_plan(plan)
    if plan['producer'] != 'cli': raise ValueError('CLI release publication plan required')
    records = verify_archives(plan, artifacts, evidence)
    # Digest every subject from the independently validated publication plan,
    # including installers/source/checksums; never infer coverage from a glob.
    subjects = {s['name']: file_digest(artifacts / s['name']) for s in plan['subjects']}
    credential = load_json(credential_sbom)
    validate(credential, subjects['releases.json'], plan['source']['commit'])
    output.mkdir(exist_ok=False)
    names = []
    for subject, (_, directory) in sorted(records.items()):
        name = subject.removeprefix('file:')
        for suffix in ('.binary.cdx.json', '.cdx.json'):
            shutil.copyfile(directory / (name + suffix), output / (name + suffix)); names.append(name + suffix)
    shutil.copyfile(credential_sbom, output / 'releases.json.cdx.json'); names.append('releases.json.cdx.json')
    receipt = dict(schema='nyxid.release-sboms.v1', source=plan['source'], subjects=subjects,
                   outputs=bind_outputs(output, names))
    write(output / 'release-sboms.json', receipt)
    verify(plan, artifacts, output)
    return receipt


def verify(plan, artifacts, evidence):
    validate_plan(plan)
    receipt = load_json(evidence / 'release-sboms.json')
    if receipt.get('schema') != 'nyxid.release-sboms.v1' or receipt.get('source') != plan['source'] or \
            receipt.get('subjects') != {s['name']: file_digest(artifacts / s['name']) for s in plan['subjects']}:
        raise ValueError('complete release SBOM subject digest closure differs from final publication set')
    expected = {'releases.json.cdx.json'}
    for subject in plan['subjects']:
        if subject.get('artifact_kind') == 'executable-zip':
            expected.update(subject['name'] + suffix for suffix in ('.binary.cdx.json', '.cdx.json'))
    if set(receipt['outputs']) != expected: raise ValueError('missing/extra planned release SBOM evidence')
    verify_outputs(evidence, receipt['outputs'])
    for subject in plan['subjects']:
        if subject.get('artifact_kind') in ('executable-zip', 'credential-manifest'):
            archive = load_json(evidence / (subject['name'] + '.cdx.json'))
            validate(archive, receipt['subjects'][subject['name']], plan['source']['commit'])
            if subject.get('artifact_kind') == 'executable-zip':
                binary = load_json(evidence / (subject['name'] + '.binary.cdx.json'))
                digest = binary['metadata']['component']['bom-ref']
                validate(binary, digest, plan['source']['commit'])
                with tempfile.TemporaryDirectory(prefix='nyxid-release-sbom-closure-') as temporary:
                    inspection = archives.inspect_archive(artifacts / subject['name'], subject, Path(temporary))
                    actual_binary = Path(temporary) / subject['binary']
                    if file_digest(actual_binary) != digest or archive != archives.archive_sbom(binary, subject, inspection):
                        raise ValueError('final release archive SBOM differs from actual binary/member digest composition')
    return receipt
