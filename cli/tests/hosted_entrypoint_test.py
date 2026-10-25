"""Opt-in Linux image regression for Kubernetes' initially writable home mount.

NYXID_HOSTED_ENTRYPOINT_TEST_IMAGE selects a locally built complete hosted image.
Only the final managed-start command is replaced; the real entrypoint performs
all directory initialization under the formal four-capability container policy.
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import uuid


REPO = Path(__file__).resolve().parents[2]
IMAGE = os.environ.get("NYXID_HOSTED_ENTRYPOINT_TEST_IMAGE")
MANIFEST = json.loads((REPO / "deploy/remote-machine/seccomp/manifest.json").read_text())


@unittest.skipUnless(IMAGE, "requires an explicitly selected local hosted image and Docker")
class HostedEntrypointTests(unittest.TestCase):
    def test_kubernetes_home_is_a_supervisor_owned_non_writable_parent(self):
        profile = REPO / "deploy/remote-machine/seccomp" / Path(MANIFEST["localhost_profile"]).name
        volume = "nyxid-entrypoint-regression-" + uuid.uuid4().hex
        subprocess.run(["docker", "volume", "create", volume], check=True, capture_output=True)
        try:
            subprocess.run([
                "docker", "run", "--rm", "--network", "none", "--read-only",
                "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
                "--mount", f"type=volume,source={volume},target=/retained",
                "--entrypoint", "/bin/sh", IMAGE, "-c",
                "chmod 0755 /retained; mkdir /retained/legacy",
            ], check=True, capture_output=True, text=True)
            with tempfile.TemporaryDirectory(prefix="nyxid-entrypoint-") as temporary:
                stub = Path(temporary) / "nyxid"
                stub.write_text("""#!/usr/bin/python3
import errno,json,os,stat
assert os.geteuid() == 0
info = os.stat('/home')
pid = os.fork()
if pid == 0:
    os.setgroups([]); os.setgid(1000); os.setuid(1000)
    try:
        os.mkdir('/home/agent-controlled-parent-entry')
    except OSError as error:
        os._exit(0 if error.errno == errno.EACCES else 2)
    os._exit(1)
_, status = os.waitpid(pid, 0)
print(json.dumps({'home_uid': info.st_uid, 'home_gid': info.st_gid,
                  'home_mode': oct(stat.S_IMODE(info.st_mode)),
                  'legacy_user_cannot_modify_parent': os.waitstatus_to_exitcode(status) == 0}))
""")
                stub.chmod(0o755)
                command = [
                    "docker", "run", "--rm", "--network", "none", "--read-only",
                    "--memory", "2g", "--cpus", "2", "--cap-drop", "ALL",
                ]
                for cap in ("CHOWN", "SETUID", "SETGID", "KILL"):
                    command.extend(["--cap-add", cap])
                command.extend([
                    "--security-opt", "no-new-privileges", "--security-opt", f"seccomp={profile}",
                    "--mount", f"type=volume,source={volume},target=/var/lib/nyxid-contexts",
                    "--mount", f"type=volume,source={volume},target=/workspace,volume-subpath=legacy",
                    "--mount", f"type=bind,source={temporary},target=/acceptance-bin,readonly",
                    "--mount", f"type=bind,source={REPO / 'cli/container/hosted-entrypoint.sh'},target=/acceptance-entrypoint,readonly",
                    "--tmpfs", "/home:rw,nosuid,nodev,mode=0777",
                    "--tmpfs", "/var/lib/nyxid-machine:rw,nosuid,nodev,mode=0711",
                    "--tmpfs", "/tmp:rw,nosuid,nodev,mode=1777",
                    "--env", "PATH=/acceptance-bin:/usr/local/bin:/usr/bin:/bin",
                    "--env", "NYXID_MANAGED_KUBERNETES=1",
                    "--env", "NYXID_MANAGED_BOOTSTRAP_URL=https://unreachable.invalid/test",
                    "--entrypoint", "/bin/sh", IMAGE, "/acceptance-entrypoint",
                ])
                result = subprocess.run(command, check=True, capture_output=True, text=True)
                actual = json.loads(result.stdout.strip().splitlines()[-1])
                self.assertEqual(actual, {
                    "home_uid": 0, "home_gid": 0, "home_mode": "0o755",
                    "legacy_user_cannot_modify_parent": True,
                })
        finally:
            subprocess.run(["docker", "volume", "rm", volume], check=True, capture_output=True)


if __name__ == "__main__":
    unittest.main()
