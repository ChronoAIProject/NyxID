---
name: aevatar-scheduler
description: Create and manage recurring Aevatar runs and route to the correct scheduling resource. Use for cron, recurring Team member workflows, scheduled skill agents, typed service invocations, pause/resume, run-now, reauthorization, deletion, or credential triage. Team member automation uses its dedicated route and Agent Key; generic schedules accept typed service invocation only. External raw actor/envelope schedules are retired and must fail closed.
version: "1.10"
metadata:
  category: plain
  tag:
    - aevatar
    - schedule
    - cron
    - recurring
    - automation
    - nyxid
    - timer
    - agent-key
    - team-member
---

# Schedule an Aevatar service on a cron

## Route to the right resource FIRST — three paths, not one

"Run it on a schedule" is three different resources with three different owners, credentials, and
APIs. Pick before you call anything; using the wrong one is not a shortcut, it is the wrong resource.

| The user wants to schedule… | Resource owner | Canonical entry | Credential |
|---|---|---|---|
| An already-bound **Studio Team member** workflow | `scope → team → member` | `aevatar_schedule_member_workflow`, or REST `/api/scopes/{scopeId}/teams/{teamId}/members/{memberId}/automations` | **Dedicated Agent Key** |
| An independent **scheduled Ornn skill agent** or a **one-shot reminder** | Scheduled agent / catalog actors | `scheduled_agent_creator`, then `agent_builder` — see `aevatar-automation` | **Dedicated Agent Key** |
| A typed **service invocation** | Generic platform schedule actor | Generic `/api/schedules` — the rest of this skill | Typed source; may be a NyxID binding exchange |

**If the owner is a Team member, the canonical path is the member automation route — not generic
`/api/schedules`.** Generic `/api/schedules` is a platform-level resource for typed service invocations. It
is supported, but it is *not* a fallback for Team member automation, and reaching for it because
you already know its shape is the most common mistake here.

Never convert between identities by route position, prefix, equality, or a familiar-looking string:
`memberId`, draft `workflowId`, `publishedServiceId`, UserService ID, catalog service ID, schedule
ID, and Agent Key ID are seven distinct things.

## Studio Team member automation (the canonical member path)

Use `aevatar_schedule_member_workflow` in-session, or the owner-scoped REST route. The owner is the
exact `scopeId + teamId + memberId` tuple; `teamId` is a containment guard. **The server** reads the
member summary and derives `publishedServiceId` — you never substitute a draft `workflowId`, a
service ID, a binding ID, a route, a model, or a credential ID for it.

**Preflight is read-only and provisions nothing.** It builds a typed authorization plan from current
facts: the exact member and containing Team, the bound workflow revision and prepared artifact,
typed connector and NyxID capability refs, the owner LLM route/model and exact UserService ID where
required, the owner-scoped NyxID authorization catalog, and policy/version/expiry/disclosure facts.
Never invent grants, wildcards, node IDs, route or model choices, or caller binding evidence.

**Create** uses `credentialProvisioningKind = dedicated_scheduled_invocation_agent_key`. The server
revalidates the confirmed permission digest and policy version against current sources before any
key-creation effect, then requests a fresh, targeted NyxID scope plan for the exact sorted
UserService IDs. Key creation fixes `allow_all_services = false` and `allow_all_nodes = false` —
only the targeted plan supplies allowed service IDs, node IDs, and the provider scope-plan digest.

**Secret custody.** The one-time raw key NyxID returns is written **only** to `ISecretVault` under
purpose `scheduled.invocation-agent-key`. Durable state, events, read models, logs, tools, and
public APIs expose only non-secret facts: a typed reference, Agent Key ID, expiry, authorization
fact, and credential generation. **Never ask the user to paste an Agent Key, accept one in schedule
JSON, print one, store one in an Ornn package, or reconstruct one from an ID.**

### After a `202` — what it does and does not prove

`202 Accepted` (and tool status `accepted` / `pending`) means **command admission only**. It does
not prove credential issuance completed, the Vault write completed, activation committed, the read
model observed it, cron fired, or the workflow succeeded.

Reread the canonical owner-scoped automation and require a **newer authoritative `stateVersion`**.
A ready automation normally shows:

- `authorizationStatus == active`
- `credentialSourceKind == scheduled_invocation_agent_key`
- `enabled == true` (firing)
- a future `credentialExpiresAtUtc`
- a positive `credentialGeneration`
- `revocationPending == false`

**`active` is credential health; `enabled` is firing. They are independent dimensions** — do not
report one as the other.

### Fire and how the workflow gets the credential

Cron and run-now use the same active credential generation. Run-now proves a **manual** fire only;
it never proves cron-origin execution.

Aevatar projects a borrowed `DurableCallerCredentialRef` into the workflow caller context. The raw
Agent Key is **not** copied into workflow state. Every LLM, tool, and connector path that needs the
caller credential resolves that reference through `ISecretVault` **at the moment it is used**, and
resolution is fail-closed for an absent, expired, revoked, mismatched, or malformed reference.

**So do not describe this path as one 300-second broker token minted at fire and shared for the
whole run.** That describes a *generic* schedule using a NyxID binding source — a different
credential source with different lifetime semantics.

### Lifecycle operations

- **Pause / resume.** Pause disables future firing but **preserves the active credential**. Resume
  re-enables firing if the credential is still usable. Neither is a revoke or a reauthorization —
  never recreate or revoke a key to resume.
- **Update.** Cron, timezone, prompt, display name, and enabled changes revalidate current
  authorization facts. An update **never silently expands key grants** or swaps the active
  credential. Authorization drift surfaces as an explicit reauthorization requirement.
- **Reauthorize** (this is what a *new external service dependency* requires). Start from a new
  preflight and confirmation, provision a **new** dedicated key generation from the newly validated
  exact plan, and only **after the replacement generation is committed** revoke the old one. Never
  mutate an existing key in place to widen its authority.
- **Delete.** The tombstone and revocation intent commit before external cleanup. **NyxID key
  revocation and Vault secret revocation are independent durable tracks.** The row stays visible as
  `deleting` / `revocation_pending` while either required track is incomplete. Retry reuses the
  **original delete operation and idempotency identity** plus fresh owner authority — it never
  synthesizes a new delete, reauthorization, or credential. Only completion of all required tracks
  lets the automation disappear.
- **Expiry and drift.** An expired, missing, revoked, unresolvable, or authorization-mismatched key
  **fails closed**: the automation moves toward `needs_authorization` and future fire leases are
  canceled. It must never fall back to an interactive user bearer, a Host default, an inferred
  binding, an inferred service, or a wildcard grant.

### Stable recovery

- Lost create response → reread by exact owner and **original operation identity**; never create a
  second schedule with new identities.
- `authorization_plan_changed` → rerun preflight before retrying.
- `needs_authorization` → new preflight and explicit reauthorize.
- `revocation_pending` → retry revocation with the **original delete operation identity**.
- Missing owner binding or authorization catalog evidence → fail closed and report the prerequisite;
  never invent evidence or trigger projection repair.
- Projection pending → report eventual visibility and the required version; never replay or prime
  projection from the query path.

---

## Generic typed-service schedule (`/api/schedules`)

Everything below is the **generic** typed service-invocation resource. External callers cannot
schedule a raw actor `EventEnvelope`; that target is retired and rejected. Never use an envelope
as a fallback for a Team member.

You create a **schedule** that fires a published service on a cron expression,
authenticated as **you** (the scope owner) through NyxID. Publish the service first
(`aevatar-service-publisher`) — you need its identity, an endpoint, and the payload type.

## Bootstrap

```bash
# Drive aevatar THROUGH the NyxID broker: it injects your scope_id claim AND auto-refreshes your
# token. A raw curl to the aevatar backend with ~/.nyxid/access_token resolves NO scope
# (scopeResolved:false) and the stored token expires — it is not a usable path.
# Prerequisite once: the `aevatar` service must be connected — `nyxid service add aevatar`.
# NOTE: the aevatar backend requires `Content-Type: application/json` on writes (POST/PUT) —
# omit it and every write returns HTTP 415 Unsupported Media Type. The helper sets it on
# every call (harmless on bodyless GETs), so the POST/PUT examples below work as written.
aev() { nyxid proxy request aevatar "$@" -H 'Content-Type: application/json'; }   # aev "<path>" [-m POST|PUT|DELETE] [-d '<json>'] [--stream]
scopeId=$(aev "api/studio/context" | jq -r .scopeId)
```

> **`jq` is only for convenience** — any JSON reader works (replace `| jq -r .scopeId` with
> `| python3 -c 'import sys,json;print(json.load(sys.stdin)["scopeId"])'`). All calls go through the
> NyxID broker (`nyxid proxy request aevatar`), which injects your scope_id claim and auto-refreshes
> the token. Reminder: the `scopeOwnerNyxId` precondition below cannot be satisfied by a bare NyxID
> **CLI** token — it needs the owner's interactive **console** NyxID login (broker binding), or
> creation 400s.

## Gather the target (one call: the scope services list)

`GET /api/scopes/{scopeId}/services` returns everything you need per service — copy it off
the entry for your service:
```bash
aev "api/scopes/$scopeId/services" \
  | jq '.[] | {tenantId, appId, namespace, serviceId, defaultServingRevisionId, invokeReady,
               endpoints: [.endpoints[] | {endpointId, requestTypeUrl}]}'
```
- **identity** — copy the explicit 4-tuple `{tenantId, appId, namespace, serviceId}` from the
  service read model or member `published-service` association. Never derive `serviceId` from
  `memberId`, a prefix, route position, or string equality.
- **endpointId** + **payloadTypeUrl** — from `endpoints[]` (`payloadTypeUrl` = the
  endpoint's `requestTypeUrl`). A workflow member's default endpoint is `chat` with
  `type.googleapis.com/aevatar.ai.ChatRequestEvent`.
- **revisionId** — use the service's `defaultServingRevisionId`. **Required** whenever you
  send `payloadJson` (see below).
- **payloadJson** — the request body as a JSON **string** (or `payloadBase64` for a packed
  proto). For a chat endpoint, `{"prompt":"…"}` is accepted.
- Confirm `invokeReady` is `true` before scheduling — a schedule against a not-yet-serving
  service will fire into nothing.

## Preview the cron first (no clock guessing)

```bash
aev "api/schedules/preview" -m POST \
  -d '{"cronExpression":"0 9 * * 1-5","timezone":"Asia/Shanghai","count":"5"}' | jq .
```
Returns the next N fire times so you can confirm the expression means what the user wants.
Use a real IANA `timezone`; the engine has no implicit local time.

## Precondition: the scope owner needs a NyxID owner (broker) binding

A scheduled service fire happens *later*, after your current token has expired, so the
platform must be able to **re-mint** the scope owner's NyxID credential at fire time. That
requires an **authenticated NyxID owner binding** (`urn:nyxid:scope:broker_binding`),
established by signing in through the Aevatar console / studio NyxID login (a browser PKCE
`authorization_code` flow → `POST /api/auth/nyxid/finalize`). A plain NyxID-CLI token is
**not** sufficient. Create-time validation does a *real* token mint, so a missing/revoked
binding fails fast at create with one of:

> HTTP 400 — "Authenticated NyxID owner binding is required for scope owner schedule auth…"
> HTTP 400 — "NyxID binding was revoked for the scheduled subject. (Parameter 'configuration')"

Diagnose from the typed create/preflight error and the Aevatar read model. Never read a stored
access token, construct an Authorization header, or call NyxID/Aevatar directly with `curl`.
A **clean** console re-login (fully
logged out first) refreshes a revoked binding — finalize replaces it on the revoked/stale
probe path — so that usually clears it; an SSO-cached login may not re-run finalize.

**There is no CLI / headless path to establish this binding** (NyxID mints broker bindings
only via the `authorization_code` grant; the only Aevatar writer is the browser finalize).
Tracked at **aevatarAI/aevatar#2491** — do not promise a CLI-only way to create a
`scopeOwnerNyxId` schedule until it lands.

### CLI-only alternative: skip the Aevatar scheduler entirely
For a recurring run **without the browser console**, don't use `scopeOwnerNyxId` scheduling.
An explicitly authorized external timer may invoke the already-published member/team endpoint
through NyxID using a dedicated minimum-authority API key kept in that timer's secret manager.
Never print the key, put it in workflow YAML, export it into a shared shell history, or expose it
in diagnostics.
The member invoke endpoint carries `scopeId` in its path, so it runs even though a bare API
key reports `scopeResolved:false` on the generic `api/studio/context` call. The same pattern
works for **event-driven external triggers** such as Lark Base automation's "send HTTP request"
action: store the NyxID API key as the external system's secret and POST to the explicit
member/team invoke path. This is not Aevatar `externalExposure`; externalExposure is only
needed when the workflow must be registered as a reusable NyxID connector/slug. Trade-off:
the timer or event sender runs outside Aevatar (a cloud cron would live in Aevatar; this does not).

## The binding-exchange fire-time credential lives 5 minutes — design the run around it

**Scope of this section: generic `/api/schedules` using a NyxID binding source only.** Read
`credentialSourceKind` before applying any of it. If the run reports
`credentialSourceKind = scheduled_invocation_agent_key`, none of the numbers below apply — jump to
*Diagnosing a scheduled-run credential failure*.

For a **binding-exchange** source, at every fire Aevatar exchanges the stored broker binding for a
**fresh access token** (OAuth token-exchange,
`subject_token_type=urn:nyxid:params:oauth:token-type:binding-id`) and projects that **one** token
into the run as its caller credential — minted once, shared by the whole run
(aevatar: `agents/Aevatar.GAgents.Channel.Identity/Broker/NyxIdRemoteCapabilityBroker.cs`,
`src/platform/Aevatar.GAgentService.Infrastructure/Schedules/ScheduledServiceInvocationDispatchPort.cs`).
Broker-issued tokens are pinned to **`BROKER_ACCESS_TTL_SECS = 300`** (NyxID
`backend/src/services/oauth_broker_service.rs`) so a revoked binding stops working within
5 minutes without introspection. Deliberate design, not a bug.

**Consequence:** every NyxID-authenticated step in such a fired run must complete within ~5 minutes
of fire. In a longer run, late steps failing with `token_expired` is the **expected** platform
behavior — keep those scheduled runs short, front-load the NyxID-authenticated steps, or split long
pipelines into separate schedules. Do not "fix" it by retrying the same run shape.

## Diagnosing a scheduled-run credential failure

**Identify the typed credential source before interpreting any 401 or `token_expired`.** Gather the
canonical path and owner tuple, `credentialSourceKind`, `authorizationStatus`,
`credentialExpiresAtUtc`, `credentialGeneration`, `stateVersion`, `lastAuthorizationErrorCode`,
`revocationPending`, `nyxIdRevocationStatus`, `vaultRevocationStatus`, and the exact run/tool
failure and timestamp.

| `credentialSourceKind` | Diagnosis |
|---|---|
| `nyxid_binding_exchange` | The 300 s broker TTL above is the right hypothesis. Confirm with the exchanged token's `exp − iat` or the repo constant, then fix run shape. |
| `scheduled_invocation_agent_key` | **Do not diagnose a fixed five-minute broker expiry merely because the call was scheduled.** Inspect key expiry, Vault resolution, reference integrity, committed caller authority, authorization fact, the exact service/node grants versus what the failing step actually called, `credentialGeneration`, and revocation state. Then look at the **downstream** service's own token. A `token_expired` six minutes after fire on this source is **not** explained by the 300 s constant — that constant belongs to a different credential class. |

**Token classes are not interchangeable — never quote one class's TTL for another.** Your
interactive login token lives for hours (`JWT_ACCESS_TTL_SECS`, a deployment config — code
default is 900 s); a binding-exchange run credential lives 300 s (fixed constant); a dedicated
scheduled-invocation Agent Key is Vault-held with a policy-governed expiry (a scheduled skill
agent's default projected lifetime is ~90 days); NyxID API keys (`nyxid api-key create`) don't
expire at all. When diagnosing any expiry, read the lifetime instead of recalling it: decode the
JWT actually in hand and report `exp − iat` (numbers only — never print the token), cite the owning
repo's constant, or read the automation's `credentialExpiresAtUtc` and `credentialGeneration`.

**Never emit secret material** in any report, log excerpt, artifact, or message: no raw Agent Key,
bearer/access/refresh/delegation/service-account token, Vault reference or ciphertext, permission
digest, unfiltered API-key inventory, or authorization header. Stable resource IDs may be reported
for management or cleanup, but never as secret material and never to derive another identity.

## Create the schedule

```bash
aev "api/schedules" -m POST -d "{
  \"displayName\": \"Weekday 9am run\",
  \"cronExpression\": \"0 9 * * 1-5\",
  \"timezone\": \"Asia/Shanghai\",
  \"enabled\": true,
  \"serviceInvocation\": {
    \"identity\": { \"tenantId\": \"<from-service-contract>\", \"appId\": \"<from-service-contract>\", \"namespace\": \"<from-service-contract>\", \"serviceId\": \"<published-service-id-from-contract>\" },
    \"endpointId\": \"chat\",
    \"payloadTypeUrl\": \"type.googleapis.com/aevatar.ai.ChatRequestEvent\",
    \"payloadJson\": $(jq -nc '{prompt:"do the thing"} | tojson'),
    \"revisionId\": \"<defaultServingRevisionId>\",
    \"auth\": { \"scopeOwnerNyxId\": { \"scope\": \"proxy\" } }
  }
}"
```

`ScheduledDispatchConfigurationHttpRequest`: `cronExpression` (required); `displayName?`,
`timezone?`, `enabled` (default true), `headers?` (string map), and exactly one
`serviceInvocation` target. Any external `envelope` target must be rejected.

> **`payloadJson` requires `revisionId`.** If you supply `payloadJson` without a
> `revisionId` (and the service has no *active* serving revision), creation fails with
> 400 "payloadJson requires a revisionId; provide one explicitly or activate a serving
> revision." Pass the service's `defaultServingRevisionId`.

> **Workflow-member services: use `payloadBase64`, not `payloadJson`.** A `member-<id>`
> service produced by a Studio **bind** (the common workflow path) carries a serving
> revision with **no protocol descriptor**, so `payloadJson` fails creation with
> 400 "payloadTypeUrl '…ChatRequestEvent' could not be resolved: revision '…' has no
> protocol descriptor set." The fix is to send the request as a packed proto in
> `payloadBase64` instead — it bypasses the descriptor-based JSON encoding. The streaming
> invoke (`…/invoke/chat:stream`) accepts the `{"prompt":"…"}` shorthand via a shim, but
> the scheduler's typed path does not. For a `ChatRequestEvent` with `prompt` at field 1:
> ```bash
> # python3 -c "import base64;print(base64.b64encode(bytes([0x0a,len(p:=b'do the thing')])+p).decode())"
> # → swap the `payloadJson` line for:  "payloadBase64": "CgxkbyB0aGUgdGhpbmc=",
> ```
> If your workflow ignores the prompt (e.g. a self-contained poll), any valid
> `ChatRequestEvent` payload triggers the run.

### Auth (`serviceInvocation.auth`)

- **`scopeOwnerNyxId: { scope }`** — fire as the scope **owner**, re-minting their NyxID at
  fire time. The right choice for owner-run schedules, but it requires the owner's broker
  binding (see **Precondition** above), otherwise creation 400s.
- **`senderNyxId: { subject: { platform, externalUserId, tenant? }, scope }`** — fire as a
  specific external subject. Only when the schedule must run as someone other than the
  owner, and that subject already has a durable NyxID binding — otherwise the fire fails at
  credential-mint time.

## Verify, then manage

```bash
sid=$(...)   # scheduleId from the create response
aev "api/schedules"            | jq '.[] | {scheduleId, displayName, cronExpression, enabled, nextFireUtc}'
aev "api/schedules/$sid"       | jq .
aev "api/schedules/$sid:run-now" -m POST   # fire once immediately to test
aev "api/schedules/$sid:disable" -m POST   # pause
aev "api/schedules/$sid:enable" -m POST    # resume
aev "api/schedules/$sid" -m PUT -d '{ ...updated configuration... }'
aev "api/schedules/$sid" -m DELETE         # remove
```
Note the action verbs use a colon (`/{scheduleId}:run-now`), not a slash.

After `:run-now`, confirm the fire actually executed — check the service's runs
(`GET /api/scopes/{scopeId}/services/{serviceId}/runs`) or the observatory
(`GET /api/workflow/observatory/runs`). A 2xx on the schedule call means *accepted*, not
*succeeded*; a fire can still fail later at credential-mint or execution time, so read the
run back before reporting success.

## Next

- Need to (re)publish the target service? `aevatar-service-publisher`.
- Want the whole picture? `aevatar-platform-map`.

If you cannot complete a step server-side after a real attempt, hand the original request
back to your caller rather than fabricating — see the fallback skill in this family.
