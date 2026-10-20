"""Execute safe final-image probes against the immutable identity-checked ID."""
import json
import subprocess
import uuid

from elf_runtime import runtime

NODE_LIBRARIES = frozenset({'libgcc_s.so.1', 'libm.so.6', 'libc.so.6', 'libpthread.so.0', 'libdl.so.2',
                            'librt.so.1', 'ld-linux-x86-64.so.2', 'ld-linux-aarch64.so.1'})


def probe(image_id, entrypoint, arguments, *, input_text=None, expected_code=0, required=None):
    # No identities, registration inputs, host files, sockets or persistent
    # volumes enter these containers. Explicit cleanup also handles timeouts.
    name = 'nyxid-runtime-' + uuid.uuid4().hex
    command = ['docker', 'run', '--rm', '--name', name, '--network=none', '--read-only',
               '--cap-drop=ALL', '--security-opt=no-new-privileges', '--pids-limit=64', '--memory=256m',
               '--tmpfs=/tmp:rw,noexec,nosuid,size=16m']
    if input_text is not None: command.append('-i')
    if entrypoint is not None: command += ['--entrypoint', entrypoint]
    command += [image_id, *arguments]
    try:
        result = subprocess.run(command, input=input_text, capture_output=True, text=True, timeout=60)
        output = result.stdout + result.stderr
        if result.returncode != expected_code or (required and required not in output):
            raise ValueError('final-image runtime probe failed: ' + json.dumps(dict(
                entrypoint=entrypoint, arguments=arguments, exit_code=result.returncode, output=output[-4000:])))
        return result
    finally:
        subprocess.run(['docker', 'rm', '-f', '-v', name], capture_output=True, timeout=30)


def check(image_id, component, binary=None):
    if binary is not None:
        elf = runtime(binary)
        print('Final ELF runtime ' + component + ': ' + json.dumps(elf, sort_keys=True))
        if component in ('mcp-demo', 'nyxid-machine-updater') and (elf['needed'] or elf['interpreter']):
            raise ValueError('scratch component is not a static executable')
    if component in ('node-agent', 'nyxid-node-machine', 'machine-node-only', 'machine-hosted-desktop', 'machine-hosted'):
        probe(image_id, '/usr/local/bin/nyxid', ['--version'], required='nyxid ')
        if component == 'node-agent' and set(elf['needed']) - NODE_LIBRARIES:
            raise ValueError('minimal node image has undeclared shared libraries: ' + ', '.join(sorted(set(elf['needed']) - NODE_LIBRARIES)))
        probe(image_id, '/usr/local/bin/nyxid', ['--help'], required='Usage:')
        # Exercise Tini and the real configured entrypoint too. Its sole-flag
        # help path must return before any writes, capability probes or effects.
        probe(image_id, None, ['--help'], required='Usage:')
        if component == 'node-agent':
            probe(image_id, None, [], expected_code=1, required='No node config found')
    elif component == 'backend':
        if set(elf['needed']) - NODE_LIBRARIES:
            raise ValueError('minimal backend image has undeclared shared libraries: ' + ', '.join(sorted(set(elf['needed']) - NODE_LIBRARIES)))
        probe(image_id, '/usr/local/bin/nyxid-server', ['--version'], required='nyxid ')
        probe(image_id, None, ['--help'], required='Usage:')
    elif component == 'frontend':
        probe(image_id, '/usr/sbin/nginx', ['-v'], required='nginx version:')
        # The image's stock entrypoint execs a non-nginx command directly.
        probe(image_id, None, ['/usr/sbin/nginx', '-v'], required='nginx version:')
    elif component == 'mcp-demo':
        result = probe(image_id, None, [], input_text='{"jsonrpc":"2.0","id":1,"method":"ping"}\n')
        if json.loads(result.stdout) != dict(jsonrpc='2.0', id=1, result={}):
            raise ValueError('final MCP demo did not answer a stdio ping')
    elif component == 'nyxid-machine-updater':
        # This deliberately narrow binary has no help/version command. Its
        # invalid-command path fails before reading a mailbox or contacting Docker.
        probe(image_id, None, ['--help'], expected_code=1,
              required='machine_update failed at startup: operation_failed')
    else:
        raise ValueError('missing final-image runtime probe')
    print('Final-image executable and entrypoint probes passed: ' + component)
