#!/usr/bin/env python3
"""Real browser/Computer restart regression under the current Remote baseline.

Uses the full-image smoke's signed managed bootstrap and authority protocol.
The test controller owns PID 1; production Tini/entrypoint/Node are real child
processes. Restart cleanup models PID namespace teardown, retaining profile/store
inodes. This is Node-process restart evidence, not Kubernetes/CSI evidence.
"""
import datetime
from concurrent.futures import ThreadPoolExecutor
import ipaddress
import json
import os
from pathlib import Path
import re
import signal
import socket
import ssl
import subprocess
import threading
import time


def wait_for(predicate, message, timeout=120):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.1)
    raise AssertionError(message)


def profiles(diagnostics=None):
    """Only main Chromium PIDs, UID and profile metadata; never log argv."""
    candidates = set()
    for process in Path('/proc').iterdir():
        if not process.name.isdigit():
            continue
        try:
            args = [arg for arg in (process / 'cmdline').read_bytes().split(b'\0') if arg]
            if len(args) == 1:
                # Chromium rewrites argv to one process-title string after
                # startup. This disposable fixture's profile paths contain no
                # whitespace; parse switches without logging the title/argv.
                args = re.findall(rb'\S+', args[0])
            if any(arg.startswith(b'--type=') for arg in args):
                continue
            for arg in args:
                if arg.startswith(b'--user-data-dir='):
                    path = Path(os.fsdecode(arg.split(b'=', 1)[1]))
                    name = path.name
                    if name in ('browser-profile', 'dev-browser-profile'):
                        candidates.add(path)
        except PermissionError:
            if diagnostics is not None:
                diagnostics['cmdline_permission_denied'] = diagnostics.get('cmdline_permission_denied', 0) + 1
            continue
        except (FileNotFoundError, ProcessLookupError):
            continue
    found = {}
    for path in candidates:
        try:
            # Forked Chromium children can temporarily retain their parent's
            # process title. The profile's SingletonLock identifies the actual
            # browser process; read it as its owner without widening 0700.
            lock = owner_command(path, "import os,sys; print(int(os.readlink(os.path.join(sys.argv[1],'SingletonLock')).rsplit('-',1)[1]))")
            if lock.returncode:
                continue
            pid = int(lock.stdout)
            status = Path('/proc', str(pid), 'status').read_text()
            if 'State:\tZ' in status:
                continue
            uid = next(line.split()[1] for line in status.splitlines() if line.startswith('Uid:'))
            assert int(uid) == path.stat().st_uid, 'browser lock identity mismatch'
            found.setdefault(path.name, []).append((pid, path))
        except (FileNotFoundError, ProcessLookupError):
            continue
    return found


def owner_command(path, script):
    meta = path.stat()

    def identity():
        os.setgroups([])
        os.setgid(meta.st_gid)
        os.setuid(meta.st_uid)

    return subprocess.run(['python3', '-c', script, str(path)], preexec_fn=identity,
                          stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=10)


def as_owner(path, script):
    result = owner_command(path, script)
    assert result.returncode == 0, 'browser-owned profile content check failed'


def stop_children(node):
    # This fixture is PID 1 in a private, non-host PID namespace. Retaining this
    # test controller lets its WSS authority survive a real Node restart.
    if node.poll() is None:
        node.terminate()
        try:
            node.wait(timeout=15)
        except subprocess.TimeoutExpired:
            node.kill()
            node.wait(timeout=5)
    for process in Path('/proc').iterdir():
        if process.name.isdigit() and int(process.name) != 1:
            try:
                os.kill(int(process.name), signal.SIGKILL)
            except ProcessLookupError:
                pass
    while True:
        try:
            pid, _ = os.waitpid(-1, os.WNOHANG)
            if pid == 0:
                break
        except ChildProcessError:
            break


def tls_server(fixture):
    from cryptography import x509
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import rsa
    from cryptography.x509.oid import NameOID
    directory = Path('/var/run/nyxid-workload/fixture-tls')
    directory.mkdir(mode=0o700)
    ca_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'Browser recovery fixture')])
    now = datetime.datetime.now(datetime.timezone.utc)
    root = (x509.CertificateBuilder().subject_name(name).issuer_name(name)
        .public_key(ca_key.public_key()).serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(minutes=1))
        .not_valid_after(now + datetime.timedelta(days=1))
        .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True)
        .sign(ca_key, hashes.SHA256()))
    certificate = (x509.CertificateBuilder()
        .subject_name(x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'localhost')]))
        .issuer_name(name).public_key(key.public_key()).serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(minutes=1))
        .not_valid_after(now + datetime.timedelta(days=1))
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(x509.SubjectAlternativeName([
            x509.DNSName('localhost'), x509.IPAddress(ipaddress.ip_address('127.0.0.1'))]), critical=False)
        .sign(ca_key, hashes.SHA256()))
    ca = directory / 'ca.pem'
    ca.write_bytes(root.public_bytes(serialization.Encoding.PEM))
    leaf = directory / 'server.pem'
    leaf.write_bytes(certificate.public_bytes(serialization.Encoding.PEM))
    private = directory / 'key.pem'
    private.write_bytes(key.private_bytes(serialization.Encoding.PEM,
        serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
    private.chmod(0o600)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(leaf, private)
    server = fixture['http'].server.ThreadingHTTPServer(('127.0.0.1', 0), fixture['LocalNodeServer'])
    server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, ca


def run(fixture):
    assert os.getpid() == 1, 'recovery fixture requires a private PID namespace'
    mode = fixture['browser_recovery']
    assert mode in ('shared_legacy', 'separated')
    checks = []
    result = {'mode': mode, 'ok': False, 'checks': checks}
    output = Path('/smoke-results') / ('browser-recovery-' + mode + '.json')
    node = None
    try:
        result['supervisor_permissions'] = fixture['supervisor_permissions']()
        checks.append('docker_default_supervisor_permissions_and_writable_root')
        server, ca = tls_server(fixture)
        page = fixture['http'].server.ThreadingHTTPServer(('127.0.0.1', 0), fixture['LocalNodeServer'])
        threading.Thread(target=page.serve_forever, daemon=True).start()
        fixture['versions'] = {}
        token = Path('/var/run/nyxid-workload/token')
        token.write_text(fixture['workload_token'])
        token.chmod(0o400)
        env = dict(os.environ, NYXID_MACHINE_MANAGED='1', NYXID_MANAGED_KUBERNETES='1',
            NYXID_MANAGED_NODE_ID=fixture['node_id'], NYXID_MANAGED_POD_UID=fixture['binding']['pod_uid'],
            NYXID_MANAGED_WORKLOAD_TOKEN_FILE=str(token), NYXID_CA_CERT=str(ca),
            NYXID_MANAGED_PREPARE_URL=f'https://localhost:{server.server_port}/api/v1/machines/managed/prepare',
            NYXID_MANAGED_BOOTSTRAP_URL=f'https://localhost:{server.server_port}/api/v1/machines/managed/bootstrap')

        def start():
            fixture['ready'].clear()
            child = subprocess.Popen(['/usr/bin/tini', '-s', '-g', '--', '/usr/local/bin/nyxid-machine-entrypoint'], env=env)
            assert fixture['ready'].wait(150), 'Node did not advertise after startup'
            assert 'fixture_error' not in fixture['observed'], 'signed WSS fixture failed'
            assert child.poll() is None, 'Node exited during startup'
            result['node_supervisor_permissions'] = fixture['supervisor_permissions'](
                int(Path('/var/lib/nyxid-machine/node/node.pid').read_text()))
            result['separated_support'] = fixture['observed']['separated']
            result['separation_evidence'] = fixture['observed']['separation_evidence']
            if mode == 'separated':
                assert fixture['observed']['separated']['available'], 'complete v4 separated evidence required'
            return child

        def call(operation, parameters):
            result['operation'] = operation + ':' + parameters.get('browser', '') + ':' + parameters.get('action', parameters.get('tool', ''))
            response = fixture['issue'](operation, parameters, timeout=100)
            if response.get('error', {}).get('code') == 12413:
                result['browser_error'] = response['error']['message']
            if response.get('error'):
                result['operation_error'] = dict(code=response['error'].get('code'),
                    message=str(response['error'].get('message', ''))[:300])
            assert not response.get('error'), operation + ' returned code ' + str(response.get('error', {}).get('code'))
            assert not response.get('isError'), operation + ' downstream tool failed'
            return response

        def observe(phase):
            result['phase'] = phase
            response = call('browser', dict(browser='secure', action='navigate',
                url=f'http://127.0.0.1:{page.server_port}/fixture'))
            assert b'Remote Machine smoke' in fixture['compact'](response), 'secure navigate missing fixture page'
            response = call('browser', dict(browser='secure', action='snapshot'))
            assert b'Remote Machine smoke' in fixture['compact'](response), 'secure snapshot missing fixture page'
            response = call('computer', dict(tool='list_windows', arguments={}))
            assert b'Remote Machine smoke' in fixture['compact'](response), 'Computer did not observe secure browser'
            call('browser', dict(browser='dev', action='navigate', url=f'http://127.0.0.1:{page.server_port}/fixture'))
            response = call('browser', dict(browser='dev', action='snapshot'))
            assert b'Remote Machine smoke' in fixture['compact'](response), 'developer snapshot missing fixture page'
            checks.append(phase + '_secure_navigate_snapshot_computer_and_developer')

        node = start()
        observe('fresh_profiles')
        result['signed_worker_permissions'] = fixture['signed_development_probe'](call)
        checks.append('signed_nonroot_worker_NNP_seccomp_and_python_node_c_development')
        diagnostics = {}
        active = profiles(diagnostics)
        result['profile_discovery'] = dict(diagnostics, found={name:len(entries) for name, entries in active.items()})
        selected = {}
        for name in ('browser-profile', 'dev-browser-profile'):
            entries = active[name]
            if mode == 'separated':
                # The daemon also owns its legacy readiness browser. Only the
                # context's s/d role directories belong to this authority.
                role = 's' if name == 'browser-profile' else 'd'
                entries = [entry for entry in entries if entry[1].is_relative_to('/var/lib/nyxid-machine/contexts')
                    and entry[1].parent.name == role]
            assert len(entries) == 1, 'ambiguous active browser profile'
            pid, path = entries[0]
            meta = path.stat()
            assert meta.st_uid != 0 and meta.st_gid == meta.st_uid and meta.st_mode & 0o7777 == 0o700
            status = dict(line.split(':', 1) for line in Path('/proc', str(pid), 'status').read_text().splitlines())
            assert int(status['CapEff'], 16) == 0 and int(status['CapPrm'], 16) == 0
            assert int(status['NoNewPrivs']) == 1, 'browser must retain process-level NNP'
            selected[name] = (pid, path, meta.st_uid, meta.st_gid, meta.st_dev, meta.st_ino)
            as_owner(path, "import pathlib,sys; (pathlib.Path(sys.argv[1])/'regression-marker').write_text('retained-profile')")
        checks.append('browser_owned_0700_profiles')
        checks.append('secure_and_developer_browser_nonroot_zero_capabilities_NNP')
        # Killing the actual secure process forces ensure_browser through the
        # existing-profile branch; Computer is deliberately the first caller.
        result['phase'] = 'browser_exit'
        secure_pid = selected['browser-profile'][0]
        os.kill(secure_pid, signal.SIGKILL)
        wait_for(lambda: all(pid != secure_pid for pid, _ in profiles().get('browser-profile', [])), 'secure process did not exit')
        call('computer', dict(tool='list_windows', arguments={}))
        observe('browser_relaunch')
        developer_path = selected['dev-browser-profile'][1]

        def kill_developer():
            entries = [(pid, path) for pid, path in profiles().get('dev-browser-profile', []) if path == developer_path]
            assert len(entries) == 1, 'ambiguous developer browser before exit'
            pid = entries[0][0]
            os.kill(pid, signal.SIGKILL)
            wait_for(lambda: all(active_pid != pid for active_pid, _ in profiles().get('dev-browser-profile', [])),
                'developer process did not exit')

        result['developer_exit_rounds'] = []
        for cycle in range(1, 6):
            result['phase'] = f'developer_exit_{cycle}'
            previous = call('browser', dict(browser='dev', action='snapshot'))
            old_tab = previous['snapshot']['tab_id']
            kill_developer()
            # A unique requested URL distinguishes this navigation from both a
            # default page and an already-restored page from the previous run.
            target = f'http://127.0.0.1:{page.server_port}/fixture#developer-recovery-{cycle}'
            recovered = call('browser', dict(browser='dev', action='navigate', url=target))
            assert target.encode() in fixture['compact'](recovered), 'first developer navigate did not reach the requested URL'
            snapshot = call('browser', dict(browser='dev', action='snapshot'))
            assert target.encode() in fixture['compact'](snapshot), 'developer recovery snapshot did not retain the requested URL'
            assert b'Remote Machine smoke' in fixture['compact'](snapshot), 'developer recovery snapshot missing fixture page'
            stale = fixture['issue']('browser', dict(browser='dev', action='snapshot', tab_id=old_tab))
            assert stale.get('status') == 'refused' and stale.get('reason') == 'stale_tab', 'old developer tab reference was remapped'
            result['developer_exit_rounds'].append(dict(cycle=cycle, first_navigate='ok', snapshot='ok', stale_tab='refused'))
        checks.append('five_developer_exits_first_navigate_snapshot_and_stale_reference_refusal')
        kill_developer()
        target = f'http://127.0.0.1:{page.server_port}/fixture#developer-concurrent-recovery'
        with ThreadPoolExecutor(max_workers=2) as callers:
            concurrent = list(callers.map(lambda _: fixture['issue']('browser', dict(browser='dev', action='navigate', url=target)), range(2)))
        assert all(not value.get('error') and target.encode() in fixture['compact'](value) for value in concurrent), 'concurrent developer recovery failed'
        assert len([entry for entry in profiles().get('dev-browser-profile', []) if entry[1] == developer_path]) == 1, 'concurrent recovery launched duplicate developer browsers'
        checks.append('concurrent_developer_recovery_has_one_live_browser')
        observe('developer_relaunch')
        previous_runtime = fixture['observed']['runtime_id']
        previous_auth = fixture['auth_count']
        fixture['ready'].clear()
        fixture['connection'].connection.shutdown(socket.SHUT_RDWR)
        wait_for(lambda: fixture['auth_count'] > previous_auth and fixture['ready'].is_set(), 'WSS did not reconnect')
        assert fixture['observed']['runtime_id'] == previous_runtime, 'WSS reconnect replaced the runtime'
        observe('wss_reconnect')
        def assert_profiles_retained():
            for _, path, uid, gid, dev, ino in selected.values():
                meta = path.stat()
                assert (meta.st_uid, meta.st_gid, meta.st_mode & 0o7777, meta.st_dev, meta.st_ino) == (uid, gid, 0o700, dev, ino)
                as_owner(path, "import pathlib,sys; assert (pathlib.Path(sys.argv[1])/'regression-marker').read_text()=='retained-profile'")
        assert_profiles_retained()
        checks.append('relaunch_and_reconnect_preserve_profile_inodes_ownership_modes_and_content')
        result['phase'] = 'node_restart'
        result['node_restart_kind'] = 'production_entrypoint_with_descendant_cleanup'
        stop_children(node)
        fixture['prepare_keys'].clear()
        node = start()
        assert fixture['observed']['runtime_id'] != previous_runtime, 'Node restart did not create a new runtime'
        if mode == 'separated':
            stale = fixture['issue']('browser', dict(browser='secure', action='snapshot'))
            assert stale.get('error', {}).get('code') == 12419 and stale['error'].get('stage') == 'provision_users', 'old context generation must be refused after daemon replacement'
            checks.append('old_context_generation_refused_after_node_restart')
            # The real backend advances context generation once on a changed
            # daemon, preserving context/assignment/workspace. The signer in
            # this fixture follows that existing contract; Node cannot choose it.
            fixture['authority_generation'] += 1
        observe('node_restart')
        result['restarted_worker_permissions'] = fixture['signed_development_probe'](call)
        checks.append('restarted_nonroot_worker_retains_NNP_seccomp_and_development_access')
        if mode == 'shared_legacy':
            assert_profiles_retained()
            checks.append('node_restart_preserves_shared_profile_inodes_ownership_modes_and_content')
        else:
            active = profiles()
            for name in ('browser-profile', 'dev-browser-profile'):
                role = 's' if name == 'browser-profile' else 'd'
                entries = [entry for entry in active[name] if entry[1].parent.name == role
                    and entry[1].parent.parent.name == 'b' + str(fixture['authority_generation'])]
                assert len(entries) == 1, 'fresh context generation profile missing'
                _, path = entries[0]
                meta = path.stat()
                assert meta.st_uid != selected[name][2] and meta.st_gid == meta.st_uid and meta.st_mode & 0o7777 == 0o700
                assert (meta.st_dev, meta.st_ino) != selected[name][-2:]
                as_owner(path, "import pathlib,sys; assert not (pathlib.Path(sys.argv[1])/'regression-marker').exists()")
            checks.append('new_generation_uses_fresh_private_profiles_without_old_content')
        result.update(ok=True, wss_authentications=fixture['auth_count'])
    except BaseException as error:
        result['error'] = str(error) if isinstance(error, AssertionError) else type(error).__name__
    finally:
        if node is not None:
            stop_children(node)
        output.write_text(json.dumps(result, indent=2) + '\n')
    if not result['ok']:
        raise SystemExit(1)
