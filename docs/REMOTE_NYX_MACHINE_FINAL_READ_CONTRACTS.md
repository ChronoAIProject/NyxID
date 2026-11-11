# Remote Machine source recovery and owner read contracts

## Backend Kubernetes credentials (2026-10-15 scoped goal and acceptance)

Add explicitly selected Google metadata access-token authentication for trusted
NyxID backend calls to the target Kubernetes/GKE API. This is credential ingress
only: native Agent Sandbox lifecycle, owner authorization, Billing, rollout and
profile/rate contracts below remain authoritative.

`NYXID_REMOTE_MACHINE_KUBE_AUTH_MODE` accepts exactly `token_file` (default only
when unset) or `google_metadata`; blank/unknown values fail configuration.
`token_file` requires an absolute `NYXID_REMOTE_MACHINE_KUBE_TOKEN_FILE` and
protected rotating reads. `google_metadata` does not require or consult that
file. There is no cross-mode fallback. Both consumers use one shared concrete
credential implementation; backend factories reuse a process-lived metadata
cache. `NYXID_REMOTE_MACHINE_KUBE_API` remains the actual HTTPS Kubernetes API,
with verified TLS and optional `NYXID_REMOTE_MACHINE_KUBE_CA_FILE` trust root.

The metadata source makes a fixed direct GET to
`http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token`
with `Metadata-Flavor: Google`, no proxy or redirect. Acquisition is bounded in
bytes and time (including refresh wait), validates token/positive `expires_in`,
uses conservative monotonic expiry and early refresh, serializes concurrent
refresh and briefly coalesces failures. Failure rejects the operation, permits
later retry, and never returns expired credentials or sends unauthenticated
requests. Tokens stay zeroizing/redacted in trusted backend memory only; no
logs, API responses, database, image, Machine, ordinary command user or caller
receives them. Existing untrusted metadata/private-network restrictions remain.

| Acceptance | Required local evidence |
|---|---|
| M1 configuration | Both modes/default; blank/unknown/API/CA errors; metadata without file; file rotation/protected reads |
| M2 metadata protocol | Exact GET/path/header; valid response; bounded body; invalid token/TTL; cache, early/expired refresh, concurrent success/failure, timeout and retry |
| M3 actual consumers | Sandbox API and workload verifier use Bearer, including separately reconstructed production factories sharing one cache; TLS/CA and selected-mode readiness |
| M4 bootstrap boundary | Original Pod-bound Kubernetes token stays in TokenReview with audience `nyxid-remote-machine-v1`; Google token is only Kubernetes request authorization; UID/owner/generation/epoch/fence checks remain |
| M5 disclosure and regression | Redacted diagnostics; existing metadata restrictions and relevant native lifecycle/bootstrap regression tests; identify real Rust paths versus HTTP/TLS doubles |
| M6 environment pending | Verify actual Google `chrono-nyxid` principal, target Kubernetes permissions, API reachability and CA in target environment; existing `nyxid-machine-manager` Kubernetes ServiceAccount RBAC is not evidence of Google-principal authorization |

Update these goal/MVP/acceptance contracts before implementation source. Local
fixtures do not establish real metadata identity or GKE permissions. This task
authorizes no live metadata/GKE call, IAM/RBAC/environment mutation, deployment,
commit or push. Physical stop bounds, storage writer isolation and selected
cluster acceptance remain deferred; this is not whole-MVP release acceptance.


## Personal Remote Machine rollout admission (2026-10-15)

The sole rollout key is `experimental:remote-machines`, default **false** in production and tests. Resolve it through the existing Feature Flag framework for the authenticated personal actor (`auth.user_id`), preserving existing override precedence. It grants no role, ownership, billing entitlement, or execution authority.

| Operation | Flag requirement | Existing authority and safety |
|---|---|---|
| Hosted setup fourth choice/list create entry; direct hosted form | Current authenticated person's flag ON; absent/loading/OFF fail closed | Keep This computer, Remote VM and Docker available; account switches cannot reuse another person's flag or intent |
| `POST /api/v1/remote-machine-quotes`, `POST /api/v1/remote-machines` | Personal actor ON before quote/funding/resource effects, including retries | First-party human, personal-owner and existing profile/consent/idempotency checks remain |
| User Resume or Rekey, dedicated and generic action routes | Personal actor ON before platform, replay, funding or outbox effects | Rekey can restore a paused runtime, so it shares this admission gate; re-enabling preserves exact replay |
| Runtime-rate stage/resync/retire writes | Personal actor ON after first-party human and platform-admin checks | Ordinary users, operators, external/delegated callers and API keys gain no write permission |
| `GET /api/v1/admin/billing/runtime` | None | Existing first-party human admin/operator read authorization remains |
| Existing owner list/get/entitlement/operations/history/billing, pause/delete | None | Existing ownership, policy, revision and consent checks remain |
| Accepted manager/outbox work, renewal, metering, settlement, insufficient-funds protection, background rate resync and cleanup | None | Continue all existing accounting and lifecycle safeguards; OFF never automatically pauses or deletes a Machine |

Flag denial uses the existing `forbidden` error (HTTP 403, numeric code 1002), with wire message `Forbidden: Remote Machines are not enabled for this account`. Existing role/credential denials retain their own errors. Free Machine `/billing` reads still return the existing 404 when no paid runtime contract exists; the flag does not change that read contract.

Admission is checked narrowly at HTTP boundaries using one service helper. Do not gate the whole Remote Machine router, shared owner eligibility, or background services. Open create/resume dialogs must close or become ineligible when the flag, authenticated actor or auth readiness changes; quote/create/resume/rekey dispatch checks current eligibility and the actor against the captured intent owner. There is no hosted rekey button today; the existing rekey mutation hook enforces this gate.

Local acceptance requires real mounted API tests with signed authentication, persisted flags and a new disposable transaction-capable MongoDB 8 replica set. Positive fixtures explicitly opt in; no registry test-default-on and no generic 4xx substitute for exact denial/profile validation. Cover absent/OFF/ON ordinary/admin users, personal isolation, rate-write role/auth boundaries, zero effects/provider calls on denial, successful free/paid admission, and OFF-after-establishment ongoing safety. Frontend component/hook and browser tests use disclosed HTTP doubles and do not establish backend enforcement. Local validation coverage and pending environment acceptance are summarized in the [MVP acceptance checklist](REMOTE_NYX_MACHINE_MVP_PLAN.md#本地验证与环境验收).

## Controlled profile and runtime rate contract (2026-10-15)

The existing `remote-machine` profile permanently means 2000 CPU millis, 2048 MiB memory and a 20 GiB workspace, with undeclared (`null`) ephemeral capacity. The server resolves this controlled spec for both free and paid creation; submitted dimensions cannot allocate resources. Only this profile and the approved new-sale price 0.000040 Credits/second are enabled. A later price requires a new immutable rate identity; existing rate IDs, metric codes, quote prices and settlement obligations remain stable.

`GET /api/v1/admin/billing/runtime` adds `profile_version` and a safe `profile` resource projection to each rate, retaining exact prices, sync/retirement state and timestamps. `POST /api/v1/admin/billing/runtime/rates` requires `profile_version` and `credits_per_second`, resolved and validated by the service before staging and the existing Lago sync/readback/retry. Unknown or unavailable profiles and unapproved new-sale prices are rejected.

Quote and billing-preflight requests accept the target `profile_version`; resume uses the saved Machine profile and rejects a conflicting assertion. The shared selector chooses the latest verified, synchronized, unretired rate for that profile and the current plan. No match returns an explicit unavailable reason and never uses another profile's rate. A quote-pinned preflight uses the quote's exact profile/rate/price even if a newer rate exists. Responses expose the safe spec and profile-bound estimate. The single-profile UI automatically sends and shows the selected/quoted/saved profile; no multi-size picker is introduced.

Create requests accept `profile_version`. An accepted paid quote is authoritative for its profile, exact price and terms; explicit request mismatches are rejected before funds or resources. Free create uses the same controlled resolver. Full normalized intent, including profile, remains bound to idempotency. Exact accepted replays retain existing expiry/outage behavior. Prepare, transactional accept/live reread, run renewal and replay validate matching rate/profile/terms. Resource rendering and quota demand use the validated persisted spec, with no new PVC/disk demand on resume. Paused identity, original image, RuntimeClass and PVC UID remain unchanged; a failed resume cannot substitute a different profile or empty workspace.

Compatibility is missing-only: historical top-level rates and embedded `MachineRuntimeRun.rate` snapshots without `profile_version` deserialize to literal `remote-machine`, established by the sole-profile historical stage constructor at `bde170f0`. Explicit null is invalid; unknown/mismatched bindings do not admit. The original profile selector alone may include absent-field rows. No current-default mapping, quote-term alias (`shell-files-v1` remains invalid), data rewrite, repricing or historical recharge occurs. No live inventory is claimed. Upgrade all backend replicas and billing workers before enabling rates for any additional profile. Local verification scope, reusable test commands and outstanding environment checks are maintained in the [MVP acceptance checklist](REMOTE_NYX_MACHINE_MVP_PLAN.md#本地验证与环境验收).

## Optional StorageClass and hosted image amendment (2026-10-15)

This owner-authorized amendment supersedes earlier explicit-only StorageClass and hosted Dockerfile paths. `NYXID_REMOTE_MACHINE_STORAGE_CLASS` is optional. A configured name must identify an existing, non-deleting StorageClass with a valid provisioner. When absent, omit `storageClassName` entirely and let Kubernetes select its cluster default; an empty string explicitly disables default selection and is not equivalent. Missing/unverifiable default, permission errors and provisioning failures remain visible failures or pending work. Existing PVC identity and retained templates remain authoritative across changed defaults; no replacement empty disk. Class-specific quota demand stays unknown until the class is known. Runtime/CSI capacity and cleanup remain real-environment acceptance.

Restore `cli/Dockerfile.machine` byte-for-byte to this branch's HEAD. Move its hosted additions into `cli/Dockerfile.machine-hosted` together with the existing Heca/CMA additions, and remove `Dockerfile.machine-heca-cma`. The hosted target builds current sources directly with the CMA named context and carries its complete startup/healthcheck dependencies. Preserve ordinary Machine behavior and shared browser/desktop/cua resources, managed identity/runtime/authorization, resource policy and on-demand tool authentication. Fresh checks must target the new image path; historical results keep their original paths and measurements.

## Single current full Machine contract (2026-10-15; written before implementation)

The undeployed development variants have no compatibility or migration path. The sole profile is `remote-machine`, product policy `remote-machine-mvp`, individual data policy `remote-machine-data`, and account deletion policy `machine-data`. Fixed resources remain 2 vCPU, 2 GiB RAM and a 20 GiB persistent `/workspace`. Temporary capacity is undeclared (`ephemeral_gib: null`), never zero or a historical numeric allocation. Commands/Files default on; Browser/Computer are optional, with Computer requiring Browser.

The single Sandbox shape omits both ephemeral-storage request/limit keys and home/tmp disk emptyDir sizeLimit keys. Protected identity Memory/token volumes, permissions, CPU/RAM/PVC, network constraints, optional RuntimeClass and exact immutable owner/Machine/workspace identity remain. CREATE requires the currently approved image; UPDATE retains the exact admitted image. A current Machine saved with image A must still quote, replay, resume and rekey using A after deployment changes to B. Missing prior-object evidence cannot authorize old-digest recreation or implicit upgrades. Temporary quota demand remains unknown, separately from create versus retained-PVC resume demand.

Current creation intent normalizes defaults and compares the entire canonical body, including name, capabilities and specialist. There is no pre-configuration digest stripping. Exact accepted current requests remain replayable after response loss, outage or quote expiry without new resources or funding. Obsolete profile/policy identities fail even when an old request is saved. New/resumed paid operations retain exact owner, operation, generation, rate, expiry, profile/image/policy/confirmation and atomic funding/outbox checks.

Temporary Node path reports carry partial usage/source/time with no fabricated capacity or available bytes. Workspace measurements remain reliable and validated; whole-Pod observations remain independent. UI consent, immutable paid retry, expiry/reset, unknown usage and full/write-failure/eviction guidance stay intact. Old persisted UI intents use the existing invalid/reset path and are never silently migrated into consent. Financial run and rate history remain readable.

The single-current phase evidence remains in `REMOTE_NYX_MACHINE_SINGLE_CURRENT_VERIFICATION_20261015.md`; subsequent optional StorageClass and hosted-image evidence belongs in `REMOTE_NYX_MACHINE_HOSTED_STORAGE_VERIFICATION_20261015.md`. Earlier design/handoff notes and their hash index remain preserved locally outside this commit; the two included verification records retain their distinct source scopes and measured results. Native lifecycle, personal/free/paid/data retention rules and existing platform deferrals remain unchanged. Real cluster/controller/runtime/CSI acceptance is pending; no deployment, live mutation or release claim is authorized.

## Persisted external authority

`machine_external_grant_service::VerifiedExternalSource::from_persisted_job` reconstructs only the version-1 external binding already persisted by NyxID. It preserves the exact parent and child identity, protected OAuth consent, original authentication cutoff, service ceiling and single Node ceiling. It is internal and not an authentication/bearer constructor. Gateway and maintenance renewal re-admit the original grant ID/revision in retry-capable job/lease transactions. Each gateway retry reloads the persisted job and current Node, reconstructs the original source, and rechecks source/grant/runtime/funding/lease authority. A transient Mongo conflict cannot reuse cached authority; downstream proxy execution begins only after the authorization transaction commits. A sibling child cannot renew the original job. The former process-local source cache has been removed, so a restart or another replica uses the same persisted authority and cannot renew an expired authentication window.

Single-job history authorization, external job listing and Machine discovery now use the same existing retry-capable transaction runner for their shared live source/grant write fence. Each attempt retains the original verified caller, Node and exact grant ID/revision and reruns live admission. Node query, renewal and provider effects remain outside the callback. This closes the one-shot read transaction boundary reached by ordinary concurrent admission/renewal without weakening original-source ownership.

External cancellation also uses that runner for live stop-source validation, the fixed job binding, cancellation metadata, the revocation outbox and lease removal. Each attempt reloads persisted transport authority and fences the current owner, key/child or OAuth consent; the immediate Node stop is sent only after commit. Narrow cancellation remains available after Machine grant revocation, including a live sibling under the same parent key, while a revoked source cannot cancel. Renewal may advance the authentication deadline but cannot replace the original job identity.

Owner OAuth Machine-consent preparation retains one original baseline across retries: the consent fingerprint, owner and client security revisions, and a consent-revocation generation. Snapshot reads capture that baseline before the first shared write can conflict. Ordinary execution-fence increments may retry; a changed security baseline returns the existing grant-changed denial. The existing owner consent-revoke transaction increments `machine_consent_revocation_revision`, so an absent → approved → revoked → absent consent cannot silently make an in-flight browser selection current again. This generation does not change unrelated live-grant security epochs. Selection validation, live owner/client checks and one-time request insertion remain transactional; no external effect is retried.

The server-owned sticky `ApiKey.machine_external_grants_present` bit now travels on the already-loaded authenticated Key. False skips unrelated Machine discovery reads, while explicit Machine tools continue to perform full live admission. New and rotated successor keys start false. The existing first-grant producer sets the sticky bit transactionally. External source/binding/revocation/principal/reference Debug implementations redact identifiers without changing serde representation.

Confirmations persist the original source authentication cutoff, protected consent and service ceiling. Human decisions reconstruct and live-admit that original source and exact child within the transaction, before any expiration sweep is required. Pending windows are at most 15 minutes; approved retry windows are at most 10 minutes and are additionally capped by original authentication and current runtime coverage. Consumption narrows the resulting authority to that original source and remains one-use. Legacy confirmations lacking the required original-source fields fail closed.

Production external MCP discovery and dispatch are mounted and active in the current source. Admission still requires verified source identity, live original-source/grant authority, current Node v3/runtime and lifecycle/funding gates; there is no temporary coordinator-owned activation constant. The five public local journeys exercise these paths with stated fixture boundaries. The preceding single-current scope verification is recorded in `REMOTE_NYX_MACHINE_SINGLE_CURRENT_VERIFICATION_20261015.md`; real client and selected-environment acceptance remain pending.


The public field contract is:

| Route | Additive request / response contract |
|---|---|
| `GET /api/v1/admin/billing/runtime` | Each `rates[]` has `profile_version` and `profile: {profile_version,cpu_millis,memory_mib,workspace_gib,ephemeral_gib}`. `rate_version`, `metric`, exact `credits_per_second`, `sync_status`, `retired`, `verified_at`, `created_at` remain. |
| `POST /api/v1/admin/billing/runtime/rates` | Required string `profile_version` and decimal-string `credits_per_second`; no resource dimension fields. Only `remote-machine` and exact 0.000040 accepted. The response is the same rate view, initially pending. |
| `POST /api/v1/remote-machine-quotes` | `profile_version` defaults to literal `remote-machine` only when omitted; explicit null/unknown fails. Existing `free_slot` and optional `machine_id` remain. Response `profile` is the selected/saved safe spec; paid response freezes `quote_id`, `rate_version`, `credits_per_second`, policy and expiry. |
| `GET /api/v1/remote-machine-entitlement?profile_version=remote-machine` | Optional query defaults to the original literal profile. `billing_preflight.profile_version` identifies the rate/estimate profile; absent matching rates report `pricing: unavailable` and `profile_pricing_unavailable` when other prerequisites are ready. |
| `POST /api/v1/remote-machines` | Optional `profile_version` defaults to the original literal; it must match the accepted quote. Arbitrary CPU/memory/workspace fields are rejected. Existing quote, consent, capability and specialist fields remain. |
| Existing resume action | Optional `profile_version` must equal the saved profile; the UI explicitly sends it. The resume quote uses `machine_id` plus saved profile. No new action or resize API exists. |

Malformed decimal prices (including `NaN`) return a request validation error, with no staged rate. A paid quote with no matching available rate uses the existing `billing_provider_unavailable` response (HTTP 402) and a requested-profile explanation; it never falls back to a foreign rate. A quote whose retained resource/image/policy terms differ from a creation intent keeps the existing HTTP 409 conflict behavior. Preflight remains advisory and does not reserve funds.

Exact pre-additive-profile retries retain the historical **full** normalized intent digest with only the newly introduced `profile_version` field absent, after validating the saved canonical Machine contract. Names, capabilities, specialist, policies, quote and every other bound field stay compared; no stripped-configuration fallback or obsolete profile alias is accepted. New writes bind the explicit normalized profile. Neither retry handling nor missing-field rate decoding rewrites stored history.

## Bounded reads

- Lifecycle events use `(created_at, _id)` descending seek cursors and return `{events,next_cursor}`. The service filters owner/Machine before limit, fetches at most limit+1, caps pages at 100, applies a three-second database time limit and rejects using both legacy `before` and a cursor. The UI follows server cursors; legacy arrays are explicitly incomplete discovery.
- Global confirmations use the same composite cursor contract, filtering owner/Node, live confirmation expiry and stored authentication cutoff before pagination. The UI resets cursors when owner/Machine changes or a decision closes, and invalidates an open review while a live refresh is in flight.
- Owner Machine discovery follows `next_cursor` across bounded pages. Billing requests `include_deleted=true`, making retained/deleted records discoverable without requiring a known ID. Known-ID owner-authorized lookup remains a fallback. Missing legacy cursor metadata is shown as incomplete discovery.
- The per-Machine grant reader is mounted at `GET /api/v1/remote-machines/{id}/grants`. It uses first-party human owner authorization, bounded composite pagination and independent live source/grant validity; delegated account-read credentials remain denied.

## Known temporary environment values in job output

The existing shared Jobs admission path validates the per-job environment, then registers every non-empty supplied value in the existing bounded redactor before spawning the child. A value that cannot be registered refuses execution before child-process effects. The existing session registry remains available for stream capture, completed-job reads and paginated reads, with its 1 MiB pattern bound; output ring/cache and job-retention limits are unchanged. Temporary env values are not inherited by a later job.

The actual ordinary and external foreground/background executor regressions cover stdout/stderr, pagination across a replaced value, post-completion reads after later admission, FIFO-gated fragmentation and full-registry refusal before a file effect. Timeout, output-cap, cache-eviction, retention and external/gateway authority are also regression targets; these preceding-phase results remain in SINGLE_CURRENT_VERIFICATION_20261015. This contract protects exact known supplied plaintext values. Existing redactor encodings are incidental reuse, not a new arbitrary-encoding guarantee; independently stored workspace secrets are outside this narrow protection.

## Owner deletion and recovery

`DELETE /api/v1/users/me` requires the existing first-party human extractor and `X-NyxID-Account-Deletion-Policy: machine-data`. The current Settings, mobile confirmation and `nyxid profile delete` send this acceptance after the explicit disclosure; CLI `--yes` retains the disclosure and supplies the same policy. The Assistant action accepts `deletionPolicyVersion: "machine-data"` with its existing email confirmation and includes both in the canonical action fingerprint. Missing/stale policy and external credentials are rejected before deletion effects or action reservation.

Current account-deletion disclosures (Settings and Assistant shared confirmation, mobile native confirmation, and CLI including `--yes`) describe queued Machine/workspace deletion with a 24-hour target. Completion uses applicable control-plane stop/absence, Sandbox object-cascade checks, terminal creation responsibilities and accounting; disk-release proof is not a gate. Underlying storage follows its reclaim policy and physical storage release is unverified. The Billing Machines policy uses the same boundary. Policy versions, consent and authorization are unchanged.

The separate single-Machine deletion confirmation explicitly discloses loss of the workspace and temporary runtime data, including HOME, `/tmp` and other temporary runtime files. It retains no-snapshot/no-recovery wording, pending stop/cleanup/accounting evidence, output denial, 30-day job metadata and 90-day terminal operation retention. Its existing acknowledgement, revision/idempotency checks and `remote-machine-data` acceptance are unchanged.

After a validated successful deletion response, the Assistant confirmation accepts the expected 401 from the revoked account/session on its verification read, as well as an absent projection. Only that read defers local sign-out until the completion message is dismissed. A still-readable account or failed deletion mutation remains an error; the deferred UI state grants no execution authority.

The existing account/source-fence transaction records the explicit owner policy on each retained ManagedMachine row before account removal. Administrative deletion does not create this basis. Missing owners and administrative removal stop/revoke execution and retain workspace data, including paused PVCs; they never supply consent or start a retention TTL. Explicit owner Machine deletion stores the existing accepted Machine data policy on that same lifecycle record.

Manager delete effects, native resource cleanup and both allocated/unallocated finality recheck the durable basis. Current accepted owner deletion writes its basis transactionally before effects; retries preserve the original accepted_at. No audit-only reconstruction may invent consent for undeployed pre-basis records. A missing or invalid basis fails closed and preserves data. Native stop observations, Sandbox/known namespace Pod/PVC cascade progress and unsettled accounting remain distinct under [deletion evidence v2](REMOTE_MACHINE_DELETE_RECOVERY.md). Known original objects still present, finalizers and unknown ownership stay pending; relevant quota is retained. PV/VolumeAttachment inventories and Retain-policy storage are unobserved by Manager and do not gate v2 finality. A submitted DELETE or an absent Sandbox alone never means all data has been physically erased.

## Operator inventory

The existing Manager independently reads controlled Pods and PVCs even when the Sandbox is absent, using at most 100 objects per Kubernetes page and 64 pages per bounded scan. It correlates exact resource/Sandbox owner UIDs, owner/Machine/operation labels, profile and generation with durable intent and retained records. An exact paused workspace PVC remains legitimate. Unbound, malformed or mismatched allocations create idempotent metadata-only investigation rows; observation never adopts or deletes them and absence never certifies cleanup.

The existing first-party human admin/operator route `GET /api/v1/admin/remote-machines/platform` adds `investigations`, with `inventory_cursor` and `inventory_limit` (default 25, maximum 100), ordered by immutable first-seen time and UUID. Safe metadata is projected before rendering; no Pod specifications, credentials or command output are exposed. The existing event sweep marks/logs investigations older than the 24-hour operator target. Existing read RBAC is sufficient; normal Pod/PVC lifecycle remains Agent Sandbox/Kubernetes-owned.

## Billing diagnostics and service attribution

`BillingService::machine_runtime_preflight(owner_id, profile_version, quote)` reads existing wallet, exact-accounting readiness, synchronized runtime pricing and the existing strict provider/subscription checks. Quote callers select the actual offered rate; entitlement diagnostics use the latest synchronized rate for the requested profile. The diagnostic is not admission, does not create an account or allowance period, and does not reserve money. The handler may return null diagnostics without disabling free runtime; paid quote/admission remains strict.

The estimate uses existing exact Credits and existing eligible grant/allowance rules. Available allowance seconds use the current definition minus consumed/reserved quantities without mutating a period. Active wallet/allowance settlement makes the estimate unknown. Estimates expose exact available/reserved/pending wallet values, matching allowance seconds, available grant Credits and whole runtime seconds. A 30-second observation is not a duration guarantee. The UI rejects an estimate belonging to another quote rate and shows stale or absent estimates as unknown.

`GET /billing/usage` accepts optional UUID `machine_id` and `job_id`. Both filter verified `machine_usage` attribution inside the owner-scoped database match, before grouping and totals. The response echoes both filters. History remains readable for owned billing records after Machine deletion; a live Node is not needed. The UI requests the server Machine filter and validates the returned owner, period and filter before showing rows.

## Platform projections

Quota diagnostics distinguish create demand from retained-volume resume, unknown/stale observations and insufficient/headroom states. Headroom never promises scheduling/storage capacity. Paid and free resume quote requests include the existing Machine ID.

Normal lifecycle readiness uses the native Sandbox contract. The backend readiness projection carries `lifecycle_basis: "agent_sandbox_native_observation"` and `physical_acceptance: "native_state_only_no_physical_guarantee"`; the UI does not promise physical shutdown or writer isolation. Pausing means stopping the compute Pod while retaining Machine identity and the same workspace. Completion requires current Sandbox and associated Pod observations, not merely an accepted request. Resume starts a fresh runtime and does not recover old processes, memory or jobs. Uncertain predecessor status stays pending for retry or operator handling; top-ups never automatically resume a paused Machine. Native quarantine is correlated into the existing operator investigation inventory, with a durable Machine latch and retried metadata append. There is no generic quarantine-clear API or manual database-reset shortcut.

The controller Deployment/image/rollout projection remains an optional human operator diagnostic. Its missing configuration, denied detailed-read permission or unavailable view does not make ordinary lifecycle admission depend on an upgrade matrix. Basic Kubernetes connectivity, required permissions, resource-creation errors and current resource checks remain required.

Node workspace reports remain the source of workspace capacity/usage and full-disk guidance. Whole-Pod temporary-storage observations remain separate from these reports and from partial Node directory scans. The existing direct kubelet collector is optional: absent or blank `NYXID_REMOTE_MACHINE_KUBELET_ENDPOINTS` returns without requiring collector CA, audience, token, Kubernetes reads or database reads; a configured collector retains its TLS, audience, limits, current-Pod identity and timestamp checks. Deployment can reuse existing monitoring and otherwise leave Pod metrics unavailable; no new monitoring framework is introduced.

The UI displays telemetry source, timestamps, components, missing reasons and independent staleness. A missing Pod aggregate leaves `used_bytes` null, even when diagnostic component values exist. Legacy partial-sum rows are normalized on read, and the UI also refuses a total without the matching aggregate component. It never adds a Pod aggregate to its components or Node path scans, never uses workspace/partial paths as full-Pod use, and never derives available Pod bytes from backing filesystem capacity. Unknown stays unknown, including when normal Machine operations and reliable workspace reporting remain available.

## Hosted creation and full-image read contract (revision5)

The single hosted form lives under `/assistant/machines/new`, the fourth option after This computer, Remote VM and Docker. Personal ownership is derived from the authenticated current person. Name uses existing Machine validation and is display metadata, independent of stable Node/Machine UUID and technical Sandbox name. Commands/Files default on; Browser/Computer default off. At least one capability is selected; Computer requires Browser. Optional specialist defaults to none. Name, normalized capabilities and specialist selection survive free/paid quote and retry and are bound into creation idempotency.

The `remote-machine` profile describes the full Machine resource contract, not a capability grant. The separate immutable `capabilities` array drives bootstrap, Node configuration and exact live capability matching, including Browser-only and Browser/Computer configurations.

Managed bootstrap uses its own versioned RSA-OAEP-SHA256/AES-256-GCM recipient
envelope in `nyxid-machine`; it has no backend dependency on the client-only
credential-injection crypto crate. The full ordered Binding is authenticated,
and ciphertext-only replay, exact recipient digests and controlled rekey remain
mandatory. Backend and hosted images must upgrade together; incompatible pending
material fails closed without replacing the stable identity or original PVC.
The deployment README specifies the wire format and pre-release upgrade boundary.

The durable Machine and Node projection, sealed bootstrap config and live Node capability checks must agree. Specialist membership uses existing assignment and authorization services. The read model distinguishes no assignment, pending/failed assignment and permission configuration still required. Enabling local capabilities is not caller authorization: existing NyxBot/specialist policy, live grant revisions, owner-turn/control and confirmations remain authoritative; external OAuth/Agent Key MCP remains shell/files only.

Hosted management uses the existing `MachineCapabilityForm`, with the live Node ceiling and observed policy revision. An unrelated Commands/Files edit preserves the owner's existing Browser, Computer, Developer browser and saved-login selections. The owner can explicitly change applicable permissions. Removing Browser clears its Computer and Developer browser dependencies; other grants remain intact. Existing owner, protocol, feature and revision checks continue to govern the same policy endpoint. This does not widen external OAuth/Agent Key tool access.

Hosted details expose the existing `/assistant/machines/$nodeId/desktop` route through the matching owner Node and its live Browser/Computer capability, connection and lifecycle state. Computer readiness remains a distinct observation: an unavailable display keeps the existing warning and viewer error/reconnect behavior, rather than being reported as usable because the package is installed. No new desktop endpoint or authorization path is introduced. Capability badges describe the saved configuration, including Browser-only and full configurations; absent configuration is reported as unavailable.

Policy/setup describes the shared full Machine image and separates installed components, enabled capabilities and caller grants. Shared workspaces/browser sessions, external-client restrictions, temporary HOME/tmp/browser profiles/downloads, output privacy, retention and charging disclosures continue to apply. The pass4 repair changes only hosted UI, its component/browser regressions and these read/verification documents; unchanged native lifecycle, identity, authorization, Billing and full-image evidence is reused with current source-closure verification.

`remote_machine_configuration` applies the selected personal specialist only after the Machine becomes Ready. The existing transactional grant service retains its feature-flag defaults: the capability editor starts new grants disabled, while the legacy path snapshots the permitted Machine capabilities. The durable `pending` assignment retries separately from runtime readiness; `assigned` is never re-applied after a later user revocation. Management reads check the live assignment and report `permissions_required` rather than interpreting the original assignment as current permission. A destroyed or inaccessible specialist yields the fixed `specialist_assignment_unavailable` category and does not expose internal errors.

Current accepted create requests compare the complete normalized intent, including default name, Commands/Files capabilities and no specialist. Changed choices conflict in both outage preflight and transactional creation; no replay changes a reserved Machine or quote.

The final hosted image is `cli/Dockerfile.machine-hosted`, containing the complete Machine build and pinned Heca/CMA tools while sharing desktop/browser/cua resources and startup. The ordinary Dockerfile and entrypoint retain their original contents. Hosted startup uses `hosted-entrypoint.sh`. NyxID CLI, Node subcommands, gh and Chromium require attributable executable/version checks. Installation, process startup, authorized operation and functional regression are separate claims. Full-image tests use exactly 2 CPU/2 GiB RAM; Docker workspace and temporary-storage enforcement must be described as actually configured, not inferred from directory names. Kubernetes 20 GiB PVC and environment-managed temporary-storage semantics remain selected-cluster acceptance where unavailable locally. Workspace persists through pause/resume; HOME/tmp and default browser profile/download directories are temporary. All descendants follow managed Node/Pod stop and expiry. No automatic upgrades or extra Agent Server.

## Local evidence scope

The current optional-storage and hosted-image checks are recorded in `REMOTE_NYX_MACHINE_HOSTED_STORAGE_VERIFICATION_20261015.md`, with exact source/build identity, commands, counts and limitations. The earlier single-current verification document retains its own source scope and results; its passing counts do not establish the relocated image or optional-storage behavior. Component/browser tests use explicit DOM/API fixtures; local Kubernetes HTTP fixtures are not a controller, scheduler or CSI. Rebuilt image smokes use real Node/Chromium/cua and local bootstrap/WSS fixtures, with actual mounts and limits recorded separately.

Selected-cluster acceptance remains: applied suspension/associated Pods, configured or node-default runtime, same-PVC resume, selected StorageClass/CSI cleanup/Retain semantics, resource/network/credential enforcement, reliable workspace telemetry and real NyxBot/OAuth/Agent Key/child-client journeys. Independent partition-resistant physical shutdown, strong predecessor writer exclusion, universal physical erasure and a controller-upgrade operations product remain explicitly deferred. The earlier single-current phase retains its own independent SSHX review record. This revision receives a separate scoped code review and does not reopen that completed review or make a whole-goal or release claim.

The local Docker seccomp profile permits Chromium child-user-namespace chroot without CAP_SYS_CHROOT. Original-namespace chroot remains kernel-denied; Kubernetes RuntimeDefault compatibility is still pending. Browser readiness is never inferred from installation or sandbox-disabled execution.
