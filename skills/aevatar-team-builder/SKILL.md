---
name: aevatar-team-builder
description: Build an Aevatar agent team and its members over the REST API. Use when a user wants to "create a team", "add a member", "make a workflow member / script member / gagent member", "set the team's entry point", or "assemble agents into a team". It creates the team, creates members whose implementation is a workflow (most common), a script, or a hosted gagent, binds each member's concrete implementation (the workflow YAML is attached here), waits for the async binding to succeed, and sets the team entry member. Author the workflow YAML first with the workflow-authoring skill; publish the result as a service with the service-publisher skill.
version: "1.5"
metadata:
  category: plain
  tag:
    - aevatar
    - team
    - member
    - workflow
    - gagent
    - script
    - studio
    - create-team
---

# Build an Aevatar team and its members

You create a **team**, fill it with **members** (each backed by a workflow, script, or
gagent), bind their implementations, and set the team's entry member — all via REST. The
output is an invocable team. Publishing it as a NyxID service is a separate step
(`aevatar-service-publisher`); scheduling is another (`aevatar-scheduler`).

## Bootstrap

```bash
# Drive aevatar THROUGH the NyxID broker: it injects your scope_id claim AND auto-refreshes your
# token. A raw curl to the aevatar backend with ~/.nyxid/access_token resolves NO scope
# (scopeResolved:false) and the stored token expires — it is not a usable path.
# Prerequisite once: the `aevatar` service must be connected — `nyxid service add aevatar`.
aev() { nyxid proxy request aevatar "$@"; }   # aev "<path>" [-m POST|PUT|DELETE] [-d '<json>'] [--stream]
scopeId=$(aev "api/studio/context" | jq -r .scopeId)
```

> **`jq` is only for convenience** — any JSON reader works (replace `| jq -r .scopeId` with
> `| python3 -c 'import sys,json;print(json.load(sys.stdin)["scopeId"])'`). All calls go through the
> NyxID broker (`nyxid proxy request aevatar`), which injects your `scope_id` claim and
> auto-refreshes the token. Create and bind calls are mutations: an empty body or lost response is
> an uncertain outcome, not permission to retry. Reconcile through the corresponding member/team/
> binding-run read model or an idempotency key before deciding whether another mutation is safe.

Member implementation kinds are the lowercase strings **`workflow`**, **`script`**,
**`gagent`**.

Member routes authorize the authenticated caller against the route `scopeId`. They do not require
the caller subject, `memberId`, or any other resource ID to be equal. Keep scope authority and
resource identity separate; never rewrite an ID to make those strings match.

## Step 1 — Create the team

```bash
teamId=$(aev "api/scopes/$scopeId/teams" -m POST \
  -d '{"displayName":"My Team","description":"what it does"}' | jq -r '.teamId // .id')
```
`CreateStudioTeamRequest`: `displayName` (required), `description?`, `teamId?` (omit to
let the server mint one). Read the returned id back — do not invent it.

## Step 2 — Create the member shell

Create the member as a **shell**. Do **not** pass `implementationRef` here — the concrete
implementation (the workflow + its YAML) is attached in Step 3.

```bash
wfId="wf-my-workflow"   # workflow draft identity; it is not the member or published-service id
memberId=$(aev "api/scopes/$scopeId/members" -m POST -d "{
  \"displayName\": \"My Workflow Member\",
  \"implementationKind\": \"workflow\",
  \"teamId\": \"$teamId\"
}" | jq -r '.memberId')
```
`CreateStudioMemberRequest`: `displayName` + `implementationKind` (required,
`workflow|script|gagent`); `description?`, `memberId?`, `teamId?` (attach now, or add
later via PATCH). The new member returns at `lifecycleStage:"created"` and is already
assigned a `publishedServiceId`; its `implementationRef` stays `null` until Step 3 fills
it in.

If the requested team does not exist, member creation and provisioning return typed HTTP 404:
`{"code":"STUDIO_TEAM_NOT_FOUND","scopeId":"<scope>","teamId":"<team>"}` (plus a
message). Treat this as a missing Team resource in the stated scope; do not retry, invent another
team ID, or reinterpret `teamId` as a member/workflow/service identity.

Keep all three identities distinct: the response `memberId` owns Team authority, `wfId` names the
workflow draft/definition, and `publishedServiceId` is the callable runtime identity. Only explicit
contracts may translate between them; never derive one from another. Use visibly different shapes
in fixtures and examples.

- **script / gagent members** are created the same way — just set `implementationKind`
  to `"script"` or `"gagent"`. Discover valid gagent kinds with `GET /api/scopes/gagent-types`.
  The concrete `scriptId` / `agentKind` is supplied in the Step 3 binding, not here.

## Step 3 — Bind the member's implementation (attach the YAML)

This is where the real implementation lands. It starts an **async binding run**. A successful bind
commits workflow/revision identity and the capability admission plan together; a later run must
preserve all three. If execution fails, do not create another binding/run until run detail and audit
identify the first failed boundary.

```bash
# Author the YAML first with aevatar-workflow-authoring; pass it inline.
runId=$(aev "api/scopes/$scopeId/members/$memberId/binding" -m PUT -d "{
  \"workflow\": { \"workflowId\": \"$wfId\", \"workflowYamls\": [ $(jq -Rs . < workflow.yaml) ] }
}" | jq -r '.bindingRunId')      # returns {status:"accepted", bindingRunId:"bind-...", ...}
```
`UpdateStudioMemberBindingRequest` carries exactly one of:
- `workflow`: `{workflowId, workflowYamls:[<yaml strings>]}`
- `script`:   `{scriptId, scriptRevision?}`
- `gAgent`:   `{agentKind, endpoints?}`

(`jq -Rs .` safely JSON-encodes the YAML file as a string.)

### Wait for the bind to succeed (it is asynchronous — typically ~1–2 minutes)

Poll the binding run **by its id** until `status` is `succeeded`:
```bash
aev "api/scopes/$scopeId/members/$memberId/binding-runs/$runId" \
  | jq '{status, failure}'
```
Status progresses `accepted → admission_pending → admitted → platform_binding_pending →
… → succeeded` (or `failed`/`rejected`). It commonly sits at `platform_binding_pending`
for a minute or two before flipping to `succeeded` — keep polling (e.g. every 5s, up to
~3 min). On `succeeded` the response carries `result.publishedServiceId` +
`result.revisionId`, and the member reaches `lifecycleStage:"bind_ready"`:
```bash
aev "api/scopes/$scopeId/members/$memberId" \
  | jq '{stage:.summary.lifecycleStage, svc:.summary.publishedServiceId, ref:.implementationRef}'
```
Do not report success on the 2xx from the PUT alone — that is only `accepted`; wait for the
run to reach `succeeded`.

Binding success is not yet invocation readiness. Read the selected endpoint contract and require
the exact bound revision, deployment, and endpoint to be visible in the invocation catalog:
```bash
aev "api/scopes/$scopeId/members/$memberId/endpoints/chat/contract" \
  | jq '{revisionId, deploymentStatus, readiness:.invocationReadiness}'
```
Proceed only when `invocationReadiness.canInvoke == true`, `status == "ready"`, and the returned
`revisionId` matches the binding result. `invocation_catalog_not_ready` means projection/catalog
materialization has not observed that endpoint revision; the member is **not invocable yet** even
though the binding run succeeded. Continue reading the same contract until it changes or report
the lag honestly; do not create a new binding to force readiness.

## Step 4 — Set the team entry member

The entry member is the team's front door (what callers hit by default).

```bash
aev "api/scopes/$scopeId/teams/$teamId/entry-member" -m PUT \
  -d "{\"memberId\":\"$memberId\"}"
```
Add more members by repeating Steps 2–3 with the same `teamId`. List the roster:
`GET /api/scopes/$scopeId/teams/$teamId/members`.

## Verify

```bash
aev "api/scopes/$scopeId/teams/$teamId"          | jq .
aev "api/scopes/$scopeId/teams/$teamId/members"  | jq .
```
Confirm the team exists, the roster contains your member(s), and the entry member is set.

## Edit / clean up

- Rename: `PATCH /api/scopes/{scopeId}/teams/{teamId}` and `PATCH …/members/{memberId}`.
- Move a member into the team later: `PATCH …/members/{memberId}` with `{teamId}`.
- Archive a team: `POST …/teams/{teamId}/archive`.
- Re-bind (new YAML/revision): repeat Step 3; each bind is a new revision.

## Next

- **Publish it as a NyxID-registered service:** `aevatar-service-publisher`.
- **Run it on a schedule:** `aevatar-scheduler`.
- Lost? Load `aevatar-platform-map` for the full panorama.

If you genuinely cannot complete a step server-side, hand the original request back to
your caller rather than fabricating — see the fallback skill in this family.
