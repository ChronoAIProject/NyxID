import datetime, hashlib, json, os, pathlib, resource, shutil, signal, subprocess, sys, time
root = pathlib.Path('/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/nyxid-artifacts')
config = '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml'
label = sys.argv[1]
args = sys.argv[2:]
if not args:
    args = ['cargo', '--config', config, 'test', '--manifest-path', 'backend/Cargo.toml', 'generated_artifacts', '--no-run']
stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
base = pathlib.Path('/tmp') / ('nyxid-artifacts-2037-' + label + '-' + stamp)
log = pathlib.Path(str(base) + '.log')
meta = pathlib.Path(str(base) + '.json')
argv_log = pathlib.Path(str(base) + '.argv.log')
env = os.environ.copy()
env.update(CARGO_TARGET_DIR='/Users/chronoai/Desktop/projects/nyxid/target', CARGO_BUILD_JOBS='2', NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:29237/?replicaSet=artifacts2037', RUST_TEST_THREADS='1')
head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
tracked = subprocess.check_output(['git', 'ls-files', 'backend', 'Cargo.toml', 'Cargo.lock', '.cargo'], cwd=root, text=True).splitlines()
digest = hashlib.sha256()
for item in tracked:
    path = root / item
    if path.is_file():
        digest.update(item.encode() + b'\0' + path.read_bytes() + b'\0')
info = dict(label=label, started_utc=stamp, cwd=str(root), source_head=head, source_digest_sha256=digest.hexdigest(), argv=args, env={k:env[k] for k in ['CARGO_TARGET_DIR','CARGO_BUILD_JOBS','NYXID_TEST_DATABASE_URL','RUST_TEST_THREADS']}, nofile=list(resource.getrlimit(resource.RLIMIT_NOFILE)), log=str(log), argv_log=str(argv_log), minimum_free_bytes=2*1024**3, disk_free_before=shutil.disk_usage(root).free)
meta.write_text(json.dumps(info, indent=2) + '\n')
print('COMMAND ' + json.dumps(args), flush=True)
print('METADATA ' + str(meta), flush=True)
print('LOG ' + str(log), flush=True)
if info['disk_free_before'] < info['minimum_free_bytes']:
    raise SystemExit('Disk guard refused start below 2 GiB')
seen = set()
with log.open('w') as output, argv_log.open('w') as actual:
    proc = subprocess.Popen(args, cwd=root, env=env, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
    info['pid'] = proc.pid
    meta.write_text(json.dumps(info, indent=2) + '\n')
    print('OWNED_PROCESS_GROUP ' + str(proc.pid), flush=True)
    stopped = False
    while proc.poll() is None:
        free = shutil.disk_usage(root).free
        info['lowest_free_bytes'] = min(info.get('lowest_free_bytes', free), free)
        try:
            lines = subprocess.check_output(['ps','-axo','pid=,ppid=,pgid=,command='], text=True).splitlines()
            for line in lines:
                fields = line.strip().split(None, 3)
                if len(fields) == 4 and fields[2] == str(proc.pid) and line not in seen:
                    seen.add(line)
                    actual.write(datetime.datetime.now(datetime.timezone.utc).isoformat() + ' ' + line.strip() + '\n')
                    actual.flush()
        except subprocess.CalledProcessError:
            pass
        if free < info['minimum_free_bytes']:
            stopped = True
            print('DISK_GUARD_STOP free_bytes=' + str(free), flush=True)
            os.killpg(proc.pid, signal.SIGTERM)
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
            break
        time.sleep(2)
    code = proc.wait()
info.update(exit_code=code, disk_guard_stopped=stopped, finished_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), disk_free_after=shutil.disk_usage(root).free)
meta.write_text(json.dumps(info, indent=2) + '\n')
print('EXIT_CODE ' + str(code), flush=True)
print('DISK_FREE_BYTES ' + str(info['disk_free_after']), flush=True)
print('LOG_TAIL', flush=True)
print('\n'.join(log.read_text(errors='replace').splitlines()[-100:]), flush=True)
sys.exit(code if code >= 0 else 128 - code)
