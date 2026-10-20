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
from apk_archive_release import stage_payload, validate_payload
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
if [ -x /case/mocks/apk ]; then
  NYXID_REAL_APK=$(command -v apk)
  export NYXID_REAL_APK PATH=/case/mocks:$PATH
fi
set +e
sh /input/apk-input-install.sh "$1" /case/catalog.tsv "$2" /case/archives
status=$?
set -e
cp /lib/apk/db/installed /case/after.db
if [ "$3" = success ]; then
  [ "$status" = 0 ]
  cp -R /usr/share/nyxid/build-inputs/apk /case/records
elif [ "$3" = download-success ]; then
  [ "$status" = 0 ]
  cmp /case/before.db /case/after.db
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
    if (folder / 'source-archives').is_dir():
        command += ['-v', str(folder / 'source-archives') + ':/source-archives:ro',
                    '-e', 'PATH=/case/mocks:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin']
    if (folder / 'archives').is_dir():
        command += ['-v', str(folder / 'archives') + ':/case/archives:ro']
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
        for line in (stage / 'download.log').read_text().splitlines():
            if line.startswith('::warning::APK'):
                print(line, flush=True)
        stage_payload(ROOT / 'security/apk' / (label + '-' + args.arch + '.tsv'), args.arch, download / 'archives', args.output / 'payload')
        rows = [line.split('\t') for line in text.splitlines() if not line.startswith('#')]
        target = next(row for row in rows if row[0] == ('nodejs' if label == 'updater-e2e' else 'python3'))
        cases = ['success', 'wrong-base', 'wrong-arch', 'duplicate-header', 'unsafe-url', 'wrong-before',
                 'wrong-checksum', 'corrupt-archive', 'bad-signature', 'missing-signature',
                 'missing-archive', 'incomplete-closure', 'signed-identity', 'wrong-after', 'post-install-drift',
                 'fallback-success', 'fallback-bad-signature', 'fallback-signed-identity']
        for case in cases:
            folder = stage / case
            folder.mkdir()
            candidate = text
            variant = case.removeprefix('fallback-')
            metadata = case in {'wrong-base', 'wrong-arch', 'duplicate-header', 'unsafe-url', 'wrong-before'}
            if not metadata:
                shutil.copytree(download / 'archives', folder / 'archives', copy_function=os.link)
            archive = folder / 'archives' / (target[0] + '-' + target[1] + '.apk')
            if case == 'wrong-base': candidate = candidate.replace(base, base.replace('@sha256:', '@sha256:0'), 1)
            if case == 'wrong-arch': candidate = candidate.replace('# arch ' + ARCHES[args.arch], '# arch noarch')
            if case == 'duplicate-header': candidate = candidate.replace('# arch ', '# arch ' + ARCHES[args.arch] + '\n# arch ', 1)
            if case == 'unsafe-url': candidate = candidate.replace('https://dl-cdn.', 'http://dl-cdn.', 1)
            if case == 'wrong-before':
                candidate = '\n'.join('# before-sha256 ' + '0' * 64 if line.startswith('# before-sha256 ') else line for line in candidate.splitlines()) + '\n'
            if case == 'wrong-after':
                candidate = '\n'.join('# after-sha256 ' + '0' * 64 if line.startswith('# after-sha256 ') else line for line in candidate.splitlines()) + '\n'
            if variant == 'signed-identity':
                wrong = list(target)
                wrong[2] = 'noarch'
                wrong[4] = wrong[4].replace('-' + target[2] + '-' + target[0] + '-', '-noarch-' + target[0] + '-')
                candidate = candidate.replace('\t'.join(target), '\t'.join(wrong), 1)
            if case == 'wrong-checksum': candidate = candidate.replace(target[3], '0' * 64)
            if case == 'corrupt-archive':
                data = archive.read_bytes() + b'corrupted'
                archive.unlink()
                archive.write_bytes(data)
            if variant in {'bad-signature', 'missing-signature'}:
                data = signature_variant(archive.read_bytes(), missing=variant == 'missing-signature')
                archive.unlink()
                archive.write_bytes(data)
                candidate = candidate.replace(target[3], digest(data))
            if case == 'missing-archive': archive.unlink()
            if case == 'incomplete-closure': candidate = candidate.replace('\t'.join(target) + '\n', '')
            if case == 'success':
                # Unreviewed files must never be globbed into installation.
                (folder / 'archives/unreviewed.apk').write_bytes(b'not a package')
            if case == 'post-install-drift':
                mocks = folder / 'mocks'
                mocks.mkdir()
                wrapper = mocks / 'apk'
                wrapper.write_text('''#!/bin/sh
set -eu
"$NYXID_REAL_APK" "$@"
case " $* " in
  *' --root '*) : ;;
  *' add '*) printf '\\nP:nyxid-post-install-drift\\nV:1-r0\\nA:noarch\\n\\n' >> /lib/apk/db/installed ;;
esac
''')
                wrapper.chmod(0o755)
            if case.startswith('fallback-'):
                (folder / 'archives').rename(folder / 'source-archives')
                mocks = folder / 'mocks'
                mocks.mkdir()
                wget = mocks / 'wget'
                wget.write_text('#!/bin/sh\nset -eu\nwhile [ "$#" -gt 1 ]; do\n  case "$1" in -O) destination=$2; shift 2 ;; -T) shift 2 ;; *) shift ;; esac\ndone\nprintf \'%s\\n\' "$1" >> /case/fetches\ncase "$1" in\n  https://github.com/*) printf \'  HTTP/1.1 404 Not Found\\n\' >&2; exit 8 ;;\n  https://dl-cdn.alpinelinux.org/*) cp "/source-archives/${1##*/}" "$destination" ;;\n  *) exit 99 ;;\nesac\n')
                wget.chmod(0o755)
            (folder / 'catalog.tsv').write_text(candidate)
            invoke(base, args.arch, folder, '--download' if metadata or case.startswith('fallback-') else '--install',
                   'success' if case == 'success' else ('download-success' if case == 'fallback-success' else ('changed-failure' if case == 'post-install-drift' else 'failure')), stage / (case + '.log'), 'offline')
            reasons = {
                'wrong-base': 'base image mismatch', 'wrong-arch': 'architecture mismatch',
                'duplicate-header': 'duplicate header', 'unsafe-url': 'invalid archive URL',
                'wrong-before': 'base inventory mismatch', 'wrong-checksum': 'archive checksum mismatch',
                'corrupt-archive': 'archive checksum mismatch',
                'bad-signature': 'archive signature verification failed',
                'missing-signature': 'archive signature verification failed',
                'missing-archive': 'archive missing', 'incomplete-closure': 'closure preflight',
                'signed-identity': 'signed identity mismatch', 'wrong-after': 'closure preflight inventory mismatch',
                'post-install-drift': 'final inventory mismatch',
            }
            if variant in reasons and reasons[variant] not in (stage / (case + '.log')).read_text():
                raise ValueError('case did not reach intended boundary: ' + case)
            if case.startswith('fallback-'):
                if '::warning::APK durable archives missing from build-inputs-apk-' not in (stage / (case + '.log')).read_text():
                    raise ValueError('fallback did not emit a release warning')
                shutil.rmtree(folder / 'source-archives')
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
    tag, files = validate_payload(ROOT / 'security/apk', args.output / 'payload', [args.arch])
    print(f'PASS durable release payload rehearsal: {tag}, {len(files)} files ({args.arch})', flush=True)
    print(f'APK closure replay: {count} passed, 0 failed', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--arch', choices=tuple(ARCHES), required=True)
    parser.add_argument('--output', type=Path, required=True)
    run(parser.parse_args())
