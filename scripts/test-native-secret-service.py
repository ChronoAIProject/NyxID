#!/usr/bin/env python3
"""Run the real CLI keychain test in a disposable, unlocked Secret Service.

Missing prerequisites, a deadlock, or any failed set/get/delete is fatal. No
existing desktop keychain, D-Bus session or real credentials enter this test.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

TEST = 'commands::node::docker_entrypoint::tests::linux_keychain_roundtrip_inside_cli_tokio_runtime'


def main():
    for command in ('cargo', 'dbus-run-session', 'dbus-send', 'gnome-keyring-daemon'):
        if not shutil.which(command):
            raise ValueError('mandatory native Secret Service test prerequisite missing: ' + command)
    build = subprocess.run(['cargo', 'test', '--locked', '-p', 'nyxid-cli', '--bin', 'nyxid',
                            '--no-run', '--message-format=json'], capture_output=True, text=True, check=True)
    print(build.stderr, end='', flush=True)
    binaries = [Path(message['executable']).resolve() for line in build.stdout.splitlines()
                if (message := json.loads(line)).get('reason') == 'compiler-artifact' and
                message.get('executable') and message['target']['name'] == 'nyxid' and
                message['profile'].get('test')]
    if len(binaries) != 1 or not binaries[0].is_file():
        raise ValueError('missing/ambiguous actual CLI test executable')
    listing = subprocess.check_output([str(binaries[0]), '--list'], text=True)
    if listing.splitlines().count(TEST + ': test') != 1:
        raise ValueError('mandatory real Secret Service test is absent from the CLI executable')
    with tempfile.TemporaryDirectory(prefix='nyxid-secret-service-') as temporary:
        root = Path(temporary)
        env = dict(os.environ)
        for key, directory in (('XDG_RUNTIME_DIR', 'runtime'), ('XDG_CONFIG_HOME', 'config'), ('XDG_DATA_HOME', 'data')):
            path = root / directory
            path.mkdir(mode=0o700)
            env[key] = str(path)
        env.pop('DBUS_SESSION_BUS_ADDRESS', None)
        script = r'''
set -eu
printf '\n' | gnome-keyring-daemon --foreground --unlock --components=secrets &
keyring_pid=$!
trap 'kill "$keyring_pid" 2>/dev/null || true; wait "$keyring_pid" 2>/dev/null || true' EXIT
ready=0
for attempt in $(seq 1 100); do
  if dbus-send --session --print-reply --dest=org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus.NameHasOwner string:org.freedesktop.secrets | grep -q 'boolean true'; then
    ready=1
    break
  fi
  sleep 0.1
done
[ "$ready" = 1 ] || { echo 'Secret Service did not acquire its session bus name' >&2; exit 1; }
"$1" "$2" --ignored --exact --nocapture
'''
        subprocess.run(['dbus-run-session', '--', 'sh', '-ec', script, 'keychain-test', str(binaries[0]), TEST],
                       env=env, check=True, timeout=90)


if __name__ == '__main__':
    main()
