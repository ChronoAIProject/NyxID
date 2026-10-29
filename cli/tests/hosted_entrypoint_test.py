"""Host storage checks and opt-in Linux image regressions for hosted startup.

NYXID_HOSTED_ENTRYPOINT_TEST_IMAGE selects a locally built complete hosted image.
Only the final managed-start command is replaced; the real entrypoint performs
all directory initialization under the existing seccomp policy; storage tests use
only CHOWN, SETUID, SETGID and KILL.
"""
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import time
import unittest
from unittest import mock
import uuid


REPO = Path(__file__).resolve().parents[2]
IMAGE = os.environ.get("NYXID_HOSTED_ENTRYPOINT_TEST_IMAGE")
PACKAGED_DESKTOP = os.environ.get("NYXID_HOSTED_ENTRYPOINT_TEST_PACKAGED_DESKTOP") == "1"
MANIFEST = json.loads((REPO / "deploy/remote-machine/seccomp/manifest.json").read_text())
CAPABILITIES = json.loads((REPO / "machine/resources/docker-supervisor-capabilities.json").read_text())


class ManagedStorageInitializationTests(unittest.TestCase):
    """Run the entrypoint's actual storage block on isolated host directories.

    Only absolute paths and observed ownership are substituted: chmod, directory
    opens and no-follow behavior use the real filesystem without requiring root.
    The Linux image tests below cover real root ownership and managed-start.
    """

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="nyxid-managed-storage-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.retained = self.root / "retained"
        self.legacy = self.retained / "legacy"
        self.legacy.mkdir(parents=True)
        self.retained.chmod(0o755)
        self.tmp = self.root / "tmp"
        self.tmp.mkdir()
        self.tmp.chmod(0o777)

    def initialize(self, owner=(0, 0), volume_owner=(0, 0)):
        source = (REPO / "cli/container/hosted-entrypoint.sh").read_text()
        storage = source.split("<<'STORAGE'\n", 1)[1].split("\nSTORAGE", 1)[0]
        paths = {"/var/lib/nyxid-contexts": self.retained,
                 "/var/lib/nyxid-contexts/legacy": self.legacy,
                 "/workspace": self.legacy, "/tmp": self.tmp}
        real_lstat, real_open, real_fstat = os.lstat, os.open, os.fstat

        def owned(info, identity):
            fields = list(info)
            fields[4:6] = identity
            return os.stat_result(fields)

        def lstat(path):
            return owned(real_lstat(paths[path]), owner if path == "/tmp" else volume_owner)

        def open_directory(path, flags):
            return real_open(paths[path], flags)

        with mock.patch("os.lstat", side_effect=lstat), \
                mock.patch("os.open", side_effect=open_directory), \
                mock.patch("os.fstat", side_effect=lambda fd: owned(real_fstat(fd), owner)):
            exec(compile(storage, "hosted-entrypoint STORAGE", "exec"), {})

    def test_retained_root_waits_for_authenticated_initialization_without_chmod(self):
        # This preliminary block cannot decide fresh allocation. The CLI must
        # validate the sealed WSS grant before changing the retained root.
        for mode in (0o700, 0o711, 0o755, 0o777):
            with self.subTest(mode=oct(mode)):
                self.retained.chmod(mode)
                before = self.retained.stat()
                self.initialize()
                after = self.retained.stat()
                self.assertEqual(stat.S_IMODE(after.st_mode), mode)
                self.assertEqual((after.st_dev, after.st_ino, after.st_ctime_ns),
                                 (before.st_dev, before.st_ino, before.st_ctime_ns))
                self.assertFalse((self.retained / "metadata").exists())

    def test_retained_root_rejects_foreign_owner_group_and_unexpected_modes(self):
        for owner in ((1000, 0), (0, 1000)):
            with self.subTest(owner=owner):
                with self.assertRaises((AssertionError, OSError, ValueError)):
                    self.initialize(volume_owner=owner)
        for mode in (0o770, 0o775, 0o1777, 0o2711, 0o4711):
            with self.subTest(mode=oct(mode)):
                self.retained.chmod(mode)
                with self.assertRaises((AssertionError, OSError, ValueError)):
                    self.initialize()
                self.assertEqual(stat.S_IMODE(self.retained.stat().st_mode), mode)

    def test_fresh_emptydir_gets_sticky_bit_without_changing_contents(self):
        marker = self.tmp / "marker"
        marker.write_text("preserve")
        before = self.tmp.stat()
        self.initialize()
        after = self.tmp.stat()
        self.assertEqual(stat.S_IMODE(after.st_mode), 0o1777)
        self.assertEqual((after.st_dev, after.st_ino), (before.st_dev, before.st_ino))
        self.assertEqual(marker.read_text(), "preserve")

    def test_existing_sticky_directory_is_unchanged(self):
        self.tmp.chmod(0o1777)
        before = self.tmp.stat()
        self.initialize()
        after = self.tmp.stat()
        self.assertEqual(stat.S_IMODE(after.st_mode), 0o1777)
        self.assertEqual(after.st_ctime_ns, before.st_ctime_ns)

    def test_abnormal_modes_are_rejected_without_normalization(self):
        for mode in (0o755, 0o1770, 0o1775, 0o2777, 0o4777):
            with self.subTest(mode=oct(mode)):
                self.tmp.chmod(mode)
                with self.assertRaises((OSError, ValueError)):
                    self.initialize()
                self.assertEqual(stat.S_IMODE(self.tmp.stat().st_mode), mode)

    def test_foreign_owner_or_group_is_rejected(self):
        for owner in ((1000, 0), (0, 1000)):
            with self.subTest(owner=owner):
                with self.assertRaises((OSError, ValueError)):
                    self.initialize(owner)
                self.assertEqual(stat.S_IMODE(self.tmp.stat().st_mode), 0o777)

    def test_symlink_is_rejected_without_changing_its_target(self):
        target = self.root / "target"
        self.tmp.rename(target)
        self.tmp.symlink_to(target, target_is_directory=True)
        with self.assertRaises((OSError, ValueError)):
            self.initialize()
        self.assertTrue(self.tmp.is_symlink())
        self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o777)

    def test_regular_file_is_rejected_without_changing_it(self):
        self.tmp.rmdir()
        self.tmp.write_text("preserve")
        self.tmp.chmod(0o777)
        with self.assertRaises((OSError, ValueError)):
            self.initialize()
        self.assertEqual(self.tmp.read_text(), "preserve")
        self.assertEqual(stat.S_IMODE(self.tmp.stat().st_mode), 0o777)


@unittest.skipUnless(IMAGE, "requires an explicitly selected local hosted image and Docker")
class HostedEntrypointTests(unittest.TestCase):
    def test_kubernetes_emptydirs_are_initialized_before_managed_start(self):
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
tmp = os.stat('/tmp')
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
                  'tmp_uid': tmp.st_uid, 'tmp_gid': tmp.st_gid,
                  'tmp_mode': oct(stat.S_IMODE(tmp.st_mode)),
                  'legacy_user_cannot_modify_parent': os.waitstatus_to_exitcode(status) == 0}))
""")
                stub.chmod(0o755)
                command = [
                    "docker", "run", "--rm", "--network", "none",
                    "--memory", "2g", "--cpus", "2", "--cap-drop", "ALL",
                ]
                for cap in ("CHOWN", "SETUID", "SETGID", "KILL"):
                    command.extend(["--cap-add", cap])
                command.extend([
                    "--security-opt", f"seccomp={profile}",
                    "--mount", f"type=volume,source={volume},target=/var/lib/nyxid-contexts",
                    "--mount", f"type=volume,source={volume},target=/workspace,volume-subpath=legacy",
                    "--mount", f"type=bind,source={temporary},target=/acceptance-bin,readonly",
                    "--mount", f"type=bind,source={REPO / 'cli/container/hosted-entrypoint.sh'},target=/acceptance-entrypoint,readonly",
                    "--tmpfs", "/home:rw,nosuid,nodev,mode=0777",
                    "--tmpfs", "/var/lib/nyxid-machine:rw,nosuid,nodev,mode=0711",
                    "--tmpfs", "/tmp:rw,nosuid,nodev,mode=0777",
                    "--env", "PATH=/acceptance-bin:/usr/local/bin:/usr/bin:/bin",
                    "--env", "NYXID_MANAGED_KUBERNETES=1",
                    "--env", "NYXID_MANAGED_BOOTSTRAP_URL=https://unreachable.invalid/test",
                    "--entrypoint", "/bin/sh", IMAGE, "/acceptance-entrypoint",
                ])
                result = subprocess.run(command, check=True, capture_output=True, text=True)
                actual = json.loads(result.stdout.strip().splitlines()[-1])
                self.assertEqual(actual, {
                    "home_uid": 0, "home_gid": 0, "home_mode": "0o755",
                    "tmp_uid": 0, "tmp_gid": 0, "tmp_mode": "0o1777",
                    "legacy_user_cannot_modify_parent": True,
                })
        finally:
            subprocess.run(["docker", "volume", "rm", volume], check=True, capture_output=True)

    def test_self_hosted_files_startup_keeps_its_independent_workspace(self):
        with tempfile.TemporaryDirectory(prefix="nyxid-self-hosted-entrypoint-") as temporary:
            stub = Path(temporary) / "nyxid"
            stub.write_text("""#!/usr/bin/python3
import json,os,pathlib,stat,sys
assert os.geteuid() == 0
if sys.argv[1:3] == ['node','setup']:
    directory=pathlib.Path(sys.argv[sys.argv.index('--config')+1])
    (directory/'config.toml').write_text('[machine]\\nfiles=true\\nshell=true\\n')
else:
    assert sys.argv[1:3] == ['node','start']
    root=pathlib.Path('/var/lib/nyxid-contexts')
    pid=os.fork()
    if pid == 0:
        os.setgroups([]); os.setgid(1000); os.setuid(1000)
        os.chdir('/workspace')
        pathlib.Path('relative').write_text('own')
        pathlib.Path('/workspace/absolute').write_text('own')
        assert pathlib.Path('relative').read_text() == 'own'
        assert pathlib.Path('/workspace/absolute').read_text() == 'own'
        os._exit(0)
    _,status=os.waitpid(pid,0)
    assert os.waitstatus_to_exitcode(status) == 0
    print(json.dumps({'self_hosted_worker':1000,'retained_root_exists':root.exists(),'metadata_created':pathlib.Path('/var/lib/nyxid-contexts/metadata').exists()}))
""")
            stub.chmod(0o755)
            command = ["docker", "run", "--rm", "--network", "none", "--read-only", "--cap-drop", "ALL"]
            for cap in ("CHOWN", "SETUID", "SETGID", "KILL"):
                command.extend(["--cap-add", cap])
            command.extend([
                "--security-opt", "no-new-privileges",
                "--mount", f"type=bind,source={temporary},target=/acceptance-bin,readonly",
                "--mount", f"type=bind,source={REPO / 'cli/container/hosted-entrypoint.sh'},target=/acceptance-entrypoint,readonly",
                "--tmpfs", "/home:rw,nosuid,nodev,mode=0755",
                "--tmpfs", "/workspace:rw,nosuid,nodev,mode=0700",
                "--tmpfs", "/var/lib/nyxid-machine:rw,nosuid,nodev,mode=0711",
                "--tmpfs", "/var/lib/nyxid-machine-update:rw,nosuid,nodev,mode=0700",
                "--env", "PATH=/acceptance-bin:/usr/local/bin:/usr/bin:/bin",
                "--env", "NYXID_MACHINE_MANAGED=0", "--env", "NYXID_MANAGED_KUBERNETES=0",
                "--entrypoint", "/bin/sh", IMAGE, "/acceptance-entrypoint", "--shell", "--files",
            ])
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout.strip().splitlines()[-1]), {
                "self_hosted_worker": 1000, "retained_root_exists": False, "metadata_created": False,
            })


DESKTOP_FIXTURE = r'''import hashlib,json,os,pathlib,socket,subprocess,sys,time
root=pathlib.Path('/tmp')
def comm(path):
    try:return (path/'comm').read_text().strip()
    except (FileNotFoundError,ProcessLookupError):return None
if len(sys.argv)>1:
    kind=sys.argv[1]
    directory=root/'.X11-unix'
    directory.mkdir(mode=0o1777,exist_ok=True)
    os.chmod(directory,0o1777)
    lock=root/'.X99-lock'
    if kind=='symlink':
        (root/'preserve-marker').write_text('preserve')
        lock.symlink_to(root/'preserve-marker')
    else:
        lock.write_text('malformed\n' if kind=='malformed' else ' '*56+'4194304\nextra' if kind=='oversized' else '   4194304\n')
        os.chown(lock,1000 if kind=='foreign' else 1001,1001)
        os.chmod(lock,0o444)
        if kind!='incomplete':
            server=socket.socket(socket.AF_UNIX)
            server.bind(str(directory/'X99'))
            os.chown(directory/'X99',1001,1001)
            os.chmod(directory/'X99',0o777)
            server.close()
    raise SystemExit(0)
case=os.environ.get('NYXID_DESKTOP_TEST_CASE','ready')
if case in ('unrelated_live_pid','bound_socket'):
    directory=root/'.X11-unix'
    directory.mkdir(mode=0o1777,exist_ok=True)
    os.chmod(directory,0o1777)
    lock=root/'.X99-lock'
    lock.write_text('%10d\n'%(os.getpid()if case=='unrelated_live_pid'else 4194304))
    os.chown(lock,1001,1001);os.chmod(lock,0o444)
    server=socket.socket(socket.AF_UNIX)
    server.bind(str(directory/'X99'))
    os.chown(directory/'X99',1001,1001);os.chmod(directory/'X99',0o777)
    if case=='unrelated_live_pid':server.close()
    before=[os.lstat(p).st_ino for p in(lock,directory/'X99')]
    result=subprocess.run(['/usr/local/bin/nyxid-machine-desktop-start'])
    assert result.returncode!=0
    assert before==[os.lstat(p).st_ino for p in(lock,directory/'X99')]
    assert not pathlib.Path('/var/lib/nyxid-machine/desktop/Xauthority').exists()
    (root/'desktop-ready.json').write_text(json.dumps({'refused':case,'artifacts_preserved':True}))
    while True:time.sleep(1)
preflight=[]
for number in (99,100):
    lock=root/('.X'+str(number)+'-lock')
    if lock.is_file() and not lock.is_symlink():
        value=lock.read_text().strip()
        if value.isdecimal():
            name=comm(pathlib.Path('/proc')/value)
            preflight.append({'display':number,'lock_uid':lock.stat().st_uid,
                'pid':int(value),'comm':name})
print('desktop_pre_start='+json.dumps(preflight),flush=True)
if case=='cancel':
    result=subprocess.Popen(['/usr/local/bin/nyxid-machine-desktop-start'])
    deadline=time.monotonic()+10
    while not (root/'session-waiting').exists():
        assert time.monotonic()<deadline and result.poll()is None
        time.sleep(0.05)
    result.terminate()
    result.wait(timeout=8)
else:
    result=subprocess.run(['/usr/local/bin/nyxid-machine-desktop-start'])
if case in ('child_failure','cancel'):
    assert result.returncode!=0
    remaining=[name for p in pathlib.Path('/proc').iterdir()if p.name.isdecimal()
        and(name:=comm(p))in('Xvfb','openbox','runuser')]
    assert not remaining,remaining
    (root/'desktop-ready.json').write_text(json.dumps({'startup_exit':result.returncode,'remaining_desktop_children':remaining}))
    while True:time.sleep(1)
if result.returncode:raise SystemExit(result.returncode)
assert (root/'preserve-marker').read_text()=='preserve'
assert pathlib.Path('/home/retention-marker').read_text()=='preserve-home'
observed={}
for user,display,auth in [('browser',':99','Xauthority'),('devbrowser',':100','dev-Xauthority')]:
    subprocess.run(['runuser','-u',user,'--','env','-i','PATH=/usr/bin:/bin',
        'HOME=/home/'+user,'DISPLAY='+display,
        'XAUTHORITY=/var/lib/nyxid-machine/desktop/'+auth,'xdpyinfo'],
        check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    info=os.stat('/home/'+user)
    observed[user]={'uid':info.st_uid,'mode':oct(info.st_mode&0o777),
        'auth_sha256':hashlib.sha256(pathlib.Path('/var/lib/nyxid-machine/desktop/'+auth).read_bytes()).hexdigest()}
observed['xvfb_pids']=[int(p.name)for p in pathlib.Path('/proc').iterdir()
    if p.name.isdecimal() and comm(p)=='Xvfb']
assert len(observed['xvfb_pids'])==2
(root/'desktop-ready.json').write_text(json.dumps(observed))
print('desktop_fixture_ready',flush=True)
while True:time.sleep(1)
'''


@unittest.skipUnless(IMAGE, "requires an explicitly selected local hosted image and Docker")
class ManagedDesktopStartupTests(unittest.TestCase):
    """Real Xvfb, users, entrypoint, seccomp and retained mounts; no Node/API calls."""

    def setUp(self):
        self.name = "nyxid-desktop-regression-" + uuid.uuid4().hex
        self.volumes = []
        self.temporary = tempfile.TemporaryDirectory(prefix="nyxid-desktop-regression-")
        self.addCleanup(self.temporary.cleanup)
        self.addCleanup(self.cleanup_fixtures)
        temporary = Path(self.temporary.name)
        (temporary / "fixture.py").write_text(DESKTOP_FIXTURE)
        desktop_start = temporary / "desktop-start"
        desktop_start.write_bytes((REPO / "cli/container/desktop-start.sh").read_bytes())
        desktop_start.chmod(0o755)
        stub = temporary / "nyxid"
        stub.write_text("#!/bin/sh\nexec python3 /acceptance/fixture.py\n")
        stub.chmod(0o755)
        self.mounts = []
        for suffix, target in (("tmp", "/tmp"), ("home", "/home"), ("identity", "/var/lib/nyxid-machine")):
            volume = self.name + "-" + suffix
            self.docker("volume", "create", volume)
            self.volumes.append(volume)
            self.mounts.extend(["--mount", f"type=volume,source={volume},target={target},volume-nocopy"])
        self.mounts.extend(["--mount", f"type=bind,source={temporary},target=/acceptance,readonly"])
        # These ticket-based desktop fixtures require an already valid /tmp;
        # fresh Kubernetes EmptyDir permissions are covered by HostedEntrypointTests.
        self.seed("import os,pathlib;os.chmod('/tmp',0o1777);pathlib.Path('/tmp/preserve-marker').write_text('preserve');"
                  "pathlib.Path('/home/retention-marker').write_text('preserve-home')")

    def docker(self, *arguments, check=True, timeout=40):
        return subprocess.run(["docker", *arguments], check=check, capture_output=True, text=True, timeout=timeout)

    def cleanup_fixtures(self):
        self.docker("rm", "--force", "--volumes", self.name, check=False)
        for volume in self.volumes:
            self.docker("volume", "rm", volume)

    def seed(self, program):
        self.docker("run", "--rm", "--pull", "never", "--network", "none", *self.mounts,
                    "--entrypoint", "python3", IMAGE, "-c", program)

    def start(self, extra=()):
        profile = REPO / "deploy/remote-machine/seccomp" / Path(MANIFEST["localhost_profile"]).name
        arguments = ["run", "--detach", "--pull", "never", "--name", self.name,
                     "--network", "none", "--memory", "2g", "--cpus", "2", "--cap-drop", "ALL"]
        for cap in CAPABILITIES:
            arguments.extend(["--cap-add", cap])
        if not PACKAGED_DESKTOP:
            arguments.extend(["--mount", f"type=bind,source={Path(self.temporary.name) / 'desktop-start'},target=/usr/local/bin/nyxid-machine-desktop-start,readonly"])
        arguments.extend(["--security-opt", f"seccomp={profile}", *self.mounts, *extra,
            "--mount", f"type=bind,source={REPO / 'cli/container/hosted-entrypoint.sh'},target=/acceptance-entrypoint,readonly",
            "--env", "PATH=/acceptance:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            "--env", "NYXID_MACHINE_MANAGED=1",
            "--env", "NYXID_MANAGED_BOOTSTRAP_URL=https://unreachable.invalid/test",
            "--env", "NYXID_MANAGED_TICKET_FILE=/unused-test-ticket",
            "--entrypoint", "/usr/bin/tini", IMAGE, "-g", "--", "/bin/sh", "/acceptance-entrypoint"])
        self.docker(*arguments)

    def wait_ready(self):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            status = json.loads(self.docker("inspect", self.name).stdout)[0]["State"]
            if not status["Running"]:
                logs = self.docker("logs", self.name)
                self.fail("desktop fixture exited: " + logs.stdout + logs.stderr[-2500:])
            result = self.docker("exec", self.name, "cat", "/tmp/desktop-ready.json", check=False)
            if result.returncode == 0:
                return json.loads(result.stdout)
            time.sleep(0.1)
        self.fail("desktop readiness deadline exceeded")

    def test_retained_tmp_and_home_restart_starts_fresh_displays(self):
        self.start()
        before = self.wait_ready()
        self.docker("exec", self.name, "rm", "/tmp/desktop-ready.json")
        self.docker("kill", "--signal", "KILL", self.name)
        self.docker("start", self.name)
        after = self.wait_ready()
        for user, uid in (("browser", 1001), ("devbrowser", 1002)):
            self.assertEqual(after[user]["uid"], uid)
            self.assertEqual(after[user]["mode"], "0o700")
            self.assertNotEqual(before[user]["auth_sha256"], after[user]["auth_sha256"])

    def test_live_display_refusal_preserves_original_displays_and_cookies(self):
        self.start()
        before = self.wait_ready()
        result = self.docker("exec", self.name, "/usr/local/bin/nyxid-machine-desktop-start", check=False, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stage=display_preflight reason=display_in_use_or_unverified", result.stderr)
        # Read-only probes retain both original authenticated connections.
        for user, display, auth in (("browser", ":99", "Xauthority"), ("devbrowser", ":100", "dev-Xauthority")):
            self.docker("exec", self.name, "runuser", "-u", user, "--", "env", "DISPLAY=" + display,
                        "XAUTHORITY=/var/lib/nyxid-machine/desktop/" + auth, "xdpyinfo")
            digest = self.docker("exec", self.name, "sha256sum", "/var/lib/nyxid-machine/desktop/" + auth).stdout.split()[0]
            self.assertEqual(digest, before[user]["auth_sha256"])
        current = self.docker("exec", self.name, "cat", "/tmp/desktop-ready.json")
        self.assertEqual(before, json.loads(current.stdout))

    def test_unverified_display_artifacts_are_preserved(self):
        for kind in ("symlink", "foreign", "malformed", "incomplete", "oversized"):
            with self.subTest(kind=kind):
                self.seed("import subprocess;subprocess.run(['python3','/acceptance/fixture.py'," + repr(kind) + "],check=True)")
                before = self.docker("run", "--rm", "--network", "none", *self.mounts,
                    "--entrypoint", "python3", IMAGE, "-c",
                    "import os,json;print(json.dumps([(p,os.lstat(p).st_ino)for p in ['/tmp/.X99-lock']]))").stdout
                self.start()
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    state = json.loads(self.docker("inspect", self.name).stdout)[0]["State"]
                    if not state["Running"]:break
                    time.sleep(0.1)
                self.assertFalse(state["Running"])
                self.assertNotEqual(state["ExitCode"], 0)
                self.assertIn("stage=display_preflight reason=display_in_use_or_unverified", self.docker("logs", self.name).stderr)
                after = self.docker("run", "--rm", "--network", "none", *self.mounts,
                    "--entrypoint", "python3", IMAGE, "-c",
                    "import os,json;assert open('/tmp/preserve-marker').read()=='preserve';print(json.dumps([(p,os.lstat(p).st_ino)for p in ['/tmp/.X99-lock']]))").stdout
                self.assertEqual(before, after)
                self.docker("rm", "--volumes", self.name)
                self.seed("import os;os.unlink('/tmp/.X99-lock');p='/tmp/.X11-unix/X99';os.path.exists(p) and os.unlink(p)")

    def assert_xvfb_exit_stage(self, display, stage):
        # Inject one invalid option into the real runuser/Xvfb command, retaining
        # its actual UID and executable. This is child-exit coverage, separate
        # from the real retained-mount restart regression above.
        wrapper = Path(self.temporary.name) / "runuser"
        wrapper.write_text("#!/usr/bin/python3\nimport os,sys\na=sys.argv[1:]\n"
                           "if 'Xvfb' in a and " + repr("DISPLAY=" + display) + " in a:a.append('-invalid-option')\n"
                           "os.execv('/usr/sbin/runuser',['runuser',*a])\n")
        wrapper.chmod(0o755)
        self.start(["--env", "NYXID_DESKTOP_TEST_CASE=child_failure"])
        observed = self.wait_ready()
        self.assertEqual(observed, {"startup_exit": 1, "remaining_desktop_children": []})
        self.assertIn("stage=" + stage + " reason=child_exited_1", self.docker("logs", self.name).stderr)

    def test_secure_xvfb_exit_reports_its_stage_and_cleans_children(self):
        self.assert_xvfb_exit_stage(":99", "secure_xvfb")

    def test_developer_xvfb_exit_cleans_the_already_started_secure_display(self):
        self.assert_xvfb_exit_stage(":100", "dev_xvfb")

    def test_startup_cancellation_cleans_only_its_own_desktop_children(self):
        session = Path(self.temporary.name) / "waiting-session"
        session.write_text("#!/bin/sh\ntouch /tmp/session-waiting\nexec sleep 30\n")
        session.chmod(0o755)
        self.start(["--env", "NYXID_DESKTOP_TEST_CASE=cancel", "--mount",
                    f"type=bind,source={session},target=/usr/local/bin/nyxid-desktop-session,readonly"])
        observed = self.wait_ready()
        self.assertEqual(observed, {"startup_exit": 1, "remaining_desktop_children": []})

    def test_reused_live_pid_is_uncertain_and_must_be_preserved(self):
        self.start(["--env", "NYXID_DESKTOP_TEST_CASE=unrelated_live_pid"])
        self.assertEqual(self.wait_ready(), {"refused": "unrelated_live_pid", "artifacts_preserved": True})
        self.assertIn("stage=display_preflight reason=display_in_use_or_unverified", self.docker("logs", self.name).stderr)

    def test_bound_socket_with_absent_lock_pid_is_not_an_orphan(self):
        self.start(["--env", "NYXID_DESKTOP_TEST_CASE=bound_socket"])
        self.assertEqual(self.wait_ready(), {"refused": "bound_socket", "artifacts_preserved": True})
        self.assertIn("stage=display_preflight reason=display_in_use_or_unverified", self.docker("logs", self.name).stderr)


if __name__ == "__main__":
    unittest.main()
