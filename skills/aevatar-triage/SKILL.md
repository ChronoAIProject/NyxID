---
name: aevatar-triage
description: Use after an Aevatar workflow, codex_exec call, schedule, channel, connector, skill, Agent Profile, or control-plane request fails or behaves unexpectedly. It applies when the agent must attribute the first broken boundary across Aevatar, NyxID, Ornn, chrono-sandbox/gVisor, the managed runner, or private SSH; distinguish credential sources and deployment gaps; preserve sanitized evidence; determine defect versus usage; or draft an issue for explicit user confirmation. Never use it to guess a root cause from one error string or auto-file.
version: "1.8"
metadata:
  category: plain
  tag:
    - aevatar
    - triage
    - diagnostics
    - root-cause
    - issue
    - nyxid
    - ornn
    - codex-exec
    - agent-key
    - credentials
---

# Aevatar triage — find the layer, read the code, then report or guide

Something broke (or looks wrong) while using Aevatar. Your job is three honest moves:
**(1) attribute** the failure to the right layer — **Aevatar / NyxID / Ornn** — by tracing the
request path; **(2) read the real code** of that layer until you have a **code-grounded root
cause** (cite `path:line`) **that matches what's actually deployed**; **(3) branch**: if it is a
genuine **platform defect** (behavior that violates the layer's *own published contract*), draft
and — only on explicit user confirmation — **file a GitHub issue** to the owning repo; if it is a
**usage / config mistake**, give the user an **authoritative, code-grounded explanation and the
correct usage**. This is the *after-it-breaks* counterpart to `aevatar-feasibility-advisor` (which
answers "is it possible, before building").

You make real calls and read real code — **no guessing, no fabricated root cause, no auto-filing.**

## The three layers (and their repos — all public)

| Layer | Repo (stack) | Owns | Canonical symptoms |
|---|---|---|---|
| **Aevatar** | `aevatarAI/aevatar` (C#/.NET) | agent runtime + tool execution, workflow engine, channels, CQRS/projection + readmodels, control-plane REST, scheduler validation | workflow validate / draft-run / run failures, member-team-service binding stuck (async never `succeeded`), the **aevatar side** of a channel bot, stale readmodel / observatory, scheduled run that stops firing, control-plane 4xx/5xx |
| **NyxID** | `ChronoAIProject/NyxID` (Rust) | credential-broker gateway: proxy + credential injection, OAuth/OIDC/PKCE/MFA, connector vault, NAT relay + SSH, approvals, MCP-from-OpenAPI | `nyxid_proxy` 401/403, "credential not found", connector/slug missing, OAuth/token/MFA failures, grant revoked (`invalid_grant`), **inbound relay not delivering**, approval stuck |
| **Ornn** | `ChronoAIProject/Ornn` (TS/Node) | skills-as-a-service registry: skill search/pull/upload/generate, skillsets, sandbox | skill not found / search / pull / upload fails, `use_skill` cannot find a skill, skillset integrity, generate (SSE) errors |

These three repos and layers are this skill's subject — name them freely. Do **not** hardcode any
user's private business / workflow / skill names.

## `codex_exec` has two request paths — locate the exact boundary

Do not force `codex_exec` into the generic three-layer table. Its two targets share the typed
Aevatar tool but have different transports, credentials, isolation, results, and owners:

```text
managed_sandbox:
  Aevatar tool/admission
    -> Aevatar Application credential readiness
    -> NyxID exact personal UserService proxy
    -> chrono-sandbox POST /codex/execute
    -> one-shot OpenSandbox workload under gVisor
    -> fixed Codex runner

private_ssh:
  Aevatar private_ssh adapter
    -> NyxID SSH UserService and node
    -> fixed host/principal wrapper
    -> host Codex CLI and fixed workspace
```

`gVisor` is the managed isolation boundary. Codex runs inside it with its inner sandbox disabled.
There is no managed Landlock/Bubblewrap preflight, sandbox-side Credential Vault substitution, or
TLS credential proxy to repair. This does **not** mean Aevatar has no secret store: the managed
readiness path legitimately resolves its persistent invocation key through Aevatar `ISecretVault`.

### Stable managed failure groups

Use the stable code to select the first investigation surface, then still prove the root cause with
the deployed revision and live evidence:

| Stable codes | First investigation surface | What to establish |
|---|---|---|
| `target_not_configured`, `managed_target_disabled`, `managed_feature_not_enabled`, `managed_identity_unavailable`, `managed_user_authorization_unavailable` | Aevatar host composition, tool admission, and Application caller authority | Is the target registered/enabled, is the exact native NyxID user eligible, and can the authenticated user call the explicit managed-credential lifecycle API? |
| `managed_user_services_unavailable`, `nyxid_identity_mismatch` | Aevatar readiness plus the user's NyxID service inventory | Does the authenticated owner directly own exactly one usable `chrono-sandbox` and one usable `chrono-llm-public` UserService? Never infer this owner from `scopeId` or another identity. |
| `managed_credential_untracked_key_exists`, `managed_credential_mutation_in_progress`, `managed_credential_commit_timeout`, `managed_credential_cleanup_pending`, `managed_credential_persistence_pending`, `managed_credential_vault_unavailable`, `managed_credential_unavailable`, `managed_credential_invalid` | Aevatar managed-credential actor/projection, distributed mutation lease, and `ISecretVault` boundary | Did the explicit authenticated credential operation commit, did its descriptor become visible, and does its typed secret reference resolve? Do not ask for or manually copy the raw key. |
| `managed_proxy_authorization_denied`, `managed_proxy_target_unavailable`, `managed_proxy_timeout`, `managed_proxy_unavailable` | Managed transport across NyxID and the user's exact `chrono-sandbox` route | `managed_proxy_timeout` means the bounded managed transport wait ended; correlate Aevatar timing with NyxID and chrono-sandbox using the sanitized `diagnostic_id`. It is not evidence to repair local sandbox tooling. |
| `managed_response_invalid`, `managed_response_too_large` | chrono-sandbox terminal contract and Aevatar bounded response parser | Did `/codex/execute` return the fixed complete shape within the response limit? Preserve bounded diagnostics, never the raw upstream body. |
| `managed_execution_nonzero_exit`, `managed_execution_cancelled`, `managed_execution_failed` | chrono-sandbox fixed command/JSONL/cleanup and the immutable runner image | Distinguish caller cancellation, runner/model failure, malformed JSONL, and cleanup failure without adding caller-selected command/model/sandbox flags. |

For the requested contrasts: `managed_credential_unavailable` is first an **Aevatar committed
credential / secret-resolution readiness** failure and is not proof of a NyxID OAuth revocation.
Prepare it explicitly through an authenticated `POST /api/managed-codex/credential`, then read
`GET /api/managed-codex/credential`; continue only when `execution_ready=true` and
`execution_readiness_reason=ready`. `status=active` alone is insufficient. Normal `codex_exec`
execution is credential-read-only: it never provisions, reconciles, rotates, repairs, or retries
credentials. Private `node_offline` is a **NyxID SSH node route**
failure: inspect node daemon connectivity, last heartbeat, and service binding, not managed
credentials or chrono-sandbox. Other private failures (`target_not_allowed`, missing SSH key,
host-key mismatch, wrapper exit 126, host Codex login, wrong Git workspace) stay in the NyxID or
user-owned host boundary named by the failure.

### Codex evidence and source selection

Preserve the exact typed code/error text, target kind, authenticated native NyxID subject,
workflow/run/step/tool-call correlation IDs when available, timestamp and elapsed/deadline facts,
sanitized `diagnostic_id`, deployed Aevatar image/commit, deployed chrono-sandbox managed revision,
and runner image digest. Keep raw operational evidence access-controlled.

Never request or expose raw access/delegation tokens, Agent Keys, authorization headers,
`auth.json`, secret references or Vault locators, unredacted process environments, raw upstream
bodies, or private prompt/output/repository data. A sanitized `diagnostic_id` is correlation
evidence, not proof of a particular root cause.

Read the repository that owns the failing boundary, always at the deployed ref:

- Aevatar tool admission, Application readiness, typed result/failure mapping:
  `aevatarAI/aevatar`.
- NyxID authorization, exact UserService routing, delegation, node, or SSH:
  `ChronoAIProject/NyxID`, after confirming Aevatar used the current typed contract.
- `/codex/execute`, gVisor workload creation, fixed command/profile, JSONL classification,
  timeouts, and confirmed cleanup: the **deployed chrono-sandbox managed revision**. Do not use a
  default branch that lacks that surface.
- Runner contents/profile: Aevatar `containers/codex-runner` plus the deployed immutable digest.
- Private host wrapper, Codex login/config, and workspace: the user's target configuration.

After locating the boundary, use `aevatar-codex-exec-node-setup` for repair and
`aevatar-codex-exec-workflow-sample` for the mandatory post-repair `CODEX_EXEC_READY` proof.
Configuration inspection or a direct chrono/SSH success alone does not close the incident.

## Step 1 — Capture the symptom precisely (don't theorize yet)

Collect, verbatim where possible: the **exact error text**; the **operation / surface** (workflow
run, connector call, channel bot, schedule, skill pull/publish, auth, a control-plane REST call);
**minimal repro**; the **ids** that let you correlate — `run_id`, `scopeId`, connector `slug`,
skill name, `commandId`/`correlationId`, schedule fire-record fields (`fireCount` / `lastFireAt` /
`nextFireAt` / `failureCount` / `lastError`), timestamps; and **what is actually deployed** — the
running **image tag / commit**, pod age + `restartCount`, and the **time window** of the failure
(live logs rotate fast, so an old window may already be gone). Missing ids are themselves a finding.

**First, rule out the cheap local causes** before blaming a layer: your own **expired / missing
credential** (a blanket `401` is often just your own stale token, not a platform bug — refresh it,
then retry only a safe/idempotent read) and a **stale local checkout** (the code you're about to read may be behind what's
deployed).

## Step 2 — Attribute to a layer (trace the request path, eliminate)

**Main rule: follow the request path and find the *first boundary* that breaks.** A user request
typically flows `your agent -> Aevatar runtime -> NyxID proxy -> third-party`, and skills flow
`Aevatar use_skill -> Ornn`. Map the symptom to where that chain first fails:

| Symptom | Most likely layer | Disambiguating evidence to gather |
|---|---|---|
| `401` / `403` on a connector/tool call | **Aevatar -> NyxID -> provider auth chain** | Did caller credential enter the run, reach the executor, select the exact UserService/route, and remain authorized downstream? Establish the first missing boundary; do not assign the layer from status alone. |
| `NYXID_PROXY_SERVICE_SCOPE_FORBIDDEN` | **Aevatar caller-authority admission** | The caller credential lacks authority for the exact selected service scope. Preserve the selected UserService/service identity and caller scope; do not collapse this into "not connected", provider rejection, or a generic 403. |
| "credential not found" / connector slug missing | **NyxID** (vault / not connected) | Compare catalog with authoritative `/api/v1/keys` for this caller. `/api/v1/user-services` is only a route projection and cannot prove execution readiness. |
| `404` on a thing you reference | **whichever registry owns it** | skill -> Ornn; connector/service -> NyxID; team/member/scope -> Aevatar |
| `502` / timeout on an external call | **proxy chain** | which hop? Aevatar tool layer vs NyxID proxy vs the target itself |
| workflow won't validate / run stalls / binding never `succeeded` | **Aevatar** (engine/runtime) | draft-run error body; run timeline / observatory; binding-run status |
| readmodel stale / observatory missing data | **Aevatar** (projection) | is the projection subscription live? event stream flowing? authoritative version vs readmodel — note readmodels **do not back-fill** (compare record age to deploy time) |
| **scheduled run stopped firing** (fired before; `nextFireAt` frozen in the past, `fireCount` flat, `failureCount=0`, empty `lastError`) | **Aevatar** (scheduler / actor not re-armed across pod churn) | compare to peer schedules; pod `startTime` / `restartCount`; is it still enabled? did a deploy/restart line up with the last good fire? |
| **scheduled run never fires, or fires but errors on credential** | **Aevatar scheduler ⨯ NyxID** (binding) | is it enabled and is `nextFireAt` computed? `lastError` like "binding not found" / "exactly one credential source" -> the *fired call's* invocation credential (scope-owner broker binding) |
| **schedule fires (`fireCount` climbs) but the real-world effect never happens** | **Aevatar** (the fired call's path / credential) | dispatch success ≠ effect — check the external side-effect out-of-band; the proxy can hand back `{"error":true}` inside a `200` |
| **scheduled run starts fine, then late steps hit `token_expired`** | **depends entirely on `credentialSourceKind` — read it before you theorize** | see *Scheduled-run credentials are not one thing* below. `nyxid_binding_exchange` ⇒ the fire-time broker token (`BROKER_ACCESS_TTL_SECS = 300`, NyxID `backend/src/services/oauth_broker_service.rs`) is the right hypothesis. `scheduled_invocation_agent_key` ⇒ it is **not** the 300 s broker TTL and a ~5-6 min correlation is coincidence, not evidence |
| **inbound bot doesn't reply** (Lark/Telegram) | **cross-layer — walk it** | did NyxID relay webhook fire? is the bot connector connected? did the Aevatar channel run start (observatory)? credential = the *sender's* NyxID, present and live? |
| Lark Base returns `91403` while `bitable:app:readonly` is enabled | **Lark document ACL / usage configuration** | The API scope and Base document sharing are separate. In the Base `...`/More menu choose **Add Applications** and add the exact Bot application used by the selected NyxID UserService; view access is enough for read-only calls. |
| **`/whoami` says "bound" but tool calls get `credential_denied`** | **NyxID** (grant revoked — false green) | live token-exchange returns `invalid_grant` while the local readmodel still reads "bound"; whoami checks only the local mirror, not the live grant |
| approval prompt stuck | **NyxID approvals + Aevatar suspension** | Read the typed run step. Tool approval identity is `executionId + toolName + toolCallId + approvalRequestId`; never infer it from prompt text or a generic bag. Check the matching NyxID request and Aevatar suspend/resume state. |
| skill search/pull/upload/generate fails | **Ornn** | Which `/api/v1/skill...` route? For search, did caller-scoped remote token resolution/authorization fail? That is an error, not an empty catalog, and must never fall back to a generic platform token. Also inspect validator violations and exact version format. |
| **`404` on a control-plane route you believe exists** (e.g. `…/agent-profiles`) | **could be "resource missing" OR "capability not deployed" — these are different verdicts** | probe the *surface*, not the resource: `GET /api/openapi.json` and check whether the **complete** route family is advertised. Family absent ⇒ the deployment does not expose that contract; a single 404 proves nothing either way. See *Deployment-gated capabilities* below |

**Do not stop at the first match.** Gather the disambiguating evidence and *eliminate* — a plausible
first guess that you haven't excluded the alternatives for is not an attribution.

### Workflow execution triage

Preview/readiness and runtime execution are separate proofs. A successful explicit-request preview
proves the call sites can be admitted for the current workflow/revision; it does not prove that a
caller credential will propagate into the run or that the downstream service will authorize it.

For one failed run, do not invoke again. Read, in order:

1. the root stream terminal frame, if available (`RUN_FINISHED` or `RUN_ERROR` only; role text,
   reasoning, tool-call, tool-result, and role terminal frames are progress);
2. run detail (`completionStatus`, `lastSuccess`, last error, final output);
3. run audit and its first failed step/tool call;
4. the binding's exact workflow/revision identity and committed capability admission plan.

Accepted-to-first-observation has a separate 30-second deadline. SSE keepalive does not count as a
projection-backed business frame. `RUN_OBSERVATION_TIMEOUT` closes the current stream because
observation stalled; it is **not** a whole-run failure classification. Query the same
`actorId + commandId` for later committed/read-model state. Do not create another run as a status
probe.

For a first NyxID HTTP auth failure, test four hypotheses separately: inbound caller credential
never entered the run; it entered the run but not the external-capability executor; the executor
selected the wrong exact UserService/route/credential; or the selected downstream UserService is
expired/unauthorized. Use distinct `memberId`, `workflowId`, and `publishedServiceId` shapes when
reproducing identity propagation. Preserve the exact error privately; report only a bounded,
sanitized classification. A failed mutation or run is never blindly retried.

**Recovery never replays uncertain provider/effectful I/O.** Activation recovery may resume typed
continuations and redispatch an exact safe postcondition, but it must not repeat an interrupted LLM
request or potentially effectful tool call. If the external operation may have completed before its
callback/result delivery was lost, keep the outcome uncertain; a committed result lost before
delivery is an explicit delivery-loss failure with its stable operation lineage. Do not turn either
case into a silent retry or claim that the real-world effect did/did not happen without external
evidence.

**Scheduled-run credentials are not one thing.** "It was a scheduled run" tells you nothing about
its credential. Read `credentialSourceKind` off the run/automation record **first**, then pick the
diagnosis. Applying the wrong class's lifetime is the single most common wrong verdict in this area:

| `credentialSourceKind` | What the run actually holds | How to diagnose a `token_expired` / 401 |
|---|---|---|
| `nyxid_binding_exchange` (generic `/api/schedules` with a NyxID binding source) | A short-lived bearer exchanged from the stored binding at fire time | The 300 s broker TTL is a legitimate hypothesis. Confirm with the actual token's `exp − iat` (numbers only) or the repo constant. Fix is run shape: front-load authenticated steps or split the schedule |
| `scheduled_invocation_agent_key` (Studio Team member automation, scheduled skill agents) | A **dedicated, restricted Agent Key**. Raw material lives only in `ISecretVault`; the run borrows a durable credential *reference* and resolves it through the Vault **at each use**, fail-closed | **Do not cite the 300 s broker TTL.** Check, in order: key `credentialExpiresAtUtc`; Vault resolution and reference integrity; `credentialGeneration` (did a reauthorization replace it mid-flight?); `authorizationStatus`; `lastAuthorizationErrorCode`; exact service/node grants vs what the failing step called; `revocationPending` / `nyxIdRevocationStatus` / `vaultRevocationStatus`; then the **downstream** service's own token |

A failure landing near the five-minute mark under `scheduled_invocation_agent_key` is **not**
self-evidently the broker TTL — that number belongs to a different credential class. Treating the
coincidence as proof is exactly the confident-but-wrong attribution this skill exists to prevent.

Two more independent dimensions not to conflate: credential health (`authorizationStatus == active`)
and firing state (`enabled == true`). A paused automation keeps its active key; an expired key does
not disable firing. Either can be true without the other.

**Deployment-gated capabilities.** Some contracts exist in the codebase — and in design docs — but
are not exposed by the running build. **Agent Profile management is currently one of them.** So:

- A `404` from a profile route is **not** a deployment probe. Probe the surface: does
  `GET /api/openapi.json` advertise the *complete* `agent-profiles` route family (create, get,
  draft, draft skill upsert/remove, `:validate`, `:publish`, plus public discovery)? Require the
  whole family — a partial or mixed-version host is not a safe mutation surface.
- Family **absent** ⇒ verdict is "capability not deployed here," not a defect and not user error.
  Say so plainly and stop; do not file an issue and do not suggest a workflow/member/service as a
  substitute resource.
- Family **present** and a specific profile still 404s ⇒ now it genuinely means missing or
  invisible-to-this-caller. Check slug and scope.
- **Feature-branch source cannot establish what production exposes.** Reading `feature/integrate`
  and finding the endpoint proves only that the code exists. The live OpenAPI is the authority for
  what is deployed. This is the same discipline as pinning code to the deployed image.

Profile-specific status codes, once the family is present: `428` = you never sent `If-Match`
(go read the ETag); `412` = stale ETag (reread and **rebuild** the mutation, never blind-replay);
`422` = publish-side validation, read the typed diagnostics; `503` = Ornn resolution / ingress
proof / actor dispatch unavailable; `202` = accepted for dispatch only, never committed or
published. A published profile that no workflow, schedule, channel, or existing conversation uses
is the **expected consumer boundary**, not a failed publish.

**Token lifetimes: read, never recall.** The stack holds several credential classes with wildly
different TTLs — interactive login access token (`JWT_ACCESS_TTL_SECS`, deployment config; code
default 900 s, production instances often set hours), broker/delegated tokens (fixed 300 s,
`oauth_broker_service.rs` / `crypto/jwt.rs`), service-account tokens (`SA_TOKEN_TTL_SECS`, default
3600 s), non-expiring NyxID API keys, and **dedicated scheduled-invocation Agent Keys** (Vault-held,
policy-governed expiry — a scheduled skill agent's default projected lifetime is ~90 days, not
minutes). Two statements like "the token lives 8 hours" and "the run credential lives 5 minutes"
can both be true — about different classes — so a TTL claim that doesn't name its token class is
not evidence. Before attributing any expiry: decode the JWT actually involved and report
`exp − iat` (numbers only, never the token), cite the owning repo's constant, or — for an Agent
Key — read the automation's `credentialExpiresAtUtc` and `credentialGeneration`. A lifetime quoted
from memory is the classic source of confident-but-wrong attributions here.

## Step 3 — Pull the repo and reach a code-grounded root cause

**Pin to the running system first — code is a hypothesis, not the live truth.** The single most
common triage failure is a confident, code-traced root cause that is *wrong* because the running
build differs from what you read. Before you treat any code-grounded cause as fact for a *live*
failure: **(1)** confirm the code you read **matches the deployed commit/image** (read the deployed
image tag; if you have a candidate fix, prove it shipped with `git merge-base --is-ancestor <fix>
<deployed-sha>`); **(2)** confirm the symptom **reproduces on fresh live evidence**, not an old log
window; **(3)** remember **auto-deploy branches roll forward under you** — re-check the image tag
mid-investigation. If code and deployment diverge, downgrade the source reading to a hypothesis and
check the commits newer than the deployed image before attributing the live symptom.

Then get the suspected layer's real source (paths below) and read until you can point at the code
that produces the behavior. **Anchors are subsystem/directory altitude — confirm exact files by
reading; the tree evolves.**

- **Aevatar** (`aevatarAI/aevatar`): tool execution + LLM dispatch in `src/Aevatar.AI.Core`;
  connector adapters in `src/Aevatar.AI.ToolProviders.*` (incl. NyxId, Ornn); readmodels in
  `src/Aevatar.CQRS.Projection.*`; engine + HTTP/OpenAPI in `src/workflow/`; **contract** in
  `src/Aevatar.AI.Abstractions` + `docs/canon/`; errors as workflow exceptions + control-plane 4xx/5xx.
  **Read the deployed branch: Aevatar currently ships from `feature/integrate` — *not* the repo
  default and *not* `dev`.** Reading the wrong branch is the fast way to a wrong root cause; confirm
  the branch against the live image tag (deploy branches move — verify, don't assume).
- **NyxID** (`ChronoAIProject/NyxID`): endpoint handlers in `backend/src/handlers/` (proxy, auth,
  oauth, approvals, mcp, node/ssh); vault logic in `backend/src/services/`; **all error variants +
  numeric codes in `backend/src/errors/`** (observed ranges — confirm: ~2000 auth, ~3000 approval,
  ~4000 ssh, ~5000 service, ~8000 credential-node); **contract** in `backend/src/api_docs.rs`
  (OpenAPI) + `README.md`.
- **Ornn** (`ChronoAIProject/Ornn`): registry + skillsets in `ornn-api/src/domains/`; NyxID/sandbox
  integration in `ornn-api/src/clients/`; **global error handler in `ornn-api/src/middleware/`**;
  **contract** in `ornn-api/src/openapi/` + `README.md`.

**Live-evidence playbook (host-gated — needs cluster access).** Pin the deployment, then pull the
*full* window, then read with native tools:
```bash
# pin what's running (do this FIRST)
kubectl -n <ns> get deploy -l <selector> -o jsonpath='{..image}'          # deployed tag / commit
kubectl -n <ns> get pod  -l <selector> -o jsonpath='{range .items[*]}{.metadata.name}{" "}{.status.startTime}{" "}{.status.containerStatuses[0].restartCount}{"\n"}{end}'
# pull the FULL window to a file — NEVER trust the -l default tail (~10 lines/pod)
kubectl -n <ns> logs -l <selector> --tail=-1 --since=<window> > dump.log
```
Then read `dump.log` with the file/Read tool: token-mangling shell wrappers can corrupt pipes, and
`grep -c` over-counts when a benign event shares a prefix with the failure — count exact matches and
**also count the success case** (see the negative-control discipline below). For the read side, the
**observatory / run-timeline readmodel** shows runs — but it **does not retroactively heal**, so
compare a record's age/version to the deploy time before trusting it. **No cluster access?** Don't
fake it — use the observatory readmodel and hand the problem back to the calling agent (Step 4c).

**Discipline:** the root-cause claim must cite `repo path:line` (+ the commit/ref you read) **and**
that ref must match what's deployed. **No citation, or code that doesn't match the running build ->
no verdict** — it's a hypothesis; downgrade to *inconclusive* (Step 4c). Attribute by **elimination
with a negative control**: count the failure *and its success counterpart* — one failure with zero
observed successes proves nothing (the operation may simply be rarely exercised). Where you can,
design a probe that **flips the symptom** (e.g. a re-bind that turns a `400` token-exchange into a
`200`) for positive proof. Never fabricate a path, a line, a version, or a slug.

**The contract test (this decides the branch):** does the layer's *current behavior* violate its
**own published contract** (README / OpenAPI / proto / `docs/canon`)? **Yes -> defect** (Step 4a).
**No** — the contract says it should behave this way and we drove it wrong **-> usage** (Step 4b).

## Step 4 — Branch on the verdict

### 4a. Platform defect -> file an issue (confirmation-gated)

1. **De-dup first, and check it isn't already fixed.** Search existing issues (open *and* closed) on
   the owning repo; if one exists, point the user to it. Then check whether the cause is **already
   fixed but not yet running** — scan commits since the deployed sha, open PRs, and unmerged branches
   (`git log <deployed-sha>..origin/<branch>`). A fix that exists upstream is not a new defect — point
   to the PR/commit instead. Two traps: a **containment guard** that only rejects the bad state is
   *not* a fix that corrects it; and a deployed fix that sits on a **code path the symptom never
   executes** hasn't fixed anything — confirm the failing operation actually traverses the changed code.
2. **Draft** with this shape:
   - **Title:** `[<layer>] <one-line symptom>`
   - **Body:** environment + version/commit; minimal repro steps; **expected vs actual**; the
     offending **`path:line`** and why it violates the published contract; correlation ids / logs.
3. **Show the user the draft. File only on an explicit "yes."** Route to the **owning repo**
   (`aevatarAI/aevatar` | `ChronoAIProject/NyxID` | `ChronoAIProject/Ornn`). If the cause spans
   layers, file in the layer that first breaks the contract and **cross-link** the others.
4. The bar for an external-repo issue is **"behavior violates its published contract"** — not "we
   wish it worked differently." If it's a feature gap, say so; don't file a defect.

### 4b. Usage / config mistake -> authoritative guidance (no issue)

Explain **grounded in the code/contract you just read** — *why* it behaves this way and *what the
correct usage is* — not generic advice. Then point the user at the right next step **generically**
(e.g. "author the workflow", "publish the service", "connect the connector in NyxID", "check
feasibility first") and hand off to the sibling skill that owns it via `aevatar-platform-map`.

**Credential failures are layered — don't conflate them, and don't reflexively prescribe a re-bind.**
"Connected" can be false green. Three distinct things: **(1)** a **binding mirror** exists locally (a
`whoami` / status check reads only this); **(2)** **local model/route preferences** were applied (a
log like `applied=true` reflects *this*, not a live grant); **(3)** the **live grant** — the only
thing that actually authorizes a tool call. A revoked grant shows `invalid_grant` on the broker's
token-exchange while local state still reads "bound." Re-binding fixes a *revoked grant*; it does
**not** fix a deferred/relayed run that lost its sender token after persistence — prescribing
unbind/re-init there is wrong. Identify which layer failed from a **live trace** before guiding.

### 4c. Inconclusive -> name the missing evidence and the next probe

State exactly what you could not determine and the cheapest probe to get there: a minimal repro;
a capability check; **live logs** (the playbook above) if you have cluster access; an observatory
run-timeline / readmodel-version check; or re-pulling the relevant code at the exact deployed commit.
**Some facts are not log-derivable** — e.g. *why* a grant died, or runtime state that lives in a
broker/projection rather than in config. When the decisive evidence lives on another layer's
authoritative side, say so and escalate there (query that layer's own API / state) rather than guessing.

## Execution paths (pick by the tools you actually have)

### Local coding agent — preferred for deep RCA
```bash
# read the suspected layer (shallow clone; reuse an existing clone if present)
# Aevatar ships from feature/integrate (NOT the repo default, NOT dev) — read THAT ref:
gh repo clone aevatarAI/aevatar -- -b feature/integrate     # NyxID / Ornn: their own deployed branch (confirm)
#   git clone --depth=1 -b feature/integrate https://github.com/aevatarAI/aevatar
# is it already fixed but not deployed? (check before drafting a defect)
git -C <repo> log <deployed-sha>..origin/<branch> --oneline
# de-dup before drafting
gh issue list  -R <owner>/<repo> --search "<keywords> in:title,body"
gh search issues "<keywords>" --repo <owner>/<repo>
# file ONLY after the user confirms the draft
gh issue create -R <owner>/<repo> --title "[<layer>] ..." --body "..."
```

### Server-side, in an aevatar session — constrained fallback
Use the runtime **`nyxid_proxy` tool** (not the `nyxid` CLI), `slug=api-github`, base
`https://api.github.com`:
- read code at the **deployed ref**: `GET /repos/{owner}/{repo}/contents/{path}?ref=<deployed-branch>`
  — for `aevatarAI/aevatar` that ref is **`feature/integrate`** (not the default, not `dev`) —
  `GET /search/code?q=...+repo:owner/repo` (repos are public; raw fetch also works).
- de-dup: `GET /search/issues?q=repo:owner/repo+is:issue+<keywords>`.
- file (confirmed only): `POST /repos/{owner}/{repo}/issues`.

Raw diagnostics require the exact connected UserService plus its slug and relative path. For the
canonical authenticated GitHub identity probe, call:

```json
{
  "service_id": "us-gh-7",
  "slug": "api-github",
  "path": "/user",
  "method": "GET"
}
```

Do not confuse this raw call with a compiled workflow operation. In a compiled workflow, an
admission proof owns UserService, slug, operation ID, method, path template, digest, schemas, and
response policy. The step-level selector stays in `capability.nyxid_operation`. Runtime
`nyxid_proxy.arguments` may contain only admitted `path_params`, `query`, `headers`, `body`, and
`response_mode`. `NYXID_OPERATION_ARGUMENT_NOT_SUPPORTED` for `slug`, `path`, `method`, or similar
route/proof fields is an **Aevatar admission-bound runtime argument rejection before HTTP
dispatch**. Remove those fields from `nyxid_proxy.arguments`; do not move or forge the proof.

**Credential reality — be honest about it.** Under a relayed/in-session call, every tool runs on the
**sender's own NyxID identity**, not the bot owner's. So filing an issue operates the **sender's**
GitHub, and it requires: the sender has connected **`api-github`** in their own NyxID (check
`GET {NYX}/api/v1/services`) with an OAuth scope that allows writing issues (`repo` / `public_repo`).
**Writes have no owner fallback** — without a live sender token you get `credential_denied`. Deep
multi-file reading over the API is clunky; prefer the local path for RCA and use this to fetch
specific files, search code, and de-dup/file.

**Neither available?** Don't fake it — hand the original problem back to the calling agent with the
evidence you gathered (the family's `fallback-to-calling-agent` ethos), so it can finish with its
own local tools.

## Honesty & safety rails

- **Never auto-file.** Always: de-dup -> draft -> explicit user confirmation -> file.
- **No `path:line`, no root cause.** An unverified hypothesis is reported as inconclusive.
- **Code is not the running system.** A code-grounded cause is a hypothesis until it matches the
  deployed commit and reproduces on fresh evidence — old logs and stale checkouts mislead.
- **Dispatch success ≠ real-world effect.** A climbing `fireCount` or a `200` proxy body can still
  mean nothing happened — verify the actual side-effect out-of-band.
- **Negative control before "systemic."** Count the success case too; one failure with no observed
  successes is not evidence of a platform-wide break.
- **Citing a memory/doc is not applying it.** Re-derive the verdict from the evidence in front of you.
- **Read `credentialSourceKind` before any scheduled-credential verdict.** A dedicated Agent Key
  is not a fire-time broker token; quoting the 300 s TTL at it is a wrong attribution, not a shortcut.
- **A 404 is not a deployment probe.** Check the live OpenAPI for the complete route family before
  deciding whether a contract is missing, undeployed, or simply invisible to this caller.
- **Never fabricate** a root cause, an issue link, a version, or a connector slug.
- **External-repo issues only when behavior violates the layer's published contract.**
- **Attribute by reading + elimination**, not first-match — exclude the alternatives.
- **Server-side writes act as the sender** — verify the connector and scope before promising a file.

## Report shape

End with a straight, evidence-bearing summary:

> **Layer:** NyxID — *evidence: 403 from `nyxid_proxy`, slug present in `/services`, OAuth scope
> missing.* **Root cause:** `backend/src/handlers/proxy.rs:NN` rejects when the granted scope lacks
> `repo` (commit `abc123`, **matches deployed image**). **Verdict:** usage. **Action:** guidance —
> reconnect `api-github` with the `repo` scope; no issue filed.

Name the layer, cite the code (and that it matches what's deployed), state the verdict (defect |
usage | inconclusive), and the action (issue drafted, awaiting confirm | guidance given | next probe).
