#!/usr/bin/env python3
"""Fail with diagnostics when the native amd64 acceptance host is unsuitable."""
import argparse
import ctypes
import json
import os
from pathlib import Path
import platform
import shutil
import stat
import subprocess

COMMANDS = ('node', 'python3', 'unshare', 'mount', 'certutil', 'openssl', 'cc', 'c++', 'make', 'cargo', 'rustc', 'npm',
            'Xvfb', 'xauth', 'openbox', 'dbus-run-session', 'bwrap', 'git', 'setpriv', 'runuser')
POLICY_DIRECTORIES = ('/etc/opt/chrome/policies/managed', '/etc/chromium/policies/managed',
                      '/etc/chromium-browser/policies/managed')


def managed_directories(policy_directory):
    if policy_directory not in POLICY_DIRECTORIES:
        raise ValueError('unknown native managed policy directory')
    native_hosts = ('/etc/opt/chrome/native-messaging-hosts' if policy_directory == POLICY_DIRECTORIES[0]
                    else '/etc/chromium/native-messaging-hosts')
    directories = {Path('/')}
    for leaf in ('/opt/nyxid/machine-browser/dev-policies', policy_directory, native_hosts):
        path = Path(leaf)
        directories.update((path, *path.parents))
    return sorted(directories, key=lambda path: (len(path.parts), str(path)))


def diagnose_managed_directories(policy_directory, root=Path('/')):
    print(f'Native managed browser identity uid={os.getuid()} euid={os.geteuid()} '
          f'gid={os.getgid()} egid={os.getegid()} groups={os.getgroups()}', flush=True)
    for directory in managed_directories(policy_directory):
        path = root / directory.relative_to('/')
        try:
            metadata = path.lstat()
            print(f'Native managed directory path={directory} uid={metadata.st_uid} gid={metadata.st_gid} '
                  f'mode={stat.S_IMODE(metadata.st_mode):04o} type={stat.filemode(metadata.st_mode)[0]}', flush=True)
        except OSError as error:
            print(f'Native managed directory path={directory} error={error.strerror}', flush=True)


def check_managed_directories(policy_directory, root=Path('/'), owner=0):
    for directory in managed_directories(policy_directory):
        metadata = (root / directory.relative_to('/')).lstat()
        if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != owner or metadata.st_mode & 0o022:
            raise ValueError(f'unsafe native managed directory: {directory}; '
                             'requires supervisor ownership, no symlinks and no group/other write access')


def prepare_managed_directories(policy_directory, root=Path('/'), owner=0, group=0):
    """Prepare only named fixture ancestors; never recurse or follow symlinks.

    The disposable hosted image deliberately chmods /opt recursively to 0777.
    Production must reject it. Preparation happens before any fixture users or
    credentials exist; / and /etc must already be protected.
    """
    for directory in managed_directories(policy_directory):
        path = root / directory.relative_to('/')
        if directory not in (Path('/'), Path('/etc')):
            try:
                path.mkdir(mode=0o755)
            except FileExistsError:
                pass
        # O_NOFOLLOW also rejects a pre-existing symlink to a real directory.
        fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            metadata = os.fstat(fd)
            if directory in (Path('/'), Path('/etc')):
                if metadata.st_uid != owner or metadata.st_mode & 0o022:
                    raise ValueError(f'unsafe system ancestor: {directory}')
            else:
                if (metadata.st_uid, metadata.st_gid) != (owner, group):
                    os.fchown(fd, owner, group)
                os.fchmod(fd, 0o755)
        finally:
            os.close(fd)
    check_managed_directories(policy_directory, root, owner)


def require_platform(system, machine, uid, abi, apparmor):
    if system != 'Linux' or machine != 'x86_64' or uid != 0:
        raise ValueError('native acceptance requires root on Linux amd64')
    if abi < 6:
        raise ValueError('native acceptance requires Landlock ABI >= 6; reported ' + str(abi))
    if apparmor != '1':
        raise ValueError('native acceptance requires the real Ubuntu AppArmor userns restriction enabled')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare-disposable-host', action='store_true',
                        help='normalize named managed-browser ancestors on an expendable root Linux amd64 runner')
    parser.add_argument('--diagnose-managed-browser', action='store_true', help='print directory metadata only')
    parser.add_argument('--policy-directory', choices=POLICY_DIRECTORIES, default=POLICY_DIRECTORIES[0])
    args = parser.parse_args()
    if args.diagnose_managed_browser:
        if args.prepare_disposable_host:
            parser.error('diagnostics cannot prepare the host')
        diagnose_managed_directories(args.policy_directory)
        return
    print('Native machine acceptance prerequisites: ' + platform.platform(), flush=True)
    if platform.system() != 'Linux' or platform.machine() != 'x86_64' or os.geteuid() != 0:
        require_platform(platform.system(), platform.machine(), os.geteuid(), -1, '')
    for name in ('/sys/kernel/security/lsm', '/sys/module/apparmor/parameters/enabled',
                 '/proc/sys/kernel/apparmor_restrict_unprivileged_userns',
                 '/proc/sys/kernel/unprivileged_userns_clone', '/proc/sys/user/max_user_namespaces'):
        path = Path(name)
        print(name + '=' + (path.read_text().strip() if path.is_file() else '<unavailable>'), flush=True)
    libc = ctypes.CDLL(None, use_errno=True)
    # landlock_create_ruleset(NULL, 0, LANDLOCK_CREATE_RULESET_VERSION), x86_64.
    abi = libc.syscall(ctypes.c_long(444), ctypes.c_void_p(), ctypes.c_size_t(0), ctypes.c_uint(1))
    print(f'Landlock ABI={abi} errno={ctypes.get_errno()}', flush=True)
    restriction = Path('/proc/sys/kernel/apparmor_restrict_unprivileged_userns')
    apparmor = restriction.read_text().strip() if restriction.is_file() else '<unavailable>'
    require_platform(platform.system(), platform.machine(), os.geteuid(), abi, apparmor)
    for command in COMMANDS:
        executable = shutil.which(command)
        if not executable:
            raise ValueError('missing mandatory native prerequisite: ' + command)
        print(command + '=' + executable, flush=True)
    # Share the harness's explicit selection and resolution diagnostics. Hosted
    # native acceptance must exercise Chrome's deb policy directory even when
    # an Ubuntu Chromium transitional shim is also installed.
    browser = select_browser()
    policy_directory = browser['policyDirectory']
    print('Native managed browser ancestry before preparation/check:', flush=True)
    diagnose_managed_directories(policy_directory)
    try:
        if args.prepare_disposable_host:
            prepare_managed_directories(policy_directory)
            print('Native disposable managed browser ancestry prepared (root:root 0755):', flush=True)
            diagnose_managed_directories(policy_directory)
        check_managed_directories(policy_directory)
    except (OSError, ValueError):
        print('Native managed browser ancestry failure:', flush=True)
        diagnose_managed_directories(policy_directory)
        raise
    subprocess.run([browser['browserBinary'], '--version'], check=True)
    subprocess.run(['node', '-e', 'require("ws")'], check=True)
    subprocess.run(['/usr/bin/cargo', '--version'], check=True)
    subprocess.run(['/usr/bin/rustc', '--version'], check=True)
    print('Native amd64 prerequisites passed; managed ancestry verified; sandbox policies unchanged.', flush=True)


def select_browser():
    selector = Path(__file__).resolve().parents[1] / 'cli/tests/machine_native_browser.mjs'
    result = subprocess.run(['node', str(selector), '--require-google-chrome'],
                            text=True, capture_output=True)
    print(result.stderr, end='', flush=True)
    if result.returncode:
        raise ValueError('native browser prerequisite failed; see candidate resolutions above')
    return json.loads(result.stdout)


if __name__ == '__main__':
    main()
