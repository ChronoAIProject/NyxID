#!/usr/bin/python3
"""Test-only native-host wrapper: lose one startup probe per context profile.

Runs the real native host with Chromium's stdin, but discards its first outbound
tabs probe. Everything else retains the real native framing and reconnect path.
No request data is printed or persisted.
"""

import json
import os
import struct
import subprocess
import sys

real = "/opt/nyxid/machine-browser/native-host-probe-test-real"
socket = os.environ.get("NYXID_BROWSER_SOCKET", "")
if "/contexts/" not in socket:
    os.execv(real, [real, *sys.argv[1:]])
profile = os.path.join(os.path.dirname(os.path.dirname(socket)), "browser-profile")
marker = os.path.join(profile, ".cold-probe-test")
try:
    with open(marker, "x") as output:
        output.write("0")
except FileExistsError:
    os.execv(real, [real, *sys.argv[1:]])

child = subprocess.Popen([real, *sys.argv[1:]], stdin=sys.stdin.buffer, stdout=subprocess.PIPE)
drop = True
try:
    while True:
        header = child.stdout.read(4)
        if not header:
            break
        assert len(header) == 4
        size = struct.unpack("<I", header)[0]
        assert 0 < size <= 1024 * 1024
        body = child.stdout.read(size)
        assert len(body) == size
        if drop:
            probe = json.loads(body)
            assert probe["operation"] == "browser" and probe["action"] == "tabs"
            with open(marker, "w") as output:
                output.write("1")
            drop = False
            continue
        sys.stdout.buffer.write(header + body)
        sys.stdout.buffer.flush()
finally:
    child.terminate()
    child.wait(timeout=5)
