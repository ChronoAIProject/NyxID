#!/bin/sh
# Current full image only. Disposable local fixtures and one temporary volume.
set -eu
image=${1:-nyxid-machine:hosted-local}
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(CDPATH= cd -- "$script_dir/../../.." && pwd)
test_dir=$(mktemp -d "${TMPDIR:-/tmp}/nyxid-full-machine-smoke.XXXXXX")
trap 'rm -rf "$test_dir"' EXIT HUP INT TERM
architecture=$(docker image inspect --format '{{.Architecture}}' "$image")
case "$architecture" in
    arm64) wheel_arch=aarch64 ;;
    amd64) wheel_arch=x86_64 ;;
    *) echo 'Unsupported smoke architecture.' >&2; exit 1 ;;
esac
python3 -m pip install --disable-pip-version-check --quiet --no-compile --timeout 15 --retries 1 \
    --only-binary=:all: --python-version 3.12 --implementation cp --abi cp312 --abi abi3 \
    --platform "manylinux_2_28_$wheel_arch" --platform "manylinux_2_17_$wheel_arch" --platform "manylinux2014_$wheel_arch" \
    --target "$test_dir/deps" 'cryptography==46.0.5' 'cffi==2.0.0' 'pycparser==2.23'
python3 - "$image" "$script_dir" "$test_dir" "$repo" <<'PY'
import atexit,datetime,json,pathlib,subprocess,sys,time,uuid
image,script_dir,test_dir,repo=sys.argv[1:]
root=pathlib.Path(test_dir)
output=root/'results'
output.mkdir()
all_results=[]
volume='nyxid-full-smoke-workspace-'+uuid.uuid4().hex[:12]
subprocess.run(['docker','volume','create',volume],check=True,stdout=subprocess.DEVNULL)
atexit.register(lambda: subprocess.run(['docker','volume','rm',volume],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL))
retention_marker=uuid.uuid4().hex
retained_node_id=str(uuid.uuid4())
for capabilities,generation in [('shell,files',1),('browser',1),('shell,files,browser,computer',1),('shell,files',2)]:
    case='all-capabilities' if 'computer' in capabilities and 'shell' in capabilities else 'browser-computer' if 'computer' in capabilities else 'browser-only' if 'browser' in capabilities else 'commands-files'
    if generation > 1:
        case+='-retained'
    name='nyxid-full-smoke-'+uuid.uuid4().hex[:12]
    command=['docker','run','--detach','--name',name,'--network','none','--read-only',
        '--cpus','2','--memory','2g','--memory-swap','2g',
        '--cap-drop','ALL','--cap-add','CHOWN','--cap-add','SETUID','--cap-add','SETGID','--cap-add','KILL',
        '--security-opt','no-new-privileges',
        '--security-opt','seccomp='+repo+'/cli/resources/machine-container/seccomp.json',
        '--tmpfs','/var/lib/nyxid-machine:rw,nosuid,nodev,mode=0711,size=64m',
        '--tmpfs','/var/run/nyxid-workload:rw,nosuid,nodev,mode=0700,size=1m',
        '--tmpfs','/tmp:rw,nosuid,nodev,size=1g',
        '--tmpfs','/home:rw,nosuid,nodev,mode=0755,size=1g',
        '--mount',f'type=bind,source={script_dir}/full-machine-image-smoke.py,target=/smoke.py,readonly',
        '--mount',f'type=bind,source={test_dir}/deps,target=/test-deps,readonly',
        '--mount',f'type=bind,source={output},target=/smoke-results',
        '--env','PYTHONPATH=/test-deps','--env','PYTHONDONTWRITEBYTECODE=1',
        '--env','NYXID_SMOKE_CAPABILITIES='+capabilities,
        '--entrypoint','python3',image,'/smoke.py']
    workspace=['--tmpfs','/workspace:rw,nosuid,nodev,size=32m']
    if capabilities=='shell,files':
        workspace=['--mount',f'type=volume,source={volume},target=/workspace,volume-nocopy',
            '--env',f'NYXID_SMOKE_GENERATION={generation}',
            '--env','NYXID_SMOKE_NODE_ID='+retained_node_id,
            '--env','NYXID_SMOKE_RETENTION_MARKER='+retention_marker]
    command[2:2]=workspace
    result=None
    try:
        subprocess.run(command,check=True,stdout=subprocess.DEVNULL)
        deadline=time.monotonic()+300
        while time.monotonic()<deadline:
            state=json.loads(subprocess.check_output(['docker','inspect',name]))[0]
            if not state['State']['Running']:
                break
            time.sleep(0.5)
        else:
            raise AssertionError(case+': container did not exit within 300 seconds')
        result_path=output/(case+'.json')
        if result_path.exists():
            result=json.loads(result_path.read_text())
        else:
            result={'case':case,'ok':False,'error':'fixture did not produce a result'}
        result['container']={key:state['State'][key] for key in ['Status','ExitCode','OOMKilled','Pid','StartedAt','FinishedAt']}
        result['image_id']=state['Image']
        result['enforced_limits']={'NanoCpus':state['HostConfig']['NanoCpus'],'Memory':state['HostConfig']['Memory'],'MemorySwap':state['HostConfig']['MemorySwap'],'Tmpfs':state['HostConfig']['Tmpfs']}
        if result.get('cutoff_ms'):
            finished=datetime.datetime.fromisoformat(state['State']['FinishedAt'].replace('Z','+00:00'))
            result['exit_after_cutoff_ms']=round(finished.timestamp()*1000-result['cutoff_ms'])
        all_results.append(result)
        # Logs stay local, are never part of the sanitized result, and must not
        # be treated as a substitute for assertions or native component tests.
        if not result.get('ok') or state['State']['ExitCode']!=0 or state['State']['OOMKilled']:
            log=subprocess.check_output(['docker','logs',name],stderr=subprocess.STDOUT)
            (root/(case+'.log')).write_bytes(log)
            print(json.dumps(result,sort_keys=True))
            if target:=__import__('os').environ.get('NYXID_SMOKE_RESULT_DIR'):
                destination=pathlib.Path(target);destination.mkdir(parents=True,exist_ok=True)
                (destination/(case+'.json')).write_text(json.dumps(result,indent=2)+'\n')
                (destination/(case+'.log')).write_bytes(log)
            continue
        assert state['State']['Pid']==0
        assert result.get('exit_after_cutoff_ms',30001)<30000
        result['checks'].append('signed_expiry_terminates_entire_container_PID_namespace')
        print(json.dumps(result,sort_keys=True))
        if target:=__import__('os').environ.get('NYXID_SMOKE_RESULT_DIR'):
            destination=pathlib.Path(target);destination.mkdir(parents=True,exist_ok=True)
            (destination/(case+'.json')).write_text(json.dumps(result,indent=2)+'\n')
    except Exception as error:
        result=result or {'case':case,'checks':[]}
        result['ok']=False
        result['error']=str(error)
        if result not in all_results:
            all_results.append(result)
        log=subprocess.run(['docker','logs',name],stdout=subprocess.PIPE,stderr=subprocess.STDOUT).stdout
        print(json.dumps(result,sort_keys=True))
        if target:=__import__('os').environ.get('NYXID_SMOKE_RESULT_DIR'):
            destination=pathlib.Path(target);destination.mkdir(parents=True,exist_ok=True)
            (destination/(case+'.json')).write_text(json.dumps(result,indent=2)+'\n')
            (destination/(case+'.log')).write_bytes(log)
    finally:
        subprocess.run(['docker','rm','--force','--volumes',name],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
failed=[result['case'] for result in all_results if not result.get('ok') or result.get('container',{}).get('ExitCode',1)!=0 or result.get('container',{}).get('OOMKilled',True)]
print(json.dumps({'cases':len(all_results),'passed':len(all_results)-len(failed),'failed':failed},sort_keys=True))
if failed:
    sys.exit(1)
print('full-machine-image functional smokes: 3 capability selections and 2-generation workspace retention passed')
PY
