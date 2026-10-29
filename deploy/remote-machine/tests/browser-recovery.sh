#!/bin/sh
# Disposable real Chromium/cua/Node lifecycle checks; no live cluster or accounts.
set -eu
image=${1:-nyxid-machine:hosted-local}
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(CDPATH= cd -- "$script_dir/../../.." && pwd)
browser_test_dir=$(mktemp -d "${TMPDIR:-/tmp}/nyxid-browser-recovery.XXXXXX")
trap 'rm -rf "$browser_test_dir"' EXIT HUP INT TERM
architecture=$(docker image inspect --format '{{.Architecture}}' "$image")
case "$architecture" in
    arm64) wheel_arch=aarch64 ;;
    amd64) wheel_arch=x86_64 ;;
    *) echo 'Unsupported browser regression architecture.' >&2; exit 1 ;;
esac
python3 -m pip install --disable-pip-version-check --quiet --no-compile --timeout 15 --retries 1 \
    --only-binary=:all: --python-version 3.12 --implementation cp --abi cp312 --abi abi3 \
    --platform "manylinux_2_28_$wheel_arch" --platform "manylinux_2_17_$wheel_arch" --platform "manylinux2014_$wheel_arch" \
    --target "$browser_test_dir/deps" 'cryptography==46.0.5' 'cffi==2.0.0' 'pycparser==2.23'
python3 - "$image" "$script_dir" "$browser_test_dir" "$repo" <<'PY'
import json,os,pathlib,subprocess,sys,uuid
image,script_dir,browser_test_dir,repo=sys.argv[1:]
root=pathlib.Path(browser_test_dir)
results=root/'results'
results.mkdir()
failed=[]
supervisor_capabilities=json.loads((pathlib.Path(repo)/'machine/resources/docker-supervisor-capabilities.json').read_text())
manifest=json.loads((pathlib.Path(repo)/'deploy/remote-machine/seccomp/manifest.json').read_text())
seccomp=pathlib.Path(repo)/'deploy/remote-machine/seccomp'/pathlib.Path(manifest['localhost_profile']).name
for mode in ('shared_legacy','separated'):
    identity=uuid.uuid4().hex[:12]
    name='nyxid-browser-recovery-'+identity
    volume=name+'-contexts'
    subprocess.run(['docker','volume','create',volume],check=True,stdout=subprocess.DEVNULL)
    try:
        subprocess.run(['docker','run','--rm','--network','none','--read-only','--cap-drop','ALL',
            '--mount',f'type=volume,source={volume},target=/var/lib/nyxid-contexts,volume-nocopy',
            '--entrypoint','python3',image,'-c',"import os; os.makedirs('/var/lib/nyxid-contexts/legacy',mode=0o755,exist_ok=True)"],check=True)
        command=['docker','run','--name',name,'--network','none',
            '--cpus','2','--memory','2g','--memory-swap','2g','--shm-size','512m',
            '--cap-drop','ALL',
            *[argument for cap in supervisor_capabilities for argument in ('--cap-add',cap)],
            '--security-opt','seccomp='+str(seccomp),
            '--tmpfs','/var/lib/nyxid-machine:rw,nosuid,nodev,mode=0711,size=64m',
            '--tmpfs','/var/run/nyxid-workload:rw,nosuid,nodev,mode=0700,size=1m',
            '--tmpfs','/tmp:rw,nosuid,nodev,size=1g','--volume','/home',
            '--mount',f'type=volume,source={volume},target=/var/lib/nyxid-contexts,volume-nocopy',
            '--mount',f'type=volume,source={volume},target=/workspace,volume-subpath=legacy,volume-nocopy',
            '--mount',f'type=bind,source={script_dir}/full-machine-image-smoke.py,target=/smoke.py,readonly',
            '--mount',f'type=bind,source={script_dir}/browser-recovery.py,target=/browser-recovery.py,readonly',
            '--mount',f'type=bind,source={root}/deps,target=/test-deps,readonly',
            '--mount',f'type=bind,source={results},target=/smoke-results',
            '--env','PYTHONPATH=/test-deps','--env','PYTHONDONTWRITEBYTECODE=1','--env','CARGO_INCREMENTAL=1',
            '--env','NYXID_SMOKE_STORAGE_INITIALIZE=1','--env','NYXID_SMOKE_CAPABILITIES=shell,files,browser,computer',
            '--env','NYXID_SMOKE_BROWSER_RECOVERY='+mode,'--entrypoint','python3',image,'/smoke.py']
        with (results/(mode+'.log')).open('wb') as log:
            process=subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,timeout=600)
        report=results/('browser-recovery-'+mode+'.json')
        value=json.loads(report.read_text()) if report.exists() else {'mode':mode,'ok':False,'error':'fixture produced no result'}
        value['container_exit_code']=process.returncode
        print(json.dumps(value,sort_keys=True),flush=True)
        if not value.get('ok') or process.returncode:
            failed.append(mode)
        if destination:=os.environ.get('NYXID_SMOKE_RESULT_DIR'):
            target=pathlib.Path(destination)
            target.mkdir(parents=True,exist_ok=True)
            (target/report.name).write_text(json.dumps(value,indent=2)+'\n')
            (target/(mode+'.log')).write_bytes((results/(mode+'.log')).read_bytes())
    finally:
        subprocess.run(['docker','rm','--force','--volumes',name],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        subprocess.run(['docker','volume','rm',volume],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
if failed:
    raise SystemExit('browser recovery regressions failed: '+', '.join(failed))
PY
