#!/usr/bin/env python3
"""Run inside the node-only image with network=none and throwaway tmpfs mounts.

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

managed = os.environ.get("NYXID_SMOKE_MANAGED") == "1"
if managed:
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import padding, rsa
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM

observed = {}
ready = threading.Event()
node_id = str(uuid.uuid4())
registration = "nyx_nreg_" + secrets.token_hex(32)
auth = "nyx_nauth_" + secrets.token_hex(32)
signing = secrets.token_hex(32)
workload_token = "fixture-pod-token-" + secrets.token_hex(32)
binding = dict(node_id=node_id, sandbox_uid=str(uuid.uuid4()), pod_uid=str(uuid.uuid4()),
               runtime_generation=1, credential_epoch=1, writer_fence=str(uuid.uuid4()),
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
    plaintext = compact(dict(auth_token=auth, signing_secret=signing))
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
        global auth_count
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
                            assert pathlib.Path("/workspace").stat().st_uid == 0
                            response["managed_runtime"] = dict(binding, credential_epoch=2)
                            self.send_frame(response)
                            # The next successful attempt must still not have
                            # initialized workspace before the exact fence ack.
                            continue
                        assert pathlib.Path("/workspace").stat().st_uid == 0
                        response["managed_runtime"] = binding
                        now_ms = time.time_ns() // 1_000_000
                        window = dict(binding=binding, control_revision=1,
                                      run_not_after_ms=now_ms + 120_000,
                                      issued_at_ms=now_ms)
                        signature = hmac.new(bytes.fromhex(signing),
                                             b"nyxid-managed-node-window-v1\n" + compact(window),
                                             hashlib.sha256).hexdigest()
                        response["managed_runtime_window"] = dict(window=window, signature=signature)
                    self.send_frame(response)
                elif message_type == "heartbeat":
                    self.send_frame({"type": "heartbeat_ack"})
                elif message_type == "status_update":
                    machine = value.get("capabilities", {}).get("machine")
                    if machine:
                        observed.update(machine)
                        ready.set()
        except (BrokenPipeError, ConnectionResetError):
            pass
        except Exception as error:
            observed["fixture_error"] = type(error).__name__
            ready.set()
        finally:
            self.close_connection = True


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), LocalNodeServer)
threading.Thread(target=server.serve_forever, daemon=True).start()
env = os.environ.copy()
if managed:
    token_file = pathlib.Path("/var/run/nyxid-workload/token")
    token_file.write_text(workload_token)
    token_file.chmod(0o400)
    env.update(NYXID_MACHINE_MANAGED="1", NYXID_MANAGED_KUBERNETES="1",
               NYXID_MANAGED_NODE_ID=node_id, NYXID_MANAGED_POD_UID=binding["pod_uid"],
               NYXID_MANAGED_WORKLOAD_TOKEN_FILE=str(token_file),
               NYXID_MANAGED_PREPARE_URL=f"http://127.0.0.1:{server.server_port}/api/v1/machines/managed/prepare",
               NYXID_MANAGED_BOOTSTRAP_URL=f"http://127.0.0.1:{server.server_port}/api/v1/machines/managed/bootstrap")
else:
    env.update(NYXID_MACHINE_MANAGED="0", NYXID_NODE_TOKEN=registration,
               NYXID_NODE_URL=f"ws://127.0.0.1:{server.server_port}/api/v1/nodes/ws")
with open("/tmp/node-startup.log", "wb") as output:
    process = subprocess.Popen(["/usr/local/bin/nyxid-machine-node-only"],
                               env=env, stdout=output, stderr=output,
                               start_new_session=True)
    try:
        deadline = time.monotonic() + 45
        while not ready.wait(0.1) and process.poll() is None and time.monotonic() < deadline:
            pass
        assert ready.is_set(), f"Node did not advertise capabilities (exit={process.poll()})"
        assert "fixture_error" not in observed, observed.get("fixture_error")
        assert process.poll() is None, "Node exited after handshake"
        assert observed["shell"] is True and observed["files"] is True
        assert observed.get("browser") is False and not observed.get("computer", False)
        assert observed["roots"] == ["/workspace"]
        assert observed["commands_isolated"] is True
        config_path = pathlib.Path("/var/lib/nyxid-machine/node/config.toml")
        config = tomllib.loads(config_path.read_text())
        if managed:
            assert prepare_count == 2 and auth_count >= 2
            assert observed["sealed_bootstrap"] is True
            assert config["managed_runtime"] == binding
            assert 3 in observed["authority_versions"]
            assert auth not in config_path.read_text() and signing not in config_path.read_text()
        assert config["machine"]["max_jobs"] == 2
        assert config["machine"]["max_timeout_secs"] == 3600
        assert config["machine"]["output_bytes"] == 1024 * 1024
        assert config["machine"]["allow_root"] is False
        assert config["machine"]["browser"] is False
        assert config["machine"]["computer"] is False
        assert config_path.parent.stat().st_mode & 0o777 == 0o700
        assert pathlib.Path("/workspace").stat().st_uid == 1000
        assert pathlib.Path("/home/agent").stat().st_uid == 1000
        def command_identity():
            os.setgroups([])
            os.setgid(1000)
            os.setuid(1000)
        identity_check = subprocess.run(
            ["python3", "-c", "from pathlib import Path; "
             "Path('/workspace/smoke.txt').write_text('ok'); "
             "assert not __import__('os').access('/var/lib/nyxid-machine/node/config.toml', 4)"],
            preexec_fn=command_identity, capture_output=True)
        assert identity_check.returncode == 0, "command identity isolation failed"
        if managed:
            assert not subprocess.run(["python3", "-c", "import os; assert not os.access('/var/run/nyxid-workload/token',4)"], preexec_fn=command_identity, capture_output=True).returncode
            print("managed-node-image-startup: rotated projected token, sealed prepare/bootstrap, rejected stale WSS fence, exact managed ack, fixed profile and protected identity passed")
        else:
            print("node-image-startup: shell/files profile, WSS capabilities, identity separation passed")
    finally:
        if process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
        server.shutdown()
