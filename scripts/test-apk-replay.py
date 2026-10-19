#!/usr/bin/env python3
"""Native Linux success/tamper matrix for every reviewed APK closure.

Downloads occur once per stage. Every replay container has no network, an
unaltered digest-pinned base and no image publication capability. Negative
pre-install cases require byte-identical base installed-package state.
"""
import argparse
import gzip
import hashlib
import io
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import zlib

from apk_inputs import ARCHES
from resolve_apk_requests import REQUESTS
import importlib.util

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('apk_generator', ROOT / 'scripts/resolve-apk-candidates.py')
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def signature_variant(data, missing=False):
    # APK v2 consists of separate gzip members: signature, control and data.
    decoder = zlib.decompressobj(31)
    signature = decoder.decompress(data) + decoder.flush()
    remainder = decoder.unused_data
    with tarfile.open(fileobj=io.BytesIO(signature)) as archive:
        entry = next(member for member in archive if member.name.startswith('.SIGN.'))
    if missing:
        return remainder
    changed = bytearray(signature)
    changed[entry.offset_data] ^= 1
    return gzip.compress(bytes(changed), mtime=0) + remainder


def invoke(base, arch, folder, mode, expected, log, phase):
    # The wrapper records exact installed DB bytes even when admission fails.
    shell = r'''
set -eu
cp /lib/apk/db/installed /case/before.db
set +e
sh /input/apk-input-install.sh "$1" /case/catalog.tsv "$2" /case/archives
status=$?
set -e
cp /lib/apk/db/installed /case/after.db
if [ "$3" = success ]; then
  [ "$status" = 0 ]
  cp -R /usr/share/nyxid/build-inputs/apk /case/records
else
  [ "$status" != 0 ]
  if [ "$3" = changed-failure ]; then
    ! cmp -s /case/before.db /case/after.db
    [ ! -d /usr/share/nyxid/build-inputs/apk ]
  else
    cmp /case/before.db /case/after.db
  fi
fi
'''
    command = ['docker', 'run', '--rm', '--user', '0', '--platform', 'linux/' + arch,
               '--memory=1536m', '--cpus=2', '--entrypoint', 'sh',
               '-v', str(ROOT / 'scripts') + ':/input:ro', '-v', str(folder) + ':/case']
    if phase == 'offline':
        command += ['--network=none']
    command += [base, '-ec', shell, 'replay', mode, base, expected]
    with log.open('w') as output:
        result = subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, timeout=600)
    if result.returncode:
        raise ValueError('APK case failed: ' + str(log) + '\n' + log.read_text()[-10000:])


def run(args):
    args.output.mkdir(parents=True, exist_ok=False)
    count = 0
    for label in REQUESTS:
        base = generator.base_image(label)
        text = (ROOT / 'security/apk' / (label + '-' + args.arch + '.tsv')).read_text()
        stage = args.output / label
        stage.mkdir()
        download = stage / 'download'
        download.mkdir()
        (download / 'catalog.tsv').write_text(text)
        # Download mode succeeds without changing installed packages; it has
        # no installed record yet, so use a dedicated download command.
        command = ['docker', 'run', '--rm', '--user', '0', '--platform', 'linux/' + args.arch,
                   '--memory=1536m', '--cpus=2', '--entrypoint', 'sh',
                   '-v', str(ROOT / 'scripts') + ':/input:ro', '-v', str(download) + ':/case',
                   base, '-ec',
                   'sh /input/apk-input-install.sh --download /case/catalog.tsv "$1" /case/archives; chown -R "$2:$3" /case/archives',
                   'download', base, str(os.getuid()), str(os.getgid())]
        with (stage / 'download.log').open('w') as output:
            result = subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, timeout=600)
        if result.returncode:
            raise ValueError('APK download failed: ' + label + '\n' + (stage / 'download.log').read_text()[-10000:])
        rows = [line.split('\t') for line in text.splitlines() if not line.startswith('#')]
        target = next(row for row in rows if row[0] == ('nodejs' if label == 'updater-e2e' else 'python3'))
        cases = ['success', 'wrong-base', 'wrong-arch', 'duplicate-header', 'unsafe-url', 'wrong-before',
                 'wrong-checksum', 'corrupt-archive', 'bad-signature', 'missing-signature',
                 'missing-archive', 'incomplete-closure', 'signed-identity', 'wrong-after']
        for case in cases:
            folder = stage / case
            folder.mkdir()
            candidate = text
            metadata = case in {'wrong-base', 'wrong-arch', 'duplicate-header', 'unsafe-url', 'wrong-before'}
            if not metadata:
                shutil.copytree(download / 'archives', folder / 'archives', copy_function=os.link)
            archive = folder / 'archives' / (target[0] + '-' + target[1] + '.apk')
            if case == 'wrong-base': candidate = candidate.replace(base, base.replace('@sha256:', '@sha256:0'), 1)
            if case == 'wrong-arch': candidate = candidate.replace('# arch ' + ARCHES[args.arch], '# arch noarch')
            if case == 'duplicate-header': candidate = candidate.replace('# arch ', '# arch ' + ARCHES[args.arch] + '\n# arch ', 1)
            if case == 'unsafe-url': candidate = candidate.replace('https://', 'http://', 1)
            if case == 'wrong-before':
                candidate = '\n'.join('# before-sha256 ' + '0' * 64 if line.startswith('# before-sha256 ') else line for line in candidate.splitlines()) + '\n'
            if case == 'wrong-after':
                candidate = '\n'.join('# after-sha256 ' + '0' * 64 if line.startswith('# after-sha256 ') else line for line in candidate.splitlines()) + '\n'
            if case == 'signed-identity':
                wrong = list(target)
                wrong[2] = 'noarch'
                candidate = candidate.replace('\t'.join(target), '\t'.join(wrong), 1)
            if case == 'wrong-checksum': candidate = candidate.replace(target[3], '0' * 64, 1)
            if case == 'corrupt-archive':
                data = archive.read_bytes() + b'corrupted'
                archive.unlink()
                archive.write_bytes(data)
            if case in {'bad-signature', 'missing-signature'}:
                data = signature_variant(archive.read_bytes(), missing=case == 'missing-signature')
                archive.unlink()
                archive.write_bytes(data)
                candidate = candidate.replace(target[3], digest(data), 1)
            if case == 'missing-archive': archive.unlink()
            if case == 'incomplete-closure': candidate = candidate.replace('\t'.join(target) + '\n', '')
            if case == 'success':
                # Unreviewed files must never be globbed into installation.
                (folder / 'archives/unreviewed.apk').write_bytes(b'not a package')
            (folder / 'catalog.tsv').write_text(candidate)
            invoke(base, args.arch, folder, '--download' if metadata else '--install',
                   'success' if case == 'success' else ('changed-failure' if case == 'wrong-after' else 'failure'), stage / (case + '.log'), 'offline')
            reasons = {
                'wrong-base': 'base image mismatch', 'wrong-arch': 'architecture mismatch',
                'duplicate-header': 'duplicate header', 'unsafe-url': 'invalid archive URL',
                'wrong-before': 'base inventory mismatch', 'wrong-checksum': 'archive checksum mismatch',
                'corrupt-archive': 'archive checksum mismatch',
                'bad-signature': 'archive signature verification failed',
                'missing-signature': 'archive signature verification failed',
                'missing-archive': 'archive missing', 'incomplete-closure': 'closure preflight failed',
                'signed-identity': 'signed identity mismatch', 'wrong-after': 'final inventory mismatch',
            }
            if case in reasons and reasons[case] not in (stage / (case + '.log')).read_text():
                raise ValueError('case did not reach intended boundary: ' + case)
            if metadata and (folder / 'archives').exists():
                raise ValueError('invalid metadata passed admission: ' + case)
            if case == 'success':
                records = list((folder / 'records').iterdir())
                if len(records) != 1 or (records[0] / 'archives.tsv').read_text() != text:
                    raise ValueError('replay record not bound to exact catalog')
                for header, filename in [('before-sha256', 'base-installed.tsv'), ('after-sha256', 'installed.tsv')]:
                    expected = next(line.split()[2] for line in text.splitlines() if line.startswith('# ' + header))
                    if digest((records[0] / filename).read_bytes()) != expected:
                        raise ValueError('record inventory digest mismatch')
            if (folder / 'archives').exists():
                shutil.rmtree(folder / 'archives')
            print('PASS ' + label + ' ' + case + ' (' + args.arch + ')', flush=True)
            count += 1
        shutil.rmtree(download / 'archives')
    print(f'APK closure replay: {count} passed, 0 failed', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--arch', choices=tuple(ARCHES), required=True)
    parser.add_argument('--output', type=Path, required=True)
    run(parser.parse_args())
