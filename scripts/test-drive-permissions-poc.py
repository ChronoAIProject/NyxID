#!/usr/bin/env python3
"""Run the actual POC binary against its in-memory Drive; no account access."""

import json
import os
from pathlib import Path
import secrets
import selectors
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def main():
    repo = Path(__file__).resolve().parents[1]
    subprocess.run(
        ["cargo", "build", "-p", "nyxid-permissions", "--bin", "nyxid-drive-permissions-poc", "--locked"],
        cwd=repo, check=True,
    )
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"], cwd=repo,
    ))
    binary = Path(metadata["target_directory"]) / "debug" / "nyxid-drive-permissions-poc"
    env = os.environ.copy()
    client_key = secrets.token_hex(32)
    env["NYXID_POC_CLIENT_KEY"] = client_key
    env.pop("NYXID_POC_UPSTREAM_KEY", None)
    scratch = tempfile.TemporaryDirectory(prefix="nyxid-drive-hooks-")
    pause_flag = Path(scratch.name) / "paused"
    policy = json.loads((repo / "permissions/examples/drive-policy.json").read_text())
    policy["hooks"] = [
        {"name": "pause-writes", "handler": "pause_switch", "stage": "before_execute",
         "operations": ["drive.files.rename", "drive.folders.create"], "config": {"path": str(pause_flag)}},
        {"name": "confidential-output", "handler": "response_markers", "stage": "after_response",
         "config": {"markers": ["CONFIDENTIAL"]}},
    ]
    policy_path = Path(scratch.name) / "policy.json"
    policy_path.write_text(json.dumps(policy))
    process = subprocess.Popen(
        [str(binary), "--listen", "127.0.0.1:0", "demo", "--policy", str(policy_path)],
        cwd=repo, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True,
    )
    try:
        selector = selectors.DefaultSelector()
        selector.register(process.stderr, selectors.EVENT_READ)
        deadline = time.monotonic() + 20
        base = None
        while time.monotonic() < deadline:
            if selector.select(timeout=0.2):
                line = process.stderr.readline()
                if line.startswith("Drive permission POC: "):
                    base = line.split(": ", 1)[1].split(";", 1)[0]
                    break
            if process.poll() is not None:
                raise RuntimeError("POC exited before listening")
        selector.close()
        if base is None:
            raise RuntimeError("POC did not start within 20 seconds")

        def request(method, path, body=None):
            req = urllib.request.Request(
                base + path, method=method,
                headers={"Authorization": "Bearer " + client_key, "Content-Type": "application/json"},
                data=None if body is None else json.dumps(body).encode(),
            )
            try:
                with urllib.request.urlopen(req, timeout=5) as response:
                    return response.status, json.load(response)
            except urllib.error.HTTPError as error:
                return error.code, json.load(error)

        checks = 0

        def check(label, method, path, expected, body=None):
            nonlocal checks
            status, result = request(method, path, body)
            assert status == expected, (label, status, result)
            checks += 1
            print(f"PASS  {label}: HTTP {status}")
            return result

        check("List permitted folder", "GET", "/drive/v3/files", 200)
        check("Read nested file", "GET", "/drive/v3/files/report", 200)
        check("Deny outside file", "GET", "/drive/v3/files/salary", 403)
        check("Deny shortcut", "GET", "/drive/v3/files/shortcut-out", 403)
        check("Deny duplicate parameter", "GET", "/drive/v3/files/report?alt=json&alt=media", 403)
        check("Rename permitted file", "PATCH", "/drive/v3/files/report", 200, {"name": "Smoke-tested report"})
        renamed = check("Verify rename", "GET", "/drive/v3/files/report", 200)
        assert renamed["name"] == "Smoke-tested report"
        check("Deny moving outside", "PATCH", "/drive/v3/files/report?addParents=payroll", 403, {"name": "escape"})
        check("Create permitted subfolder", "POST", "/drive/v3/files", 200,
              {"name": "POC folder", "mimeType": "application/vnd.google-apps.folder", "parents": ["client-a"]})
        check("Deny sharing", "POST", "/drive/v3/files/report/permissions", 403, {"type": "anyone"})
        initialized = check("MCP initialize", "POST", "/mcp", 200,
                            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
        assert initialized["result"]["protocolVersion"] == "2025-03-26"
        for file_id, denied in [("report", False), ("salary", True)]:
            result = check(f"MCP check {file_id}", "POST", "/mcp", 200, {
                "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": {"name": "drive_request", "arguments": {"method": "GET", "path": f"/drive/v3/files/{file_id}"}},
            })
            assert result["result"]["isError"] is denied
        pause_flag.touch()
        check("Pre-execution hook pauses REST writes", "PATCH", "/drive/v3/files/report", 403, {"name": "blocked"})
        result = check("Pre-execution hook pauses MCP writes", "POST", "/mcp", 200, {
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "drive_request", "arguments": {"method": "PATCH", "path": "/drive/v3/files/report", "body": {"name": "blocked"}}},
        })
        assert result["result"]["isError"] is True
        unchanged = check("Reads continue; denied writes had no effect", "GET", "/drive/v3/files/report", 200)
        assert unchanged["name"] == "Smoke-tested report"
        pause_flag.unlink()
        withheld = check("Accepted write with response withheld", "PATCH", "/drive/v3/files/report", 502, {"name": "CONFIDENTIAL report"})
        assert "upstream accepted the write" in withheld["error"]
        blocked = check("Response hook withholds REST output", "GET", "/drive/v3/files/report", 403)
        assert "CONFIDENTIAL report" not in json.dumps(blocked)
        blocked = check("Response hook withholds MCP output", "POST", "/mcp", 200, {
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": {"name": "drive_request", "arguments": {"method": "GET", "path": "/drive/v3/files/report"}},
        })
        assert blocked["result"]["isError"] is True
        assert "CONFIDENTIAL report" not in json.dumps(blocked)
        check("Writes resume when pause flag is removed", "PATCH", "/drive/v3/files/report", 200, {"name": "Public report"})
        restored = check("Verify resumed write", "GET", "/drive/v3/files/report", 200)
        assert restored["name"] == "Public report"
        print(f"All {checks} checks passed. Only the in-memory Drive was used.")
    finally:
        process.terminate()
        try:
            process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.communicate()
        scratch.cleanup()


if __name__ == "__main__":
    main()
