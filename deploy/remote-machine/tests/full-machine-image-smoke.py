#!/usr/bin/env python3
"""Run the full production entrypoint as PID 1 with disposable loopback fixtures.

The loopback fixture implements only registration, auth, and heartbeats. It
uses disposable random credentials and never prints protocol frames. This is
real Node/container startup evidence, not physical provider-fence evidence.
"""
import base64
import hashlib
import hmac
import http.server
import json
import os
import pathlib
import secrets
import signal
import struct
import subprocess
import threading
import tomllib
import time
import uuid

managed = True
selected = sorted(os.environ.get("NYXID_SMOKE_CAPABILITIES", "shell,files").split(","))
generation = int(os.environ.get("NYXID_SMOKE_GENERATION", "1"))
retention_marker = os.environ.get("NYXID_SMOKE_RETENTION_MARKER")
write_lock = threading.Lock()
results_lock = threading.Condition()
responses = {}
connection = None
run_checks = []
run_resources = {}
run_revocation = {}
case = "all-capabilities" if "computer" in selected and "shell" in selected else "browser-computer" if "computer" in selected else "browser-only" if "browser" in selected else "commands-files"
if generation > 1:
    case += "-retained"
result_path = pathlib.Path("/smoke-results") / (case + ".json")
if managed:
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import padding, rsa
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM

observed = {}
ready = threading.Event()
usage_ready = threading.Event()
latest_usage = {}
node_id = os.environ.get("NYXID_SMOKE_NODE_ID", str(uuid.uuid4()))
initial_workspace_uid = pathlib.Path("/workspace").stat().st_uid
registration = "nyx_nreg_" + secrets.token_hex(32)
auth = "nyx_nauth_" + secrets.token_hex(32)
signing = secrets.token_hex(32)
workload_token = "fixture-pod-token-" + secrets.token_hex(32)
binding = dict(node_id=node_id, sandbox_uid=str(uuid.uuid4()), pod_uid=str(uuid.uuid4()),
               runtime_generation=generation, credential_epoch=generation, writer_fence=str(uuid.uuid4()),
               operation_id=str(uuid.uuid4()), delivery_id=str(uuid.uuid4()))
prepare_keys = []
prepare_count = 0
auth_count = 0


def compact(value):
    return json.dumps(value, separators=(",", ":")).encode()


def seal(public):
    # Independent implementation of nyxid_machine::bootstrap's managed v1
    # envelope. Test-only cryptography wheels are mounted read-only.
    public_der = base64.urlsafe_b64decode(public + "=" * (-len(public) % 4))
    recipient = serialization.load_der_public_key(public_der)
    assert isinstance(recipient, rsa.RSAPublicKey) and recipient.key_size == 3072
    assert recipient.public_numbers().e == 65537
    assert recipient.public_bytes(serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo) == public_der
    key = secrets.token_bytes(32)
    nonce = secrets.token_bytes(12)
    domain = b"nyxid-machine-bootstrap-v1"
    aad = domain + b"\0" + struct.pack("!I", 1) + compact(binding)
    plaintext = compact(dict(auth_token=auth, signing_secret=signing, capabilities=selected))
    assert len(plaintext) <= 16 * 1024
    cipher = AESGCM(key).encrypt(nonce, plaintext, aad)
    wrapped_key = recipient.encrypt(key, padding.OAEP(
        mgf=padding.MGF1(hashes.SHA256()), algorithm=hashes.SHA256(), label=domain))
    encode = lambda value: base64.urlsafe_b64encode(value).decode().rstrip("=")
    return dict(version=1, wrapped_key=encode(wrapped_key), nonce=encode(nonce), ciphertext=encode(cipher))


class LocalNodeServer(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def read_frame(self):
        header = self.rfile.read(2)
        if len(header) != 2:
            return None, None
        kind, size = header[0] & 15, header[1] & 127
        if size == 126:
            size = struct.unpack("!H", self.rfile.read(2))[0]
        elif size == 127:
            size = struct.unpack("!Q", self.rfile.read(8))[0]
        if size > 1024 * 1024:
            raise ValueError("fixture frame too large")
        mask = self.rfile.read(4) if header[1] & 128 else None
        body = self.rfile.read(size)
        if mask:
            body = bytes(value ^ mask[i % 4] for i, value in enumerate(body))
        return kind, body

    def send_frame(self, value, opcode=1):
        data = json.dumps(value).encode() if opcode == 1 else value
        header = bytes([128 | opcode])
        if len(data) < 126:
            header += bytes([len(data)])
        else:
            header += bytes([126]) + struct.pack("!H", len(data))
        with write_lock:
            self.wfile.write(header + data)
            self.wfile.flush()

    def json_response(self, status, value):
        data = compact(value)
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        global prepare_count, workload_token
        try:
            assert managed
            length = int(self.headers.get("Content-Length", "0"))
            assert 0 < length <= 65536
            value = json.loads(self.rfile.read(length))
            assert value["ticket"]["token"] == workload_token
            public = value["recipient_public_key"]
            decoded = base64.urlsafe_b64decode(public + "=" * (-len(public) % 4))
            assert value["ticket"]["recipient_key_digest"] == hashlib.sha256(decoded).hexdigest()
            prepare_keys.append(public)
            assert all(key == prepare_keys[0] for key in prepare_keys)
            if self.path == "/api/v1/machines/managed/prepare":
                assert value["node_id"] == node_id and value["pod_uid"] == binding["pod_uid"]
                prepare_count += 1
                if prepare_count == 1:
                    workload_token = "fixture-pod-token-" + secrets.token_hex(32)
                    replacement = pathlib.Path("/var/run/nyxid-workload/next-token")
                    replacement.write_text(workload_token)
                    replacement.chmod(0o400)
                    replacement.replace("/var/run/nyxid-workload/token")
                    self.json_response(503, {})
                else:
                    self.json_response(200, dict(binding=binding))
            elif self.path == "/api/v1/machines/managed/bootstrap":
                assert value["binding"] == binding
                self.json_response(200, dict(binding=binding, sealed=seal(public), writer_admitted=True,
                    server_url=f"ws://127.0.0.1:{self.server.server_port}/api/v1/nodes/ws"))
                observed["sealed_bootstrap"] = True
            else:
                self.json_response(404, {})
        except Exception as error:
            observed["fixture_error"] = type(error).__name__
            ready.set()
            self.json_response(500, {})

    def do_GET(self):
        global auth_count, connection
        if self.path == "/fixture":
            body=b"<!doctype html><title>Remote Machine smoke</title><h1>Remote Machine smoke</h1><input aria-label='Test input'><button>Smoke button</button>"
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path != "/api/v1/nodes/ws" or self.headers.get("Upgrade", "").lower() != "websocket":
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        challenge = self.headers["Sec-WebSocket-Key"] + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
        accept = base64.b64encode(hashlib.sha1(challenge.encode()).digest()).decode()
        self.send_response(101)
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accept)
        self.end_headers()
        try:
            while True:
                kind, body = self.read_frame()
                if kind is None or kind == 8:
                    break
                if kind == 9:
                    self.send_frame(body, opcode=10)
                    continue
                if kind != 1:
                    continue
                value = json.loads(body)
                message_type = value.get("type")
                if message_type == "register":
                    assert value["token"] == registration
                    self.send_frame({"type": "register_ok", "node_id": node_id,
                                     "auth_token": auth, "signing_secret": signing})
                elif message_type == "auth":
                    assert value["node_id"] == node_id
                    assert value["token"] == auth
                    response = {"type": "auth_ok", "node_id": node_id,
                                "heartbeat_interval_secs": 10,
                                "capabilities": {"proxy_binary_chunks": True}}
                    if managed:
                        assert value["managed_runtime"] == binding
                        auth_count += 1
                        if auth_count == 1:
                            assert pathlib.Path("/workspace").stat().st_uid == initial_workspace_uid
                            response["managed_runtime"] = dict(binding, credential_epoch=generation + 1)
                            self.send_frame(response)
                            # The next successful attempt must still not have
                            # initialized workspace before the exact fence ack.
                            continue
                        assert pathlib.Path("/workspace").stat().st_uid == initial_workspace_uid
                        response["managed_runtime"] = binding
                        now_ms = time.time_ns() // 1_000_000
                        window = dict(binding=binding, control_revision=1,
                                      run_not_after_ms=now_ms + 240_000,
                                      issued_at_ms=now_ms)
                        signature = hmac.new(bytes.fromhex(signing),
                                             b"nyxid-managed-node-window-v1\n" + compact(window),
                                             hashlib.sha256).hexdigest()
                        response["managed_runtime_window"] = dict(window=window, signature=signature)
                    connection = self
                    self.send_frame(response)
                elif message_type == "managed_machine_usage":
                    assert value["managed_runtime"] == binding
                    assert abs(time.time_ns() // 1_000_000 - value["sampled_at_ms"]) < 30_000
                    temporary = value["temporary"]
                    assert temporary.get("capacity_bytes") is None
                    assert temporary.get("available_bytes") is None
                    assert temporary["complete"] is False
                    assert temporary["measurement"] == "node_paths_lower_bound"
                    assert temporary["used_bytes"] >= 0
                    workspace = value["workspace"]
                    assert workspace["capacity_bytes"] > 0 and workspace["complete"] is True
                    assert workspace["available_bytes"] <= workspace["capacity_bytes"]
                    # Capture safe measurements only, never runtime credentials.
                    latest_usage.update(sampled_at_ms=value["sampled_at_ms"], workspace=workspace,
                                        temporary=temporary, disk_full=value["disk_full"])
                    usage_ready.set()
                elif message_type == "heartbeat":
                    self.send_frame({"type": "heartbeat_ack"})
                elif message_type == "machine_result":
                    with results_lock:
                        responses[value["request_id"]] = value["result"]
                        results_lock.notify_all()
                elif message_type == "machine_service_call" and value.get("operation") == "job_finished":
                    self.send_frame({"type":"machine_job_finished_ack", "request_id":value["request_id"]})
                elif message_type == "status_update":
                    machine = value.get("capabilities", {}).get("machine")
                    if machine:
                        versions['nyxid_node_agent_version'] = value.get('agent_version')
                        observed.update(machine)
                        ready.set()
        except (BrokenPipeError, ConnectionResetError):
            pass
        except Exception as error:
            observed["fixture_error"] = type(error).__name__
            ready.set()
        finally:
            self.close_connection = True


conversation = str(uuid.uuid4())
turn = str(uuid.uuid4())
owner = str(uuid.uuid4())
context = str(uuid.uuid4())
agent = str(uuid.uuid4())
authority_revision = 1


def make_authority(capabilities=None):
    caps = {name: name in selected for name in ("shell", "files", "browser", "computer")}
    caps["developer_browser"] = False
    if capabilities is not None:
        caps = capabilities
    return dict(require_v2=True, context_id=context, generation=1, mode="shared_legacy",
        agent_id=agent, owner_id=owner, actor_id=owner, group_id=None,
        runtime_id=observed["runtime_id"], conversation_id=conversation, turn_id=turn,
        revision=authority_revision, lease_id=str(uuid.uuid4()), expires_at_ms=time.time_ns()//1_000_000+44_000,
        capabilities=caps)


def issue(operation, parameters, capabilities=None, timeout=60, authority=None):
    parameters = dict(conversation_id=conversation, turn_id=turn, **parameters)
    if operation == "exec":
        parameters.setdefault("runtime_id", observed["runtime_id"])
        parameters.setdefault("job_id", str(uuid.uuid4()))
        parameters.setdefault("services", [])
    authority = authority if authority is not None else make_authority(capabilities)
    value = dict(type="machine_request_v2", version=2, authority=authority,
        request_id=str(uuid.uuid4()), node_id=node_id, operation=operation,
        parameters=parameters, timestamp=int(time.time()), nonce=str(uuid.uuid4()))
    canonical = lambda v: json.dumps(v, sort_keys=True, separators=(",", ":")).encode()
    mac=hmac.new(bytes.fromhex(signing), b"nyxid.machine.request.v2\0", hashlib.sha256)
    mac.update(hashlib.sha256(canonical(authority)).digest())
    mac.update(struct.pack("!I", 2))
    fields=[value["request_id"].encode(), node_id.encode(), compact(operation),
            hashlib.sha256(canonical(parameters)).digest(), struct.pack("!q",value["timestamp"]),value["nonce"].encode()]
    for field in fields:
        mac.update(struct.pack("!Q",len(field)))
        mac.update(field)
    value["signature"]=mac.hexdigest()
    connection.send_frame(value)
    deadline=time.monotonic()+timeout
    with results_lock:
        while value["request_id"] not in responses:
            remaining=deadline-time.monotonic()
            assert remaining>0, "timed out: "+operation
            results_lock.wait(remaining)
        return responses.pop(value["request_id"])


def identity_check(uid):
    def identity():
        os.setgroups([])
        os.setgid(uid)
        os.setuid(uid)
    check = subprocess.run(["python3","-c", "import os; "
        "assert not os.access('/var/lib/nyxid-machine/node/config.toml',4); "
        "assert not os.access('/var/run/nyxid-workload/token',4)"],
        preexec_fn=identity, capture_output=True)
    assert check.returncode == 0, "identity separation failed"


def process_counts():
    counts={}
    for directory in pathlib.Path('/proc').iterdir():
        if not directory.name.isdigit():
            continue
        try:
            name=(directory/'comm').read_text().strip()
            if name in ('chromium','cua-driver','Xvfb','openbox'):
                counts[name]=counts.get(name,0)+1
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            pass
    return counts


def filesystem_usage(path):
    value=os.statvfs(path)
    return {'capacity_bytes':value.f_blocks*value.f_frsize,
            'used_bytes':(value.f_blocks-value.f_bfree)*value.f_frsize,
            'available_bytes':value.f_bavail*value.f_frsize}


def run_cases():
    global context, authority_revision
    checks=run_checks
    assert ready.wait(150), "Node did not advertise capabilities"
    assert "fixture_error" not in observed, "local protocol fixture failed"
    assert prepare_count == 2 and auth_count >= 2
    assert observed.get("sealed_bootstrap"), "sealed bootstrap missing"
    assert versions.get('nyxid_node_agent_version') and versions['nyxid_cli'].endswith(versions['nyxid_node_agent_version']), 'bundled Node version does not match CLI version'
    for name in ('shell','files','browser','computer'):
        assert observed.get(name,False) == (name in selected), "advertised capabilities mismatch"
    config_path=pathlib.Path('/var/lib/nyxid-machine/node/config.toml')
    config=tomllib.loads(config_path.read_text())
    assert config['managed_runtime']==binding
    assert config['managed_capabilities']==selected
    assert auth not in config_path.read_text() and signing not in config_path.read_text()
    assert config['machine']['max_jobs']==2 and config['machine']['max_timeout_secs']==3600
    assert config['machine']['roots']==['/workspace'] and not config['machine']['allow_root']
    assert config_path.parent.stat().st_mode & 0o777==0o700
    assert pathlib.Path('/workspace').stat().st_uid==1000
    assert pathlib.Path('/home/agent').stat().st_uid==1000
    for uid in (1000,1001,1002):
        identity_check(uid)
    checks.extend(['sealed_bootstrap','projected_token_rotation','stale_WSS_fence_rejected','exact_WSS_ack','selected_capabilities','protected_identity'])
    cgroup=pathlib.Path('/sys/fs/cgroup')
    cpu=(cgroup/'cpu.max').read_text().strip()
    memory=int((cgroup/'memory.max').read_text())
    quota,period=map(int,cpu.split())
    assert quota/period==2 and memory==2*1024**3
    resources={'cpu_max':cpu,'memory_max':memory,'workspace':'32 MiB disposable tmpfs for smoke; 20 GiB PVC not verified',
               'ephemeral':'/tmp and /home individually capped at 1 GiB tmpfs; this is not Kubernetes Pod ephemeral accounting'}
    if retention_marker:
        resources['workspace'] = 'Disposable named Docker volume retained for two generations; capacity unbounded, 20 GiB enforcement not verified'
    resources['memory_current']=int((cgroup/'memory.current').read_text())
    resources['memory_peak']=int((cgroup/'memory.peak').read_text())
    resources['memory_events']=(cgroup/'memory.events').read_text().strip()
    run_resources.update(resources)
    checks.append('actual_cgroup_2cpu_2gib')
    assert subprocess.run(['/usr/local/bin/nyxid-machine-healthcheck'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0
    checks.append('production_managed_healthcheck')
    assert usage_ready.wait(35), 'current Node did not send unknown-capacity managed usage'
    assert 'fixture_error' not in observed, 'managed usage fixture rejected a frame'
    resources['node_usage'] = dict(latest_usage)
    checks.append('actual_Node_wire_unknown_temporary_capacity_with_source_time_and_workspace')
    if 'shell' in selected:
        command="id -u && python3 -c \"import os; assert not os.access('/var/lib/nyxid-machine/node/config.toml',4); assert not os.access('/var/run/nyxid-workload/token',4)\" && gh --version"
        result=issue('exec',dict(command=command,cwd='/workspace',timeout_secs=15))
        assert result.get('exit_code')==0 and result.get('stdout','').splitlines()[0]=='1000', 'signed command failed'
        assert versions['gh'] in result.get('stdout',''), 'gh not usable by signed command identity'
        checks.extend(['signed_command_as_agent','gh_executed_by_agent_without_bootstrap_credentials'])
        denied=issue('exec',dict(command='true',timeout_secs=5),dict(shell=False,files=True,browser=False,computer=False,developer_browser=False))
        assert denied.get('error',{}).get('code')==12420, 'signed authority narrowing failed'
        checks.append('signed_authority_denial')
    if 'files' in selected:
        roundtrip_path=f'/workspace/full-image-smoke-{generation}.txt'
        result=issue('write_file',dict(path=roundtrip_path,content='full image round trip',encoding='text',mode='create'))
        assert not result.get('error'), 'signed write failed'
        result=issue('read_file',dict(path=roundtrip_path))
        assert result.get('content')=='full image round trip', 'signed file read failed'
        result=issue('read_file',dict(path='/var/lib/nyxid-machine/node/config.toml'))
        # The worker drops to the command identity before canonicalization.
        # An unreadable protected parent can therefore refuse access before
        # the root-allowlist check; both paths must expose no file content.
        assert result.get('error',{}).get('code') in (12402,12407) and 'content' not in result, 'protected path read not denied'
        result=issue('read_file',dict(path='/etc/passwd'))
        assert result.get('error',{}).get('code')==12402 and 'content' not in result, 'readable outside-root path not denied'
        checks.extend(['signed_file_roundtrip','protected_file_path_denied','readable_outside_root_path_denied'])
        if retention_marker:
            if generation == 1:
                result=issue('write_file',dict(path='/workspace/retention-marker.txt',content=retention_marker,encoding='text',mode='create'))
                assert not result.get('error'), 'retention marker write failed'
                checks.append('signed_workspace_marker_written_generation_1')
            else:
                result=issue('read_file',dict(path='/workspace/retention-marker.txt'))
                assert result.get('content') == retention_marker, 'workspace marker not retained across generations'
                checks.append('signed_workspace_marker_retained_generation_2')
    if 'browser' in selected:
        assert config['machine']['managed_browser']['container'] is True
        assert observed.get('cua_version')=='0.31.0'
        result=issue('browser',dict(browser='secure',action='navigate',url=f'http://127.0.0.1:{server.server_port}/fixture'))
        assert result.get('status')=='ok', 'actual secure Chromium navigation failed: '+str(result.get('error',{}).get('code'))
        result=issue('browser',dict(browser='secure',action='snapshot'))
        assert b'Remote Machine smoke' in compact(result), 'actual Chromium page snapshot failed'
        checks.extend(['real_Chromium_navigation','real_Chromium_snapshot'])
        for capability,operation,parameters in [('shell','exec',dict(command='true',timeout_secs=5)),('files','read_file',dict(path='/workspace/absent'))]:
            if capability not in selected:
                denied=issue(operation,parameters)
                assert denied.get('error',{}).get('code') in (12400,12420), 'unselected capability was not denied'
                checks.append('unselected_'+capability+'_denied')
        denied=issue('browser',dict(browser='secure',action='snapshot'),dict(shell=False,files=False,browser=False,computer=False,developer_browser=False))
        assert denied.get('error',{}).get('code')==12420
        checks.append('browser_authority_denial')
        for uid,name in ((1001,'browser'),(1002,'devbrowser')):
            path=pathlib.Path('/home')/name
            assert path.stat().st_uid==uid and path.stat().st_mode&0o777==0o700
    else:
        denied=issue('browser',dict(browser='secure',action='snapshot'))
        assert denied.get('error',{}).get('code') in (12400,12420)
        assert not process_counts(), 'unselected desktop components started'
        checks.append('browser_disabled_without_desktop_processes')
    if 'computer' in selected:
        result=issue('computer',dict(tool='get_desktop_state',arguments={'max_image_dimension':640}))
        assert not result.get('error') and not result.get('isError')
        assert any(item.get('type')=='image' and item.get('data') for item in result.get('content',[])), 'actual cua desktop capture failed'
        result=issue('computer',dict(tool='list_windows',arguments={}))
        assert not result.get('error') and not result.get('isError')
        assert b'Remote Machine smoke' in compact(result), 'actual cua Chromium window observation failed'
        checks.extend(['real_cua_desktop_capture','real_cua_window_observation'])
        if 'shell' in selected:
            result=issue('exec',dict(command='printf desktop-command-coexistence',cwd='/workspace',timeout_secs=15))
            assert result.get('exit_code')==0 and result.get('stdout')=='desktop-command-coexistence'
            assert process_counts().get('chromium',0)>0 and process_counts().get('cua-driver',0)>0
            checks.append('signed_command_coexists_with_Chromium_and_cua')
    else:
        denied=issue('computer',dict(tool='get_desktop_state',arguments={}))
        assert denied.get('error',{}).get('code') in (12400,12420), 'unselected Computer capability was not denied'
        checks.append('unselected_computer_denied')

    # Exercise an actual signed runtime revocation, separately from narrower
    # request capabilities. Fresh nonces/leases make the denial a revision
    # fence check rather than a duplicate-message or duplicate-lease rejection.
    operation,parameters = ('exec',dict(command='printf valid-before-revoke',cwd='/workspace',timeout_secs=15)) if 'shell' in selected else ('browser',dict(browser='secure',action='snapshot'))
    prior_authority=make_authority()
    valid=issue(operation,parameters,authority=prior_authority)
    if 'shell' in selected:
        assert valid.get('exit_code')==0 and valid.get('stdout')=='valid-before-revoke', 'authority was not valid before revocation'
    else:
        assert b'Remote Machine smoke' in compact(valid), 'browser authority was not valid before revocation'
    checks.append('valid_authority_before_runtime_revoke')
    run_revocation.update(prior_revision=authority_revision,revoked_revision=authority_revision+1,prior_authority_valid=True)
    revocation_job=None
    revocation_pid=None
    if 'shell' in selected:
        revocation_path=f'/workspace/revocation-{generation}.pid'
        command='python3 -c \'import os,time; open("'+revocation_path+'","w").write(str(os.getpid())); time.sleep(300)\''
        job_authority=make_authority()
        revocation_job=issue('exec',dict(command=command,cwd='/workspace',background=True,timeout_secs=300),authority=job_authority).get('job_id')
        assert revocation_job, 'revocation target job did not start'
        for _ in range(30):
            result=issue('read_file',dict(path=revocation_path))
            if result.get('content','').isdigit():
                revocation_pid=int(result['content'])
                break
            time.sleep(0.1)
        assert revocation_pid is not None, 'revocation target PID not observed'
        os.kill(revocation_pid,0)
        result=issue('job',dict(job_id=revocation_job))
        assert result.get('status')=='running', 'revocation target job was not running'
        checks.append('runtime_revoke_target_job_running')
    revoked_authority=dict(make_authority(),revision=authority_revision+1)
    revoked_at=time.monotonic()
    accepted=issue('authority_revoke',{},authority=revoked_authority)
    assert accepted.get('accepted') is True, 'signed runtime revocation not accepted'
    checks.append('signed_runtime_authority_revoke_accepted')
    stale=issue(operation,parameters,authority=make_authority())
    assert stale.get('error',{}).get('code')==12421, 'previously valid authority revision survived revocation'
    checks.append('previous_valid_authority_denied_after_runtime_revoke')
    run_revocation.update(control_accepted=True,old_revision_reuse_error=12421)
    context=str(uuid.uuid4())
    authority_revision=revoked_authority['revision']
    fresh=issue(operation,parameters)
    if 'shell' in selected:
        assert fresh.get('exit_code')==0 and fresh.get('stdout')=='valid-before-revoke', 'fresh context authority was not admitted'
    else:
        assert b'Remote Machine smoke' in compact(fresh), 'fresh browser context authority was not admitted'
    checks.append('fresh_context_current_revision_authority_admitted')
    run_revocation.update(fresh_context_admitted=True,remaining_expiry_uses_fresh_context=True)
    if revocation_job:
        terminal=issue('job',dict(job_id=revocation_job,wait_secs=10),timeout=15)
        assert terminal.get('status')=='finished' and terminal.get('exit_code')==-1 and terminal.get('timed_out') is False, 'revoked active job did not terminate by cancellation'
        proc=pathlib.Path('/proc')/str(revocation_pid)/'stat'
        try:
            process_live=proc.read_text().rsplit(')',1)[1].split()[0]!='Z'
        except FileNotFoundError:
            process_live=False
        assert not process_live, 'revoked command process remains live'
        cancelled_ms=round((time.monotonic()-revoked_at)*1000)
        assert cancelled_ms<10000 and time.time_ns()//1_000_000<job_authority['expires_at_ms'], 'job termination did not precede ordinary lease expiry'
        checks.append('runtime_revoke_cancels_active_job_before_lease_expiry')
        run_revocation.update(active_job_running_before_revoke=True,active_job_status_after_revoke=terminal['status'],active_job_exit_code=terminal['exit_code'],active_job_timed_out=terminal['timed_out'],active_job_process_live=False,cancellation_ms=cancelled_ms,cancelled_before_lease_expiry=True)
    counts=process_counts()
    if 'browser' in selected:
        assert counts.get('chromium',0)>0 and counts.get('Xvfb',0)==2 and counts.get('cua-driver',0)>0
    if 'shell' in selected:
        detached_path=f'/workspace/detached-{generation}.pid'
        detached='python3 -c \'import os,time; child=os.fork(); os.setsid() if child==0 else None; open("'+detached_path+'","w").write(str(os.getpid())) if child==0 else None; time.sleep(300)\''
        started=issue('exec',dict(command=detached,background=True,timeout_secs=300))
        assert started.get('job_id'), 'long-lived command did not start'
        detached_pid=None
        for _ in range(20):
            result=issue('read_file',dict(path=detached_path))
            if result.get('content','').isdigit():
                detached_pid=int(result['content'])
                break
            time.sleep(0.1)
        assert detached_pid is not None and os.getpgid(detached_pid)==detached_pid, 'detached descendant did not start'
        os.kill(detached_pid,0)
        checks.append('detached_session_descendant_started_before_expiry')
    # A signed, exact-binding deadline update—not a fixture signal—terminates
    # the actual Node/Tini/PID namespace while all descendants are present.
    now=time.time_ns()//1_000_000
    cutoff=now+2500
    window=dict(binding=binding,control_revision=2,run_not_after_ms=cutoff,issued_at_ms=now)
    signature=hmac.new(bytes.fromhex(signing),b'nyxid-managed-node-window-v1\n'+compact(window),hashlib.sha256).hexdigest()
    resources['memory_current']=int((cgroup/'memory.current').read_text())
    resources['memory_peak']=int((cgroup/'memory.peak').read_text())
    resources['memory_events']=(cgroup/'memory.events').read_text().strip()
    resources['filesystems']={path:filesystem_usage(path) for path in
        ('/tmp','/home','/var/lib/nyxid-machine','/var/run/nyxid-workload','/workspace')}
    for path,capacity in [('/tmp',1024**3),('/home',1024**3),('/var/lib/nyxid-machine',64*1024**2),('/var/run/nyxid-workload',1024**2)]:
        assert resources['filesystems'][path]['capacity_bytes']==capacity, 'temporary filesystem capacity differs from declared limit'
    checks.append('actual_temporary_filesystem_capacities')
    payload={'case':case,'ok':True,'checks':checks,'selected_capabilities':selected,'runtime_generation':generation,'versions':versions,
        'started_process_counts':counts,'resources':resources,'revocation':run_revocation,'cutoff_ms':cutoff,
        'component_state':{
            'nyxid_cli_node':{'installed':True,'runtime_started':True,'bundled_executable':True},
            'gh':{'installed':True,'executed_by_signed_command':'shell' in selected},
            'chromium':{'installed':True,'runtime_started':counts.get('chromium',0)>0,'browser_authorized_and_functional':'browser' in selected},
            'cua':{'installed':True,'runtime_started':counts.get('cua-driver',0)>0,'computer_authorized_and_functional':'computer' in selected}},
        'authorization':'real Node verifies signed v2 local fixture; real backend/client authorization remains separate'}
    result_path.write_text(json.dumps(payload,indent=2)+'\n')
    connection.send_frame(dict(type='managed_runtime_window',window=dict(window=window,signature=signature)))
    # The hosting shell verifies container exit and PID=0. This fixture remains
    # alive so a clean container exit cannot come from test completion.
    time.sleep(30)
    payload['ok']=False
    payload['error']='PID namespace did not terminate after signed expiry'
    result_path.write_text(json.dumps(payload,indent=2)+'\n')
    os.kill(1,signal.SIGTERM)


assert os.getpid()==1, 'fixture must initially own PID 1 before execing production Tini'
versions={}
for name,args in [('nyxid_cli',['nyxid','--version']),('gh',['gh','--version']),('chromium',['chromium','--version']),('cua',['/opt/nyxid/cua/cua-driver','--version'])]:
    result=subprocess.run(args,capture_output=True,text=True,timeout=15)
    assert result.returncode==0, name+' executable version check failed'
    versions[name]=result.stdout.strip().splitlines()[0]
assert subprocess.run(['nyxid','node','--help'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0
versions['nyxid_node']=versions['nyxid_cli']+' (same executable; node subcommand verified)'
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),LocalNodeServer)
token_file=pathlib.Path('/var/run/nyxid-workload/token')
token_file.write_text(workload_token)
token_file.chmod(0o400)
env=os.environ.copy()
env.update(NYXID_MACHINE_MANAGED='1',NYXID_MANAGED_KUBERNETES='1',NYXID_MANAGED_NODE_ID=node_id,
    NYXID_MANAGED_POD_UID=binding['pod_uid'],NYXID_MANAGED_WORKLOAD_TOKEN_FILE=str(token_file),
    NYXID_MANAGED_PREPARE_URL=f'http://127.0.0.1:{server.server_port}/api/v1/machines/managed/prepare',
    NYXID_MANAGED_BOOTSTRAP_URL=f'http://127.0.0.1:{server.server_port}/api/v1/machines/managed/bootstrap')
fixture_pid=os.fork()
if fixture_pid==0:
    threading.Thread(target=server.serve_forever,daemon=True).start()
    try:
        run_cases()
    except BaseException as error:
        # Only fixed test assertion text is persisted; never emit protocol
        # frames, exceptions carrying credential material, or Node logs.
        result_path.write_text(json.dumps({'case':case,'ok':False,
            'error':str(error) if isinstance(error,AssertionError) else type(error).__name__,
            'checks':run_checks,'resources':run_resources,'selected_capabilities':selected,
            'versions':versions,'started_process_counts':process_counts(),'revocation':run_revocation})+'\n')
        os.kill(1,signal.SIGTERM)
    os._exit(1)
server.server_close()
os.execve('/usr/bin/tini',['/usr/bin/tini','-g','--','/usr/local/bin/nyxid-machine-entrypoint'],env)
