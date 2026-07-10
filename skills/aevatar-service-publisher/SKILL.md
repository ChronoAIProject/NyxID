---
name: aevatar-service-publisher
description: Publish an Aevatar member, team, or workflow as an invocable service and (host permitting) register it with NyxID, then verify, invoke, or wire external HTTP triggers such as Lark Base automation — all over the REST API. Use when a user wants to "publish/bind a service", "expose my workflow/team as a service", "register it with NyxID", "make it callable", "get the service slug/URL", "invoke my service", "let Lark Base call my workflow", "trigger this workflow from an external webhook", or "version/deploy/roll out a service". It covers the simple scope binding, reading back a member's published service, the full account-level service lifecycle (revision → publish → deploy → rollout), how to confirm the NyxID registration (slug + status), how to invoke an endpoint, and how to distinguish direct NyxID proxy triggering from host-gated externalExposure. Build the team/member first with the team-builder skill.
version: "1.5"
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

A bound member **already has a published service** — binding it was the publish. The
`published-service` endpoint only returns ids; the real detail (endpoints, readiness,
NyxID exposure) lives in the scope services list:
```bash
# minimal: publishedServiceId + key
aev "api/scopes/$scopeId/members/$memberId/published-service" | jq .
# full: identity, endpoints, serving revision, invokeReady, externalExposure
aev "api/scopes/$scopeId/services" \
  | jq '.[] | select(.serviceId=="member-'"$memberId"'")
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
The reliable smoke test is the **streaming** path (`…/invoke/{endpointId}:stream`, SSE),
which accepts the `{"prompt":"…"}` shorthand and returns workflow-run frames ending in a
`runFinished` event with the result:
```bash
aev "api/scopes/$scopeId/members/$memberId/invoke/chat:stream" -m POST --stream \
  -d '{"prompt":"smoke test"}'
# look for:  data: {... "runFinished": { "result": { "output": "..." } } }
```
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

### Direct NyxID proxy trigger

Create a non-expiring NyxID API key with `proxy` scope and store it as the external
system's secret:

```bash
nyxid api-key create --name "lark-base-aevatar-trigger" --scopes proxy
```

Then configure the external HTTP action as:

```http
POST https://nyx-api.chrono-ai.fun/api/v1/proxy/s/aevatar/api/scopes/{scopeId}/members/{memberId}/invoke/chat:stream
Authorization: Bearer <NYXID_API_KEY>
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

### Adapter path

Use a small webhook/HTTP adapter (or a custom NyxID service) when you need a normal JSON
ACK, body transformation, request signing, idempotency, or Lark-specific payload cleanup.
The adapter receives the Lark Base request, validates its own secret, maps the record fields
to an Aevatar prompt or typed payload, calls the NyxID proxy, and returns a simple 2xx/JSON
response to Lark Base.

### Host-managed webhook ingress

Aevatar also has a host-configured workflow webhook ingress at:

```http
POST /api/workflow-webhooks/{routeKey}
```

It supports binding-level `RouteKey`, `SourceId`, `WorkflowName`, `ScopeId`,
`PromptTemplate` or `PromptJsonPath`, delivery-id extraction, replay dedupe, and HMAC-SHA256
auth (`X-Aevatar-Timestamp` plus `X-Aevatar-Signature` over `timestamp.body` by default).
This is a good first-class bridge for Lark Base-style webhooks **if the host configures the
binding and secret** (`WorkflowWebhookIngress:Enabled` + binding + replay store). It is not
currently a normal client self-serve operation, so report it as host-managed.

## Next

- **Schedule this service on a cron:** `aevatar-scheduler` — it needs the service identity
  (`tenantId/appId/namespace/serviceId`), an `endpointId`, and the payload type you found
  in the contract above.
- Lost? Load `aevatar-platform-map`.

If you cannot complete a step server-side after a real attempt, hand the original request
back to your caller rather than fabricating — see the fallback skill in this family.
