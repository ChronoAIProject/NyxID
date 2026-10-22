# Remote Machine deletion recovery

Deletion recovery reuses the accepted owner delete operation and the Manager's bounded observation pass. It retains `native_uncertainty` and the investigation record, keeps execution disabled, and binds each persisted evaluation to the original owner basis, operation, runtime generation, credential epoch, complete writer fence, revision and actual resource identities. An owner can renew an exhausted retry budget through the manual retry endpoint; no owner evidence upload is accepted.

The current contract is Sandbox-cascade evidence version 2. Manager suspends and deletes only the original Sandbox; Agent Sandbox controller manages workloads, Kubernetes GC reclaims correctly owned Pod/PVC children, and storage controllers/CSI handle underlying storage according to reclaim policy. A completed delete means applicable control-plane stop/absence evidence, original Sandbox and known namespace child absence, terminal NyxID creation responsibilities and accounting checks passed. It does not assert a physical shutdown instant, PV/VolumeAttachment absence, disk detach or disk erasure. Manager does not query storage inventories or directly delete Pod/PVC objects.

## Durable creation responsibility

Running submissions use the existing outbox and a bounded per-dispatch journal. Each external dispatch records its responsibility before I/O; a timeout or worker-lease expiry leaves that responsibility outstanding. Late results are retained before ordinary runtime authority is rejected, and a newer successful retry cannot hide an older unresolved request. An Applied delivery or a bound Sandbox UID cannot resolve missing original request history.

A current owner delete may transactionally cancel a genuinely pending, unattempted request under the original deletion basis and complete machine fence. A first delivery claim carries a private non-dispatch attestation which the journal clears before I/O. Legacy attempted or reclaimed claims never acquire that authority. Paid never-submitted deletion rechecks the original responsibilities inside the machine-fenced transaction before settling any interval. These checks concern NyxID's own submitted work; they do not require proof of the CSI driver's internal request or retry history.

The unresolved-responsibility query has a three-second work bound and returns at most one blocker. Completed deadline renewals impose no lifetime history cap. Each cancellation transaction seals at most 64 unattempted rows, and the preclaim sweep selects at most 64 candidate effects. Later claims and observations retain their existing pass bounds. Unresolved dispatches and late Kubernetes results remain attached to the original delete.

## Control-plane evidence and cleanup

The ordinary native stop contract remains available. A valid prior pause stop retains its real source operation, evidence reference and observation upper bound when deletion only changes the Sandbox operation label. A metadata relabel cannot manufacture a newly observed delete stop. A real ordinary suspension follows the existing generation-anchor path. Partial-creation recovery uses the UID/resourceVersion-preconditioned minimal Suspended patch, preserves Pod/PVC templates, then captures the exact observed generation and resourceVersion after the controller confirms that generation. Both paths retain their corresponding controller observation.

When the original Sandbox exists, the private delete path persists versioned controller Suspended evidence from that exact object, with current operation, generation and resourceVersion bindings. Partial creation can produce this observation without a Pod or Node ever being bound; those identities remain absent. When the original Sandbox and applicable original Pods are already absent, complete original-scope Kubernetes reads, known original resource identities and terminal NyxID creation work can instead establish `ControlPlaneWorkloadAbsent`. A lone NotFound cannot establish that combined result, but no prior Suspended record is mandatory for this absence branch. Known force deletion, unexpected replacement, unsupported uncertainty reasons, read failures and identity conflicts retain their blockers.

`ControlPlaneWorkloadAbsent` uses source `sandbox-kubernetes-control-plane`, the current delete as its source operation, a new UUID evidence reference and the actual first complete-query observation time as its accounting upper bound. Its Sandbox generation is zero and resourceVersion is empty specifically because no live Sandbox was observed; those values do not pretend to describe an object or a Suspended condition. It is neither a physical-stop time nor a never-running assertion. Existing correctly attributed evidence retains its original source and upper bound when resources later disappear; retries do not replace it with a newer absence observation.

The fixed profile requests workspace storage. Recovery captures a newly discovered original claim UID once under exact owner/Sandbox identity and CAS; it never overwrites an existing UID. Pending claims need not become Bound. The controller sets PVC ownerReferences from volumeClaimTemplates; Manager neither repairs missing ownership nor adopts foreign/replacement objects.

Once the applicable stop evidence is durable, Manager deletes the original Sandbox with UID/resourceVersion preconditions and explicit `propagationPolicy: Foreground`. Acceptance is not completion. A 409 requires fresh identity validation; 404 is only one absence observation. No child DELETE, force delete, Orphan propagation, finalizer removal or reclaim-policy rewrite is used. Known original children, foreign claim users, replacement UIDs and identity conflicts remain pending and require GC or operator resolution. Foreground only waits for dependencies GC can track with effective blockOwnerDeletion; it does not prove all storage is released.

Complete bounded Pod/PVC inventories remain restricted to the original validated namespace. Pod discovery uses `/api/v1/namespaces/{namespace}/pods` without a label selector, so an unrelated unlabelled claim user still blocks cleanup. Manager never requests cluster Pod, PV or VolumeAttachment inventories. An exact GET/DELETE 404 means that object is absent; LIST 404 is a failed observation. Failed or incomplete inventories cannot authorize completion.

Typed Pod, PersistentVolumeClaim, PersistentVolume and VolumeAttachment Lists validate their outer kind/apiVersion and page metadata before interpreting items. An item may omit `kind` or `apiVersion`; only absent fields inherit the already-validated List type, and explicit matching strings remain valid. Explicit null, non-string, empty or conflicting fields fail validation. Normalized kind remains available to downstream observations. TypeMeta inheritance is restricted to these typed List items. Existing strict recovery-object validation remains unchanged for pre-suspension Sandbox reads and Lease reads. Every page and item must pass the existing namespace, UID, owner reference, claimRef and pagination checks before filtering. Stable resourceVersion, continuation/remaining-item metadata, unique identities, page/item/body limits and timeouts remain enforced. Failure on a later page returns no partial inventory. Malformed data cannot disappear through filtering, and multi-request observations do not claim an atomic Kubernetes snapshot.

Current deletion evidence uses `evidence_version=2`, `ControlPlaneSuspended` or `ControlPlaneWorkloadAbsent` workload semantics and `SandboxCascadeObserved` (`sandbox_cascade_observed`) disposition, with source `sandbox-kubernetes-control-plane`. This disposition establishes original Sandbox/known namespace child absence; storage was not observed. It must never be converted to `ControlPlaneReleased`, an empty storage inventory or a claim of storage release.

Drain all old Manager workers before rolling out v2 and before withdrawing permissions: old consumers cannot interpret the narrowed semantics safely. Pending v0/v1 records reset cleanup milestones and require fresh v2 observations; only valid correctly attributed v1 first-stop evidence retains its accounting boundary. Historical PV/CSI fields and `ControlPlaneReleased` retain historical meaning and do not prove v2. Finalized v1 tombstones may upgrade through fresh bounded namespace observation without reopening execution, accounting or slot movements. Neither contract implicitly proves the other.

## Accounting and finality

Cleanup and accounting retry independently. Resource completion and its evidence identity remain durable while accounting is pending. The private paid consumer rechecks terminal NyxID creation work for a workload-absence result, retains the first applicable control-plane observation upper bound and reuses the original runtime run, funding snapshot and billing dedupe identities. That bound is a control-plane accounting observation, not a physical terminal timestamp. Repeated settlement cannot extend it or charge twice.

Final deleted/closed/revoked state, slot release and original-operation success require a still-valid original owner delete, disabled creation/execution, terminal NyxID creation responsibilities, the applicable versioned control-plane workload and resource-release evidence, no known residual or identity conflict, and accounting finality. Resource cleanup can proceed when independently safe during an accounting outage; final state and quota release continue to wait. Delete evidence never authorizes ordinary running, Ready, resume, replacement, coverage or rekey.

The Manager reserves half of its existing 64-machine observation bound for due original deletes, with eight concurrent tasks. Accepted, applying and reconciling original deletes with an available retry budget remain eligible after restart, including applied outbox effects and lost responses. Future `next_attempt_at` values are honored after owner authorization is checked. Named blockers survive generic retry reporting. Deletion does not depend on new-machine admission, usage data, kubelet statistics or storage utilization.

## Observation diagnostics and overdue cleanup

Deletion classifies failures at the credential, request, response decode, List/item/identity validation, pagination and size/time boundaries. This covers the delete path's Sandbox GET/PATCH/DELETE, Pod/PVC GET/LIST. Existing blockers for verified UID, owner or claimRef conflicts remain distinct from malformed response data.

| Observation failure | Persisted blocker |
|---|---|
| HTTP 401 | `delete_kubernetes_authentication_failed` |
| HTTP 403 | `delete_kubernetes_access_denied` |
| Request or individual observation timeout | `delete_kubernetes_observation_timed_out` |
| Credential preparation unavailable | `delete_kubernetes_credentials_unavailable` |
| Transport failure | `delete_kubernetes_transport_failed` |
| Other unsuccessful HTTP status | `delete_kubernetes_http_failed` |
| Invalid JSON, type, metadata or identity format | `delete_kubernetes_response_invalid` |
| Incomplete pagination, inconsistent token/resourceVersion or duplicate UID | `delete_kubernetes_inventory_incomplete` |
| Response body, page count or item limit exceeded | `delete_kubernetes_inventory_limit_exceeded` |
| Unclassified exception | `delete_resource_observation_unknown` |

The complete attempt's existing 40-second timeout remains `delete_attempt_timed_out`. A failed observation cannot authorize cleanup completion or destructive effects based on an incomplete inventory. One structured warning per actual failure correlates the verified internal `machine_id` and `operation_id` with fixed `stage`, `resource_kind`, `verb`, `error_kind` and optional numeric `http_status` fields. Stages are `request`, `decode_response`, `validate_list`, `validate_item`, `validate_identity` and `pagination`; unclassified exceptions use a fixed `unclassified` classification. Diagnostics never include upstream error text, response bodies, URLs/query strings, headers, credentials, writer fences, authorization digests or resource specifications.

The same delete-only mapping persists `delete_recovery.blocker` and the pending original operation's `error_code` even when failure occurs before the first cleanup milestone. Writes retain the attempt's authorized owner/operation/runtime tuple, evolving revision and active retry claim; an old executor cannot reload a successor's revision to overwrite it. Generic `runtime_evidence_pending` or `sandbox_effect_pending` reporting does not replace a specific observation failure. Diagnostic persistence consumes no extra retry attempt. Successful new progress or completion clears resolved blockers through the existing flow. A finalized late-resource scan may update cleanup diagnostics, but keeps the original operation Succeeded and the Machine deleted.

After 24 hours, projection retains the deadline, pending cleanup state, one `cleanup_overdue` timeline/notification event and one durable operator escalation. It does not replace `error_code` or `error_message`; a specific failure remains actionable beside the overdue event, and an operation with no known error may keep both fields empty. Terminal operations are never reopened, including when completion races an overdue update. An older stored `cleanup_overdue` error is replaced by the next actual classified observation through the normal delete path; projection does not infer a historical cause or backfill existing rows.

## Bounded pending-delete retries

Each pending delete has a round of at most ten recovery attempts, including the initial attempt. The shared original-operation entry atomically reserves one attempt for both outbox delivery and Manager observation. It increments the lifetime `retry_count` and current-round counter once; polling, internal milestones and generic error reporting do not increment them. The next automatic attempt is scheduled one minute after completion. Each invocation is bounded to 40 seconds under a 60-second claim, so a lost process consumes its reserved attempt and leaves a recoverable reservation. An expired claim cannot overwrite a successor's scheduling.

After ten attempts, automatic pending-delete recovery stops. The original operation remains nonterminal with cleanup pending and its last blocker/evidence retained; execution remains disabled, and accounting and quota responsibilities remain. Stopped operations are excluded before the Manager's candidate limit. Existing operations without round metadata use their cumulative count as the consumed budget; an already exhausted operation such as `retry_count=15639` waits for manual retry. The API reports `delete_retry_stopped`, current-round attempts and the limit, and reports no next automatic attempt while stopped.

The Machine detail operation card displays **Retry deletion** when the current original delete has stopped. The human owner can call `POST /api/v1/remote-machine-operations/{operation_id}/retry-delete` with an `Idempotency-Key` and `{"expected_revision":<machine revision>,"expected_retry_count":<lifetime count>}`. The transaction checks the exhausted round, absence of a live claim, the original owner/operation/Machine binding and existing deletion authorization. It advances the Machine revision fence, resets only the round budget to zero, schedules the same operation, and appends a chained owner audit. The lifetime count, original operation ID, evidence, execution denial and accounting/slot responsibilities are preserved. Replaying the most recently accepted request returns its current operation without renewing the budget again; requests from older rounds fail the cumulative-count check. Stale requests and non-owner, API-key or delegated callers cannot renew it. Cleanup remains available when new-machine admission is disabled.

The button is disabled while submitting and reports request failures. An unknown network result reuses the same saved request and idempotency key. A successful manual retry starts another round of up to ten attempts under the same one-minute schedule. These limits apply to pending deletion; the existing finalized-tombstone cleanup scans below retain their separate behavior.

## Continued scans after finalization

Manager cleanup does not stop at `resource_state=deleted`. It continually revisits a bounded, fairly selected set of finalized Machine tombstones under the retained original delete operation and owner-authorized scope. A completed inventory is the observed control-plane result at that pass, not a promise that a controller can never surface another original resource. Finalized scans handle late original resources as well as work that arrives while deletion is still pending. They are distinct from the existing read-only investigation inventory, which remains a diagnostic view.

Finalized scans reuse the original authorization and versioned workload-evidence gates, revalidating the full runtime tuple and original anchors. They suspend/delete a late original Sandbox through the same identity-protected Foreground path. Late original Pod/PVC objects are read-only diagnostics and remain GC responsibilities. New original claims use the same identity CAS rules; saved identities are never overwritten. Replacement UIDs, foreign ownership, claim users, failed reads and incomplete inventories keep diagnostics pending. Scans never call child DELETE or storage LIST, including for historical tombstones.

These scans keep execution disabled and preserve the original Machine's finalized state and the original operation's Succeeded result. They do not accept another delete, reopen execution, rerun stop settlement, post additional financial movements, reacquire or release a slot again. A discovered late original resource changes cleanup progress to pending (`resources_complete=false`) and retains a safe blocker while the Machine and operation remain final. Once complete inventories are clear again, the same orchestration restores cleanup completion. Read failures and conflicts retain pending diagnostics for retry. Each pass selects at most eight finalized tombstones fairly by `manager_observed_at`, runs at most four finalized scan tasks concurrently and bounds each scan to 45 seconds. The ordinary observation pass retains its own bound of eight concurrent tasks. Continued scans supplement the original completion flow; they cannot justify completing a delete while NyxID's own original dispatch responsibility, known resource conflicts or accounting remain unresolved.

## Authorized capability boundary and permissions

The authorized external surface is Agent Sandbox and Kubernetes Sandbox/Pod/PVC/Node/Lease observation plus identity-protected Sandbox suspension/deletion. Existing KubernetesConnection authentication and TLS are reused. Direct GCE, CSI and CRI APIs, cloud metadata credential reads, deletion-specific cloud configuration, node exec and new host/provisioner observers are outside this contract. No external fact source is required for a successful control-plane delete.

The deletion path requires these existing API capabilities for the actual authenticated backend principal:

| Scope | API group / resource | Verbs |
|---|---|---|
| Original `nyxid-machine` namespace | `agents.x-k8s.io / sandboxes` | get, list, create, patch, delete |
| Original `nyxid-machine` namespace | core / pods | get, list |
| Original `nyxid-machine` namespace | core / persistentvolumeclaims | get, list |

Manager must be denied Pod/PVC DELETE, PV LIST, VolumeAttachment LIST and all-namespace Pod LIST. `healthcheck.sh` loads the same configured backend token-file credential (`NYXID_REMOTE_MACHINE_KUBE_API`, `NYXID_REMOTE_MACHINE_KUBE_TOKEN_FILE`, optional CA) into a disposable kubeconfig, verifies `auth whoami` against `NYXID_MACHINE_MANAGER_PRINCIPAL` (defaulting to the configured Manager service account), and performs these denials plus required namespace reads/Sandbox writes without impersonation. Metadata mode is intentionally outside this shell check and remains covered by backend readiness/API checks. Independent operator-context infrastructure reads remain separate. PV GET remains for independent known-PV investigation; VolumeAttachment GET has no caller and is omitted. Check the union of all RoleBindings/ClusterRoleBindings, not only these manifests. Controller, GC and storage identities must remain independent and retain their own required permissions.

The existing Kubernetes roles and the separate `deploy/remote-machine/node-lease-role.yaml` remain within this boundary. The Lease role stays separate because the namespace-setting kustomization would rewrite `kube-node-lease`. Existing Node/Lease checks and optional Controller Deployment diagnostics are not new deletion-completion gates. Deployment acceptance must validate the actual implementation permissions. Do not withdraw them before old workers are drained.

The user identified the environment as GKE Standard; whether PD CSI is managed remains unverified. The incident's earlier GCE disk/users observations remain historical context in the operations plan. Neither fact expands the API boundary or creates a current requirement for cloud-disk facts.

## Verification status

The effective control-plane deletion contract is described above and in [`deploy/remote-machine/README.md`](../deploy/remote-machine/README.md). Incident-specific operations plans and local acceptance artifacts are maintained separately from the repository documentation.

Earlier implementation snapshots recorded six backend Cargo groups with 153 passing tests, including an additional now-obsolete external-fact group, plus a sandbox updater group with 55 passing tests, healthcheck contracts with 54 passing tests, offline kustomize and diff checks. Those six backend groups differed from the preceding A5 grouping of five backend groups and one updater group. Those results belong to the earlier external-fact contract and frozen source hashes. They remain historical evidence; they do not establish that the current control-plane revision passed its tests. Earlier diagnostic stack overrides and GCE-enabled snapshots are also historical only.

The observation/scope repair requires nonzero test counts for updater `sandbox` and backend `remote_machine_delete_recovery`, `remote_machine_manager` (including native tests), `remote_machine_events` and `remote_machine_delete_retry`, plus healthcheck contracts, formatting and diff checks. Database tests use the isolated replica-set helpers in `backend/src/test_utils.rs`; unavailable or skipped tests are not passing evidence. Coverage includes typed List omission and malformed data, namespace boundaries, classified/redacted errors, early-failure and revision/claim races, both overdue/error orderings and terminal operations, partial creation, initially absent original workloads with terminal NyxID work, Pending claims, identity conflicts, retained resources, original dispatch recovery, repeated accounting, final transaction gates, ten-attempt exhaustion and fair finalized-tombstone cleanup without financial or slot effects. No GCE/CSI/CRI or deletion-specific metadata-credential path is introduced. Local deterministic substitutes verify code behavior; selected-cluster controller/GC behavior and preproduction/production recovery require separate acceptance records. No such remote acceptance is claimed here. B/C admission, placement, usage and deployment architecture changes remain outside scope.

### Owner diagnostic visibility

The owner operation API allowlists the fixed deletion observation codes above,
including `delete_attempt_timed_out`, only for delete operations. It returns
static guidance rather than the stored upstream error message. Unknown codes
remain `operation_reconciling`. Resource details, upstream responses, credentials,
writer fences and internal request/claim material are never exposed by this
projection. This does not change retry budgets, authorization or finality.

Typed List compatibility does not resolve an independent unfinished creation
dispatch: even after Kubernetes resources are released, `creation_result_unknown`
continues to block final deletion and slot release until the original creation
responsibilities are terminal. Do not rewrite database state to infer success.
