---
name: aevatar-triage
description: Use AFTER something goes wrong while using Aevatar — a user hits an error, failure, or confusing behavior and you must find whether it lives in Aevatar, NyxID, or Ornn, then act. Triggers - "aevatar is erroring", "why did my workflow fail", "my scheduled run did not fire", "my bot does not reply", "connector 401/403", "skill won't pull/upload", "is this an aevatar, nyxid, or ornn bug", "file an issue", "am I using this right". It attributes the failure by tracing the request path, pulls that layer's real public source for a code-grounded root cause citing file and line, then branches - draft and, only on explicit user confirmation, file a precise GitHub issue when behavior violates the layer's published contract, or explain the correct usage from the code when it is a usage mistake. The after-it-breaks counterpart to aevatar-feasibility-advisor; never auto-files, de-dups first, never claims a root cause without a code citation. Works locally (git + gh) and server-side (nyxid_proxy + api-github).
version: "1.3"
metadata:
  category: plain
  tag:
    - aevatar
    - triage
    - diagnostics
    - debugging
    - root-cause
    - issue
    - nyxid
    - ornn
    - support
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

## Step 1 — Capture the symptom precisely (don't theorize yet)

Collect, verbatim where possible: the **exact error text**; the **operation / surface** (workflow
run, connector call, channel bot, schedule, skill pull/publish, auth, a control-plane REST call);
**minimal repro**; the **ids** that let you correlate — `run_id`, `scopeId`, connector `slug`,
skill name, `commandId`/`correlationId`, schedule fire-record fields (`fireCount` / `lastFireAt` /
`nextFireAt` / `failureCount` / `lastError`), timestamps; and **what is actually deployed** — the
running **image tag / commit**, pod age + `restartCount`, and the **time window** of the failure
(live logs rotate fast, so an old window may already be gone). Missing ids are themselves a finding.

**First, rule out the cheap local causes** before blaming a layer: your own **expired / missing
credential** (a blanket `401` is often just your own stale token, not a platform bug — refresh it
and retry) and a **stale local checkout** (the code you're about to read may be behind what's
deployed).

## Step 2 — Attribute to a layer (trace the request path, eliminate)

**Main rule: follow the request path and find the *first boundary* that breaks.** A user request
typically flows `your agent -> Aevatar runtime -> NyxID proxy -> third-party`, and skills flow
`Aevatar use_skill -> Ornn`. Map the symptom to where that chain first fails:

| Symptom | Most likely layer | Disambiguating evidence to gather |
|---|---|---|
| `401` / `403` on a connector/tool call | **NyxID** (auth/vault) | is the `slug` in `GET {NYX}/api/v1/services`? token expired? OAuth scope present? |
| "credential not found" / connector slug missing | **NyxID** (vault / not connected) | catalog vs services; was it ever connected for *this* identity? |
| `404` on a thing you reference | **whichever registry owns it** | skill -> Ornn; connector/service -> NyxID; team/member/scope -> Aevatar |
| `502` / timeout on an external call | **proxy chain** | which hop? Aevatar tool layer vs NyxID proxy vs the target itself |
| workflow won't validate / run stalls / binding never `succeeded` | **Aevatar** (engine/runtime) | draft-run error body; run timeline / observatory; binding-run status |
| readmodel stale / observatory missing data | **Aevatar** (projection) | is the projection subscription live? event stream flowing? authoritative version vs readmodel — note readmodels **do not back-fill** (compare record age to deploy time) |
| **scheduled run stopped firing** (fired before; `nextFireAt` frozen in the past, `fireCount` flat, `failureCount=0`, empty `lastError`) | **Aevatar** (scheduler / actor not re-armed across pod churn) | compare to peer schedules; pod `startTime` / `restartCount`; is it still enabled? did a deploy/restart line up with the last good fire? |
| **scheduled run never fires, or fires but errors on credential** | **Aevatar scheduler ⨯ NyxID** (binding) | is it enabled and is `nextFireAt` computed? `lastError` like "binding not found" / "exactly one credential source" -> the *fired call's* invocation credential (scope-owner broker binding) |
| **schedule fires (`fireCount` climbs) but the real-world effect never happens** | **Aevatar** (the fired call's path / credential) | dispatch success ≠ effect — check the external side-effect out-of-band; the proxy can hand back `{"error":true}` inside a `200` |
| **scheduled run starts fine, then late steps hit `token_expired` ~5 min after fire** | **by design (NyxID broker TTL)** | the run's caller credential is minted once at fire and pinned to `BROKER_ACCESS_TTL_SECS = 300` (NyxID `backend/src/services/oauth_broker_service.rs`) — fast-revocation design, not a bug; correlate failure onset with **fire time** (per-run ~5 min mark ⇒ broker TTL) vs a fixed wall-clock time (⇒ something else); fix by shortening the run / front-loading NyxID-authenticated steps, not by retrying |
| **inbound bot doesn't reply** (Lark/Telegram) | **cross-layer — walk it** | did NyxID relay webhook fire? is the bot connector connected? did the Aevatar channel run start (observatory)? credential = the *sender's* NyxID, present and live? |
| **`/whoami` says "bound" but tool calls get `credential_denied`** | **NyxID** (grant revoked — false green) | live token-exchange returns `invalid_grant` while the local readmodel still reads "bound"; whoami checks only the local mirror, not the live grant |
| approval prompt stuck | **NyxID approvals + Aevatar suspension** | NyxID approval request id; Aevatar workflow wait/suspend state |
| skill search/pull/upload/generate fails | **Ornn** | which `/api/v1/skill...` route? validator violations? version format? |

**Do not stop at the first match.** Gather the disambiguating evidence and *eliminate* — a plausible
first guess that you haven't excluded the alternatives for is not an attribution.

**Token lifetimes: read, never recall.** The stack holds several credential classes with wildly
different TTLs — interactive login access token (`JWT_ACCESS_TTL_SECS`, deployment config; code
default 900 s, production instances often set hours), broker/delegated tokens (fixed 300 s,
`oauth_broker_service.rs` / `crypto/jwt.rs`), service-account tokens (`SA_TOKEN_TTL_SECS`, default
3600 s), non-expiring NyxID API keys. Two statements like "the token lives 8 hours" and "the run
credential lives 5 minutes" can both be true — about different classes — so a TTL claim that
doesn't name its token class is not evidence. Before attributing any expiry: decode the JWT
actually involved and report `exp − iat` (numbers only, never the token), or cite the owning
repo's constant. A lifetime quoted from memory is the classic source of confident-but-wrong
attributions here.

## Step 3 — Pull the repo and reach a code-grounded root cause

**Pin to the running system first — code is a hypothesis, not the live truth.** The single most
common triage failure is a confident, code-traced root cause that is *wrong* because the running
build differs from what you read. Before you treat any code-grounded cause as fact for a *live*
failure: **(1)** confirm the code you read **matches the deployed commit/image** (read the deployed
image tag; if you have a candidate fix, prove it shipped with `git merge-base --is-ancestor <fix>
<deployed-sha>`); **(2)** confirm the symptom **reproduces on fresh live evidence**, not an old log
window; **(3)** remember **auto-deploy branches roll forward under you** — re-check the image tag
mid-investigation. If code and deployment diverge, the bug may already be fixed (an unmerged branch
or a follow-up commit) or the running build is simply different.

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
