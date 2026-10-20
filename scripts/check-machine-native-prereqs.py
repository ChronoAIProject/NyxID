#!/usr/bin/env python3
"""Fail with diagnostics when the native amd64 acceptance host is unsuitable."""
import ctypes
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess

COMMANDS = ('node', 'python3', 'unshare', 'mount', 'certutil', 'openssl', 'cc', 'c++', 'make', 'cargo', 'rustc', 'npm',
            'Xvfb', 'xauth', 'openbox', 'dbus-run-session', 'bwrap', 'git', 'setpriv', 'runuser')


def require_platform(system, machine, uid, abi, apparmor):
    if system != 'Linux' or machine != 'x86_64' or uid != 0:
        raise ValueError('native acceptance requires root on Linux amd64')
    if abi < 6:
        raise ValueError('native acceptance requires Landlock ABI >= 6; reported ' + str(abi))
    if apparmor != '1':
        raise ValueError('native acceptance requires the real Ubuntu AppArmor userns restriction enabled')


def main():
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
    subprocess.run([browser['browserBinary'], '--version'], check=True)
    subprocess.run(['node', '-e', 'require("ws")'], check=True)
    subprocess.run(['/usr/bin/cargo', '--version'], check=True)
    subprocess.run(['/usr/bin/rustc', '--version'], check=True)
    print('Native amd64 prerequisites passed without changing sandbox policies.', flush=True)


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
