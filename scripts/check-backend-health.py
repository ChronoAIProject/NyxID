#!/usr/bin/env python3
"""Exercise the final backend image's shell-free health check on a real server.

Uses the job's disposable MongoDB replica set and generated test-only keys.
The process always runs as the image's configured non-root user. Nothing is
published, and no repository or deployment credential is consumed.
"""
import argparse
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import time
from urllib.error import URLError
from urllib.request import urlopen
import uuid

BINARY = '/usr/local/bin/nyxid-server'


def run(command, **kwargs):
    return subprocess.run(command, check=True, capture_output=True, text=True, timeout=30, **kwargs)


def check(image, database_url):
    image_id = run(['docker', 'image', 'inspect', '--format', '{{.Id}}', image]).stdout.strip()
    name = 'nyxid-backend-health-' + uuid.uuid4().hex
    with tempfile.TemporaryDirectory(prefix='nyxid-backend-health-') as temporary:
        keys = Path(temporary) / 'keys'
        keys.mkdir(mode=0o755)
        run(['openssl', 'genpkey', '-algorithm', 'RSA', '-pkeyopt', 'rsa_keygen_bits:2048', '-out', str(keys / 'private.pem')])
        run(['openssl', 'pkey', '-in', str(keys / 'private.pem'), '-pubout', '-out', str(keys / 'public.pem')])
        for path in keys.iterdir(): path.chmod(0o444)
        with socket.socket() as reservation:
            reservation.bind(('127.0.0.1', 0))
            port = reservation.getsockname()[1]
        env_file = Path(temporary) / 'server.env'
        env_file.write_text('\n'.join((
            'DATABASE_URL=' + database_url,
            'ENCRYPTION_KEY=' + secrets.token_hex(32),
            'JWT_PRIVATE_KEY_PATH=/app/keys/private.pem', 'JWT_PUBLIC_KEY_PATH=/app/keys/public.pem',
            'PORT=' + str(port), 'ENVIRONMENT=development', 'INSTANCE_NAME=' + name,
            'BASE_URL=http://127.0.0.1:' + str(port), 'CHAIN_VERIFY_INTERVAL_SECS=0',
        )) + '\n')
        env_file.chmod(0o600)
        try:
            run(['docker', 'run', '--detach', '--name', name, '--network=host', '--read-only',
                 '--cap-drop=ALL', '--security-opt=no-new-privileges', '--pids-limit=256', '--memory=1g',
                 '--tmpfs=/tmp:rw,noexec,nosuid,size=128m', '--env-file', str(env_file),
                 '--mount', 'type=bind,source=' + str(keys) + ',target=/app/keys,readonly', image_id])
            deadline = time.monotonic() + 120
            while True:
                state = run(['docker', 'inspect', '--format', '{{.State.Running}}', name]).stdout.strip()
                if state != 'true': raise ValueError('final backend server exited before health readiness')
                try:
                    with urlopen(f'http://127.0.0.1:{port}/health', timeout=3) as response:
                        if 200 <= response.status < 300: break
                except (URLError, TimeoutError, ConnectionError):
                    pass
                if time.monotonic() >= deadline: raise ValueError('final backend server did not become healthy within 120 seconds')
                time.sleep(1)
            # These invalid values prove dispatch precedes config/key loading.
            result = run(['docker', 'exec', '--env', 'DATABASE_URL=invalid', '--env', 'ENCRYPTION_KEY=invalid',
                          '--env', 'JWT_PRIVATE_KEY_PATH=/missing', '--env', 'JWT_PUBLIC_KEY_PATH=/missing',
                          name, BINARY, 'health-check'])
            if result.stdout or result.stderr: raise ValueError('successful health check emitted unexpected output')
            invalid = subprocess.run(['docker', 'exec', '--env', 'PORT=invalid', name, BINARY, 'health-check'],
                                     capture_output=True, text=True, timeout=10)
            if invalid.returncode != 1 or invalid.stdout or invalid.stderr != 'health-check: invalid PORT\n':
                raise ValueError('invalid PORT did not fail with the stable health-check reason')
            print('Final backend non-root server and shell-free health-check passed: ' + image_id)
        except Exception:
            # Generated fixture values do not leave this job. Limit diagnostics
            # to ordinary server logs; never print Docker Config.Env or keys.
            logs = subprocess.run(['docker', 'logs', '--tail', '80', name], capture_output=True, text=True, timeout=15)
            print(logs.stdout + logs.stderr)
            raise
        finally:
            subprocess.run(['docker', 'rm', '-f', '-v', name], capture_output=True, timeout=30)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True)
    parser.add_argument('--database-url', required=True)
    args = parser.parse_args()
    check(args.image, args.database_url)
