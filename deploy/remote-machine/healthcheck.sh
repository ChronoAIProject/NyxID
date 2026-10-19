#!/bin/sh
# Read-only native API/admission deployment check. It never creates resources
# or establishes physical stop/isolation. Optional controller diagnostics run
# only when explicitly requested; no provider or kubelet credential is needed.
set -eu
namespace="${NYXID_MACHINE_NAMESPACE:-nyxid-machine}"
manager="${NYXID_MACHINE_MANAGER_SA:-system:serviceaccount:${namespace}:nyxid-machine-manager}"
for resource in sandboxes.agents.x-k8s.io pods persistentvolumeclaims resourcequotas; do
  kubectl auth can-i --as="$manager" get "$resource" -n "$namespace" >/dev/null
  kubectl auth can-i --as="$manager" list "$resource" -n "$namespace" >/dev/null
done
kubectl auth can-i --as="$manager" create sandboxes.agents.x-k8s.io -n "$namespace" >/dev/null
kubectl auth can-i --as="$manager" patch sandboxes.agents.x-k8s.io -n "$namespace" >/dev/null
kubectl auth can-i --as="$manager" delete sandboxes.agents.x-k8s.io -n "$namespace" >/dev/null
# Recovery may delete exact residual children in this namespace. The API uses
# original UID/resourceVersion guards; child creation/patch/exec stay forbidden.
for resource in pods persistentvolumeclaims; do
  kubectl auth can-i --as="$manager" delete "$resource" -n "$namespace" >/dev/null
done
for action in 'create pods' 'create persistentvolumeclaims' 'patch pods' 'patch persistentvolumeclaims' 'create pods/exec'; do
  if kubectl auth can-i --as="$manager" $action -n "$namespace" >/dev/null; then
    echo manager_has_forbidden_workload_permissions >&2
    exit 2
  fi
done
kubectl auth can-i --as="$manager" create tokenreviews.authentication.k8s.io >/dev/null
kubectl auth can-i --as="$manager" get nodes >/dev/null
kubectl auth can-i --as="$manager" get persistentvolumes >/dev/null
kubectl auth can-i --as="$manager" list persistentvolumes >/dev/null
kubectl auth can-i --as="$manager" list pods --all-namespaces >/dev/null
kubectl auth can-i --as="$manager" get leases.coordination.k8s.io -n kube-node-lease >/dev/null
for verb in get list; do
  kubectl auth can-i --as="$manager" "$verb" volumeattachments.storage.k8s.io >/dev/null
done
for verb in get list; do
  kubectl auth can-i --as="$manager" "$verb" storageclasses.storage.k8s.io >/dev/null || {
    echo storageclass_read_permission_unverified >&2; exit 2;
  }
done
kubectl get crd sandboxes.agents.x-k8s.io >/dev/null
kubectl get --raw /apis/agents.x-k8s.io/v1beta1 >/dev/null
kubectl get resourcequotas --as="$manager" -n "$namespace" >/dev/null
kubectl get limitrange remote-machine-defaults -n "$namespace" >/dev/null
kubectl get configmap nyxid-machine-approved-runtime -n "$namespace" >/dev/null
kubectl get validatingadmissionpolicybinding nyxid-machine-sandbox-contract >/dev/null
warnings=$(kubectl get validatingadmissionpolicy nyxid-machine-sandbox-contract -o jsonpath='{.status.typeChecking.expressionWarnings}')
[ -z "$warnings" ] || [ "$warnings" = '[]' ] || { echo admission_typecheck_failed >&2; exit 2; }
runtime_class="${NYXID_REMOTE_MACHINE_RUNTIME_CLASS:-${NYXID_MACHINE_RUNTIME_CLASS:-}}"
storage_class="${NYXID_REMOTE_MACHINE_STORAGE_CLASS:-}"
if [ -n "$runtime_class" ]; then
  kubectl get runtimeclass "$runtime_class" >/dev/null || { echo runtimeclass_unverified >&2; exit 2; }
fi
# Validate a real API object using only the manager's read permissions. A
# StorageClass has no Ready condition: this does not establish CSI capacity or
# successful provisioning. Python 3 is used only for JSON/name/time validation.
command -v python3 >/dev/null 2>&1 || { echo storageclass_validation_requires_python3 >&2; exit 2; }
# Match the backend: trim configured names and treat whitespace-only as unset.
storage_class=$(python3 -c 'import sys; print(sys.argv[1].strip())' "$storage_class")
if [ -n "$storage_class" ]; then
  storage_error=storageclass_unverified
  storage_document=$(kubectl get storageclass "$storage_class" --as="$manager" -o json) || {
    echo "$storage_error" >&2; exit 2;
  }
else
  storage_error=storageclass_default_unverified
  storage_document=$(kubectl get storageclasses --as="$manager" --chunk-size=100 -o json) || {
    echo "$storage_error" >&2; exit 2;
  }
fi
printf '%s' "$storage_document" | python3 -c '
import datetime, json, re, sys

def dns_name(value):
    return (isinstance(value, str) and 0 < len(value) <= 253
            and all(len(part) <= 63 and re.fullmatch(r"[a-z0-9](?:[-a-z0-9]*[a-z0-9])?", part)
                    for part in value.split(".")))

def qualified_name(value):
    if not isinstance(value, str):
        return False
    parts = value.split("/")
    return (len(parts) in (1, 2) and (len(parts) == 1 or dns_name(parts[0]))
            and 0 < len(parts[-1]) <= 63
            and re.fullmatch(r"[A-Za-z0-9](?:[A-Za-z0-9_.-]*[A-Za-z0-9])?", parts[-1]))

def order(value):
    metadata = value["metadata"]
    timestamp = metadata["creationTimestamp"]
    if not isinstance(timestamp, str) or not re.fullmatch(
            r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})", timestamp):
        raise ValueError("unverified creation time")
    instant = datetime.datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
    return (-instant.timestamp(), metadata["name"])

try:
    document = json.load(sys.stdin)
    selected = sys.argv[1]
    if selected:
        storage = document
    else:
        candidates = [item for item in document["items"] if any(
            item.get("metadata", {}).get("annotations", {}).get(key) == "true"
            for key in ("storageclass.kubernetes.io/is-default-class",
                        "storageclass.beta.kubernetes.io/is-default-class"))]
        storage = sorted(candidates, key=order)[0]
    metadata = storage["metadata"]
    if not (dns_name(metadata["name"]) and (not selected or metadata["name"] == selected)
            and isinstance(metadata.get("uid"), str) and metadata["uid"]
            and metadata.get("deletionTimestamp") is None
            and qualified_name(storage.get("provisioner"))):
        raise ValueError("unverified StorageClass")
except (AttributeError, KeyError, IndexError, TypeError, ValueError, OverflowError):
    sys.exit(1)
' "$storage_class" || { echo "$storage_error" >&2; exit 2; }

# A detailed controller Deployment selection is an operator tool, not an
# ordinary create/run/pause/resume dependency.
if [ "${NYXID_REMOTE_MACHINE_CHECK_CONTROLLER:-0}" = 1 ]; then
  controller_namespace="${NYXID_REMOTE_MACHINE_CONTROLLER_NAMESPACE:-}"
  controller_deployment="${NYXID_REMOTE_MACHINE_CONTROLLER_DEPLOYMENT:-}"
  [ -n "$controller_namespace" ] && [ -n "$controller_deployment" ] || { echo controller_diagnostic_selection_required >&2; exit 2; }
  kubectl auth can-i --as="$manager" get deployment/"$controller_deployment" -n "$controller_namespace" >/dev/null
  kubectl get deployment/"$controller_deployment" --as="$manager" -n "$controller_namespace" >/dev/null
  kubectl rollout status deployment/"$controller_deployment" -n "$controller_namespace" --timeout=10s >/dev/null
fi
printf '%s\n' native_api_contract_healthy_selected_cluster_acceptance_still_required
