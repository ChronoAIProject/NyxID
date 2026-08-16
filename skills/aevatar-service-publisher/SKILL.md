---
name: aevatar-service-publisher
description: Publish an Aevatar member, team, or workflow as an invocable service and (host permitting) register it with NyxID, then verify, invoke, or wire external HTTP triggers such as Lark Base automation — all over the REST API. Use when a user wants to "publish/bind a service", "expose my workflow/team as a service", "register it with NyxID", "make it callable", "get the service slug/URL", "invoke my service", "let Lark Base call my workflow", "trigger this workflow from an external webhook", or "version/deploy/roll out a service". It covers the simple scope binding, reading back a member's published service, the full account-level service lifecycle (revision → publish → deploy → rollout), how to confirm the NyxID registration (slug + status), how to invoke an endpoint, and how to distinguish direct NyxID proxy triggering from host-gated externalExposure. Build the team/member first with the team-builder skill.
version: "2.0"
metadata:
  category: plain
  tag:
    - aevatar
    - service
    - publish
    - binding
    - nyxid
    - register
    - invoke
    - deploy
---

# Publish an Aevatar artifact as a (NyxID) service

You turn a member / team / workflow into an **invocable service** and verify whether it is
**registered with NyxID** as a brokered connector. Build the artifact first
(`aevatar-team-builder`). Then pick the path that matches what you have.

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
> the token, so you never touch the aevatar backend or a stored token directly. On the streaming
> `:stream` invoke, the SSE `data:` frames interleave lifecycle frames
> (`stepStarted`/`stepFinished`/`runFinished`/`stateSnapshot`/`usage`, keyed by a top-level field)
> with raw observation frames (`custom.name: aevatar.raw.observed`) that carry the step **output
> text** — there is no flat `type` field, so parse for those keys, not `obj.type`.

## First, the honest constraint about NyxID registration

Registration to NyxID is **automatic but host-gated**. When a service deployment becomes
**active**, the platform reconciles it to NyxID — *only if* the host has external exposure
enabled and the service is in scope of that policy. You can drive publish + activation and
read the result, but you cannot turn host exposure on from the client. So always **verify**
(below) and report honestly: if no NyxID slug appears, the service is still usable
in-scope, it just is not a NyxID-brokered connector.

Do not confuse that with an **external trigger**. An external system such as Lark Base does
not need the workflow itself registered as a NyxID connector if it is only triggering the
owner's existing member/team/service. It can call the already-connected NyxID `aevatar`
proxy with a NyxID API key, using an explicit Aevatar scope/member/team invoke path. Ask
for host `externalExposure` only when the requirement is "make this Aevatar service a
reusable NyxID connector/slug for other callers."

## Path A — A member you already bound (from team-builder)

A bound member **already has an explicit published-service association**. Read it first, then
use the returned `publishedServiceId` to select the service read model. Never derive a service
identity from `memberId`, a `member-` prefix, route position, or string equality:
```bash
publishedServiceId=$(aev "api/scopes/$scopeId/members/$memberId/published-service" | jq -r .publishedServiceId)
aev "api/scopes/$scopeId/services" | jq --arg serviceId "$publishedServiceId" '
      .[] | select(.serviceId==$serviceId)
        | {serviceId, defaultServingRevisionId, invokeReady, invokeReadinessStatus,
           endpoints: [.endpoints[] | {endpointId, requestTypeUrl}], externalExposure}'
```
A workflow member exposes endpoint `chat` (`type.googleapis.com/aevatar.ai.ChatRequestEvent`)
and reports `invokeReady:true` once serving. Then jump to **Verify** and **Invoke**.

## Path B — One-shot: publish a workflow as the scope's service

The fastest way to expose a single workflow/script/gagent as a service for your scope:

```bash
aev "api/scopes/$scopeId/binding" -m PUT -d "{
  \"implementationKind\": \"workflow\",
  \"displayName\": \"My Service\",
  \"serviceId\": \"my-service\",
  \"workflow\": { \"workflowId\": \"my-workflow\", \"workflowYamls\": [ $(jq -Rs . < workflow.yaml) ] }
}"
```
`UpsertScopeBindingHttpRequest`: `implementationKind` (required, `workflow|script|gagent`)
plus the matching typed block (`workflow` / `script` / `gAgent`), `displayName?`,
`serviceId?`, `appId?`, `revisionId?`. List your scope services and read exposure:
```bash
aev "api/scopes/$scopeId/services" | jq '.[] | {serviceId, displayName, externalExposure}'
```

## Path C — Account-level service lifecycle (versioned / advanced)

For a standalone, independently versioned service with staged rollout. Identity is the
4-tuple **`tenantId / appId / namespace / serviceId`** (reuse it on every call).

```bash
# 1. Create the service shell + its endpoint contract(s)
aev "api/services" -m POST -d '{
  "tenantId":"<t>","appId":"<a>","namespace":"<ns>","serviceId":"my-service",
  "displayName":"My Service",
  "endpoints":[{"endpointId":"invoke","displayName":"Invoke","kind":"unary",
    "requestTypeUrl":"<type.googleapis.com/...>","responseTypeUrl":"<...>","description":"..."}]
}'
# 2. Add an implementation revision (one of static / scripting / workflow)
aev "api/services/my-service/revisions" -m POST -d '{
  "tenantId":"<t>","appId":"<a>","namespace":"<ns>","revisionId":"r1",
  "implementationKind":"workflow",
  "workflow":{"workflowName":"my-workflow","workflowYaml":"<yaml>","definitionActorId":null,"inlineWorkflowYamls":null}
}'
# 3. Prepare → publish → deploy
aev "api/services/my-service/revisions/r1:prepare" -m POST
aev "api/services/my-service/revisions/r1:publish" -m POST
aev "api/services/my-service:deploy" -m POST -d '{ ... serving target ... }'
```
Optional staged rollout: `POST …/rollouts` then `:advance` / `:pause` / `:resume` /
`:rollback`; inspect `GET …/serving` and `GET …/traffic`. Bindings (connector + secret)
and access policies: `POST …/bindings`, `POST …/policies`. The service self-describes at
`GET /api/services/{serviceId}/openapi.json`.

Prepare and publish are idempotent for an existing prepared/published revision. A published
revision remains published through replay and duplicate prepare/publish commands; do not mint a
replacement revision or demote it because invocation readiness is temporarily behind. Reconcile
the existing revision and its projection/catalog state.

## Verify (always)

```bash
# Account-level service + its NyxID exposure block
aev "api/services/my-service" | jq '{serviceId, externalExposure}'
# Its own OpenAPI (proves it is serving)
aev "api/services/my-service/openapi.json" | jq '.info, (.paths|keys)'
```
The `externalExposure` block is the NyxID-registration truth:
- `nyxidSlug` — the brokered connector slug (empty ⇒ not registered).
- `status` — registration status; `lastError` — why it failed, if it did.
- `desiredSpecHash` vs `registeredSpecHash` — equal ⇒ NyxID is up to date with the current
  contract; unequal ⇒ a re-registration is pending/needed.
- block entirely absent/empty ⇒ host external exposure is off for this service. Report
  that plainly (see the honest constraint above).

## Invoke

The endpoint contract tells you the path, readiness, and a ready-to-run curl example:
```bash
aev "api/scopes/$scopeId/members/$memberId/endpoints/chat/contract" \
  | jq '{invokePath, canInvoke:.invocationReadiness.canInvoke, curlExample}'
```
Invocation readiness is exact, not a generic "deployed" flag. Require the invocation catalog to
contain the selected revision **and deployment** and to report every selected endpoint ready. The
contract must show `invocationReadiness.canInvoke:true`, `status:"ready"`, the expected
`revisionId`, and a non-empty matching deployment identity. `invocation_catalog_not_ready` is a
materialization lag and means **not invocable yet**; reread the same contract/read model instead of
republishing, creating another revision, or claiming readiness.

The streaming path (`…/invoke/{endpointId}:stream`, SSE) accepts the `{"prompt":"…"}` shorthand.
An opened stream or accepted receipt is not success. Follow the same run to terminal completion,
then read run detail/audit and require completed status, `lastSuccess=true`, expected non-empty
step output, non-empty final output, and no failed audit step:
```bash
aev "api/scopes/$scopeId/members/$memberId/invoke/chat:stream" -m POST --stream \
  -d '{"prompt":"smoke test"}'
# retain the returned run identity privately, then read its detail and audit
```
After acceptance, the stream must emit its first projection-backed business frame within 30
seconds. SSE `: keepalive` is transport-only and does not extend that deadline. Only root
`RUN_FINISHED` and root `RUN_ERROR` terminate observation; role text, reasoning, tool-call,
tool-result, and role terminal frames are progress. Root
`RUN_ERROR(code=RUN_OBSERVATION_TIMEOUT)` closes the stream because observation stalled and is not
proof that the whole run failed. Query the same `actorId + commandId`; never invoke again just to
check status.

The **non-streaming** `…/invoke/{endpointId}` expects the full typed envelope (it rejects a
bare `{prompt}` with 400 "payloadTypeUrl is required") — prefer `:stream` for a quick check.
Teams and account-level services invoke the same way:
`POST /api/scopes/{scopeId}/teams/{teamId}/invoke/{endpointId}[:stream]`,
`POST /api/scopes/{scopeId}/services/{serviceId}/invoke/{endpointId}[:stream]`.

Watch runs: `GET /api/scopes/{scopeId}/services/{serviceId}/runs` and `…/runs/{runId}`
(and `:resume` / `:stop` / `:signal`). For a visual timeline use the observatory:
`GET /api/workflow/observatory/runs`.

## External HTTP triggers (Lark Base, webhook sender, external cron)

Feasibility rule: if the external system can send an HTTPS request to a public URL with
headers and a JSON body, it can usually trigger an Aevatar member/team workflow through
NyxID without Aevatar service `externalExposure`. Lark Base automation's "send HTTP
request" action fits this class: it is an outbound HTTP action to a specified URL, not an
Aevatar inbound chat channel.

### Choose input authority before transport

Do not make `record_id` read-back a universal rule. Choose one ingress contract and keep its
authority explicit:

| Ingress | What it proves | Field policy |
|---|---|---|
| Shared/unverified webhook or broadly shared API key | At most that some caller reached the endpoint | Treat pushed business fields as claims. Accept a locator such as `record_id` and read authoritative fields through an admitted provider capability. |
| Direct NyxID proxy with a credential dedicated exclusively to one Base automation | The dedicated automation principal invoked Aevatar | The workflow may accept pushed fields as input. Do not claim this if people or unrelated automations share the key. |
| NyxID trigger webhook target into Aevatar's Host ingress | Inbound token/HMAC verification, exact outbound body HMAC, stable delivery identity, and durable webhook dedupe/history | Prefer this when Base can set a static bearer token but cannot compute HMAC. The Host may map `payload.<field>` directly into the workflow prompt. |

Even on a trusted path, read Base when the workflow needs fields omitted from the event, must
observe a newer state than the triggering snapshot, or has an independent business requirement to
revalidate. Record the delivery id and source; never infer trust from a route name or payload field.

### Direct NyxID proxy trigger

Create a dedicated NyxID API key with the minimum required proxy authority and store it only in
the external system's secret manager. Never print, log, paste into workflow YAML, or return the
secret value in chat:

```bash
nyxid api-key create --name "lark-base-aevatar-trigger" --scopes proxy
```

Then configure the external HTTP action as:

```http
POST https://nyx-api.chrono-ai.fun/api/v1/proxy/s/aevatar/api/scopes/{scopeId}/members/{memberId}/invoke/chat:stream
Authorization: Bearer [secret configured in the external system]
Content-Type: application/json
Accept: text/event-stream

{"prompt":"Apply Lark email approval for {{record_id}} / {{email}}"}
```

Use the **member** or **team** invoke path that already carries `scopeId`; do not rely on
`api/studio/context` from a bare API key, because that generic context can report
`scopeResolved:false`. For a team entry member:

```http
POST /api/v1/proxy/s/aevatar/api/scopes/{scopeId}/teams/{teamId}/invoke/chat:stream
```

Trade-offs:
- `.../invoke/chat:stream` accepts the prompt shorthand but returns SSE. Use it when the
  external sender can ignore/accept a streaming response and you only need to trigger the run.
- Non-stream `.../invoke/{endpointId}` returns a JSON receipt, but it expects a typed
  request envelope (`payloadTypeUrl` plus `payloadBase64`, or `payloadJson` only when the
  serving revision has a descriptor). A bare `{ "prompt": "..." }` is invalid.
- If the external tool cannot set headers, cannot keep secrets safely, cannot tolerate SSE,
  or cannot build the typed envelope, use an adapter path below instead of forcing it.

If the workflow actually calls the Lark Base read API, `bitable:app:readonly` is only an application API scope. The exact Base document
must also grant the selected Bot application access. Error `91403` is a document ACL denial, not
evidence that the API scope is missing. In the current Base UI, open `...`/More, choose **Add
Applications** (the document-application entry), and add the exact Bot application used by the
selected NyxID UserService. If advanced permissions are enabled, ensure that application is
associated with a role that covers the target table; `1254302 RolePermNotAllow` means that role
coverage is still insufficient.

There is no generic Aevatar "Test connection" response implied by binding or publication. Before
enabling the real trigger, run one explicitly authorized **read-only** probe against a known sample
`record_id`, using the same UserService, Base/app token, table, and admitted read contract. Require
the expected record fields, not only HTTP 2xx. This proves downstream document/role access only; it
does not prove durable workflow admission, trigger delivery, or a later write. Do not use a create,
approval, send, or update call as a connection test. Skip this Base probe entirely when the trusted
push contract supplies all required fields and the workflow contains no Base read.

### Adapter path

Use a small webhook/HTTP adapter (or a custom NyxID service) when you need a normal JSON
ACK, body transformation, request signing, idempotency, or Lark-specific payload cleanup.
The adapter receives the Lark Base request, validates its own secret, maps the record fields
to an Aevatar prompt or typed payload, calls the NyxID proxy, and returns a simple 2xx/JSON
response to Lark Base.

### Scope-owned webhook ingress bindings (self-serve)

Aevatar has a workflow webhook ingress at:

```http
POST /api/workflow-webhooks/{routeKey}
```

On hosts from 2026-08-13 onward, its bindings are **scope-owned data**, managed self-serve
(bearer-authenticated as the scope) with no host configuration or redeploy:

```http
PUT    /api/scopes/{scopeId}/workflow-webhooks/{routeKey}
GET    /api/scopes/{scopeId}/workflow-webhooks
DELETE /api/scopes/{scopeId}/workflow-webhooks/{routeKey}
```

Binding fields and guarantees:

- **Exact target:** `definitionActorId` is required. It must identify a committed workflow
  **Definition** in the caller's scope; a run actor, `memberId`, `workflowId`, or
  `publishedServiceId` is not accepted. The server pins the committed `targetRevisionId`; send the
  revision you read as an expectation and treat a mismatch as 409 drift. `workflowName`, when
  supplied, is only a consistency check against that exact Definition, never an alternative
  target. The webhook payload cannot choose the workflow or revision.
- **Required mapping and delivery identity:** provide exactly the intended `promptTemplate` or
  `promptJsonPath`, plus a valid `deliveryIdJsonPath` whose value is inside the signed body. A
  configured `deliveryIdHeader` is only a second consistency check and must equal that signed-body
  value when present.
- **Structured prompt template:** it must be valid JSON. Place `{{payload.field}}` paths and the
  ingress variables `{{@run_date}}` / `{{@received_at_unix_ms}}` only in JSON string values, never
  property names. `@run_date` is local `yyyy-MM-dd`, derived from the received time through the binding's IANA
  `timeZoneId`; omitted timezone defaults to **UTC**, not UTC+8. Set `Asia/Singapore` explicitly
  when that is the business timezone.
- **HMAC:** `hmacSecret` is required and must contain at least 32 UTF-8 bytes. By default the
  signature is `sha256=HMAC_SHA256(secret, timestamp + "." + rawBody)` in
  `X-Aevatar-Signature`, with Unix seconds in `X-Aevatar-Timestamp` and 300 seconds of skew.
  Override header names only to match a known sender contract.
- **Replay admission:** the signed-body delivery ID is durable first-writer-wins and reuse with
  different payload bytes is rejected. This prevents ordinary duplicate starts, but it is not a
  terminal run lease/completion protocol and must not be advertised as crash-safe exactly-once.
  Business effects still need idempotency and committed run/effect evidence.
- **Secrets:** `hmacSecret` is write-only (views expose `hmacSecretSet`, never its value);
  `previousHmacSecret` permits bounded zero-downtime rotation. Never persist a user bearer token
  in the binding.
- **Route ownership:** dynamic and host-configured routes share one global namespace. A host-reserved
  route or route owned by another scope returns 409.
- **Accepted is not complete:** 202 means only that start was accepted. Reconcile the returned run
  identity/status URL to a committed root terminal state and verify any external effect separately.

Resolve the exact target from the scope workflow read model, never from a guessed ID:

```bash
aev "api/scopes/$scopeId/workflows/<linked-workflow-id>" \
  | jq '{available, definitionActorId:.source.definitionActorId,
         revisionId:.workflow.activeRevisionId, deploymentStatus:.workflow.deploymentStatus}'
```

Require `available=true`, a non-empty Definition actor and revision, and a ready/active deployment.
If you started from a Team member, first read its linked workflow identity; do not use the member or
published service ID in the webhook target field.

One complete NyxID-trigger binding request has this shape (keep the secret in a private variable;
never print the rendered body):

```bash
binding=$(jq -nc \
  --arg actor "$definitionActorId" \
  --arg revision "$targetRevisionId" \
  --arg secret "$deliverySigningSecret" \
  '{definitionActorId:$actor,targetRevisionId:$revision,
    promptTemplate:({record_id:"{{payload.record_id}}",run_date:"{{@run_date}}"}|tojson),
    timeZoneId:"Asia/Singapore",deliveryIdJsonPath:"event_id",
    deliveryIdHeader:"X-NyxID-Delivery-Id",hmacSecret:$secret,
    hmacSignatureHeader:"X-NyxID-Signature",hmacTimestampHeader:"X-NyxID-Timestamp",
    enableUnattendedEffects:false}')
aev "api/scopes/$scopeId/workflow-webhooks/<route-key>" -m PUT -d "$binding"
```

Read the binding list back and compare the exact actor, revision, prompt string, timezone, delivery
mapping, headers, and `hmacSecretSet=true`. For a direct webhook call, retain the accepted run/status
identity. Through a NyxID trigger, correlate the stable signed `event_id`/source and delivery window
to the one new committed Aevatar run; delivery history proves transport, not workflow completion.

### Optional unattended effects

Bindings start runs by default; HMAC authenticity alone grants no downstream write authority.
`enableUnattendedEffects=true` is accepted only from an authenticated direct-human NyxID authority
for an exact, versioned, Durable Definition. The server seals the caller's binding authority and
only the eligible authored-request write call sites from that committed revision; it never stores
the bearer. The permit does not flow into LLM, fork, or subworkflow contexts.

This opt-in crosses only Aevatar's local tool-approval gate. NyxID operation policy and the target
provider can still reject or require approval. It is not a substitute for Team-schedule operation
authority, so never use a webhook-effect permit to make an authored write-capable schedule pass.

Older hosts keep the appsettings-managed binding list (`WorkflowWebhookIngress:Enabled` +
bindings); report those as host-managed. If the management API answers 503, the host lacks a
binding store (its error message names the required configuration).

NyxID v0.10+ can provide the missing sender-facing adapter and durable delivery contract:

```bash
nyxid trigger create \
  --label "Lark Base to Aevatar" \
  --verification bearer \
  --delivery webhook \
  --delivery-url "https://<aevatar-host>/api/workflow-webhooks/<route-key>" \
  --output json
```

Store the returned inbound `secret` only in the Base automation's `Authorization: Bearer ...`
header. Store the one-time `delivery_signing_secret` only in the Aevatar binding's
`hmacSecret` (write-only). Register the binding with
`hmacSignatureHeader=X-NyxID-Signature`, `hmacTimestampHeader=X-NyxID-Timestamp`, and
`deliveryIdHeader=X-NyxID-Delivery-Id`, and set `deliveryIdJsonPath=event_id`; NyxID signs the exact bytes
`timestamp + "." + body` and sends `X-NyxID-Key-Id` for rotation. The delivered JSON wraps the
original Base body under `payload`, so map fields with `promptTemplate` placeholders such as
`{{payload.name}}`. Never place either secret in workflow YAML, chat, logs, issue text, or a URL.
The binding does not select secrets by `X-NyxID-Key-Id`; rotate zero-gap by running
`nyxid trigger rotate-delivery-secret`, re-PUTting the binding with the new `hmacSecret` and
the old value as `previousHmacSecret`, then re-PUTting without `previousHmacSecret` once the
sender has flipped.
Use `nyxid trigger deliveries` and an explicit `nyxid trigger redeliver` for failed retained
deliveries; never replay by issuing a second business mutation blindly. Load the NyxID skill's
`references/triggers.md` for the full trigger lifecycle and retention limits.

## Next

- **Schedule this service on a cron:** `aevatar-scheduler` — it needs the service identity
  (`tenantId/appId/namespace/serviceId`), an `endpointId`, and the payload type you found
  in the contract above.
- Lost? Load `aevatar-platform-map`.

If you cannot complete a step server-side after a real attempt, hand the original request
back to your caller rather than fabricating — see the fallback skill in this family.
