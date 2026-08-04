---
name: aevatar-agent-profile-management
description: Use when a user wants to create, edit, validate, publish, or explain an Aevatar Agent Profile; set purpose or instructions; bind exact Ornn skills and activation modes; restrict tool authority; or diagnose ETag, validation, publication, deployment exposure, or runtime binding. Agent Profile is a separate resource, not a workflow, member, team, service, or schedule. Use the opaque profileId, exact `{skillGuid, literalVersion, expectedName, expectedPublisherId}` references, strong ETags, accepted-vs-committed semantics, and capability detection before mutation.
version: "1.1"
metadata:
  category: plain
  tag:
    - aevatar
    - agent-profile
    - profile
    - instructions
    - skill-binding
    - tool-policy
    - ornn
    - routing
---

# Aevatar Agent Profile management

An **Agent Profile** answers *"who is this agent, and what may it do?"* — display name,
purpose, instructions, an ordered set of **exact** Ornn skill bindings, and the **maximum**
tool authority any turn under that profile may hold.

It is an **independent resource surface**. It is not another phase of the studio lifecycle:

```
build & operate :  scope → team → member (workflow|script|gagent) → service → schedule
Agent Profile   :  ownerHandle/profileSlug → opaque profileId → draft → published snapshot
```

`profileId`, `workflowId`, `memberId`, `publishedServiceId`, and a conversation actor id are
**five different identities**. No prefix, string match, or lifecycle story converts one into
another. Creating or publishing a Profile creates **no** workflow, member, team, service,
schedule, or conversation binding.

## Step 0 — Detect the capability. Do this before anything else.

The Profile contract is a **deployment-gated** capability. It ships in the Aevatar codebase but
a given deployment may not expose it. **A 404 on a profile resource is not a deployment probe** —
probe the surface itself, and stop honestly when it is absent.

**In-session tool mode.** Use the Profile path **only if the exact tool `agent_profiles` is
present in your current tool list.** Never infer it from another `aevatar_*` tool, from Ornn
access, from the system prompt, or from this skill being loaded.

**Client REST mode.** Read the live document and require the **complete** route family:

```bash
nyxid proxy request aevatar "api/openapi.json" \
  | jq -r '.paths|keys[]|select(test("agent-profiles"))'
```

```text
POST   /api/scopes/{scopeId}/agent-profiles
GET    /api/scopes/{scopeId}/agent-profiles/{profileSlug}
PUT    /api/scopes/{scopeId}/agent-profiles/{profileSlug}/draft
PUT    /api/scopes/{scopeId}/agent-profiles/{profileSlug}/draft/skills/{bindingId}
DELETE /api/scopes/{scopeId}/agent-profiles/{profileSlug}/draft/skills/{bindingId}
POST   /api/scopes/{scopeId}/agent-profiles/{profileSlug}:validate
POST   /api/scopes/{scopeId}/agent-profiles/{profileSlug}:publish
GET    /api/agent-profiles/{ownerHandle}/{profileSlug}
```

Require the **whole family**, not one route — a partially deployed or mixed-version host is not
a safe mutation surface.

**If the surface is absent:** say so plainly — *"this deployment does not expose Agent Profile
management"* — and **stop before any mutation**. You may draft a proposed profile for later use,
but you must not claim you created, validated, published, or bound anything. **Do not fall back
to creating a workflow, member, team, service, or schedule instead** — those are different
resources and would not give the user a Profile.

Once the family *is* present, a later `404` means only *this profile does not exist or is not
visible to you* — it no longer means the contract is undeployed.

## The authority model

- Ownership is a typed authenticated user identity **plus** an owning scope; the caller owner is
  implicit. System profiles use a reserved system owner handle you cannot write to.
- The human reference is two separate fields: **`ownerHandle`** and **`profileSlug`**. Each is
  lowercase kebab (`[a-z0-9]+(-[a-z0-9]+)*`). `profileId` is **opaque and immutable** — never
  parse it, never construct one.
- A profile holds a mutable **draft** and, after publication, an immutable server-sealed
  **published snapshot**. They advance on independent counters: `draftRevision` and
  `publishedRevision`, each with its own SHA-256 digest.

## Lifecycle

1. **create** the profile and its initial draft.
2. **get** the owner management model — this is where the strong **ETag** comes from.
3. **mutate** under that ETag: replace the draft, or upsert/remove one skill binding.
4. **validate** the complete canonical draft.
5. **publish** a server-sealed snapshot.
6. **reread** until the accepted mutation is materially visible.

### Draft body vs skill bindings — a real trap

`PUT …/draft` carries **only** `displayName`, `purpose`, `instructions`, `toolPolicy`, and
optional `recoveryToolPolicy`. **It cannot carry skill bindings.** Bindings are managed *only*
one at a time through `PUT`/`DELETE …/draft/skills/{bindingId}`, where `bindingId` is a
**path parameter you choose**, not a body field. Updating the draft does not clear bindings.

### Request bodies are closed

Every request body rejects unknown members (`400`). Every listed field is required — including
`toolNames` and `toolSetRefs`, which must be present even when empty (`[]`). Enums travel as
their literal screaming-snake strings (`ALWAYS`, `EXPLICIT_ALLOWLIST`, `READ_ONLY`, …).

## Concurrency: the strong ETag protocol

`GET …/agent-profiles/{profileSlug}` returns header `ETag: "agent-profile-v<N>"` where `<N>` is
the authority state version. The server re-parses that string **exactly**: quotes included, no
leading zeros, no weak-tag prefix, exactly one `If-Match` value.

| You did | You get |
|---|---|
| Omitted `If-Match` on a versioned mutation | **`428` `AGENT_PROFILE_IF_MATCH_REQUIRED`** — you never read state; go read it |
| Sent a malformed/rewritten ETag | `400` `INVALID_AGENT_PROFILE_IF_MATCH` — pass it back **verbatim** |
| Sent a stale (but well-formed) ETag | `412` — someone else moved the profile; **reread and rebuild** the intended mutation, never blindly replay |

`428` and `412` are different failures. Do not report one as the other.

**Idempotency.** `Idempotency-Key` is **required on create** (`400 IDEMPOTENCY_KEY_REQUIRED`
without it) and optional on the other mutations. Reuse the same key when retrying the *same*
intent; mint a new one for a new intent.

Versioned mutations = `update_draft`, `upsert_skill`, `remove_skill`, `publish`.
`get` and `validate` take no ETag.

## Skill bindings must be exact

Every binding carries exactly four identity facts, and all four are verified against Ornn at
validate/publish time:

| Field | Meaning |
|---|---|
| `skillGuid` | Ornn's stable skill GUID |
| `literalVersion` | literal `<major>.<minor>` — `^(0\|[1-9][0-9]*)\.(0\|[1-9][0-9]*)$` |
| `expectedName` | canonical skill name |
| `expectedPublisherId` | publisher id you expect to own it |

**Name-only references, `latest`, dist-tags, version ranges, inferred publishers, and inline
skill bodies are all invalid Profile authority.** Resolve the four facts from Ornn first
(search, then read the skill detail), then bind. A mismatch at publish time surfaces as
`SEALED_SKILL_GUID_MISMATCH` / `…_LITERAL_VERSION_MISMATCH` /
`…_CANONICAL_NAME_MISMATCH` / `…_PUBLISHER_ID_MISMATCH`.

Ornn is a **publish-side** dependency only. Runtime turns never search Ornn or refetch profile
authority — publication seals normalized instructions, declared tools, and assets into the
snapshot with deterministic digests.

### Activation modes

| Mode | Behavior | Routing policy |
|---|---|---|
| `ALWAYS` | Joins every profiled prompt. Cannot widen tools. | **Must be absent** — attaching one is `UNEXPECTED_SKILL_ROUTING_POLICY` |
| `ROUTED` | Selected by exact trigger alias or bounded classifier | **Required** (`MISSING_SKILL_ROUTING_POLICY`) |
| `DEFAULT_FOR_UNMATCHED_TURN` | Eligible only on a true no-match / no routed candidates. **At most one may be published** (`MULTIPLE_DEFAULT_SKILLS`) | **Required** |

A routing policy is: `intentId` (`[a-z0-9]+([._-][a-z0-9]+)*`, unique across the profile),
`routingDescription`, `explicitTriggerAliases` (≤ 16, globally unique), `taskToolPolicy`, and a
`sideEffectClass` of `READ_ONLY` | `EXTERNAL_HANDOFF` | `SERVICE_CALL` | `MAINTENANCE`.

### Tool policies are ceilings, never grants

- Profile `toolPolicy` is the **maximum**: `INHERIT_ROUTE_MAXIMUM` or `EXPLICIT_ALLOWLIST`.
- `recoveryToolPolicy` and every `taskToolPolicy` are **narrowing** policies. They must be
  `EXPLICIT_ALLOWLIST` and must be a **subset** of the profile maximum — inheriting is itself a
  violation (`RECOVERY_TOOL_POLICY_EXCEEDS_PROFILE_MAXIMUM`,
  `TASK_TOOL_POLICY_EXCEEDS_PROFILE_MAXIMUM`).
- **No profile, policy, or skill ever grants anything** — no credentials, OAuth scopes, API keys,
  tools, or services. A policy can only *remove* authority the route and caller already hold.

Bounds worth knowing: ≤ 32 skill bindings, ≤ 128 tool names, ≤ 32 tool-set refs, ≤ 16 aliases
per policy, plus aggregate prompt byte/token caps (`AGGREGATE_PROMPT_BYTES_EXCEEDED`,
`MATERIALIZED_PROFILE_LAYER_TOKENS_EXCEEDED`).

## Validate, then publish

`:validate` resolves every binding against Ornn and returns **`200 OK` with a report** —
`{valid, draftRevision, draftDigest, diagnostics[], resolvedSkills[]}`. **`valid: false` is a
successful call, not an error.** Read `diagnostics[]` (each has `code`, `message`, `path`) and
fix the draft. Do not treat a `200` as "the draft is fine" without reading `valid`.

Both `validate` and `publish` need the caller's live NyxID access token for Ornn resolution —
without it you get `ORNN_ACCESS_TOKEN_REQUIRED`; if Ornn itself is down,
`ORNN_DEPENDENCY_UNAVAILABLE` / `503`.

Publish only after a `valid: true` report, using the **latest** reread ETag.

## `202` means accepted — nothing more

Every mutation returns **`202 Accepted`** with
`{accepted, ackStage:"accepted", operationId, commandId, correlationId, actorId, profileId, resourceUrl}`.

That receipt promises **dispatch only**. It does **not** promise actor commit, projection
visibility, publication, or any runtime binding. Record the operation and correlation facts, then
**reread the management model** and confirm the expected `draftRevision` / `publishedRevision` /
digest actually moved before you report success. `lastMutation.status` will read `APPLIED`,
`NO_CHANGE`, or `REJECTED` — `NO_CHANGE` means your mutation was a semantic no-op, not a failure.

## What consumes a published profile (be honest about this)

Publication is **not** runtime binding. In the current design, the only runtime consumer is
**newly created NyxID direct conversations selected by a host-owned rollout admission manifest**.
The binder resolves the reference, loads the protected execution model by opaque `profileId`,
verifies the expected revision, snapshot digest, exact closure, and admission pins, then freezes a
complete immutable binding into the conversation at creation time.

Therefore, state plainly when asked:

- **existing conversations do not hot-upgrade** — the binding is frozen at creation;
- **publishing does not bind anything** by itself;
- **arbitrary owner profiles are not auto-selected** by the system rollout — admission is
  host-controlled, exactly like NyxID external exposure, and you cannot enable it as a client;
- **Workflow, Studio, relay, Channel, Scheduled, AgentRun, services, and schedules are not
  current profile consumers.** If a user asks to "bind my profile to this schedule/workflow",
  say it is not a consumer and **do not mutate the schedule or workflow** to fake it.

Turn-time selection can only preserve or reduce the committed ceiling. Failures degrade to the
recovery policy or a restricted-empty tool set — never back to unrestricted authority.

## In-session tool contract (`agent_profiles`)

Closed schema, snake_case, `additionalProperties: false`. Actions:
`create` · `get` · `update_draft` · `upsert_skill` · `remove_skill` · `validate` · `publish`.
`profile_slug` is required by every action; `owner_handle` is accepted **only** by `create`.
Pass the ETag in the `etag` field **verbatim including its quotes** (escaped in JSON):

```json
{
  "action": "upsert_skill",
  "profile_slug": "<profile-slug>",
  "etag": "\"agent-profile-v23\"",
  "binding_id": "<binding-id>",
  "activation_mode": "ROUTED",
  "skill": {
    "skill_guid": "<stable-guid>",
    "literal_version": "<major.minor>",
    "expected_name": "<canonical-name>",
    "expected_publisher_id": "<publisher-id>"
  },
  "routing_policy": {
    "intent_id": "<stable-intent-id>",
    "routing_description": "<when to pick this skill>",
    "explicit_trigger_aliases": ["<alias>"],
    "task_tool_policy": { "mode": "EXPLICIT_ALLOWLIST", "tool_names": [], "tool_set_refs": [] },
    "side_effect_class": "READ_ONLY"
  }
}
```

Do not invent fields: there is no `operation`, `profile`, `owner_profile`, `if_match`,
`validation_id`, or `exact_ornn_skill_reference`. The exact reference lives in `skill`.
This tool manages only the caller's own profiles — not `system/*`, another owner, another scope,
or channel binding.

## Diagnosing a failure

| Observation | What it means | Do |
|---|---|---|
| Route family absent / tool absent | Capability not deployed in this session | Stop before mutation; report honestly |
| `404` after the family exists | Profile missing or invisible **to you** | Check slug + scope; do not call the contract undeployed |
| `400` `IDEMPOTENCY_KEY_REQUIRED` | Create without the header | Send one |
| `400` `INVALID_AGENT_PROFILE_HTTP_BODY` | Unknown/missing member in a closed body | Send exactly the documented fields |
| `428` | You never read the ETag | `get`, then retry |
| `412` | Stale ETag | Reread **and rebuild** the mutation |
| `422` | Publish-side validation failed | Read typed diagnostics: draft shape, exact Ornn identity, routing policy, tool subset, sealing |
| `503` | Ornn resolution, ingress proof, or actor dispatch unavailable | Report accepted/committed state honestly; retry only when safe |
| `202` | Accepted for dispatch | Reread before claiming anything |
| Published profile unused by a workflow/schedule/existing chat | Expected consumer boundary | Explain the boundary — not a failed publish |

## Honesty rules

- **Detect before you act.** Never attempt a profile mutation without confirming the surface.
- **Never substitute a different resource.** No workflow/member/service stand-in for a Profile.
- **Never fabricate** a `profileId`, ETag, skill GUID, publisher id, version, or digest.
- **Exact Ornn identity or nothing** — no `latest`, no name-only, no inline content.
- **`202` is not done.** Read the model back before reporting success.
- **Ceilings only.** Never describe a profile or policy as granting a capability.
- **Rollout admission is host-owned**, like NyxID external exposure — surface it as a dependency,
  never as something you can turn on.
