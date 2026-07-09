---
name: aevatar-platform-map
description: Entry point, panorama, and router for the entire Aevatar skill family — load this FIRST whenever someone wants to build, run, publish, schedule, externally trigger, or operate anything on Aevatar ("create an agent team", "make a workflow / member", "publish or bind a service", "register it with NyxID", "set up a recurring / cron run", "invoke my service", "let Lark Base trigger my workflow"), wants to know whether something is even possible ("can Aevatar do X?", "能不能用 aevatar 实现"), or just wants to know what Aevatar can do. It teaches the object model (scope → team → member[workflow|script|gagent] → service → schedule/external trigger), how to authenticate as a NyxID-bearer REST client, how to resolve your scope, and the two caller modes (client REST vs in-session server-side tools). It does not do the work itself — it routes you to the right companion skill (feasibility-advisor, workflow-authoring, team-builder, service-publisher, scheduler, plus diagnostics probes and the safety-net fallback), held together by the shared `aevatar` tag.
version: "1.7"
metadata:
  category: plain
  tag:
    - aevatar
    - control-plane
    - overview
    - routing
    - nyxid
    - team
    - workflow
    - service
    - schedule
---

# Aevatar control plane — the map

You are the **router and reference** for the Aevatar skill family — you do **not** execute the
work yourself. Your job: orient the agent (object model, auth, caller mode), then hand off to the
*one* companion skill that owns the step the user is on. Read this map first; each spoke is
self-contained, so you can also jump straight in once you know the step.

**What Aevatar is.** A control plane driven entirely over REST at
`https://aevatar-console-backend-api.aevatar.ai`. Everything hangs off your **scope** (your NyxID
subject id), and a request almost always walks one chain:

```
scope → team → member (workflow | script | gagent) → service → schedule / external trigger
```

**Settle three things before you route** (each has a full section below — this is the checklist):
1. **Is it even feasible?** For anything non-trivial, start with **`aevatar-feasibility-advisor`** —
   it says whether the goal is possible and what must be in place first (which NyxID connector to
   configure, what's host-gated, what's impossible + the alternative). Don't build something that
   can't ship.
2. **Which caller mode are you in?** A plain-REST **client** holding a NyxID bearer, or the model
   running **in-session** with server-side tools? Only `aevatar-workflow-authoring` needs the
   server-side tools; everything else is REST either way. See *Two caller modes*.
3. **Carry the honesty rules** into every hand-off — you make real HTTP calls (no magic
   server-side action), most steps are async (read state back, never trust a bare 2xx), and NyxID
   registration is host-gated. See *Honesty rules*.

Then match the user's words to a step in the router below, load that skill, and don't reinvent what
a spoke already owns.

## The object model (one picture)

```
scope  (= your NyxID subject id; your private workspace; everything hangs off it)
  ├── team       a group of members with one "entry member" as its front door
  │     └── member   a callable unit; its implementation is ONE of:
  │            • workflow   (a YAML pipeline of roles + steps)   ← most common
  │            • script     (an app script)
  │            • gagent     (a hosted agent actor)
  ├── service    a member/team published so it can be invoked + (host-gated) registered to NyxID
  ├── schedule   fires a service on a cron, authenticated as you (NyxID)
  └── external trigger  Lark Base / webhook / external cron calls the service invoke path
```

The lifecycle the user almost always wants:
**author a workflow → wrap it in a member → group members into a team → publish as a
service (register to NyxID) → schedule it.**

## Authenticate (every request)

Drive aevatar **through the NyxID broker** — it forwards your identity **and injects the
`scope_id` claim** that aevatar's backend needs. This is the load-bearing detail: sending a raw
NyxID token straight to `https://aevatar-console-backend-api.aevatar.ai` authenticates but resolves
**no scope** (`scopeResolved:false`, `scopeId:null`), so every studio resource under
`/api/scopes/{scopeId}/…` is unreachable and workflow/team creation fails silently. Always go
through the broker:

- **Prerequisite (once):** the `aevatar` service must be connected in NyxID. Verify with
  `nyxid proxy discover | grep aevatar`; if absent, `nyxid service add aevatar` (no credential —
  it is an admin-mode system service that resolves the caller via NyxID `/me`).
- **Every call is** `nyxid proxy request aevatar "<path>" [-m POST -d '<json>']`. NyxID injects the
  bearer **and** the scope claim; you never read or send the token yourself. (The broker forwards to
  base URL `https://aevatar-console-backend-api.aevatar.ai`; you only ever pass the `<path>`.)
- **Resolve your scope once** — `scopeId` is your NyxID subject id:
  ```bash
  scopeId=$(nyxid proxy request aevatar "api/studio/context" \
    | python3 -c 'import sys,json;print(json.load(sys.stdin)["scopeId"])')
  ```
  (`api/auth/me` and `api/workflow/observatory/me` also return `scopeId`.) **If `scopeId` comes back
  null / `scopeResolved:false`, you are not going through the broker — fix the transport, do not
  proceed**: nothing under `/api/scopes/{scopeId}/…` will work.
- All studio resources live under `api/scopes/{scopeId}/…`. Account-level service and schedule
  management live under `api/services` and `api/schedules`. Pass these as the `<path>` to
  `nyxid proxy request aevatar`.

**In-session / platform mode:** if you are the model running *inside* an aevatar session with the
nyxid MCP connected, you instead have server-side tools (`aevatar_start_workflow`, `use_skill`,
`nyxid_services`, …) that already run inside your resolved scope — see *Two caller modes*. Reserve
the raw-token-to-backend call only for environments where neither the `nyxid` CLI nor those tools
exist, and expect to handle scope resolution yourself.

## Two caller modes (this matters for the workflow skill)

Most of this family is **plain REST you call as a client** through the NyxID broker above
(`nyxid proxy request aevatar "<path>"`) — that is the
default assumption here and in `team-builder` / `service-publisher` / `scheduler`. The one
exception is **`aevatar-workflow-authoring`**, written for the model running *inside* an aevatar
session with the nyxid MCP connected, where it uses the **server-side tools**
`aevatar_start_workflow` / `ornn_publish_skill` / `use_skill` / `nyxid_services`. If you are an
external client **without** those tools, that skill also documents a full **client REST path**:
dry-run a workflow with `POST /api/scopes/{scopeId}/workflow/draft-run` (body
`{prompt, workflowYamls:[…]}`), and publish the workflow skill to ornn by POSTing a zip to
`…/api/v1/proxy/s/ornn-api/api/v1/skills` (with the workflow YAML under `assets/`). Pick whichever
surface your tool list actually supports — do not try to call the server-side tools as HTTP
endpoints (they are not).

## Which skill for which task (router)

| You want to… | Use the skill | Key endpoints |
|---|---|---|
| **Decide if a goal is even possible** + what must be in place first (use FIRST, before building) | `aevatar-feasibility-advisor` | read-only `GET /api/v1/services`, `GET /api/v1/catalog` (NyxID) |
| **Triage a failure** — is it an aevatar / nyxid / ornn problem? read the code, then file an issue or get authoritative usage guidance (use AFTER something breaks) | `aevatar-triage` | reads repos via `gh` or `nyxid_proxy` `api-github`; `gh issue` |
| Turn an idea into a runnable **workflow YAML** | `aevatar-workflow-authoring` | server-side tools `aevatar_start_workflow`/`ornn_publish_skill`, **or** client REST `…/workflow/draft-run` + ornn zip publish (see *Two caller modes*) |
| Create a **team**, create **members** (workflow/script/gagent), bind them, set the entry member | `aevatar-team-builder` | `/api/scopes/{id}/teams`, `/members`, `/members/{id}/binding` |
| **Publish** a member/team **as a service** and **register it to NyxID**; verify it | `aevatar-service-publisher` | `/api/scopes/{id}/binding`, `/api/services/*`, `/members/{id}/published-service` |
| Run it on a **cron schedule** (authenticated as you) | `aevatar-scheduler` | `/api/schedules`, `:run-now`, `:enable`, `:disable` |
| Trigger an existing workflow from an external HTTP sender such as **Lark Base** | `aevatar-feasibility-advisor` first, then `aevatar-service-publisher` | NyxID `/api/v1/proxy/s/aevatar/api/scopes/{scopeId}/members/{memberId}/invoke/...`, host-managed `/api/workflow-webhooks/{routeKey}` if configured |
| **Invoke**, watch **runs**, observe | (this map + service-publisher's invoke section) | `/invoke/{endpointId}`, `/runs/*`, `/api/workflow/observatory/*` |

If a companion skill is not already loaded, find it with an ornn skill search for the
capability (e.g. "aevatar team builder", "aevatar service publisher", "aevatar
scheduler"), then load it. None of them depend on this map at run time — they restate the
minimal bootstrap above.

## The full aevatar skill collection

ornn has no separate "collection" object — the aevatar capability set is held together by
a shared **`aevatar` tag** and indexed by this map. An ornn skill search for **`aevatar`**
returns the whole family as one set; load whichever member you need with `use_skill`. This
map is the canonical entry point; the rest are pulled on demand.

**Scope first — feasibility** (`category: plain`, public)
- `aevatar-feasibility-advisor` — *use before building*: is the goal possible, what are its
  prerequisites (which NyxID connector to configure, what's host-gated), and what's impossible
  + the alternative. Teaches the connector-vs-channel split and the prerequisite matrix.

**Diagnose & report — triage** (`category: plain`)
- `aevatar-triage` — *use after something breaks*: attribute a failure across aevatar / NyxID /
  Ornn, read the layer's real code for a code-grounded root cause, then file a GitHub issue
  (confirmation-gated) for a genuine platform defect, or give authoritative, code-grounded usage
  guidance for a misuse. The after-it-breaks counterpart to `aevatar-feasibility-advisor`.

**Build & operate — the control-plane family** (client REST, `category: plain`, public)
- `aevatar-platform-map` — *this map*: object model, auth + scope bootstrap, routing.
- `aevatar-team-builder` — create teams; create + bind members (workflow/script/gagent); set the entry member.
- `aevatar-service-publisher` — publish a member/team/workflow as a service; verify NyxID registration; invoke.
- `aevatar-scheduler` — cron schedules that fire a service (scope-owner NyxID auth).

**Author a workflow** (`category: tool-based`, public)
- `aevatar-workflow-authoring` — turn a request into a validated, persisted workflow YAML
  (server-side `aevatar_start_workflow` / `ornn_publish_skill`, **or** the client REST path —
  `draft-run` + ornn zip publish — see *Two caller modes* above). Its output is the workflow
  a `team-builder` member binds or a `service-publisher` scope binding publishes.

**Diagnose — capability probes** (`category: plain`; currently private/owner-only)
- `aevatar-capability-probe`, `aevatar-workflow-engine-probe`, `aevatar-scripting-probe`,
  `aevatar-vision-probe`, `aevatar-attachment-probe`, `aevatar-file-extract-probe` —
  small self-tests that check whether a given platform capability is available in the
  current scope before you depend on it.

**Safety net — cross-cutting** (`category: plain`, public)
- `fallback-to-calling-agent` — when you genuinely cannot finish a request server-side,
  hand the original problem back to the calling agent instead of failing opaquely. Generic
  (no `aevatar` tag), but part of how this family degrades safely.

## The golden path, end to end

0. **Scope check (do this first)** — confirm the goal is feasible and collect its
   prerequisites (connectors to configure, host-gated pieces, hard limits) —
   `aevatar-feasibility-advisor`. Skip only when the ask is obviously in-scope.
1. **Author** the workflow YAML — `aevatar-workflow-authoring`.
2. **Create team** — `POST /api/scopes/{scopeId}/teams {displayName}`.
3. **Create + bind a workflow member** — `POST /api/scopes/{scopeId}/members`, then
   `PUT /api/scopes/{scopeId}/members/{memberId}/binding` (carries the YAML). The bind is
   async; wait for its binding run to reach `succeeded`. — `aevatar-team-builder`.
4. **Set the team entry member** — `PUT /api/scopes/{scopeId}/teams/{teamId}/entry-member {memberId}`.
5. **Publish as a service + register to NyxID**, then verify the NyxID slug —
   `aevatar-service-publisher`.
6. **Schedule** it on a cron, authenticated as the scope owner — `aevatar-scheduler`; or
   trigger it from an external HTTP system such as Lark Base by calling NyxID's `aevatar`
   proxy with a NyxID API key — `aevatar-service-publisher`.

## Honesty rules (so you never over-promise)

- **You are a client.** Everything here is plain REST you call with the user's NyxID
  bearer token. There is no server-side tool that creates teams/members/services for you —
  you make the HTTP calls.
- **NyxID registration is host-gated.** Publishing a service only results in a NyxID
  connector if the platform host has external exposure enabled (and the service is in
  scope of that policy). You drive publish + verify; you cannot force registration on. If
  the service's `externalExposure` block stays empty, say so: the service is still usable
  in-scope, just not exposed as a NyxID-brokered connector. (Details in
  `aevatar-service-publisher`.)
- **External HTTP trigger is different from externalExposure.** Lark Base / external cron /
  webhook senders can often trigger an existing member/team by calling the NyxID proxy for
  `aevatar` with a NyxID API key and an explicit scope path. Host externalExposure is only
  for turning the Aevatar service itself into a reusable NyxID connector/slug.
- **Many steps are async.** Bindings, deployments, and runs settle over time. Read state
  back (binding run status, invocation readiness, run timeline) instead of assuming
  success from a 2xx.
- **Never fabricate ids.** Always use the ids returned by the create/bind responses.

## If you get stuck

If after a genuine attempt you cannot complete the request server-side (missing
capability, a hard failure, or something that needs the caller's local environment), hand
the original request back to your caller cleanly rather than failing opaquely — see the
fallback skill in this family.
