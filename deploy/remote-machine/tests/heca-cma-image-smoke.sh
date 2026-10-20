#!/bin/sh
# Real packaged binaries; no network, account credentials or model charges.
# CMA's model/credential responses are local fixtures, not a hosted service.
set -eu
image=${1:?Usage: heca-cma-image-smoke.sh IMAGE CMA_CHECKOUT}
cma=$(CDPATH= cd -- "${2:?Pass the CMA checkout used for the image build}" && pwd)
source_dir="$cma/api/crates/layer3-adapter/src/codex"
test -f "$source_dir/native_tools_fixture.cjs"
docker run --rm -i --network none --read-only --cpus 2 --memory 2g --memory-swap 2g \
    --cap-drop ALL --security-opt no-new-privileges --user 1000:1000 \
    --env HOME=/home/agent --env PYTHONDONTWRITEBYTECODE=1 \
    --tmpfs /home/agent:rw,nosuid,nodev,uid=1000,gid=1000,mode=0700,size=512m \
    --tmpfs /tmp:rw,nosuid,nodev,size=1g \
    --mount "type=bind,source=$source_dir,target=/cma-codex,readonly" \
    --entrypoint /usr/bin/tini "$image" -g -- python3 - <<'PY'
import hashlib
import json
import os
from pathlib import Path
import selectors
import shutil
import subprocess
import time


def command(*args, timeout=30):
    return subprocess.run(args, check=True, capture_output=True, text=True, timeout=timeout).stdout.strip()


assert os.getuid() == 1000
for binary in ['nyxid', 'gh', 'chromium', 'node', 'codex', 'codex-acp', 'heca', 'heca-daemon', 'heca-mcp-server']:
    assert shutil.which(binary), f'{binary} is missing'
assert Path('/opt/nyxid/cua/cua-driver').is_file()
assert command('node', '--version') == 'v24.19.0'
assert command('codex', '--version') == 'codex-cli 0.147.0'
assert command('heca', '--version') == 'heca 0.1.0-nightly-20260929-1'
bridge = Path('/usr/local/lib/node_modules/@agentclientprotocol/codex-acp/dist/index.js')
assert hashlib.sha256(bridge.read_bytes()).hexdigest() == 'af393f07cda5fcea0c89b2659440094ebe31ca1c65b1920457b4126de4b7d602'
assert not Path('/var/lib/nyxid-machine/node/config.toml').exists()
assert not Path('/home/agent/.codex/auth.json').exists()
print('PASS: packaged tools, pinned versions and patched ACP digest', flush=True)

# Retain upstream TLS and local authentication. No --no-auth, no relay, no login.
daemon = subprocess.Popen(['heca', 'daemon', 'start', '--no-relay'],
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
mcp = None
try:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        state = subprocess.run(['heca', '--json', 'daemon', 'status'],
                               capture_output=True, text=True, timeout=5)
        if state.returncode == 0:
            status = json.loads(state.stdout)
            break
        assert daemon.poll() is None, 'Heca exited before it became ready'
        time.sleep(0.2)
    else:
        raise AssertionError('Heca did not become ready within 30 seconds')
    assert status['server_version'] == '0.1.0-nightly-20260929-1'
    assert status['agent_count'] == 0
    assert status['pid'] > 0

    mcp = subprocess.Popen(['heca-mcp-server'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                           stderr=subprocess.DEVNULL, text=True, bufsize=1)
    with selectors.DefaultSelector() as selector:
        selector.register(mcp.stdout, selectors.EVENT_READ)
        mcp.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {
            'protocolVersion': '2024-11-05', 'capabilities': {},
            'clientInfo': {'name': 'nyxid-image-smoke', 'version': '1'}}}) + '\n')
        mcp.stdin.flush()
        assert selector.select(timeout=15), 'MCP initialize timed out'
        response = json.loads(mcp.stdout.readline())
        assert response.get('id') == 1 and 'result' in response, 'MCP initialize failed'
        assert 'serverInfo' in response['result']
    mcp.terminate()
    mcp.wait(timeout=5)
    mcp = None
    command('heca', 'daemon', 'stop')
    daemon.wait(timeout=10)
    assert daemon.returncode == 0
    assert subprocess.run(['heca', '--json', 'daemon', 'status'],
                          capture_output=True, timeout=5).returncode != 0
    print('PASS: Heca authenticated local daemon status, MCP initialize and graceful stop', flush=True)
finally:
    for child in [mcp, daemon]:
        if child is not None and child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=5)

# These upstream tests exercise real Codex/ACP with temporary local models and
# credential protocol fixtures. native_tools.test requires CMA layer4-runtime
# and deliberately is not part of this installation-only image contract.
result = subprocess.run(['node', '--test', '--test-concurrency=1', *[
    '/cma-codex/' + name + '.test.cjs' for name in [
        'persist_conversations', 'subscription_auth', 'managed_model_catalog', 'live_skill_install']]],
    capture_output=True, text=True, timeout=240)
print(result.stdout, end='', flush=True)
if result.returncode:
    # Only fixture inputs exist in this disconnected, disposable container.
    print(result.stderr, flush=True)
assert result.returncode == 0, 'CMA Codex/ACP regressions failed'
assert not Path('/tmp/cma-checkpoints').exists()
print(json.dumps({'ok': True, 'uid': os.getuid(), 'memory_max': Path('/sys/fs/cgroup/memory.max').read_text().strip(),
                  'memory_peak': Path('/sys/fs/cgroup/memory.peak').read_text().strip(),
                  'cpu_max': Path('/sys/fs/cgroup/cpu.max').read_text().strip()}), flush=True)
PY
