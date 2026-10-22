#!/usr/bin/env python3
"""Exercise the packaged default entrypoint and invalid-origin fail-closed boot.

Linux Docker only. No entrypoint/command override, bind mounts or relaxed OS
isolation. The image is supplied by the caller and recorded in the evidence.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time
from urllib.request import urlopen
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    parser.add_argument("--evidence-dir", type=Path, required=True)
    args = parser.parse_args()
    args.evidence_dir.mkdir(parents=True, exist_ok=True)
    inspect = subprocess.run(["docker", "image", "inspect", args.image], check=True, capture_output=True, text=True)
    (args.evidence_dir / "image-inspect.json").write_text(inspect.stdout)
    assert json.loads(inspect.stdout)[0]["Config"]["Entrypoint"] == ["/docker-entrypoint.sh"]
    results = []
    cases = [None, ("BACKEND_URL", "http://127.0.0.1:18489;add_header X injected;"),
             ("NYXID_PUBLIC_ORIGIN", "https://example.test\nadd_header X injected;"),
             ("NYXID_BROWSER_CONNECT_ORIGINS", "https://example.test/path")]
    for index, invalid in enumerate(cases):
        name = "nyxid-entrypoint-" + uuid.uuid4().hex[:12]
        env = {"BACKEND_URL": "http://127.0.0.1:18489", "NYXID_PUBLIC_ORIGIN": "https://app.example.test",
               "NYXID_BROWSER_CONNECT_ORIGINS": "https://api.example.test"}
        if invalid:
            env[invalid[0]] = invalid[1]
        command = ["docker", "run", "-d", "--name", name, "--network", "host", "--read-only", "--tmpfs", "/tmp",
                   "--tmpfs", "/etc/nginx/nyxid/generated:uid=101,gid=101,mode=0700"]
        for key, value in env.items():
            command += ["--env", key + "=" + value]
        command += [args.image]
        try:
            subprocess.run(command, check=True, capture_output=True)
            if invalid:
                exited = subprocess.run(["docker", "wait", name], check=True, capture_output=True, text=True, timeout=15)
                assert int(exited.stdout) != 0
                logs = subprocess.run(["docker", "logs", name], check=True, capture_output=True, text=True)
                assert "NyxID nginx configuration rejected" in logs.stderr + logs.stdout
                results.append({"field": invalid[0], "exit": int(exited.stdout)})
            else:
                for _ in range(100):
                    try:
                        with urlopen("http://127.0.0.1:8080/build-version.json", timeout=1) as response:
                            assert response.headers["Strict-Transport-Security"] == "max-age=31536000"
                            assert response.headers["Referrer-Policy"] == "no-referrer"
                            assert len(response.headers.get_all("Content-Security-Policy")) == 1
                            results.append({"default_entrypoint": "ready", "status": response.status})
                        break
                    except OSError:
                        time.sleep(0.1)
                else:
                    raise RuntimeError("default entrypoint readiness deadline exceeded")
            logs = subprocess.run(["docker", "logs", name], check=True, capture_output=True, text=True)
            (args.evidence_dir / f"case-{index}.log").write_text(logs.stdout + logs.stderr)
        finally:
            subprocess.run(["docker", "rm", "-f", name], check=True, capture_output=True)
    (args.evidence_dir / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps({"checks": len(results), "results": results}, indent=2))


if __name__ == "__main__":
    main()
