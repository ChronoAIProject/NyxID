#!/usr/bin/env python3
"""Real root-run APT regression with transport inputs below a private parent.

Acquire the signed noble snapshots with the real _apt sandbox. Stop at the
update boundary, before the installer can write receipts or install packages.
This uses only isolated lists/cache and never changes the host package state.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
STOP_AFTER_UPDATE = 73


def run():
    if sys.platform != 'linux' or os.geteuid() != 0:
        raise SystemExit('Run this regression as root on Linux with apt and the Ubuntu archive keyring')
    if not shutil.which('runuser') or not Path('/usr/bin/apt-get').is_file():
        raise SystemExit('Real apt-get and runuser are required')
    status = Path('/var/lib/dpkg/status')
    before = status.read_bytes()
    os.umask(0o022)
    with tempfile.TemporaryDirectory(prefix='nyxid-apt-sandbox-', dir='/tmp') as temporary:
        work = Path(temporary)
        work.chmod(0o755)
        private = work / 'private-inputs'
        private.mkdir(mode=0o700)
        for name in ('apt-snapshot-install.sh', 'apt-snapshot-select.awk', 'apt-transport-ca.awk'):
            shutil.copyfile(ROOT / 'scripts' / name, private / name)
        for name in ('apt-snapshots.tsv', 'apt-transport-ca.pem', 'apt-transport-ca.tsv'):
            shutil.copyfile(ROOT / 'security' / name, private / name)
        ca = private / 'apt-transport-ca.pem'
        denied = subprocess.run(['runuser', '-u', '_apt', '--', 'cat', str(ca)], capture_output=True)
        if denied.returncode == 0 or b'Permission denied' not in denied.stderr:
            raise ValueError('Control failed: _apt must not be able to traverse the original input directory')
        tools = work / 'bin'
        tools.mkdir()
        wrapper = tools / 'apt-get'
        wrapper.write_text('''#!/usr/bin/python3
import hashlib, os, subprocess, sys
from pathlib import Path
args = sys.argv[1:]
assert args[-1] == 'update', 'Regression must stop before any package operation'
options = dict(args[i + 1].split('=', 1) for i, arg in enumerate(args) if arg == '-o')
assert options['Acquire::https::Verify-Peer'] == 'true'
assert options['Acquire::https::Verify-Host'] == 'true'
assert options['APT::Get::AllowUnauthenticated'] == 'false'
assert options['Acquire::AllowInsecureRepositories'] == 'false'
ca = Path(options['Acquire::https::CaInfo'])
assert ca != Path(os.environ['NYXID_APT_ORIGINAL_CA'])
assert ca == Path(options['Dir::State::lists']).parent / 'transport-ca.pem'
assert ca.stat().st_mode & 0o777 == 0o644
assert ca.parent.stat().st_mode & 0o777 == 0o755
subprocess.run(['runuser', '-u', '_apt', '--', 'test', '-r', str(ca)], check=True)
assert ca.read_bytes() == Path(os.environ['NYXID_APT_ORIGINAL_CA']).read_bytes()
result = subprocess.run(['/usr/bin/apt-get', '-o', 'APT::Sandbox::User=_apt', *args],
                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
sys.stdout.buffer.write(result.stdout)
sys.stdout.buffer.flush()
if result.returncode:
    sys.exit(result.returncode)
assert b'unsandboxed' not in result.stdout.lower(), 'APT must actually use its acquisition sandbox'
lists = Path(options['Dir::State::lists'])
for line in Path(os.environ['NYXID_APT_CATALOG']).read_text().splitlines():
    if line.startswith('#'):
        continue
    distro, codename, suite, components, url, digest = line.split('\\t')
    if (distro, codename) != ('ubuntu', 'noble'):
        continue
    prefix = (url.removeprefix('https://') + '/dists/' + suite + '/').replace('/', '_')
    assert hashlib.sha256((lists / (prefix + 'InRelease')).read_bytes()).hexdigest() == digest
    for component in components.split(','):
        matches = list(lists.glob(prefix + component + '_binary-' + os.environ['NYXID_APT_ARCH'] + '_Packages*'))
        assert len(matches) == 1, 'Missing authenticated native package index'
Path(os.environ['NYXID_APT_SANDBOX_MARKER']).write_text('authenticated sandbox update passed\\n')
sys.exit(73)
''')
        wrapper.chmod(0o755)
        marker = work / 'sandbox-passed'
        arch = subprocess.check_output(['dpkg', '--print-architecture'], text=True).strip()
        result = subprocess.run(
            ['sh', str(private / 'apt-snapshot-install.sh'), '--install-only',
             str(private / 'apt-snapshots.tsv'), 'ubuntu:noble', '--', 'libdbus-1-dev'],
            env={**os.environ, 'PATH': str(tools) + os.pathsep + os.environ['PATH'],
                 'NYXID_APT_ORIGINAL_CA': str(ca), 'NYXID_APT_CATALOG': str(private / 'apt-snapshots.tsv'),
                 'NYXID_APT_ARCH': arch, 'NYXID_APT_SANDBOX_MARKER': str(marker)},
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=600)
        sys.stdout.buffer.write(result.stdout)
        sys.stdout.buffer.flush()
        if result.returncode != STOP_AFTER_UPDATE or not marker.is_file():
            raise ValueError('Real APT sandbox regression failed before its authenticated update boundary')
        if status.read_bytes() != before:
            raise ValueError('Regression unexpectedly changed the host package database')
    print('APT private-input sandbox: 1 passed, 0 failed (real root/_apt acquisition, no package install)')


if __name__ == '__main__':
    run()
