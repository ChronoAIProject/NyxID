#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
profile="$root/../../cli/resources/remote-machine/fixed-machine.profile"
image="$root/../../cli/Dockerfile.machine-hosted"
entrypoint="$root/../../cli/container/hosted-entrypoint.sh"
grep -F 'profile_version = "remote-machine"' "$profile" >/dev/null
grep -F 'max_jobs = 2' "$profile" >/dev/null
grep -F 'workspace_size = "20Gi"' "$profile" >/dev/null
cmp "$profile" "$root/fixed-machine.profile"
if grep -E '^[[:space:]]*temporary_storage_size[[:space:]]*=' "$profile" >/dev/null; then
  printf '%s\n' 'current profile must not declare temporary capacity' >&2
  exit 1
fi
if grep -F 'ephemeral-storage:' "$root/limit-range.yaml" "$root/resource-quota.yaml" >/dev/null; then
  printf '%s\n' 'namespace policies must not inject or budget fixed temporary capacity' >&2
  exit 1
fi
grep -F 'memory: 2Gi' "$root/limit-range.yaml" >/dev/null
grep -F 'requests.memory: 200Gi' "$root/resource-quota.yaml" >/dev/null
grep -F 'requests.cpu: "200"' "$root/resource-quota.yaml" >/dev/null
grep -F 'persistentvolumeclaims: "100"' "$root/resource-quota.yaml" >/dev/null
grep -F 'ENTRYPOINT ["/usr/bin/tini"' "$image" >/dev/null
grep -F ' git gh python3 ' "$image" >/dev/null
grep -F 'cua-release.json' "$image" >/dev/null
grep -F 'nyxid-machine-desktop-start' "$image" >/dev/null
grep -F -- 'exec nyxid node managed-start' "$entrypoint" >/dev/null
grep -F -- 'nyxid-machine-desktop-start' "$entrypoint" >/dev/null
grep -F 'apiGroups: ["agents.x-k8s.io"]' "$root/manager-role.yaml" >/dev/null
grep -F 'resources: ["pods", "persistentvolumeclaims"]' "$root/manager-role.yaml" >/dev/null
grep -F 'verbs: ["get", "list", "watch"]' "$root/manager-role.yaml" >/dev/null
# StorageClass observations are cluster-wide reads, never provisioning rights.
python3 - "$root/identity-review-role.yaml" <<'PYROLE'
import json, re, sys
from pathlib import Path
role = Path(sys.argv[1]).read_text().split("---", 1)[0]
rules = re.split(r"(?m)^  - apiGroups: ", role)[1:]
storage = [rule for rule in rules if json.loads(rule.splitlines()[0]) == ["storage.k8s.io"]
           and json.loads(re.search(r"(?m)^    resources: (.+)$", rule).group(1)) == ["storageclasses"]]
assert len(storage) == 1, "one explicit StorageClass read rule required"
for field, expected in (("resources", ["storageclasses"]), ("verbs", ["get", "list"])):
    value = re.search(r"(?m)^    " + field + r": (.+)$", storage[0])
    assert value and json.loads(value.group(1)) == expected, "StorageClass rule must grant only get/list"
for rule in rules:
    resources = json.loads(re.search(r"(?m)^    resources: (.+)$", rule).group(1))
    assert not set(resources) & {"pods", "volumeattachments"}, "no cluster child/storage reads"
pv = [rule for rule in rules if json.loads(re.search(r"(?m)^    resources: (.+)$", rule).group(1)) == ["persistentvolumes"]]
assert len(pv) == 1 and json.loads(re.search(r"(?m)^    verbs: (.+)$", pv[0]).group(1)) == ["get"]
manager = Path(sys.argv[1]).with_name("manager-role.yaml").read_text()
for rule in re.split(r"(?m)^  - apiGroups: ", manager)[1:]:
    resources = json.loads(re.search(r"(?m)^    resources: (.+)$", rule).group(1))
    if set(resources) & {"pods", "persistentvolumeclaims"}:
        assert json.loads(re.search(r"(?m)^    verbs: (.+)$", rule).group(1)) == ["get", "list", "watch"]

PYROLE
grep -F 'failurePolicy: Fail' "$root/admission.yaml" >/dev/null
grep -F 'podTemplate.spec.automountServiceAccountToken == false' "$root/admission.yaml" >/dev/null
grep -F 'parameterNotFoundAction: Deny' "$root/admission.yaml" >/dev/null
grep -F "['CHOWN', 'SETUID', 'SETGID', 'KILL']" "$root/admission.yaml" >/dev/null
grep -F 'pod-security.kubernetes.io/enforce: baseline' "$root/namespace.yaml" >/dev/null
kubectl kustomize "$root" >/dev/null
# Evaluate the actual admission expressions against SandboxIntent::apply_object
# output for the single current profile and hostile variants. An optional first argument
# supplies freshly exported fixtures to coordinate Cargo builds with other suites.
# Dependencies live only in a disposable tool directory;
# the repository's Go/Rust dependency graphs are unchanged.
if command -v go >/dev/null 2>&1; then
  test_dir=$(mktemp -d "${TMPDIR:-/tmp}/nyxid-admission-cel.XXXXXX")
  trap 'rm -rf "$test_dir"' EXIT HUP INT TERM
  fixtures=${1:-}
  if [ -z "$fixtures" ]; then
    fixtures="$test_dir/sandbox-fixtures.json"
    cargo run --quiet --manifest-path "$root/../../Cargo.toml" -p nyxid-machine-updater --example sandbox_contracts >"$fixtures"
  fi
  test -s "$fixtures"
  # go run executes in its own directory, so make caller-supplied paths absolute.
  fixtures=$(CDPATH= cd -- "$(dirname "$fixtures")" && pwd)/$(basename "$fixtures")
  cat >"$test_dir/go.mod" <<'MOD'
module nyxid.local/admission-contract

go 1.23.0

require github.com/google/cel-go v0.26.1
MOD
  cat >"$test_dir/main.go" <<'GO'
package main

import (
  "encoding/json"
  "fmt"
  "os"
  "strings"

  "github.com/google/cel-go/cel"
  "github.com/google/cel-go/common/types"
)

type object = map[string]any
func clone(value object) object { b,_:=json.Marshal(value); var out object; if err:=json.Unmarshal(b,&out);err!=nil{panic(err)};return out }
func pod(v object) object{return v["spec"].(object)["podTemplate"].(object)["spec"].(object)}
func node(v object) object{return pod(v)["containers"].([]any)[0].(object)}
func envs(v object) []any{return node(v)["env"].([]any)}
func mounts(v object) []any{return node(v)["volumeMounts"].([]any)}
func volumes(v object) []any{return pod(v)["volumes"].([]any)}
func workspace(v object) object{return v["spec"].(object)["volumeClaimTemplates"].([]any)[0].(object)["spec"].(object)}
func main(){
 data,err:=os.ReadFile(os.Args[1]);if err!=nil{panic(err)}
 environment,err:=cel.NewEnv(cel.Variable("object",cel.DynType),cel.Variable("oldObject",cel.DynType),cel.Variable("params",cel.DynType));if err!=nil{panic(err)}
 programs:=[]cel.Program{}
 for _,line:=range strings.Split(string(data),"\n") {
  line=strings.TrimSpace(line)
  if !strings.HasPrefix(line,"- expression: "){continue}
  var expression string
  if err:=json.Unmarshal([]byte(strings.TrimPrefix(line,"- expression: ")),&expression);err!=nil{panic(err)}
  ast,issues:=environment.Compile(expression);if issues.Err()!=nil{panic(issues.Err())}
  program,err:=environment.Program(ast);if err!=nil{panic(err)}
  programs=append(programs,program)
 }
 if len(programs)<17 {panic("admission validators missing")}
 fixtureBytes,err:=os.ReadFile(os.Args[2]);if err!=nil{panic(err)}
 var fixtures map[string]object
 if err:=json.Unmarshal(fixtureBytes,&fixtures);err!=nil{panic(err)}
 fixture,retainedCurrent,defaultStorage:=fixtures["current"],fixtures["retained_current"],fixtures["current_default_storage"]
 if len(fixtures)!=3||fixture==nil||retainedCurrent==nil||defaultStorage==nil{panic("actual renderer must provide current, retained image, and default-storage fixtures")}
 if _,present:=workspace(defaultStorage)["storageClassName"];present{panic("actual default renderer must omit storageClassName entirely")}
 probe:=node(fixture)["startupProbe"].(object)
 parameters:=object{"data":object{"image":node(fixture)["image"],"runtime-class":"sandboxed","storage-class":"verified","bootstrap-base-url":"https://nyxid.example"}}
 totalCases:=0
 checkWithOld:=func(name string,v object,old any,want bool){
  totalCases++
  accepted:=true
  for index,p:=range programs {
   result,_,err:=p.Eval(map[string]any{"object":v,"oldObject":old,"params":parameters})
   if err!=nil||result!=types.True {accepted=false;if want{panic(fmt.Sprintf("valid fixture %s refused by validator %d: %v %v",name,index,result,err))}}
  }
  if accepted!=want {panic(fmt.Sprintf("admission mismatch: %s accepted=%v",name,accepted))}
 }
 check:=func(name string,v object,want bool){checkWithOld(name,v,nil,want)}
 check("current actual renderer",fixture,true)
 checkWithOld("current retained update",fixture,fixture,true)
 obsoleteCases:=0
 for _,profile:=range []string{"remote-shell-files-v1","remote-shell-files-v2"}{
  obsolete:=clone(fixture);obsolete["metadata"].(object)["labels"].(object)["nyxid.dev/profile"]=profile
  check("obsolete profile create",obsolete,false);obsoleteCases++
  checkWithOld("obsolete profile saved update",obsolete,obsolete,false);obsoleteCases++
  checkWithOld("old profile cannot become current",fixture,obsolete,false);obsoleteCases++
 }
 imageCases:=0
 for _,retained:=range []object{retainedCurrent}{
  if node(retained)["image"]==node(fixture)["image"]{panic("retained image fixture must differ from current approval")}
  previous:=clone(retained);previous["spec"].(object)["operatingMode"]="Suspended"
  checkWithOld("retained saved image resume",retained,previous,true);imageCases++
  check("retired image cannot create or recreate a Sandbox",retained,false);imageCases++
  replaced:=clone(retained);node(replaced)["image"]=node(fixture)["image"]
  checkWithOld("even approved replacement cannot rewrite saved image",replaced,previous,false);imageCases++
  replaced=clone(retained);node(replaced)["image"]="node@sha256:"+strings.Repeat("d",64)
  checkWithOld("unapproved replacement image",replaced,previous,false);imageCases++
  for _,label:=range []string{"nyxid.dev/managed","nyxid.dev/owner-id","nyxid.dev/machine-id","nyxid.dev/profile"}{
   foreign:=clone(previous);foreign["metadata"].(object)["labels"].(object)[label]="foreign"
   checkWithOld("retained image cannot reuse foreign "+label,retained,foreign,false);imageCases++
  }
  malformed:=clone(retained);node(malformed)["image"]="node:mutable"
  checkWithOld("retained digest validation remains mandatory",malformed,malformed,false);imageCases++
  missing:=clone(previous);delete(node(missing),"image")
  checkWithOld("prior image evidence required",retained,missing,false);imageCases++
  replaced=clone(retained);node(replaced)["image"]="foreign/"+node(retained)["image"].(string)
  checkWithOld("image repository is also immutable",replaced,previous,false);imageCases++
 }
 for _,shape:=range []object{fixture}{
  for _,resource:=range []string{"requests","limits"}{
   _,present:=node(shape)["resources"].(object)[resource].(object)["ephemeral-storage"]
   if present {panic("renderer temporary resource key does not match profile")}
  }
 }

 noClass:=clone(fixture);delete(pod(noClass),"runtimeClassName")
 check("explicit runtime requires field",noClass,false)
 parameters["data"].(object)["runtime-class"]=""
 check("default runtime omits field",noClass,true)
 check("default runtime rejects explicit field",fixture,false)
 emptyClass:=clone(noClass);pod(emptyClass)["runtimeClassName"]=""
 check("default runtime rejects empty field",emptyClass,false)
 delete(parameters["data"].(object),"runtime-class")
 check("absent optional parameter uses default",noClass,true)
 check("absent optional parameter rejects override",fixture,false)
 checkWithOld("retained explicit class under node-default deployment",fixture,fixture,true)
 parameters["data"].(object)["runtime-class"]="replacement-runtime"
 checkWithOld("retained class under changed deployment",fixture,fixture,true)
 checkWithOld("retained node-default under explicit deployment",noClass,noClass,true)
 classChanged:=clone(fixture);pod(classChanged)["runtimeClassName"]="replacement-runtime"
 checkWithOld("runtime class replacement rejected",classChanged,fixture,false)
 parameters["data"].(object)["runtime-class"]="sandboxed"
 storageCases:=0
 storageCheck:=func(name string,v object,old any,want bool){checkWithOld(name,v,old,want);storageCases++}
 storageCheck("explicit storage requires actual class field",defaultStorage,nil,false)
 for _,configured:=range []any{"",nil}{
  if configured==nil{delete(parameters["data"].(object),"storage-class")}else{parameters["data"].(object)["storage-class"]=configured}
  storageCheck("actual default renderer with optional storage setting",defaultStorage,nil,true)
  storageCheck("default setting rejects explicitly selected class",fixture,nil,false)
  for _,field:=range []any{"",nil,"verified"}{
   malformed:=clone(defaultStorage);workspace(malformed)["storageClassName"]=field
   storageCheck("default setting rejects any present class field",malformed,nil,false)
  }
  storageCheck("existing explicit PVC survives cluster default configuration",fixture,fixture,true)
  storageCheck("existing omitted PVC survives cluster default configuration",defaultStorage,defaultStorage,true)
 }
 parameters["data"].(object)["storage-class"]="replacement-storage"
 storageCheck("retained explicit PVC survives changed deployment",fixture,fixture,true)
 storageCheck("retained omitted PVC survives configured replacement",defaultStorage,defaultStorage,true)
 storageCheck("old explicit class cannot create under changed deployment",fixture,nil,false)
 storageCheck("omitted class cannot create under explicit deployment",defaultStorage,nil,false)
 for _,base:=range []object{fixture,defaultStorage}{
  previous:=clone(base);previous["spec"].(object)["operatingMode"]="Suspended"
  storageCheck("same exact PVC survives resume after configuration change",base,previous,true)
  for _,field:=range []any{"replacement-storage","",nil}{
   changed:=clone(base);workspace(changed)["storageClassName"]=field
   storageCheck("retained PVC class cannot be replaced, blanked, or nulled",changed,previous,false)
  }
  for _,field:=range []any{"",nil}{
   malformed:=clone(base);workspace(malformed)["storageClassName"]=field
   storageCheck("malformed old class cannot legitimize malformed new class",malformed,malformed,false)
  }
  renamed:=clone(base);renamed["spec"].(object)["volumeClaimTemplates"].([]any)[0].(object)["metadata"].(object)["name"]="replacement"
  storageCheck("retained PVC identity cannot change",renamed,previous,false)
 }
 omitted:=clone(fixture);delete(workspace(omitted),"storageClassName")
 storageCheck("existing explicit PVC cannot switch to cluster default",omitted,fixture,false)
 bound:=clone(defaultStorage);workspace(bound)["storageClassName"]="verified"
 storageCheck("existing omitted template cannot acquire resolved default",bound,defaultStorage,false)
 missingPrior:=clone(fixture);delete(missingPrior["spec"].(object),"volumeClaimTemplates")
 storageCheck("retained PVC needs exact prior evidence",fixture,missingPrior,false)
 parameters["data"].(object)["storage-class"]=nil
 storageCheck("null ConfigMap setting cannot authorize null class",defaultStorage,nil,false)
 parameters["data"].(object)["storage-class"]="verified"
 mutations:=map[string]func(object){
 "missing env":func(v object){node(v)["env"]=envs(v)[:5]},
 "duplicate env":func(v object){node(v)["env"]=append(envs(v),clone(envs(v)[0].(object)))},
 "duplicate replaces env":func(v object){envs(v)[5]=clone(envs(v)[0].(object))},
 "secret env source":func(v object){envs(v)[0].(object)["valueFrom"]=object{"secretKeyRef":object{"name":"leak","key":"token"}}},
 "alternate pod uid source":func(v object){envs(v)[2].(object)["valueFrom"].(object)["fieldRef"].(object)["fieldPath"]="metadata.name"},
 "foreign bootstrap":func(v object){envs(v)[5].(object)["value"]="https://evil.example/bootstrap"},
 "missing mount":func(v object){node(v)["volumeMounts"]=mounts(v)[:4]},
 "duplicate mount":func(v object){node(v)["volumeMounts"]=append(mounts(v),clone(mounts(v)[0].(object)))},
 "replaced mount":func(v object){mounts(v)[4]=clone(mounts(v)[0].(object))},
 "identity subpath":func(v object){mounts(v)[1].(object)["subPath"]="public"},
 "subpath expression":func(v object){mounts(v)[1].(object)["subPathExpr"]="$(PUBLIC)"},
 "propagating mount":func(v object){mounts(v)[1].(object)["mountPropagation"]="Bidirectional"},
 "writable token":func(v object){mounts(v)[2].(object)["readOnly"]=false},
 "missing volume":func(v object){pod(v)["volumes"]=volumes(v)[:3]},
 "duplicate volume":func(v object){pod(v)["volumes"]=append(volumes(v),clone(volumes(v)[0].(object)))},
 "replaced volume":func(v object){volumes(v)[3]=clone(volumes(v)[0].(object))},
 "disk identity":func(v object){volumes(v)[0].(object)["emptyDir"].(object)["medium"]=""},
 "token mode":func(v object){volumes(v)[1].(object)["projected"].(object)["defaultMode"]=420},
 "extra token source":func(v object){p:=volumes(v)[1].(object)["projected"].(object);p["sources"]=append(p["sources"].([]any),object{"secret":object{"name":"leak"}})},
 "root lifecycle":func(v object){node(v)["lifecycle"]=object{"postStart":object{"exec":object{"command":[]any{"sh","-c","bad"}}}}},
 "root probe":func(v object){node(v)["startupProbe"].(object)["exec"].(object)["command"]=[]any{"sh","-c","bad"}},
 "additional probe handler":func(v object){node(v)["startupProbe"].(object)["httpGet"]=object{"path":"/","port":80}},
 "new readiness command":func(v object){node(v)["readinessProbe"]=clone(probe)},
 "privileged":func(v object){node(v)["securityContext"].(object)["privileged"]=true},
 "unconfined apparmor":func(v object){node(v)["securityContext"].(object)["appArmorProfile"]=object{"type":"Unconfined"}},
 "unconfined seccomp":func(v object){node(v)["securityContext"].(object)["seccompProfile"].(object)["type"]="Unconfined"},
 "shared process namespace":func(v object){pod(v)["shareProcessNamespace"]=true},
 "pod supplemental identity":func(v object){pod(v)["securityContext"]=object{"fsGroup":1000}},
 "ephemeral root container":func(v object){pod(v)["ephemeralContainers"]=[]any{object{"name":"root"}}},
 "init root container":func(v object){pod(v)["initContainers"]=[]any{object{"name":"root"}}},
 "host namespace":func(v object){pod(v)["hostPID"]=true},
 "host network":func(v object){pod(v)["hostNetwork"]=true},
 "cpu widening":func(v object){node(v)["resources"].(object)["limits"].(object)["cpu"]="4"},
 "memory missing":func(v object){delete(node(v)["resources"].(object)["requests"].(object),"memory")},
 "memory widening":func(v object){node(v)["resources"].(object)["limits"].(object)["memory"]="4Gi"},
 "foreign image":func(v object){node(v)["image"]="other@sha256:"+strings.Repeat("b",64)},
 "foreign workspace class":func(v object){v["spec"].(object)["volumeClaimTemplates"].([]any)[0].(object)["spec"].(object)["storageClassName"]="other"},
 "workspace resizing":func(v object){v["spec"].(object)["volumeClaimTemplates"].([]any)[0].(object)["spec"].(object)["resources"].(object)["requests"].(object)["storage"]="40Gi"},
 "tmpfs temporary":func(v object){volumes(v)[2].(object)["emptyDir"].(object)["medium"]="Memory"},
 "home PVC":func(v object){volumes(v)[3]=object{"name":"home","persistentVolumeClaim":object{"claimName":"workspace"}}},
 "workspace extra label":func(v object){v["spec"].(object)["volumeClaimTemplates"].([]any)[0].(object)["metadata"].(object)["labels"].(object)["nyxid.dev/operation-id"]="old"},
 "workspace foreign owner":func(v object){v["spec"].(object)["volumeClaimTemplates"].([]any)[0].(object)["metadata"].(object)["labels"].(object)["nyxid.dev/owner-id"]="other"},
 "unknown profile":func(v object){v["metadata"].(object)["labels"].(object)["nyxid.dev/profile"]="unknown"},
 }
 for _,base:=range []object{fixture}{for name,change:=range mutations{v:=clone(base);change(v);check(name,v,false)}}
 for _,base:=range []object{retainedCurrent}{for name,change:=range mutations{v:=clone(base);change(v);checkWithOld("retained "+name,v,base,false)}}
 delete(parameters["data"].(object),"storage-class")
 for name,change:=range mutations{v:=clone(defaultStorage);change(v);check("default storage "+name,v,false)}
 parameters["data"].(object)["storage-class"]="replacement-storage"
 for name,change:=range mutations{v:=clone(defaultStorage);change(v);checkWithOld("retained default storage "+name,v,defaultStorage,false)}
 parameters["data"].(object)["storage-class"]="verified"
 temporaryCases:=0
 for _,base:=range []object{fixture}{
  for _,field:=range []string{"requests","limits","tmp","home"}{
   for _,value:=range []any{nil,"0","0Gi","","2Gi"}{
    v:=clone(base)
    var target object
    key:="ephemeral-storage"
    if field=="tmp"||field=="home" {
     index:=2;if field=="home"{index=3}
     target=volumes(v)[index].(object)["emptyDir"].(object);key="sizeLimit"
    }else{target=node(v)["resources"].(object)[field].(object)}
    target[key]=value
    check(fmt.Sprintf("temporary field %s value %v",field,value),v,false)
    temporaryCases++
   }
   v:=clone(base)
   if field=="tmp"||field=="home"{index:=2;if field=="home"{index=3};delete(volumes(v)[index].(object)["emptyDir"].(object),"sizeLimit")}else{delete(node(v)["resources"].(object)[field].(object),"ephemeral-storage")}
   check("removed temporary field "+field,v,true)
   temporaryCases++
  }
 }
 fmt.Printf("admission CEL: %d actual validators, 3 Rust-rendered objects, 1 current profile, %d obsolete-profile rejections, %d image-retention cases, 10 runtime-selection cases, %d storage-selection/retention cases, %d temporary-field cases and %d hostile variants passed (%d total admission cases)\n",len(programs),obsoleteCases,imageCases,storageCases,temporaryCases,len(mutations)*4,totalCases)
}
GO
  (cd "$test_dir" && GOWORK=off go mod tidy && GOWORK=off go run . "$root/admission.yaml" "$fixtures")
else
  printf '%s\n' 'admission CEL execution requires Go; structural render checks only' >&2
  exit 1
fi
printf '%s\n' 'remote-machine platform contracts: ok'
