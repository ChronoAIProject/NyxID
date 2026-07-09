---
name: aevatar-scheduler
description: Create and manage cron schedules that fire an Aevatar service on a recurring basis, authenticated as the scope owner via NyxID — over the REST API. Use when a user wants to "schedule", "run on a cron", "set up a recurring run", "run every day/hour/Monday", "automate this service on a timer", "preview a cron", "pause/resume/disable a schedule", or "run it now" — or hits token_expired on a scheduled run's late steps. It builds the schedule against a published service (identity + endpoint + payload + serving revision), uses scope-owner NyxID auth (which requires the owner's NyxID broker binding), documents the fire-time credential's fixed 5-minute lifetime and how to design runs around it, and covers preview, enable/disable, run-now, update, and delete. Publish the service first with the service-publisher skill.
version: "1.6"
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
---

# Schedule an Aevatar service on a cron

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
- **identity** — the 4-tuple `{tenantId, appId, namespace, serviceId}`. For a workflow
  member the `serviceId` is `member-<memberId>`.
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

**Diagnose before re-logging in** — the binding lives on the NyxID side, so check it directly:
```bash
NYX=$(tr -d '\n' < ~/.nyxid/base_url); TOK=$(tr -d '\n' < ~/.nyxid/access_token)
curl -s -H "Authorization: Bearer $TOK" "$NYX/api/v1/users/me/broker-bindings" \
  | jq -r '.bindings[] | "\(.client_name)  scopes=\(.scopes|join(","))  last_used=\(.last_used_at)"'
```
A non-revoked `aevatar` binding with the `proxy` scope means NyxID is healthy and the fault
is Aevatar-side (it can be pinned to a stale binding). A **clean** console re-login (fully
logged out first) refreshes a revoked binding — finalize replaces it on the revoked/stale
probe path — so that usually clears it; an SSO-cached login may not re-run finalize.

**There is no CLI / headless path to establish this binding** (NyxID mints broker bindings
only via the `authorization_code` grant; the only Aevatar writer is the browser finalize).
Tracked at **aevatarAI/aevatar#2491** — do not promise a CLI-only way to create a
`scopeOwnerNyxId` schedule until it lands.

### CLI-only alternative: skip the Aevatar scheduler entirely
For a recurring run **without the browser console**, don't use `scopeOwnerNyxId` scheduling
at all. The published service is already invocable — drive it from an **external timer**
(cron, `launchd`, a node) that hits the invoke endpoint with a **non-expiring NyxID API key**
(`nyxid api-key create --scopes proxy`; export as `NYXID_ACCESS_TOKEN`). No broker binding,
no console:
```bash
NYXID_ACCESS_TOKEN="$KEY" nyxid proxy request aevatar \
  "api/scopes/$scopeId/members/$memberId/invoke/chat:stream" -m POST --stream \
  -H 'Content-Type: application/json' -d '{"prompt":"poll"}'
```
The member invoke endpoint carries `scopeId` in its path, so it runs even though a bare API
key reports `scopeResolved:false` on the generic `api/studio/context` call. Trade-off: the
timer runs on whatever machine you put it on (a cloud cron would live in Aevatar; this does not).

## The fire-time credential lives 5 minutes — design the run around it

At every fire, Aevatar exchanges the stored broker binding for a **fresh access token** (OAuth
token-exchange, `subject_token_type=urn:nyxid:params:oauth:token-type:binding-id`) and projects
that **one** token into the run as its caller credential — minted once, shared by the whole run
(aevatar: `agents/Aevatar.GAgents.Channel.Identity/Broker/NyxIdRemoteCapabilityBroker.cs`,
`src/platform/Aevatar.GAgentService.Infrastructure/Schedules/ScheduledServiceInvocationDispatchPort.cs`).
Broker-issued tokens are pinned to **`BROKER_ACCESS_TTL_SECS = 300`** (NyxID
`backend/src/services/oauth_broker_service.rs`) so a revoked binding stops working within
5 minutes without introspection. Deliberate design, not a bug.

**Consequence:** every NyxID-authenticated step in the fired run must complete within ~5 minutes
of fire. In a longer run, late steps failing with `token_expired` is the **expected** platform
behavior — keep scheduled runs short, front-load the NyxID-authenticated steps, or split long
pipelines into separate schedules. Do not "fix" it by retrying the same run shape.

**Token classes are not interchangeable — never quote one class's TTL for another.** Your
interactive login token lives for hours (`JWT_ACCESS_TTL_SECS`, a deployment config — code
default is 900 s); the scheduled run's credential lives 300 s (fixed constant); NyxID API keys
(`nyxid api-key create`) don't expire at all. When diagnosing any expiry, read the lifetime
instead of recalling it: decode the JWT actually in hand and report `exp − iat` (numbers only —
never print the token), or cite the owning repo's constant.

## Create the schedule

```bash
aev "api/schedules" -m POST -d "{
  \"displayName\": \"Weekday 9am run\",
  \"cronExpression\": \"0 9 * * 1-5\",
  \"timezone\": \"Asia/Shanghai\",
  \"enabled\": true,
  \"serviceInvocation\": {
    \"identity\": { \"tenantId\": \"$scopeId\", \"appId\": \"default\", \"namespace\": \"default\", \"serviceId\": \"member-<memberId>\" },
    \"endpointId\": \"chat\",
    \"payloadTypeUrl\": \"type.googleapis.com/aevatar.ai.ChatRequestEvent\",
    \"payloadJson\": $(jq -nc '{prompt:"do the thing"} | tojson'),
    \"revisionId\": \"<defaultServingRevisionId>\",
    \"auth\": { \"scopeOwnerNyxId\": { \"scope\": \"proxy\" } }
  }
}"
```

`ScheduledDispatchConfigurationHttpRequest`: `cronExpression` (required); `displayName?`,
`timezone?`, `enabled` (default true), `headers?` (string map), and **exactly one** target:
`serviceInvocation` (above) or `envelope` (a raw actor `EventEnvelope` — advanced).

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
