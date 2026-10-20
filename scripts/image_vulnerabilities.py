"""Scan final local image bytes and bind the result to a replayable OCI subject."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

from oci_subjects import directory_reader, inspect
from source_artifacts import IMAGES

DIGEST = re.compile(r'sha256:[0-9a-f]{64}')
BLOCKED = {'HIGH', 'CRITICAL', 'UNKNOWN'}


def sha(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def evaluate(report, image_id, component):
    if report.get('SchemaVersion') != 2 or report.get('ArtifactType') != 'container_image' or \
            report.get('Metadata', {}).get('ImageID') != image_id:
        raise ValueError('Trivy report does not bind the actual final image config')
    if not isinstance(report.get('Results'), list) or not report['Results']:
        raise ValueError('Trivy produced no package coverage')
    if report.get('Metadata', {}).get('OS', {}).get('EOSL'):
        raise ValueError('end-of-support image distribution')
    findings = []
    covered = 0
    for result in report['Results']:
        covered += len(result.get('Packages') or [])
        for vulnerability in result.get('Vulnerabilities') or []:
            severity = vulnerability.get('Severity')
            if severity not in {'LOW', 'MEDIUM', 'HIGH', 'CRITICAL', 'UNKNOWN'}:
                raise ValueError('missing/invalid vulnerability severity')
            if severity in BLOCKED:
                findings.append(dict(id=vulnerability['VulnerabilityID'], package=vulnerability['PkgName'],
                                     version=vulnerability['InstalledVersion'], severity=severity))
    if not covered:
        raise ValueError('Trivy produced an empty package inventory')
    if component not in IMAGES:
        raise ValueError('unreviewed application image component')
    _, _, package, binary, _ = IMAGES[component]
    if component != 'nyxid-machine-updater' and not any(
            item.get('Class') == 'os-pkgs' and item.get('Packages') for item in report['Results']):
        raise ValueError('missing final distribution package inventory')
    if package and not any(item.get('Class') == 'lang-pkgs' and item.get('Type') == 'rustbinary' and
                           item.get('Target', '').lstrip('/') == binary.lstrip('/') and
                           any(pkg.get('Name') == package for pkg in item.get('Packages') or [])
                           for item in report['Results']):
        raise ValueError('missing embedded dependency inventory for final application executable')
    return findings


def layout_subject(layout, digest, platform, commit):
    if not DIGEST.fullmatch(digest):
        raise ValueError('invalid staged image digest')
    for path in layout.rglob('*'):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise ValueError('nonregular OCI staging input')
    blobs = layout / 'blobs/sha256'
    for path in blobs.iterdir():
        if not re.fullmatch(r'[0-9a-f]{64}', path.name) or not path.is_file() or sha(path) != path.name:
            raise ValueError('staged OCI blob bytes differ from their digest')
    index = json.loads((layout / 'index.json').read_text())
    # A containerd docker-save archive has OCI and Docker compatibility views.
    # regctl imports both, retaining an untagged alias for the same digest. Bind
    # the one explicit import destination, then verify its digest/config/bytes.
    roots = [item for item in index['manifests'] if
             item.get('annotations', {}).get('org.opencontainers.image.ref.name') == 'staged']
    if len(roots) != 1:
        raise ValueError('staged OCI subject missing or ambiguous')
    if roots[0]['digest'] != digest:
        raise ValueError('staged OCI tag differs from resolved subject digest')
    return inspect(roots[0], directory_reader(layout), [platform], commit)


def scan(image, component, platform, commit, trivy, regctl, stage):
    stage.mkdir(parents=True, exist_ok=False)
    environment = {key: value for key, value in os.environ.items() if not key.startswith('TRIVY_')}
    info = json.loads(subprocess.check_output(['docker', 'image', 'inspect', image]))
    if len(info) != 1 or not DIGEST.fullmatch(info[0]['Id']):
        raise ValueError('one immutable final image is required')
    image_id = info[0]['Id']
    if info[0].get('Os') + '/' + info[0].get('Architecture') != platform:
        raise ValueError('final vulnerability scan platform mismatch')
    archive = stage / 'image.tar'
    subprocess.run(['docker', 'image', 'save', '--output', str(archive), image_id], check=True)
    archive_hash = sha(archive)
    layout = stage / 'oci'
    local = 'ocidir://' + str(layout) + ':staged'
    subprocess.run([str(regctl), 'image', 'import', local, str(archive)], check=True)
    digest = subprocess.check_output([str(regctl), 'manifest', 'digest', local], text=True).strip()
    subject = layout_subject(layout, digest, platform, commit)
    if subject['runnable'][0]['config'] != image_id:
        raise ValueError('staged OCI config differs from scanned final image')
    with tempfile.TemporaryDirectory(prefix='nyxid-trivy-') as temporary:
        config = Path(temporary) / 'trivy.yaml'
        config.write_text('{}\n')
        report_path = stage / 'trivy.json'
        # All OS and language results, including unfixed and development
        # dependencies, remain in the report. No local ignore/VEX policy loads.
        subprocess.run([str(trivy), 'image', '--config', str(config), '--ignorefile=',
                        '--scanners', 'vuln', '--format', 'json', '--list-all-pkgs',
                        '--parallel', '2', '--disable-telemetry', '--no-progress',
                        '--timeout', '20m', '--output', str(report_path), '--input', str(archive)],
                       cwd=temporary, env=environment, check=True)
    report = json.loads(report_path.read_text())
    findings = evaluate(report, image_id, component)
    if sha(archive) != archive_hash:
        raise ValueError('scanner input changed during inspection')
    layout_subject(layout, digest, platform, commit)
    receipt = dict(schema_version=1, status='failed' if findings else 'passed', component=component,
                   platform=platform, source_commit=commit, image_id=image_id, subject=subject['subject'],
                   archive_sha256=archive_hash, report_sha256=sha(report_path), blocking=findings,
                   threshold=sorted(BLOCKED))
    (stage / 'image-scan.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(f"Final {component} ({platform}) image scan: {len(findings)} blocking vulnerabilities; subject {digest}")
    for item in findings:
        print('::error::' + ' '.join(str(item[key]) for key in ('severity', 'id', 'package', 'version')))
    return 1 if findings else 0


def verify_stage(stage):
    receipt = json.loads((stage / 'image-scan.json').read_text())
    if receipt.get('schema_version') != 1 or receipt.get('status') != 'passed' or \
            receipt.get('threshold') != sorted(BLOCKED) or receipt.get('blocking') != []:
        raise ValueError('publication requires a passing final image vulnerability scan')
    if sha(stage / 'image.tar') != receipt['archive_sha256'] or sha(stage / 'trivy.json') != receipt['report_sha256']:
        raise ValueError('scanned image archive/report changed before publication')
    subject = layout_subject(stage / 'oci', receipt['subject']['digest'], receipt['platform'], receipt['source_commit'])
    if subject['subject'] != receipt['subject'] or subject['runnable'][0]['config'] != receipt['image_id'] or \
            evaluate(json.loads((stage / 'trivy.json').read_text()), receipt['image_id'], receipt['component']):
        raise ValueError('publication scan does not bind this exact staged subject')
    return receipt
