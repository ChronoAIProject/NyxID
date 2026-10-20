"""Prepare and validate repository-owned APK release payloads; never resolve packages."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
from urllib.parse import urlsplit

from apk_inputs import ARCHES, catalog
from resolve_apk_requests import REQUESTS

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    digest = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def entries(path, arch):
    text = path.read_text()
    base = next(line.split()[2] for line in text.splitlines() if line.startswith('# base-image '))
    rows = [line.split('\t') for line in catalog(ROOT, path, base, ARCHES[arch]).splitlines()]
    return base, rows


def stage_payload(catalog_path, arch, archives, output):
    _, rows = entries(catalog_path, arch)
    output.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(catalog_path, output / catalog_path.name)
    for name, version, _, expected, durable, _ in rows:
        source = archives / (name + '-' + version + '.apk')
        if source.is_symlink() or sha(source) != expected:
            raise ValueError('payload archive checksum mismatch: ' + str(source))
        destination = output / Path(urlsplit(durable).path).name
        if destination.exists():
            if destination.is_symlink() or sha(destination) != expected:
                raise ValueError('conflicting archive payload')
        else:
            shutil.copyfile(source, destination)


def expected_assets(catalogs, arches):
    expected, dates = {}, set()
    for label in REQUESTS:
        for arch in arches:
            path = catalogs / (label + '-' + arch + '.tsv')
            _, rows = entries(path, arch)
            expected[path.name] = sha(path)
            for _, _, _, checksum, durable, _ in rows:
                url = urlsplit(durable)
                dates.add(url.path.split('/')[-2])
                asset = Path(url.path).name
                if asset in expected and expected[asset] != checksum:
                    raise ValueError('conflicting archive checksum')
                expected[asset] = checksum
    if len(dates) != 1:
        raise ValueError('release payload requires one catalog refresh date')
    return dates.pop(), expected


def validate_payload(catalogs, payload, arches):
    tag, expected = expected_assets(catalogs, arches)
    actual = {p.name for p in payload.iterdir()}
    if actual != set(expected):
        raise ValueError('release payload has missing or unexpected files')
    for name, checksum in expected.items():
        path = payload / name
        if path.is_symlink() or not path.is_file() or sha(path) != checksum:
            raise ValueError('release payload checksum mismatch: ' + name)
    return tag, sorted(expected)


def prepare(root, arch, output):
    output.mkdir(parents=True, exist_ok=False)
    payload = output / 'payload'
    for label in REQUESTS:
        path = root / 'security/apk' / (label + '-' + arch + '.tsv')
        base, _ = entries(path, arch)
        work = output / label
        work.mkdir()
        command = ['docker', 'run', '--rm', '--user=0', '--platform=linux/' + arch,
                   '--memory=1536m', '--cpus=2', '--entrypoint=sh',
                   '-v', str(ROOT / 'scripts') + ':/input:ro',
                   '-v', str(path.resolve()) + ':/catalog.tsv:ro', '-v', str(work.resolve()) + ':/out',
                   base, '-ec', 'sh /input/apk-input-install.sh --download /catalog.tsv "$1" /out/archives provision-upstream; chown -R "$2:$3" /out/archives',
                   'prepare', base, str(os.getuid()), str(os.getgid())]
        with (output / (label + '.log')).open('w') as log:
            subprocess.run(command, check=True, stdout=log, stderr=subprocess.STDOUT, timeout=900)
        stage_payload(path, arch, work / 'archives', payload)
        shutil.rmtree(work)
    tag, files = validate_payload(root / 'security/apk', payload, [arch])
    print(f'Verified payload rehearsal: {tag}, {len(files)} files ({arch})', flush=True)
