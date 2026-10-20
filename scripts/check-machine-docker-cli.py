#!/usr/bin/env python3
"""Drive the real Docker CLI/engine against built images and an ephemeral local WSS fixture."""
import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import re
import select
import shutil
import ssl
import subprocess
import sys
import tempfile
import time
from urllib.request import urlopen
import uuid

from image_artifacts import copy, image_container

ROOT = Path(__file__).resolve().parents[1]
OFFICIAL = 'ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:latest'


def docker(*args):
    return subprocess.check_output(['docker', *args], text=True).strip()


def resources():
    return set(docker('ps', '-aq').split()), set(docker('volume', 'ls', '-q').split())


def require_probe(arguments):
    if '--probe-separation' not in arguments or '--rm' not in arguments or OFFICIAL not in arguments or \
            any(arg in ('-v', '--volume', '--mount', '-e', '--env', '--env-file') or
                arg.startswith(('-v', '-e', '--volume=', '--volumes-from', '--mount=', '--env=', '--env-file=')) for arg in arguments):
        raise ValueError('probe must use the actual official image without identity mounts or token/environment inputs')


def observe_docker(directory):
    executable = shutil.which('docker')
    if not executable:
        raise ValueError('real Docker executable missing')
    observer = directory / 'bin'
    observer.mkdir()
    trace = directory / 'docker-calls.jsonl'
    script = observer / 'docker'
    # This records argv and execs the real engine client unchanged. It never
    # supplies an answer, alters arguments or fabricates capability evidence.
    script.write_text('#!' + sys.executable + '\nimport json,os,sys\n'
                      'from pathlib import Path\n'
                      'with Path(' + repr(str(trace)) + ').open("a") as output:\n'
                      '    output.write(json.dumps(sys.argv[1:])+"\\n")\n'
                      'os.execv(' + repr(executable) + ', [' + repr(executable) + ']+sys.argv[1:])\n')
    script.chmod(0o700)
    return observer, trace


@contextmanager
def fixture(directory, gateway):
    certificate, key = directory / 'ca.pem', directory / 'tls.key'
    subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
                    '-subj', '/CN=NyxID local Docker CI transport',
                    '-addext', f'subjectAltName=IP:127.0.0.1,IP:{gateway}',
                    '-keyout', str(key), '-out', str(certificate)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    key.chmod(0o600)
    process = subprocess.Popen(['node', str(ROOT / 'scripts/machine_docker_fixture.mjs'), str(certificate), str(key)],
                               stdout=subprocess.PIPE, text=True)
    try:
        ready, _, _ = select.select([process.stdout], [], [], 15)
        if not ready:
            raise ValueError('local WSS fixture did not become ready')
        info = json.loads(process.stdout.readline())
        context = ssl.create_default_context(cafile=str(certificate))

        def state():
            with urlopen(f'https://127.0.0.1:{info["port"]}/__fixture/state', context=context, timeout=5) as response:
                return json.load(response)

        yield info, certificate, state
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        process.stdout.close()


def wait_for(state, predicate, timeout=180):
    deadline = time.monotonic() + timeout
    while True:
        current = state()
        if current['errors'] or current['registration_attempts'] > 1:
            raise ValueError('local WSS protocol/registration invariant failed')
        if predicate(current):
            return current
        if time.monotonic() >= deadline:
            raise ValueError('machine startup/reconnect did not reach the expected local fixture state')
        time.sleep(.5)


def custody(container):
    # Output stays in runner memory. Do not log raw config, key bytes or the
    # ciphertexts used for equality; only the fixed pass/fail message is public.
    program = '''import hashlib,json,os,stat,tomllib
from pathlib import Path
root=Path('/var/lib/nyxid-machine/node')
raw=(root/'config.toml').read_bytes(); c=tomllib.loads(raw.decode()); k=(root/'.keyfile').read_bytes()
assert len(k)==32 and stat.S_IMODE((root/'.keyfile').stat().st_mode)==0o600
assert stat.S_IMODE((root/'config.toml').stat().st_mode)==0o600
assert c['storage_backend']=='file' and c['node']['auth_token_encrypted'] and c['signing']['shared_secret_encrypted']
assert b'nyx_nauth_' not in raw and b'nyx_nreg_' not in raw
print(json.dumps({'id':c['node']['id'],'auth':c['node']['auth_token_encrypted'],'signing':c['signing']['shared_secret_encrypted'],
'key_digest':hashlib.sha256(k).hexdigest(),'choice':c['machine'].get('separation_choice'),'machine':{n:c['machine'].get(n) for n in ['shell','files','computer','browser','roots']}}))'''
    result = json.loads(docker('exec', container, 'python3', '-c', program))
    if result['choice'] != 'shared_legacy' or result['machine']['shell'] is not True or result['machine']['files'] is not True:
        raise ValueError('explicit shared-legacy choice/capabilities were not persisted')
    return result


def sanitized(text):
    text = re.sub(r'nyx_(?:nreg|nauth)_[A-Za-z0-9_-]+', '[synthetic-token]', text)
    return re.sub(r'(?i)[0-9a-f]{64}', '[synthetic-key-or-digest]', text)


def cli(binary, action, profile, environment, separation=None, expect_success=True):
    args = [str(binary), 'node', 'docker', action, '--machine', '--profile', profile]
    if separation:
        args += ['--separation', separation]
    result = subprocess.run(args, env=environment, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=120)
    if (result.returncode == 0) != expect_success:
        # Only synthetic transport values exist here; still redact the test
        # token before exposing diagnostics to the hosted log.
        error = sanitized(result.stderr)
        raise ValueError(f'actual nyxid node docker {action} returned unexpected status {result.returncode}: {error[-6000:]}')
    return result


def require_unchanged(before, after):
    if after != before:
        raise ValueError('Docker CLI left a probe container, persistent state, or anonymous volume behind')


def run_image(image, kind, directory, gateway):
    baseline = resources()
    names = []
    observer, trace = observe_docker(directory)
    with image_container(image) as (container, info):
        executable = directory / 'cli'
        copy(container, '/usr/local/bin/nyxid', executable)
        binary = executable / 'nyxid'
        binary.chmod(0o700)
        image_id = info['Id']
    docker('tag', image_id, OFFICIAL)
    with fixture(directory, gateway) as (transport, ca, state):
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith('NYXID_') and key not in ('SSL_CERT_FILE', 'SSL_CERT_DIR')}
        environment.update(PATH=str(observer) + os.pathsep + environment['PATH'],
                           NYXID_NODE_TOKEN=transport['registration_token'], NYXID_CA_CERT=str(ca),
                           NYXID_NODE_URL=f'wss://{gateway}:{transport["port"]}/api/v1/nodes/ws')
        try:
            if kind == 'node-only':
                # Browserless image is deterministically unsupported, regardless
                # of the hosted kernel's future browser/Landlock capabilities.
                for selection in (None, 'automatic', 'require-separated'):
                    profile = 'ci-refuse-' + uuid.uuid4().hex[:10]
                    names.append('nyxid-node-machine-' + profile)
                    before = resources()
                    local_profile = Path.home() / '.nyxid-node/profiles' / profile
                    if local_profile.exists():
                        raise ValueError('disposable refusal profile already exists')
                    start = len(trace.read_text().splitlines()) if trace.exists() else 0
                    result = cli(binary, 'start', profile, environment, selection, expect_success=False)
                    if 'shared-legacy' not in result.stderr:
                        raise ValueError('unsupported refusal lacks the explicit legacy-consent guidance')
                    calls = [json.loads(line) for line in trace.read_text().splitlines()[start:]]
                    probes = [args for args in calls if '--probe-separation' in args]
                    if len(probes) != 1:
                        raise ValueError('unsupported start must perform one real capability preflight')
                    require_probe(probes[0])
                    require_unchanged(before, resources())
                    if local_profile.exists():
                        raise ValueError('unsupported start created a local credential/config directory')
                    if any(state()[key] != 0 for key in ('connections', 'registration_attempts', 'requests')):
                        raise ValueError('unsupported start contacted registration before refusing')
                print('Machine Docker CLI: omitted/automatic/require-separated refused before registration or state; real probes cleaned', flush=True)
            else:
                # Measure the desktop's actual capabilities too. Do not invent
                # support on hosts where its browser sandbox is unavailable.
                before = resources()
                evidence = json.loads(docker('run', '--rm', '--init', '--shm-size=512m', '--security-opt',
                    'seccomp=' + str(ROOT / 'cli/resources/machine-container/seccomp.json'),
                    '--entrypoint=/usr/local/bin/nyxid', image_id, 'node', 'setup', '--container', '--probe-separation'))
                if evidence.get('version') != 2 or evidence.get('os') != 'linux' or not isinstance(evidence.get('managed_browser_configured'), bool):
                    raise ValueError('desktop did not return real versioned Linux preflight evidence')
                require_unchanged(before, resources())
                print('Machine Docker CLI: actual desktop capability probe completed and cleaned', flush=True)
            profile = 'ci-legacy-' + uuid.uuid4().hex[:10]
            name = 'nyxid-node-machine-' + profile
            names.append(name)
            cli(binary, 'start', profile, environment, 'shared-legacy')
            current = wait_for(state, lambda item: item['registrations'] == 1 and item['statuses'] >= 1 and item['heartbeats'] >= 1)
            original = custody(name)
            if original['id'] != current['node_id'] or not current['machine']['shell'] or not current['machine']['files'] or current['machine'].get('separation_choice') != 'shared_legacy':
                raise ValueError('live machine identity/capability advertisement differs from recorded config')
            if kind == 'node-only' and any(original['machine'][key] is not False or current['machine'].get(key) is not False for key in ('browser', 'computer')):
                raise ValueError('node-only preset enabled browser/computer capabilities')
            inspected = json.loads(docker('inspect', name))[0]
            if inspected['Image'] != image_id or not {'NYXID_MACHINE_MANAGED=0', 'NYXID_MACHINE_SEPARATION=shared-legacy'} <= set(inspected['Config']['Env']):
                raise ValueError('real CLI did not start the selected actual image with the explicit self-hosted choice')
            for action in ('restart', 'stop-start'):
                auth_before, status_before, heartbeat_before = current['authentications'], current['statuses'], current['heartbeats']
                if action == 'stop-start':
                    cli(binary, 'stop', profile, environment)
                    cli(binary, 'start', profile, environment)
                else:
                    cli(binary, 'restart', profile, environment)
                current = wait_for(state, lambda item: item['authentications'] > auth_before and item['statuses'] > status_before and item['heartbeats'] > heartbeat_before)
                after = custody(name)
                if after != original or current['registrations'] != 1 or current['registration_attempts'] != 1:
                    raise ValueError('restart changed custody/recorded choice or re-registered the node')
            print(f'Machine Docker CLI ({kind}): explicit legacy registration, real WSS capabilities/heartbeat, restart and stop/start preserve identity/choice', flush=True)
        except BaseException:
            for name in names:
                logs = subprocess.run(['docker', 'logs', '--tail', '60', name], capture_output=True, text=True)
                if logs.returncode == 0:
                    print('Actual machine startup failure diagnostics (' + name + '):\n' + sanitized(logs.stdout + logs.stderr)[-8000:], file=sys.stderr)
            raise
        finally:
            # Remove only this harness's unique names and its own named volumes.
            for name in names:
                subprocess.run(['docker', 'rm', '-f', '-v', name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                for suffix in ('-state', '-workspace'):
                    volume = name + suffix
                    if subprocess.run(['docker', 'volume', 'inspect', volume], capture_output=True).returncode == 0:
                        subprocess.run(['docker', 'volume', 'rm', volume], check=True, stdout=subprocess.DEVNULL)
    require_unchanged(baseline, resources())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--machine-image', required=True)
    parser.add_argument('--node-only-image', required=True)
    args = parser.parse_args()
    if sys.platform != 'linux':
        raise SystemExit('this required real-Docker check runs on hosted Linux')
    bridge = json.loads(docker('network', 'inspect', 'bridge'))[0]
    gateway = bridge['IPAM']['Config'][0]['Gateway']
    for image, kind in ((args.node_only_image, 'node-only'), (args.machine_image, 'desktop')):
        with tempfile.TemporaryDirectory(prefix='nyxid-docker-cli-') as directory:
            run_image(image, kind, Path(directory), gateway)
    print('Machine Docker CLI E2E: all actual-image registration/separation/restart/cleanup checks passed', flush=True)


if __name__ == '__main__':
    main()
