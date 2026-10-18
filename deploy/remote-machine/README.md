# Remote Nyx Machine native MVP deployment contract

## Single current resource contract (2026-10-15)

The sole profile `remote-machine` fixes 2 vCPU, 2 GiB memory and a 20 GiB `/workspace` PVC. Product consent is `remote-machine-mvp`; individual deletion uses `remote-machine-data`, account deletion `machine-data`. The undeployed development variants have no aliases, compatibility templates, historical quote acceptance or migrations. Wire field names and existing API/signature/AAD/deadline/Billing protocol versions stay unchanged.

Temporary storage has no fixed capacity promise or surcharge. Pods omit requests/limits ephemeral-storage and home/tmp disk emptyDir sizeLimit entirely. Identity Memory/token bounds, security and network constraints remain. LimitRange/ResourceQuota do not inject or budget temporary allocations; unknown temporary demand is not unlimited. Selected-cluster defaults/admission, final Pod shape, pressure/eviction and CSI behavior require later acceptance.

CREATE must use the deployment-approved image. UPDATE retains the exact already admitted image on the same owner/Machine/profile and exact stable workspace template, even after deployment image A changes to B. It cannot upgrade A to B implicitly. Without prior Sandbox identity, saved database data cannot authorize recreating an unapproved image or replacing the original workspace. UID/resourceVersion guards remain mandatory.

Current optional StorageClass and hosted-image evidence is recorded in [HOSTED_STORAGE_VERIFICATION_20261015](../../docs/REMOTE_NYX_MACHINE_HOSTED_STORAGE_VERIFICATION_20261015.md). `docs/REMOTE_NYX_MACHINE_SINGLE_CURRENT_VERIFICATION_20261015.md` retains the preceding resource-contract phase; prior records retain their original bodies and historical labels. A changed Node must be rebuilt before runtime claims. Docker tmpfs and unbounded named-volume fixtures do not prove Kubernetes disk emptyDir or the 20 GiB PVC. No live deployment/mutation, commit or push is part of this cleanup.

This directory implements the owner’s 2026-10-14 complete Machine revision5 scope.
It contains source manifests and read-only deployment checks; local verification
has not deployed them to a selected cluster. Agent Sandbox remains the only
Pod/PVC controller. NyxID creates, patches, observes and deletes Sandbox objects
and reads their associated Pods, Nodes, PVCs and PVs.

The vendored CRD is pinned to Agent Sandbox commit
`86ad94178a85aa26788077a55894cdc234c5ea6a`; verify `upstream/SHA256` before use.
The controller source at that same commit defines native suspension in
`controllers/sandbox_controller.go::computeSuspendedCondition`:

- `spec.operatingMode: Suspended` requests suspension.
- `status.conditions[type=Suspended].status: True`, reason
  `PodTerminated`, reports the controller’s applied suspension only
  after its associated Pod is absent and its Pod lookup has not failed.
- `observedGeneration` is on each condition and must equal the current
  `metadata.generation`. There is no `status.observedGeneration` in this CRD.
- NyxID also checks the Sandbox UID, resource version, accepted operation and
  runtime tuple, associated Pod/UID and current Node observations. A successful
  PATCH or desired mode alone never completes pause.
- A Suspended intent removes NyxID’s SSA-owned `shutdownTime`. The pinned
  controller’s expiry path otherwise skips suspension-condition reconciliation;
  `operatingMode: Suspended` still forbids Pod creation. Running intents retain
  the committed funded or native lease cutoff.

`volumeClaimTemplates` is immutable in this pinned CRD. New workspace templates
use stable owner/Machine/profile labels, not changing runtime/operation labels.
UID-preconditioned updates preserve a previously created template only after
checking its exact fixed storage specification and stable ownership. Resume
must find the original PVC UID before submitting Running; a missing workspace
is a pending recovery condition, never permission to create a replacement disk.

The current `remote-machine` profile fixes 2 vCPU, 2 GiB memory and a
20 GiB workspace, with two aggregate commands and a 3600 second command
maximum. Temporary storage has no declared fixed capacity. The profile does not grant tools.
Creation selects shell/files/browser/computer (default shell/files); computer
requires browser. The sealed bootstrap carries the selected capabilities and
the Node persists them as an immutable configuration ceiling. Enabling a
capability does not authorize a client. There is no inbound Service, public
port or arbitrary user PodSpec. The supervisor retains only
`CHOWN`, `SETUID`, `SETGID`, `KILL`; commands clear supplementary groups, switch
to `agent` and set `no_new_privs`. Node credentials and the projected workload
identity token are root-only on separate protected mounts. The custom admission
policy and default-deny network policy retain these boundaries. Namespace
PodSecurity is `baseline`; this root supervisor does not claim the Kubernetes
`restricted` policy.

## Full image, startup and retained data

The hosted target is `cli/Dockerfile.machine-hosted`. It contains the full
Machine build and the pinned Heca Daemon / CMA Codex tools; the ordinary
`cli/Dockerfile.machine` remains its original standalone target. The hosted
entrypoint is `cli/container/hosted-entrypoint.sh`, with shared desktop scripts
and the managed healthcheck. See [the hosted Heca/CMA guide](../../docs/MACHINE_HECA_CMA_IMAGE.md)
for tool authentication and checks. Heca/Codex do not start or log in automatically.

Build directly from the repository root with the CMA named context:

```sh
docker build --file cli/Dockerfile.machine-hosted \
  --build-context cma="$HOME/Work/Project/aevatarAI/cma" \
  --tag nyxid-machine:hosted-local .
```

Builds normally use the official Debian mirrors. `DEBIAN_MIRROR` and
`DEBIAN_SECURITY_MIRROR` can select an alternate transport host while retaining
the Debian suites, archive keyring and APT signature/package-hash verification.
For an offline Cargo build, a disposable build snapshot may include only public
crates.io `cache/` and sparse `index/` data under
`cli/container/public-cargo-cache/` and pass `--build-arg NYXID_CARGO_OFFLINE=1`.
The image seeds the ordinary registry cache, writes the standard public registry
configuration, and runs `cargo build --locked --offline`; host Cargo config,
authentication files, source substitutions and prebuilt binaries are excluded.

Pin the resulting image by digest in the deployment configuration. The final
image contains the nyxid CLI and Node subcommand, gh, Chromium, Xvfb/Openbox,
and the checksum-pinned cua driver. It uses the existing Machine image's
installation and shared desktop startup, not the limited node-only helper image.
`NYXID_MANAGED_KUBERNETES=1` selects managed bootstrap and does not launch the
installed Heca/Codex tools, a CMA control plane, an Agent Server, or an updater. The two desktop sessions start only when the
sealed selection enables browser/computer; Node capabilities and operation
checks continue to enforce that selection. Secure Chromium and cua start through
the existing Machine runtime. Installed, started, authorized and functionally
verified are separate states; image installation alone proves none of the last
three.

Only `/workspace` is persistent. Mount the home emptyDir at `/home`, so the
startup can recreate separate `/home/agent`, `/home/browser` and
`/home/devbrowser` identities. Browser profiles under the protected desktop
state, downloads under the browser home, and `/tmp` are temporary. The Node
identity and workload token are never made readable to the command/browser
users. Managed workspace ownership changes only after the exact WSS runtime
acknowledgement. Signed local expiry shuts down the Node; Tini then exits and
the container PID namespace terminates all browser, desktop, driver and job
descendants. Pause/delete still require the native controller observations;
local expiry is not a partition-resistant provider stop guarantee.

The baseline Pod retains `RuntimeDefault` seccomp. Chromium's user-namespace
sandbox requires namespace syscalls that some runtimes (including Docker's
default seccomp) deny. The checked-in
`cli/resources/machine-container/seccomp.json` is the existing narrowly extended
Machine profile used by local Docker tests.
The profile allows `chroot` for Chromium's child user namespace; the kernel
still requires the corresponding namespace capability, and the container's
four-capability set does not include `SYS_CHROOT`. This preserves Chromium's
namespace sandbox without granting chroot in the original namespace.
Test the selected cluster runtime's
actual default behavior before advertising browser/computer readiness. A
cluster that needs a Localhost profile must install and select the exact
reviewed profile through its runtime configuration; these baseline manifests
do not claim that node installation or selection has happened. Do not disable
Chromium sandboxing or use an unconfined profile to make a smoke test pass.

`tests/full-machine-image-smoke.sh IMAGE` uses the actual final image and
entrypoint with disposable local bootstrap/WSS fixtures. It checks CLI/Node,
gh, Chromium and cua versions, signed Commands/Files, secure Chromium
navigation/snapshot, actual cua desktop capture/window observation, denied
capability requests, protected identities and signed-expiry container teardown.
The selections are Commands/Files, Browser alone, and all four capabilities;
the last selection executes a command while Chromium and cua remain active.
Independent selections still run after a failed case, preserving each result.
The fixtures prove Node-side protocol behavior, not live backend authorization
or Kubernetes acceptance. Containers enforce 2 CPU and 2 GiB memory; local
temporary mounts are `/home` and `/tmp` at 1 GiB tmpfs each, protected identity
at 64 MiB tmpfs and workload token at 1 MiB tmpfs. The container has no swap.
Browser cases use a 32 MiB tmpfs scratch workspace. Commands/Files cases use one disposable named Docker volume across
two managed generations and verify a signed file marker survives replacement.
This tests volume retention with unbounded capacity, not a 20 GiB quota. The cluster's 20 GiB workspace
capacity, actual namespace admission/defaulting, disk-backed temporary volume
behavior, node disk pressure and eviction remain selected-cluster acceptance
items. This Docker tmpfs fixture does not validate Kubernetes disk emptyDir
or temporary-storage capacity and is not proof of new Node telemetry unless
the image is rebuilt from the changed source. Browser profiles in memory-backed identity storage also consume the
same 2 GiB memory budget; do not silently increase the product limits after
startup failure, OOM or contention.

## Required cluster selection and configuration

Use an immutable hosted image digest, the pinned Sandbox API/controller and a
reachable NyxID HTTPS bootstrap endpoint. Admission requires the operator-created
ConfigMap `nyxid-machine-approved-runtime` with `image` and `bootstrap-base-url`.
`runtime-class` and `storage-class` are optional and must match backend selection:

- Runtime: set `runtime-class` and `NYXID_REMOTE_MACHINE_RUNTIME_CLASS` to an
  existing verified RuntimeClass, or omit/empty both to omit `runtimeClassName`
  and use the node default runtime.
- Storage: set `storage-class` and `NYXID_REMOTE_MACHINE_STORAGE_CLASS` to an
  existing confirmed StorageClass, or omit/empty both to omit the PVC template's
  `storageClassName` and use the **cluster default StorageClass**. Do not put an
  empty string or null in the PVC field: an empty string disables default-class
  provisioning. Kubernetes StorageClass defaults are cluster-scoped, not node-scoped.

Explicit class selection is read from the cluster and rejects missing, deleting
or invalid-provisioner objects. Default selection requires a discoverable usable
default; no default, unreadable API or missing permissions must surface an error
or pending state. Kubernetes still owns actual PVC defaulting and provisioning.
A StorageClass has no Ready condition: existence/configuration checks do not prove
that its CSI backend can provision or enforce 20 GiB. Record the actual provisioner,
volume binding/reclaim policy, finalizers and tested pause/resume/delete behavior.
Do not create a StorageClass, CSI or generic adapter merely for this MVP.

Existing Sandbox workspace templates and PVC identity remain authoritative on
resume, even if deployment settings or cluster defaults change. No missing or
failed retained volume may be replaced with an empty one. Class-scoped quota
demand is unknown until the relevant class can be established; unknown demand is
not free capacity. Namespace quota and fixed CPU/RAM/workspace policies remain.

Record each scheduled node’s actual `status.nodeInfo.containerRuntimeVersion`,
its default runtime handler and tested isolation limits. With a RuntimeClass,
record the selected handler as well. This repository has not selected or tested
a deployment runtime and does not claim VM isolation. Container resources,
privileges, networks and supervisor/command identity separation remain enforced
with either runtime selection.

Configure `NYXID_REMOTE_MACHINE_KUBE_API` as the actual target Kubernetes API
HTTPS origin, optional `NYXID_REMOTE_MACHINE_KUBE_CA_FILE` as a PEM trust root,
and `NYXID_REMOTE_MACHINE_BOOTSTRAP_BASE_URL` (or the server base URL). TLS chain
and hostname verification remain required. Never set the API to metadata.

Select backend authentication with `NYXID_REMOTE_MACHINE_KUBE_AUTH_MODE`:

- Unset or `token_file`: requires an absolute
  `NYXID_REMOTE_MACHINE_KUBE_TOKEN_FILE`. The protected regular file is read
  for every request, including projected-token symlink rotation. Empty, oversized,
  malformed, group/world-writable or unreadable files fail closed.
- `google_metadata`: no token file is required or consulted. The trusted backend
  directly calls `GET http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token`
  with `Metadata-Flavor: Google`. No production endpoint override, proxy,
  redirect, ADC chain, sidecar or token exporter is used.
- Blank, whitespace-padded or unknown modes are configuration errors. No mode
  falls back to another source or sends an unauthenticated Kubernetes request.

Both Sandbox manager and Kubernetes workload verifier use the same credential
source. A process-lived metadata cache survives reconstruction of either
client. Acquisition has a 1-second connect timeout, 3-second request timeout,
and 5-second total budget including refresh wait. Responses are bounded to
64 KiB, tokens to 16 KiB, and `expires_in` must be an integer from 1 to 86400
seconds. Expiry starts before the request; refresh starts `min(60 seconds,
10% of lifetime)` early. One refresh runs at a time; failures coalesce for one
second, then the next caller may retry. Failure during early refresh also fails
closed (no stale-token fallback). Secrets and credential paths are redacted;
Google tokens remain only in backend memory and outgoing Kubernetes Bearer
headers, never Machine/bootstrap material, logs, API results or persistence.

Configuration readiness is mode-aware: metadata mode has no missing-file
blocker. Configuration alone does not prove credentials or authorization.
Existing async provisioning/storage and operator probes acquire credentials
and exercise the actual API; unavailable metadata, TLS, 401/403 or inaccessible
resources remain unavailable/unknown through those paths. Existing untrusted
metadata/private-network URL and Machine egress restrictions are unchanged.

The backend Pod's Google identity (`chrono-nyxid` in the intended deployment)
is distinct from the Kubernetes ServiceAccount `nyxid-machine-manager` in the
example RBAC. That binding does **not** establish access for a Google principal.
Before environment acceptance, an operator must record the actual authenticated
principal, its least-privilege Sandbox writes and required read-only Pod/PVC,
direct Node/PV GET, StorageClass GET/LIST, TokenReview and diagnostic permissions,
API reachability, trusted CA and hostname. Inventory the permissions in the
existing manifests/probes against that principal. This change grants no IAM or
RBAC roles and does not authorize modifying the target environment. Real Google
metadata identity and target GKE authentication/authorization remain pending;
local HTTP/TLS doubles cannot establish them.

The workload ServiceAccount still has no RoleBinding. Its sole projected token
has audience `nyxid-remote-machine-v1`, mode 0400 and a 600-second lifetime.
This Pod-bound **Kubernetes** token authenticates managed bootstrap; the backend
Google access token authorizes the TokenReview HTTP request only. It never
substitutes for the workload token or its Pod/Sandbox UID, owner, runtime
generation, credential epoch, writer-fence and grant checks.

`managed-start --kubernetes` performs `/api/v1/machines/managed/prepare` and
`/api/v1/machines/managed/bootstrap` with that Pod-bound identity and a protected
recipient key. Both calls verify live Pod UID, Sandbox owner UID, runtime
generation, credential epoch and writer fence. A new controller-created Pod
must complete fresh controlled bootstrap and execution admission; it cannot
adopt old jobs. Pause preserves Machine identity and `/workspace`; resume
starts a fresh generation without old processes, memory or task continuation.

Managed credential delivery uses `nyxid_machine::bootstrap::Envelope` version 1,
independent of the client credential-injection protocol. Recipients publish a
canonical base64url-unpadded RSA-3072 SPKI public key (exponent 65537); the ticket
binds SHA-256 of its DER bytes. RSA-OAEP-SHA256/MGF1-SHA256, with label
`nyxid-machine-bootstrap-v1`, wraps a fresh AES-256-GCM key. The envelope carries
`version`, `wrapped_key`, `nonce` and `ciphertext` (tag appended). Authenticated
data is that domain, a NUL byte, the big-endian u32 version, and the complete
ordered `Binding` JSON. All existing Pod, generation, epoch, delivery and
writer-admission checks still apply. The replay row contains only ciphertext;
protected recipient keys persist across lost responses and are never replaced
merely because a request failed.

This undeployed MVP wire change requires a coordinated backend and hosted-image
upgrade. Old pending envelopes/recipient keys are incompatible and fail closed.
Recreate disposable test state; retained activated identities require the existing
controlled rekey/new-generation path. Do not delete an activated delivery or
replace its recipient to retry the same epoch, and do not delete the original
Machine/PVC to resolve a bootstrap error. Local tests cover replay, binding
mismatch, obsolete pending-key preservation and independent Python/Rust wire
interoperability; they do not establish real cluster workload authentication.

No `NYXID_REMOTE_MACHINE_PROVIDER_*` variable is required for ordinary native
create/run/pause/resume/delete. The pre-existing signed-provider library is
future platform material, not an unsigned-success substitute or a native-flow
prerequisite. Sandbox `shutdownTime` with `shutdownPolicy: Retain` and NyxID’s
funded runtime/credential leases provide the MVP’s existing deadline behavior.
Paid admission and renewal still require atomically reserved exact funds and
accepted rates. Funds exhaustion pauses; top-up requires a manual resume.

## Fault and deletion boundaries

Native observations do not establish an independent physical stop bound during
control-plane/node partition or prove that an old instance has lost every
storage write capability. For ordinary running and new-writer admission, known
force deletion, unhealthy/unreachable nodes, stale observations, UID or ownership
conflicts, and indeterminate predecessors retain their guards and block new writers. A forced
Pod deletion followed by NotFound cannot authorize a successor. Use the existing
retry/operator handling path after investigating the underlying resource; do
not clear the guard by inventing a stop timestamp.

Delete immediately revokes access and retains its original owner-authorized
operation. Completion uses original-identity-bound Kubernetes cleanup results
from the normal Agent Sandbox/Kubernetes lifecycle: applicable persisted
control-plane workload evidence, original Sandbox/Pod/PVC and bound-PV release,
complete related Pod/claim/PV/VolumeAttachment inventories, terminal NyxID
creation dispatches and accounting. Pending claims do not need to become Bound.
Direct runtime history, CSI request-terminal logs and GCE disk facts are not
required; no direct GCE/CSI/CRI API or cloud credential permission is added.
Present resources, finalizers, identity conflicts, claim users, attachments and
Retain/unknown policy keep their cleanup responsibility and relevant quota.
Normal API release is not a physical shutdown, never-provisioned or
cloud-erasure claim. Cleanup and accounting retry separately, while final
deleted state, operation success and slot release wait for both. Unknown runtime
periods are not universally charged, and later top-ups do not fund uncapped
residual costs.

Initial absence of the original Sandbox and Pods can establish
`ControlPlaneWorkloadAbsent` after complete original-scope reads, verified
original identities and terminal NyxID creation work; one 404 is insufficient.
Its actual observation time is the accounting upper bound, with generation zero
and empty resourceVersion representing no live object. Paid settlement rechecks
original-work terminality. Legacy cloud disk handles remain discovery hints and
do not require external proof when Kubernetes cleanup is complete; actual
Kubernetes PV/claim/CSI identity conflicts still block.

Finalized Machine tombstones also receive bounded, fair continued scans under
that same original delete authorization, current evidence version and persisted
control-plane workload-evidence gate, reusing the same cleanup orchestration.
Each pass handles at most eight tombstones with four concurrent scans and a
45-second per-scan timeout. Late original Sandbox/Pod/PVC cleanup uses retained
UID, owner and claim bindings; Pod DELETE also requires the full original tuple
and UID/resourceVersion. PVs wait for normal GC/reclaim and receive no direct
DELETE. Read failures, incomplete inventories and conflicting or replacement
resources remain blockers. The Machine stays deleted and the original operation
stays Succeeded; cleanup progress becomes pending while a late resource or
blocker remains and returns to complete after inventories clear. These scans do
not reopen execution, rerun settlement or move funds or slots again. The
existing investigation inventory is a read-only diagnostic view, not the cleanup
scanner.

See [deletion recovery](../../docs/REMOTE_MACHINE_DELETE_RECOVERY.md) for the
versioned evidence and original-operation recovery contract.

## Existing investigation entry and uncertainty runbook

Use the Machine detail and operation views to retain the Machine ID, accepted
operation ID, current state, stop reason and recent error. A first-party human
with admin/operator access can read
`GET /api/v1/admin/remote-machines/platform`; its paginated `investigations`
section exposes resource identity, reason, first/last observation and overdue
status. `inventory_cursor` and `inventory_limit` page existing records. This is
a read-only investigation entry, not a repair or quarantine-clear endpoint.

1. Inspect the exact Sandbox, associated and previously bound Pod, PVC and PV
   identities through read-only Kubernetes API access. Compare UIDs, owner
   references, resource versions, current condition generations, the accepted
   operation and Node readiness/heartbeat. Record known force deletion, node
   loss, finalizers and the selected CSI's actual reclaim policy. A resource's
   disappearance does not resolve an uncertain predecessor or retained volume.
2. Keep owner access revoked while stop or cleanup is pending. The existing
   owner pause/delete actions and their required confirmation remain available
   where lifecycle state permits them. Retry an accepted request with its same
   idempotency key and body, or allow the bounded background retry to continue;
   the owner operation history remains the authority for the accepted action.
   Retry does not authorize a new writer or erase the original cleanup record.
3. Repair ordinary API connectivity, required read permissions or controller
   availability through the cluster's normal operational process. Such repair
   can allow pending observations to progress. It does not automatically clear
   a persisted `native_uncertainty` latch. The admin view, owner action routes
   and background retries provide no successful manual-clear API.
4. For an accepted delete with an explicitly supported uncertainty reason,
   allow its private recovery path to validate the exact Sandbox controller
   Suspended observation, or known original Sandbox/Pod absence through complete
   scoped Kubernetes reads and terminal NyxID creation work, plus normal cleanup. Keep the latch and
   investigation record. Missing runtime history, CSI terminal logs or GCE
   facts do not block this delete contract. Known force deletion, unsupported
   reasons, identity conflicts, claim users, attachments, Retain and incomplete
   Kubernetes queries still retain their responsibilities. Do not clear the
   latch in MongoDB, replace the workspace, remove protection finalizers or
   treat a lone NotFound as complete evidence. No prior stop record is mandatory
   for a valid ControlPlaneWorkloadAbsent result; it retains the actual first
   complete-query observation bound, generation zero and empty resourceVersion
   without claiming a Suspended object or physical-stop time.
5. Manual recovery cannot reopen a writer without validated evidence from the
   selected runtime establishing that the predecessor is safe to replace and
   that the original workspace binding remains authoritative. This MVP does
   not supply a general evidence adapter or a quarantine-clear operation.
   Cases requiring such recovery remain pending for a separately reviewed,
   environment-specific recovery procedure; healthy replacement Pods or
   operator acknowledgement alone are insufficient. Keep the investigation
   and cleanup records until that procedure and its acceptance are complete.

Native quarantine also appears in this existing inventory as `native_runtime_uncertainty`, correlated to the exact Machine/runtime/Sandbox. Its detailed cause stays on the Machine stop reason. A lost diagnostic append is retried by the existing Manager sweep even when Kubernetes is unavailable; repeated observations reuse one investigation row.

## Deployment checks and optional diagnostics

`healthcheck.sh` requires `kubectl` and Python 3 (standard library only) on the
operator machine; Python is not an extra workload service or Pod dependency.
It performs read-only API discovery, required RBAC, admission,
quota/limit configuration, explicit or cluster-default StorageClass selection
and optional RuntimeClass existence checks. Missing provider configuration, RuntimeClass,
kubelet monitor or detailed Deployment selection does not make it fail.
Storage selection uses only `NYXID_REMOTE_MACHINE_STORAGE_CLASS`, matching the
backend; an old `NYXID_MACHINE_STORAGE_CLASS` value cannot override default
selection. The existing operator-only `NYXID_MACHINE_RUNTIME_CLASS` alias remains;
prefer the backend’s `NYXID_REMOTE_MACHINE_RUNTIME_CLASS` variable.
Set `NYXID_REMOTE_MACHINE_CHECK_CONTROLLER=1` only when explicitly
requesting additional controller Deployment/rollout diagnostics, with
`NYXID_REMOTE_MACHINE_CONTROLLER_NAMESPACE` and
`NYXID_REMOTE_MACHINE_CONTROLLER_DEPLOYMENT` configured.

`render-observer-rbac.py` is an optional offline renderer for that exact
Deployment observer. Its optional `--telemetry-node` selections add only exact
`nodes/stats` GET permissions; the baseline manifests do not need them. Detailed
Deployment/image/compatibility views are operator tools, not ordinary lifecycle
admission gates. The owner quota view remains conservative: namespace quotas
are paginated, exact quantities are aggregated, and missing or ambiguous usage
is unknown rather than fabricated headroom.

Workspace capacity/usage comes from the bounded Node workspace filesystem
report, preserving source and time. Full-disk errors remain visible; remove
unneeded workspace files and retry. Pausing retains the disk and its contents;
it does not free workspace space or automatically delete it.

Whole-Pod ephemeral metrics are optional. Prefer an existing cluster monitoring
source; without a usable whole-Pod observation the UI reports unknown or
unavailable, not zero. Workspace usage or a few directories must never stand in
for whole-Pod ephemeral usage. The existing opt-in direct kubelet collector may
be retained by an operator who already has that monitoring path. Its separate
`NYXID_REMOTE_MACHINE_KUBELET_ENDPOINTS`, `..._CA_FILE`, `..._TOKEN_FILE` and
`..._AUDIENCE` are not baseline deployment requirements. That collector binds
live Pod UID/runtime/container starts, excludes PVC and Memory emptyDir usage,
checks each source timestamp and does not double-count Pod totals. It never
authorizes lifecycle or billing.

## Verification boundaries

`tests/contracts.sh` exports current objects from the Rust
`sandbox_contracts` example using `SandboxIntent::apply_object`, executes the
actual CEL admission expressions against them, and renders Kustomize locally.
It checks the four absent temporary-storage fields, immutable retained
identity and distinct saved image digests, rejection of image replacements
and unapproved new allocations, configured and omitted RuntimeClass cases,
explicit/default StorageClass omission and retained-template selection,
and hostile template mutations for creates and retained updates. To coordinate builds, first run
`cargo run --quiet -p nyxid-machine-updater --example sandbox_contracts > /tmp/nyxid-sandbox-contracts.json`,
then pass that fresh output to `tests/contracts.sh /tmp/nyxid-sandbox-contracts.json`.
`tests/healthcheck-contracts.py` drives the actual
healthcheck with an explicitly declared local kubectl double; it checks command
and failure behavior without contacting a cluster. Rust adapter tests use real
parsing/HTTP clients against local Kubernetes API fixtures; those fixtures are
not a Kubernetes controller, scheduler, runtime or CSI. Image startup smokes use
throwaway isolated containers and protocol fixtures and are reported separately
from cluster acceptance.

Still required on the selected cluster: actual API/controller generation and
suspension transitions, healthy and partitioned node behavior, Pod-bound
bootstrap/WSS admission, default or selected runtime limits, same-PVC pause and
resume, Kubernetes PV reclaim/finalizer/Retain handling, delete recovery with
complete inventories, finalized-tombstone late-resource scans and original
dispatch/accounting gates, workspace/full-disk
reporting, optional monitoring provenance, real Billing funding integration and
actual external-client journeys. Hosted-image/storage phase results and source
provenance belong in `docs/REMOTE_NYX_MACHINE_HOSTED_STORAGE_VERIFICATION_20261015.md`;
the preceding single-current phase retains its own dated verification record.

Explicitly deferred platform work: independent partition-resistant stop bounds,
cross-runtime deadline enforcement, strong predecessor storage exclusion and
physical cloud/CSI cleanup proof. None is a claimed property of native API
observations or a baseline MVP provider configuration prerequisite.

Hosted-image/storage phase evidence: [hosted image and optional storage checks](../../docs/REMOTE_NYX_MACHINE_HOSTED_STORAGE_VERIFICATION_20261015.md). Earlier dated results remain stage-specific history.
