#!/usr/bin/env python3
"""Execute real APT success/tamper cases in pinned bases, never on host APT.

The local HTTP proxy retains distro authentication and changes only the tested
boundary. Failures must leave dpkg state unchanged; metadata failures must also
precede all .deb downloads. Logs and package inventories are plain CI artifacts.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import urllib.error
import urllib.parse
import urllib.request

from apt_inputs import INPUTS

ROOT = Path(__file__).resolve().parents[1]
# Establish that the unmodified snapshot works before testing rejection paths.
MODES = ('success', 'wrong-digest', 'malformed-signature', 'bad-signature', 'absent-signature',
         'missing-index', 'bad-index', 'bad-package', 'missing-archive')
METADATA_FAILURES = frozenset(('wrong-digest', 'malformed-signature', 'bad-signature',
                               'absent-signature', 'missing-index', 'bad-index'))


def mutate_signature(data, mode):
    before, signature = data.split(b'-----BEGIN PGP SIGNATURE-----', 1)
    if mode == 'absent-signature':
        return before.split(b'\n\n', 1)[1]
    lines = (before if mode == 'bad-signature' else signature).splitlines(keepends=True)
    for index, line in enumerate(lines):
        if mode == 'bad-signature' and line.startswith(b'Origin: '):
            lines[index] = line.replace(b'Origin: ', b'Origin: NyxID fixture ', 1)
            return b''.join(lines) + b'-----BEGIN PGP SIGNATURE-----' + signature
        if mode == 'malformed-signature' and len(line.strip()) > 50 and not line.startswith((b'Version:', b'Comment:')):
            lines[index] = (b'A' if line[:1] != b'A' else b'B') + line[1:]
            return before + b'-----BEGIN PGP SIGNATURE-----' + b''.join(lines)
    raise ValueError('official signature fixture boundary missing')


class Proxy(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        parsed = urllib.parse.urlsplit(self.path)
        if (parsed.hostname not in {'snapshot.debian.org', 'snapshot.ubuntu.com'} or
                parsed.scheme != 'http' or parsed.query or parsed.fragment):
            self.send_error(403)
            return
        state = self.server.state
        state['requests'].append(parsed.path)
        index = ('/dists/' + state['suite'] + '/main/binary-') in parsed.path
        if state['mode'] == 'missing-index' and index:
            state['injected'] += 1
            self.send_error(404)
            return
        try:
            cache = self.server.cache / hashlib.sha256(self.path.encode()).hexdigest()
            with self.server.cache_lock:
                cache_lock = self.server.cache_locks.setdefault(str(cache), threading.Lock())
            with cache_lock:
                if not cache.exists():
                    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                    with opener.open(urllib.parse.urlunsplit(parsed._replace(scheme='https')), timeout=120) as response:
                        data = response.read(128 * 1024 * 1024 + 1)
                    if len(data) > 128 * 1024 * 1024:
                        raise ValueError('fixture response exceeds bound')
                    cache.write_bytes(data)
                data = cache.read_bytes()
            if parsed.path.endswith('/dists/' + state['suite'] + '/InRelease') and state['mode'] in (
                    'malformed-signature', 'bad-signature', 'absent-signature'):
                data = mutate_signature(data, state['mode'])
                state['injected'] += 1
            if ((state['mode'] == 'bad-index' and index) or
                    (state['mode'] == 'bad-package' and parsed.path.endswith('.deb'))):
                data += b'NyxID checksum-negative fixture'
                state['injected'] += 1
        except urllib.error.HTTPError as error:
            if error.code != 404:  # APT may probe optional index encodings.
                state['errors'].append('upstream HTTP ' + str(error.code) + ': ' + self.path)
            self.send_error(error.code)
            return
        except Exception as error:
            state['errors'].append(type(error).__name__ + ': ' + str(error))
            self.send_error(502)
            return
        self.send_response(200)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        try:
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass  # APT may stop reading a rejected response early.


def validate_case(mode, state, log, output):
    debs = [path for path in state['requests'] if path.endswith('.deb')]
    if state['errors']:
        raise ValueError('upstream proxy transport failed: ' + repr(state['errors']))
    if not state['requests']:
        raise ValueError('fixture boundary not reached: no upstream requests')
    if mode in METADATA_FAILURES and debs:
        raise ValueError('package download preceded metadata rejection')
    if mode not in ('success', 'wrong-digest', 'missing-archive') and not state['injected']:
        reason = ('unmodified signed metadata failed catalog digest verification' if
                  'signed snapshot bytes differ from the reviewed catalog' in log else
                  'APT stopped before the selected boundary; inspect command.log and requests.txt')
        raise ValueError('selected injection did not execute (' + mode + '): ' + reason)
    if mode == 'wrong-digest' and 'signed snapshot bytes differ from the reviewed catalog' not in log:
        raise ValueError('catalog digest rejection did not execute')
    if mode == 'bad-signature':
        diagnostic = log.lower()
        if not ('badsig' in diagnostic or 'bad signature' in diagnostic or
                ('verifying signature:' in diagnostic and 'message has been manipulated' in diagnostic)):
            raise ValueError('APT cryptographic bad-signature diagnostic missing')
        if 'signed snapshot bytes differ from the reviewed catalog' in diagnostic:
            raise ValueError('catalog check ran before signature rejection')
    if mode in ('bad-index', 'bad-package') and not any(value in log.lower() for value in (
            'hash sum mismatch', 'unexpected size', 'filesize', 'hashes of expected file')):
        raise ValueError('APT checksum/size rejection diagnostic missing')
    if mode == 'missing-archive' and not (output / 'missing-archive-injected').is_file():
        raise ValueError('offline install missing-archive boundary not reached')
    if mode in ('bad-package', 'missing-archive') and not debs:
        raise ValueError('package archive boundary not reached')


def run(args):
    args.evidence.mkdir(parents=True, exist_ok=False)
    inputs = args.evidence / 'inputs'
    inputs.mkdir()
    for name in INPUTS:
        shutil.copyfile(ROOT / name, inputs / Path(name).name)
    catalog = inputs / 'apt-snapshots.tsv'
    wrong = []
    for line in catalog.read_text().splitlines():
        fields = line.split('\t')
        if len(fields) == 6 and fields[2].endswith('-security'):
            fields[-1] = '0' * 64
        wrong.append('\t'.join(fields))
    (inputs / 'wrong-digest.tsv').write_text('\n'.join(wrong) + '\n')
    pins = json.loads((ROOT / 'security/build-input-pins.json').read_text())['images']
    images = {item['image']: item['image'] + '@' + item['digest'] for item in pins}
    bases = {'bookworm': ('debian', images['debian:bookworm-slim']),
             'trixie': ('debian', images['debian:trixie-slim']),
             'focal': ('ubuntu', images['buildpack-deps:focal']),
             'noble': ('ubuntu', images['ubuntu:24.04'])}
    count = 0
    with tempfile.TemporaryDirectory(prefix='nyxid-apt-http-') as temporary:
        server = ThreadingHTTPServer(('0.0.0.0', 0), Proxy)
        server.cache = Path(temporary)
        server.cache_lock = threading.Lock()
        server.cache_locks = {}
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            for codename in args.base or bases:
                distro, image = bases[codename]
                for mode in args.mode or MODES:
                    label = codename + '-' + mode
                    output = args.evidence / label
                    output.mkdir()
                    state = {'mode': mode, 'suite': codename + '-security', 'requests': [], 'injected': 0, 'errors': []}
                    server.state = state
                    selected = 'wrong-digest.tsv' if mode == 'wrong-digest' else 'apt-snapshots.tsv'
                    script = r'''
set -eu
[ "$(dpkg --print-architecture)" = "$1" ]
dpkg-query -W -f='${Package}\t${Version}\t${Architecture}\t${db:Status-Status}\n' | LC_ALL=C sort > /out/before.tsv
apt-get --version | head -n 1 > /out/apt-version.txt
mkdir /fixture-bin
cat > /fixture-bin/apt-get <<'WRAPPER'
#!/bin/sh
# Test-only forwarding proxy exposes bytes for signature/hash fault injection.
# Production installer writes HTTPS; only this disposable wrapper rewrites the
# isolated fixture sources, without changing signature/hash enforcement.
for option in "$@"; do
  case "$option" in
    Dir::Etc::sourcelist=*) sed -i 's#https://#http://#g' "${option#*=}" ;;
  esac
done
if [ "$NYXID_APT_TEST_MODE" = missing-archive ]; then
  case " $* " in
    *' --no-download '*)
      for option in "$@"; do
        case "$option" in Dir::Cache::archives=*) archives=${option#*=} ;; esac
      done
      for archive in "$archives/"*.deb; do
        [ -f "$archive" ] || continue
        rm "$archive"
        touch /out/missing-archive-injected
        break
      done
      ;;
  esac
fi
exec /usr/bin/apt-get "$@"
WRAPPER
chmod 0755 /fixture-bin/apt-get
export PATH=/fixture-bin:$PATH
result=0
sh /inputs/apt-snapshot-install.sh "/inputs/$2" "$3" -- libdbus-1-dev || result=$?
dpkg-query -W -f='${Package}\t${Version}\t${Architecture}\t${db:Status-Status}\n' | LC_ALL=C sort > /out/after.tsv
if [ "$NYXID_APT_TEST_MODE" = success ]; then
  [ "$result" = 0 ]
  [ "$(dpkg-query -W -f='${db:Status-Status}' libdbus-1-dev)" = installed ]
  find /usr/share/nyxid/build-inputs/apt -type f -exec sha256sum '{}' + > /out/first-records.sha256
  sh /inputs/apt-snapshot-install.sh "/inputs/$2" "$3" -- libdbus-1-dev
  sha256sum -c /out/first-records.sha256
  sh /inputs/apt-snapshot-install.sh --install-only "/inputs/$2" "$3" -- libdbus-1-dev
  sha256sum -c /out/first-records.sha256
  cp -r /usr/share/nyxid/build-inputs /out/
else
  [ "$result" != 0 ]
  cmp /out/before.tsv /out/after.tsv
fi
'''
                    container = 'nyxid-apt-' + label
                    command = ['docker', 'run', '--rm', '--name', container, '--platform', args.platform,
                               '--add-host=nyxid-apt-fixture:host-gateway',
                               '-e', 'http_proxy=http://nyxid-apt-fixture:' + str(server.server_port),
                               '-e', 'NYXID_APT_TEST_MODE=' + mode,
                               '-v', str(inputs.resolve()) + ':/inputs:ro', '-v', str(output.resolve()) + ':/out',
                               image, 'sh', '-c', script, 'test', args.platform.split('/')[1], selected, distro + ':' + codename]
                    log = output / 'command.log'
                    try:
                        with log.open('w') as stream:
                            result = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT, timeout=1200)
                        (output / 'requests.txt').write_text('\n'.join(state['requests']) + '\n')
                        (output / 'proxy-state.json').write_text(json.dumps(state, indent=2) + '\n')
                        if state['errors']:
                            raise ValueError('upstream proxy transport failed: ' + repr(state['errors']))
                        if result.returncode:
                            raise ValueError('container assertions failed (' + mode + '); inspect command.log')
                        validate_case(mode, state, log.read_text(), output)
                    except Exception:
                        print(log.read_text()[-16000:], flush=True)
                        raise
                    finally:
                        subprocess.run(['docker', 'rm', '-f', container], capture_output=True)
                    count += 1
                    print('PASS ' + label + ' (' + args.platform + ')', flush=True)
        finally:
            server.shutdown()
            server.server_close()
    print(f'APT snapshot boundaries: {count} passed, 0 failed', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--platform', choices=('linux/amd64', 'linux/arm64'), required=True)
    parser.add_argument('--base', choices=('focal', 'bookworm', 'trixie', 'noble'), action='append')
    parser.add_argument('--mode', choices=MODES, action='append')
    run(parser.parse_args())
