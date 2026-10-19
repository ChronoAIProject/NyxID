#!/usr/bin/env python3
"""Destructive fixtures ONLY in an expendable container/VM, never on an owner node."""
import argparse
import ctypes
import errno
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import zipfile

parser = argparse.ArgumentParser()
parser.add_argument('--disposable', action='store_true', required=True,
                    help='Confirm this is an expendable container/VM, never an owner node')
parser.add_argument('--native-profile', action='store_true')
parser.add_argument('--probes-only', action='store_true')
args = parser.parse_args()
assert os.getuid() == 0
launcher = '/usr/local/bin/nyxid-context-spike'
base = Path('/var/lib/nyxid-machine/m13-profile/contexts' if args.native_profile else '/var/lib/nyxid-machine/contexts')
assert not base.exists(), 'Use a fresh, expendable container/VM for each run'
uid_a, uid_b = 21001, 21002
legacy_uid = 21003 if args.native_profile else 1000
for name, uid in [('m13-a', uid_a), ('m13-b', uid_b), ('nyxagent-m13', legacy_uid)]:
    if not subprocess.run(['getent', 'passwd', str(uid)], stdout=subprocess.DEVNULL).returncode:
        continue
    subprocess.run(['useradd', '--uid', str(uid), '--create-home', '--shell', '/usr/sbin/nologin', name], check=True)

def mkdir(path, uid=0, mode=0o711):
    path = Path(path)
    path.mkdir(parents=True, exist_ok=True)
    os.chown(path, uid, uid)
    path.chmod(mode)
    return path

base.mkdir(parents=True)
for path in [base, *list(base.parents)[:-1]]:
    assert path.stat().st_uid == 0 and path.stat().st_mode & 0o022 == 0
roots = [mkdir(base / name) for name in ['a', 'b']]
a, b = roots
for root, uid in zip(roots, [uid_a, uid_b]):
    for name in ['workspace', 'home', 'tmp']:
        mkdir(root / name, uid, 0o700)
workspace = a / 'workspace'

legacy = mkdir('/srv/nyxagent-m13/workspace' if args.native_profile else '/workspace', 0, 0o770)
os.chown(legacy, 0, legacy_uid)
private = mkdir('/var/lib/nyxid-machine/m13-node', 0, 0o700)
browser = mkdir('/home/nyxbrowser-m13', legacy_uid, 0o700)
paths = []
for directory in [legacy, b/'workspace', b/'home', b/'tmp', private, browser, Path('/tmp'), Path('/dev/shm')]:
    path = directory / 'm13-sentinel'
    path.write_text('test-only-private-data')
    os.chown(path, uid_b if b in directory.parents else legacy_uid, uid_b if b in directory.parents else legacy_uid)
    path.chmod(0o600 if directory == Path("/dev/shm") else 0o666)
    paths.append(path)
# Same UID outside the allowlist: filesystem restriction must add to DAC.
outside_same = Path('/tmp/m13-same-uid')
outside_same.write_text('same-uid-outside-context')
os.chown(outside_same, uid_a, uid_a)
outside_same.chmod(0o600)
paths.append(outside_same)

results = []
observations = {}
tool_failures = []
victims = []

def child(code, mode='confined', root=a, uid=uid_a, pass_fds=()):
    return subprocess.run([launcher, mode, str(root), str(uid), '/usr/bin/python3', '-c', code],
                          capture_output=True, text=True, timeout=45, pass_fds=pass_fds)

def check(name, code, mode='confined', **kw):
    compile(code, name, 'exec')
    result = child(code, mode, **kw)
    if result.returncode:
        # Fixtures contain no owner content or credentials. Bound failure detail.
        print(json.dumps({'FAIL': name, 'status': result.returncode, 'detail': result.stderr[-1600:]}), flush=True)
        raise AssertionError(name)
    results.append(name)
    print('PASS ' + name, flush=True)
    return result

deny = '''
import os, errno, ctypes, socket
def refused(fn):
    try: fn()
    except OSError as e:
        assert e.errno in (errno.EACCES, errno.EPERM, errno.EXDEV, errno.EBADF, errno.ENOENT), e
    else: raise AssertionError('operation unexpectedly allowed')
'''

def tool(name, script, timeout=120, required=True):
    start = time.monotonic()
    run = subprocess.run([launcher, 'confined', str(a), str(uid_a), '/bin/sh', '-eu', '-c', script],
                         capture_output=True, text=True, timeout=timeout)
    if run.returncode:
        print(json.dumps({'FAIL': name, 'status': run.returncode, 'detail': run.stderr[-2400:]}), flush=True)
        if not required:
            tool_failures.append(name)
            return
        raise AssertionError(name)
    results.append(name)
    print(f'PASS {name} {time.monotonic()-start:.3f}s', flush=True)

print(json.dumps({'kernel': os.uname().release, 'arch': os.uname().machine,
                  'profile': 'separate-users' if args.native_profile else 'official-container',
                  'probe': subprocess.check_output([launcher, 'abi'], text=True).strip()}), flush=True)
try:
    check('identity, groups, NNP, umask, private temp', f'''
import ctypes, os, tempfile
assert os.getuid() == {uid_a} and os.getgid() == {uid_a} and os.getgroups() == []
assert ctypes.CDLL(None).prctl(39, 0, 0, 0, 0) == 1
assert os.umask(0o077) == 0o077
assert tempfile.gettempdir() == {str(a/'tmp')!r}
with tempfile.NamedTemporaryFile() as f: f.write(b'own-temp')
assert os.environ['HOME'] == {str(a/'home')!r}
''')
    check('existing seccomp remains inherited', deny + '''
lib=ctypes.CDLL(None,use_errno=True)
assert lib.unshare(0) == -1 and ctypes.get_errno() == errno.EPERM
assert lib.setns(-1,0) == -1 and ctypes.get_errno() == errno.EPERM
''')
    for index, path in enumerate(paths):
        check(f'outside read/write denied {index}', deny + f'''
refused(lambda: open({str(path)!r}).read())
refused(lambda: open({str(path)!r},'w').write('wrong'))
refused(lambda: os.truncate({str(path)!r},0))
''')
    for root in [b/'workspace', b/'home', b/'tmp']:
        root.chmod(0o755)
    check('negative control: UIDs alone expose world-readable roots', f'''
assert open({str(b/'workspace/m13-sentinel')!r}).read() == 'test-only-private-data'
''', mode='uid-only-control')
    check('Landlock denies world-readable sibling directories', deny + f'''
for p in {[str(b/n) for n in ['workspace','home','tmp']]!r}:
    refused(lambda: os.listdir(p))
    refused(lambda: open(p+'/m13-sentinel').read())
''')
    check('absolute and relative traversal', deny + f'''
refused(lambda: open('../../b/workspace/m13-sentinel').read())
refused(lambda: os.chdir({str(legacy)!r}))
refused(lambda: open({str(a/'workspace/../../b/workspace/m13-sentinel')!r}).read())
refused(lambda: open('/proc/self/root'+{str(legacy/'m13-sentinel')!r}).read())
''')
    check('symlink and chained symlink escape', deny + f'''
os.symlink({str(b/'workspace')!r},'sibling-link')
os.symlink('sibling-link','chained-link')
for p in ['sibling-link/m13-sentinel','chained-link/m13-sentinel']:
    refused(lambda: open(p).read())
    refused(lambda: open(p,'w').write('wrong'))
os.symlink({str(legacy/'m13-sentinel')!r},'legacy-link')
refused(lambda: open('legacy-link').read())
''')
    check('hardlink escape', deny + f'''
refused(lambda: os.link({str(legacy/'m13-sentinel')!r},'legacy-hardlink'))
refused(lambda: os.link({str(outside_same)!r},'same-uid-hardlink'))
open('own-link-source','w').write('ours')
refused(lambda: os.link('own-link-source',{str(legacy/'injected-hardlink')!r}))
os.link('own-link-source','own-link-ok')
assert open('own-link-ok').read() == 'ours'
''')
    check('rename escape and supervisor parent integrity', deny + f'''
refused(lambda: os.rename({str(outside_same)!r},'renamed-outside'))
refused(lambda: os.rename('own-link-ok',{str(legacy/'renamed-inside')!r}))
refused(lambda: os.rename({str(a/'workspace')!r},{str(a/'old-workspace')!r}))
os.mkdir('rename-dir')
os.rename('own-link-ok','rename-dir/ok')
assert open('rename-dir/ok').read() == 'ours'
''')
    check('shared tmp inaccessible; private temp works', deny + '''
for directory in ['/tmp']:
    refused(lambda: os.listdir(directory))
    refused(lambda: open(directory+'/m13-injection','w'))
    refused(lambda: os.mkdir(directory+'/m13-dir'))
with open(os.environ['TMPDIR']+'/own','w') as f: f.write('ok')
''')
    check('legacy DAC ancestor denies metadata and chmod widening', deny + f'''
refused(lambda: os.utime({str(legacy/'m13-sentinel')!r},None))
refused(lambda: os.chmod({str(legacy)!r},0o777))
refused(lambda: os.stat({str(legacy/'m13-sentinel')!r}))
''')
    check('shm contents protected by DAC; names and capacity shared by design', deny + '''
assert 'm13-sentinel' in os.listdir('/dev/shm')
refused(lambda: open('/dev/shm/m13-sentinel').read())
refused(lambda: open('/dev/shm/m13-sentinel','w'))
refused(lambda: os.unlink('/dev/shm/m13-sentinel'))
from multiprocessing import get_context
s=get_context('spawn').Semaphore(1)
p='/dev/shm/sem.'+s._semlock.name.lstrip('/')
assert os.stat(p).st_uid == os.getuid() and os.stat(p).st_mode & 0o777 == 0o600
''')
    # Landlock's documented unhandled metadata syscalls need evidence, too.
    # A content-only denial must not be reported as "all outside writes denied".
    system_sentinel = Path('/tmp/m13-world-writable')
    system_sentinel.write_text('public-system-fixture')
    system_sentinel.chmod(0o666)
    stamp = system_sentinel.stat().st_mtime_ns
    mutation = child(f"import os; os.utime({str(system_sentinel)!r},None)")
    observations['allowed_by_design_system_metadata_utime'] = (
        'allowed' if mutation.returncode == 0 and system_sentinel.stat().st_mtime_ns != stamp else 'denied')
    mutation = child(f"import os; os.chmod({str(outside_same)!r},0o644)")
    observations['allowed_by_design_system_same_uid_chmod'] = 'allowed' if mutation.returncode == 0 else 'denied'
    leaked = os.open(paths[0], os.O_RDWR)
    try:
        check('negative control: Landlock alone preserves an inherited descriptor', f'''
import os
assert os.read({leaked},64) == b'test-only-private-data'
''', mode='leaked-fd-control', pass_fds=(leaked,))
        os.lseek(leaked, 0, 0)
        check('inherited descriptor and proc-self-fd scrubbed', deny + f'''
refused(lambda: os.read({leaked},64))
refused(lambda: os.write({leaked},b'wrong'))
refused(lambda: open('/proc/self/fd/{leaked}').read())
''', pass_fds=(leaked,))
    finally:
        os.close(leaked)
    leaked_dir = os.open(legacy, os.O_RDONLY | os.O_DIRECTORY)
    try:
        check('inherited directory descriptor cannot openat outside', deny + f'''
refused(lambda: os.open('m13-sentinel',os.O_RDONLY,dir_fd={leaked_dir}))
refused(lambda: os.chdir({leaked_dir}))
''', pass_fds=(leaked_dir,))
    finally:
        os.close(leaked_dir)
    for name, uid in [('sibling',uid_b),('legacy',legacy_uid),('same-uid-other-domain',uid_a)]:
        process = subprocess.Popen(['/usr/bin/python3','-c',
                                    f'import os,time; fd=os.open({str(system_sentinel)!r},os.O_RDONLY); print(fd,flush=True); time.sleep(300)'],
                                   user=uid, group=uid, extra_groups=[], stdout=subprocess.PIPE, text=True)
        victims.append(process)
        victim_fd = int(process.stdout.readline().strip())
        assert process.poll() is None
        check('signals and ptrace blocked: '+name, deny + f'''
import signal
for sig in [signal.SIGTERM,signal.SIGCONT]:
    refused(lambda: os.kill({process.pid},sig))
lib=ctypes.CDLL(None,use_errno=True)
assert lib.ptrace(16,{process.pid},None,None)==-1 and ctypes.get_errno() in [errno.EPERM,errno.EACCES]
refused(lambda: open('/proc/{process.pid}/environ').read())
refused(lambda: open('/proc/{process.pid}/mem','rb'))
refused(lambda: os.listdir('/proc/{process.pid}/fd'))
refused(lambda: open('/proc/{process.pid}/fd/{victim_fd}').read())
''')
    victim = victims[0]
    check('negative control: cross-UID SIGCONT in shared session', f'''
import os, signal
os.kill({victim.pid},signal.SIGCONT)
''', mode='uid-only-control')
    check('signals remain usable within the context', '''
import subprocess,signal
p=subprocess.Popen(['/bin/sleep','30'])
p.terminate()
assert p.wait(timeout=5) == -signal.SIGTERM
''')
    check('negative control: Python semaphores use shared shm despite TMPDIR', '''
import multiprocessing, os
s=multiprocessing.get_context('spawn').Semaphore(1)
path='/dev/shm/sem.'+s._semlock.name.lstrip('/')
assert os.stat(path).st_uid == os.getuid()
assert not path.startswith(os.environ['TMPDIR'])
''', mode='uid-only-control')
    abstract = socket.socket(socket.AF_UNIX)
    abstract.bind('\0nyxid-m13-outside')
    abstract.listen()
    check('abstract Unix socket outside domain denied', deny + '''
s=socket.socket(socket.AF_UNIX)
refused(lambda: s.connect('\\0nyxid-m13-outside'))
''')
    abstract.close()
    check('own Unix sockets remain usable', '''
import os,socket
for address in ['\\0nyxid-m13-inside',os.environ['TMPDIR']+'/own.sock']:
    listener=socket.socket(socket.AF_UNIX); listener.bind(address); listener.listen()
    client=socket.socket(socket.AF_UNIX); client.connect(address)
    server,_=listener.accept(); client.sendall(b'ok'); assert server.recv(2)==b'ok'
    server.close(); client.close(); listener.close()
''')
    # Pathname sockets are deliberately observed, not misrepresented as blocked:
    # Linux ABI <=8 does not mediate connect(2) to them (private dirs still do).
    public = socket.socket(socket.AF_UNIX)
    public.bind('/tmp/m13-public.sock'); os.chmod('/tmp/m13-public.sock',0o777); public.listen()
    probe = child("import socket; s=socket.socket(socket.AF_UNIX); s.connect('/tmp/m13-public.sock')")
    observations['allowed_by_design_public_pathname_unix_socket_connect'] = 'allowed' if probe.returncode == 0 else 'denied'
    public.close()
    # A sibling's private pathname socket remains protected by DAC.
    (b/'tmp').chmod(0o700)
    private_socket=socket.socket(socket.AF_UNIX)
    private_socket.bind(str(b/'tmp/private.sock')); os.chmod(b/'tmp/private.sock',0o777); private_socket.listen()
    check('private sibling Unix socket denied', deny+f'''
s=socket.socket(socket.AF_UNIX)
refused(lambda: s.connect({str(b/'tmp/private.sock')!r}))
''')
    private_socket.close()
    # Admission validates actual descriptor ownership and modes.
    (a/'tmp').chmod(0o777)
    assert child('pass').returncode == 125
    (a/'tmp').chmod(0o700)
    results.append('bad context permissions fail admission')
    os.rename(a/'tmp',a/'real-tmp'); os.symlink(a/'real-tmp',a/'tmp')
    assert child('pass').returncode == 125
    (a/'tmp').unlink(); os.rename(a/'real-tmp',a/'tmp')
    results.append('symlink context directory fails admission')
    if args.probes_only:
        print(json.dumps({'passed':len(results),'observations':observations,
                          'acceptance_gate':'separated_boundary_passed',
                          'tests':results}),flush=True)
        sys.exit(0)
    tool('git init, commit, branch, clone, merge and gc', r'''
git init -q source
cd source
git config user.email fixture@example.invalid
git config user.name Fixture
printf hello > file
git add file
git commit -qm initial
git checkout -qb work
printf more >> file
git commit -qam change
git checkout -q -
git merge -q work
git gc --prune=now
cd ..
git clone -q source clone
test "$(cat clone/file)" = hellomore
''')
    tool('Python venv, subprocess, multiprocessing and temporary files', r'''
python3 -m venv --without-pip pyenv
pyenv/bin/python -c 'import multiprocessing,subprocess,tempfile; p=multiprocessing.Process(target=print,args=("worker",)); p.start(); p.join(); assert p.exitcode==0; assert subprocess.check_output(["/bin/echo","ok"]).strip()==b"ok"; tempfile.TemporaryFile().write(b"ok")'
''')
    tool('Node and child process', r'''
node -e 'const fs=require("fs"),cp=require("child_process"); fs.writeFileSync("node-result","ok"); if(cp.execFileSync("node",["-e","process.stdout.write(\"ok\")"]).toString()!=="ok")process.exit(1)'
''')
    tool('Python multiprocessing.Pool with private semaphores', r'''
pyenv/bin/python -c 'from multiprocessing import Pool; p=Pool(2); assert p.map(abs,[-1,-2])==[1,2]; p.close(); p.join()'
''')
    tool('npm local package pack/install and module import', r'''
mkdir package app
printf '%s' '{"name":"fixture","version":"1.0.0","main":"index.js"}' > package/package.json
printf '%s' 'module.exports="ok"' > package/index.js
cd package
npm pack --offline --ignore-scripts
cd ../app
npm install --offline --no-audit --no-fund ../package/fixture-1.0.0.tgz
node -e 'if(require("fixture")!=="ok")process.exit(1)'
''')
    tool('C/C++ compiler and make', r'''
printf '%s\n' '#include <stdio.h>' 'int main(void){puts("ok");}' > hello.c
cc -Wall -Werror hello.c -o c-hello
test "$(./c-hello)" = ok
printf '%s\n' '#include <iostream>' 'int main(){std::cout<<"ok";}' > hello.cpp
c++ -Wall -Werror hello.cpp -o cpp-hello
test "$(./cpp-hello)" = ok
printf 'all:\n\t./c-hello\n' > Makefile
make
''')
    tool('Cargo offline build, test and run', r'''
cargo new --quiet --vcs none rust-project
cd rust-project
cargo test --offline
cargo run --offline
''')
    tool('pip local wheel install', r'''
pyenv/bin/python -m ensurepip --upgrade --default-pip
pyenv/bin/python - <<'MAKE_WHEEL'
import zipfile
with zipfile.ZipFile('fixture-1.0-py3-none-any.whl','w') as z:
    z.writestr('fixture.py','VALUE = "ok"')
    z.writestr('fixture-1.0.dist-info/METADATA','Metadata-Version: 2.1\nName: fixture\nVersion: 1.0\n')
    z.writestr('fixture-1.0.dist-info/WHEEL','Wheel-Version: 1.0\nGenerator: spike\nRoot-Is-Purelib: true\nTag: py3-none-any\n')
    z.writestr('fixture-1.0.dist-info/RECORD','')
MAKE_WHEEL
pyenv/bin/python -m pip install --no-index fixture-1.0-py3-none-any.whl
pyenv/bin/python -c 'import fixture; assert fixture.VALUE=="ok"'
''')
    # Persistent roots must remain unchanged after every adversarial attempt.
    assert all(p.read_text() == ('same-uid-outside-context' if p == outside_same else 'test-only-private-data') for p in paths)
    results.append('all outside sentinel contents intact')
    blocked = bool(tool_failures)
    assert set(observations.values()) == {'allowed'}, observations
    results.append('documented system metadata, shm and pathname-socket residual controls')
    print(json.dumps({'passed':len(results),'observations':observations,'tool_failures':tool_failures,
                      'acceptance_gate':'blocked' if blocked else 'separated_boundary_passed','tests':results}),flush=True)
    if blocked:
        sys.exit(3)
finally:
    print(json.dumps({'observations':observations}),flush=True)
    for victim in victims:
        victim.terminate()
        victim.wait(timeout=5)
