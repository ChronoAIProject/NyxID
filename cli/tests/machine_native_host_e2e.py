#!/usr/bin/python3
"""Exercise the production native bridge without Chromium or a WebSocket.

The old blocking-pool stdio bridge intermittently stalled its first outbound
frame and then failed to exit on socket EOF while Chromium kept stdin open.
Only fixed phase names and attempt numbers are reported on failure.
"""

import json
import os
import select
import socket
import struct
import subprocess
import tempfile
import time
import uuid


def frame(body):
    return struct.pack("<I", len(body)) + body


def read_exact(fd, size):
    result = bytearray()
    deadline = time.monotonic() + 3
    while len(result) < size:
        remaining = deadline - time.monotonic()
        assert remaining > 0 and select.select([fd], [], [], remaining)[0], "bridge_read_timeout"
        part = os.read(fd, size - len(result))
        assert part, "bridge_unexpected_eof"
        result.extend(part)
    return result


pin = json.load(open("/resources/machine-browser/package.json"))
origin = "chrome-extension://" + pin["extension_id"] + "/"
with tempfile.TemporaryDirectory() as directory:
    with socket.socket(socket.AF_UNIX) as listener:
        path = directory + "/native.sock"
        listener.bind(path)
        listener.listen()
        listener.settimeout(3)
        for attempt in range(200):
            process = subprocess.Popen(
                ["/opt/nyxid/machine-browser/native-host", origin],
                env={**os.environ, "NYXID_BROWSER_SOCKET": path},
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            )
            phase = "connect"
            try:
                with listener.accept()[0] as peer:
                    phase = "hello"
                    hello = frame(json.dumps({"type": "hello", "extension_id": pin["extension_id"]}).encode())
                    process.stdin.write(hello)
                    process.stdin.flush()
                    assert read_exact(peer.fileno(), len(hello)) == hello, "hello_mismatch"
                    phase = "first_exchange"
                    probe = frame(json.dumps({"nonce": str(uuid.uuid4()), "operation": "browser", "action": "tabs"}).encode())
                    peer.sendall(probe)
                    assert read_exact(process.stdout.fileno(), len(probe)) == probe, "probe_mismatch"
                phase = "socket_eof_with_stdin_open"
                assert process.wait(timeout=3) != 0, "unexpected_clean_exit"
            except Exception as error:
                raise AssertionError(f"native bridge attempt={attempt} phase={phase} failed") from error
            finally:
                if process.poll() is None:
                    process.kill()
                process.wait()
                process.stdin.close()
                process.stdout.close()
print("Native host: 200 first exchanges and socket-EOF exits with stdin open passed")
