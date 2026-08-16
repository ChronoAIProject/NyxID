---
name: aevatar-automation
description: "Aevatar scheduling & workflow automation: scheduled_agent_creator (cron/one-shot) for independent scheduled Ornn skill agents and reminders — NOT an alias for Studio Team member automation — the long-running task automation playbook, workflow creation semantics (Scope Workflow vs Ornn publish), agent_builder lifecycle, the typed required_nyx_services contract (exact user_service_id is the authorization; a slug snapshot never substitutes), dedicated Agent Key semantics (vault-held raw material, borrowed durable reference late-resolved per use, exact non-wildcard grants, ~90-day projected expiry, reauthorize-then-revoke, independent NyxID and Vault revocation tracks), accepted-is-admission-only receipts, and a token_expired triage protocol that reads credentialSourceKind before assuming any TTL."
version: "1.3"
metadata:
  category: plain
  tag:
    - aevatar
    - aevatar-system
    - scheduling
    - workflow
    - automation
    - agent-builder
    - credentials
    - token-expired
---

#### scheduled_agent_creator (scheduled Ornn skill agents)

Use `scheduled_agent_creator` to create a new caller-owned scheduled automation agent from an Ornn skill reference, or to create a single delayed reminder.

**This tool is for an independent scheduled Ornn skill agent or a one-shot reminder. It is NOT an alias for Studio Team member automation.** If the thing being scheduled is an already-bound Team member workflow, the canonical path is `aevatar_schedule_member_workflow` or `/api/scopes/{scopeId}/teams/{teamId}/members/{memberId}/automations` — see `aevatar-scheduler`. Do not route a Team member here, and do not route an independent skill agent to generic `/api/schedules`.

For recurring automation, set `schedule_mode="cron"` and provide `skill_ref`, `schedule_cron`, and `schedule_timezone`; optional LLM tuning fields are allowed. If the loaded skill body will call connected NyxID services through `nyxid_proxy` beyond Ornn and the Lark outbound channel, declare them with the typed requirement contract:

```text
required_nyx_services[] = { user_service_id, service_slug_snapshot }
```

**The exact `user_service_id` is the authorization; the slug snapshot is an integrity/display value and can never substitute for it.** `nyx_user_service_id` identifies the exact outbound provider where applicable. The older `required_service_slugs` contract has been removed — do not recommend it, and never infer a UserService ID from a slug, a vendor name, or a catalog service ID (a connected UserService ID and a catalog service ID are different identifiers and are not interchangeable).

If the scheduled agent needs deterministic fire-date fields, its execution prompt may use the same
fire-time Chat template as other schedules: `{{@schedule.run_date}}`, `run_year`, `run_month`,
`days_until_month_end`, `fire_at_utc`, and `timezone`. The prompt must then be valid JSON and each
placeholder must be inside a JSON string value. The values come from the authoritative logical fire
and configured IANA timezone; catch-up keeps the original occurrence and run-now uses the manual
instant. Unknown placeholders or invalid JSON fail closed. A manual run is not cron-rearm proof.

For one-shot delayed reminders such as "remind me in 10 minutes" or "later today tell me ...", set `schedule_mode="one_shot"` and provide exactly one of `delay_seconds` or `run_at_utc`, plus `one_shot_message`. Prefer `delay_seconds` when the user gave a relative delay. Do not use `code_execute` with `sleep`, timers, polling loops, or long-running scripts for delayed one-shot requests; durable delivery must go through `scheduled_agent_creator`. Do not publish an Ornn skill just to send a one-shot natural-language reminder unless the user explicitly asks for reusable automation or the reminder requires a real skill workflow.

Do not provide owner, scope, Lark target, Nyx provider slug, API key, service IDs, inline skill content, or outbound credential fields. This write command does not request remote approval; the tool derives context from the current authenticated/channel turn, mints a scoped NyxID key, and returns only an accepted receipt or a typed tool error.

`skill_ref` must be unversioned for now. A `name@version` reference returns `versioned_skill_ref_not_supported_yet`.

### The scheduled agent's credential is a dedicated Agent Key

A `scheduled_agent_creator` agent does **not** run on a replayed user bearer, and it does **not**
run on a fire-time broker token. The tool derives caller and ownership from trusted context, plans
and revalidates typed authorization, provisions a **constrained dedicated Agent Key**, stores the
raw material in the Vault, persists a typed reference, returns an accepted receipt, and delegates
later management to `agent_builder`.

**Secret custody.** Raw key material is written **only** to `ISecretVault`. Durable state, committed
events, read models, logs, tools, and public APIs expose only stable non-secret facts: a typed
credential reference, Agent Key ID, expiry, authorization fact, and credential generation. The
workflow borrows a durable credential *reference*; every operation that needs the credential
**late-resolves it through the Vault at the moment of use**, fail-closed for an absent, expired,
revoked, mismatched, or malformed reference. Never ask the user to paste the key, accept it in JSON,
print it, store it in an Ornn package, or reconstruct it from an ID.

**Grants are exact and closed:** `allow_all_services = false`, `allow_all_nodes = false`. Only the
targeted plan supplies allowed service IDs and node IDs.

**Expiry.** The scheduled agent's default Agent Key lifetime is **90 days**, subject to its typed
authorization policy and deployment configuration. Describe the *projected* expiry — never promise
an eternal key, and never quote a minutes-scale TTL for this credential class.

**Lifecycle.** Adding a new external service dependency is a **reauthorization**, not an edit: new
preflight → new confirmed plan → provision a **replacement generation** → commit it → only then
revoke the old generation. Never mutate an existing key in place to widen its authority. Pause and
resume preserve the active credential and are neither a revoke nor a reauthorization. Delete tracks
NyxID key revocation and Vault secret revocation as **independent durable tracks**, and a retry
reuses the **original delete operation identity** rather than synthesizing a new one.

### `202` / `accepted` is admission only

An accepted receipt or `pending` status proves command/effect admission — nothing more. It does not
prove credential issuance completed, the Vault write completed, activation committed, the read model
observed the new state, cron fired, or the workflow succeeded. Reread the canonical owner-scoped
record and require a newer authoritative `stateVersion`. Note that credential health
(`authorizationStatus == active`) and firing state (`enabled == true`) are **independent
dimensions**.

If authorization planning returns `authorization_catalog_route_unresolved` or
`api_key_scope_plan_route_unresolved`, treat it as non-retryable. Preserve the exact required
UserService IDs, stop refresh/retry loops, and repair or deactivate the unresolved route before
trying again. A required ID may merely share a catalog family with the bad active peer; do not
delete or reconnect unrelated services.

### Token classes: verified lifetimes, not folklore

**Never quote one class's lifetime for another.** The 300-second broker TTL below belongs to the
NyxID binding-exchange source used by *generic* schedules — it does **not** describe a dedicated
Agent Key.

| Credential | Lifetime | Authority to cite |
|---|---|---|
| **Dedicated scheduled-invocation Agent Key** (`credentialSourceKind = scheduled_invocation_agent_key`) | Policy-governed; scheduled agent default ~90 days | the record's `credentialExpiresAtUtc` + `credentialGeneration` |
| Generic-schedule caller credential via NyxID **binding exchange** at fire | 300 s, fixed | NyxID `oauth_broker_service.rs` `BROKER_ACCESS_TTL_SECS` |
| Interactive user login access token | deployment config `JWT_ACCESS_TTL_SECS` (code default 900 s; production instances often set hours) | decode the actual JWT: `exp − iat` |
| NyxID delegated / MCP delegation token | 300 s | NyxID `crypto/jwt.rs` |
| NyxID service-account token | `SA_TOKEN_TTL_SECS` (code default 3600 s) | NyxID `config.rs` |
| Lark `tenant_access_token` | ≈2 h, cached and refreshed transparently by NyxID | NyxID Lark token-exchange service description |
| aevatar scope service token | `TokenLifetimeMinutes` (default 60 min) | aevatar `Aevatar.Authentication.ScopeServiceTokens/ScopeServiceTokenOptions.cs` |

**Anti-hallucination protocol for `token_expired` / 401 in a scheduled run** — complete these steps
before naming any cause:

1. Never assert a token lifetime from memory. Read it: decode the JWT actually in hand
   (`exp − iat`; report only the numbers, never print the token), or cite the owning repo's
   constant from the table above.
2. **Read `credentialSourceKind` before anything else.** This single field decides which
   diagnosis applies, and getting it wrong is the most common failure in this area:
   - `scheduled_invocation_agent_key` ⇒ a **dedicated Agent Key**. Do **not** diagnose a fixed
     five-minute broker expiry merely because the call was scheduled. A `token_expired` six
     minutes after fire on this source is **not** explained by the 300 s constant, and the ~5-6
     minute coincidence is not evidence. Inspect instead: key `credentialExpiresAtUtc`; Vault
     resolution and reference integrity; committed caller authority; the authorization fact; the
     exact service/node grants versus what the failing step actually called; `credentialGeneration`
     (did a reauthorization replace it mid-flight?); `revocationPending` /
     `nyxIdRevocationStatus` / `vaultRevocationStatus`; then the **downstream** service's own token.
   - `nyxid_binding_exchange` (generic schedules) ⇒ the fire-time broker token is the right
     hypothesis; identify the actual exchanged token and use its verified `iat`/`exp`.
3. Identify the token class first. "The access token lives 8 hours" (login class) and "the run
   credential lives 5 minutes" (binding-exchange class) can both be true — about different tokens.
   A claim that conflates them is wrong even if each number is right.
4. Correlate failure onset with FIRE TIME, not wall clock — **but only once the source is known.**
   For a binding-exchange source, failures beginning ~5 minutes after fire in every run ⇒ the
   fire-time-minted broker credential aged out. Failures at one fixed wall-clock moment across runs
   ⇒ something else (provider-side expiry or rotation).
5. Separate blast radius. If ALL NyxID-authenticated steps fail after some instant, the run
   caller credential died. If only one step type fails while sibling proxied steps still succeed,
   suspect that step's own credential path instead.
6. Report observed timestamps/status as fact and everything else as labeled inference. Any number
   you did not verify in this session is an inference.
7. Never emit secret material in any diagnosis, log excerpt, or artifact: no raw Agent Key,
   bearer/access/refresh/delegation/service-account token, Vault reference or ciphertext,
   permission digest, unfiltered API-key inventory, or authorization header. Stable resource IDs
   may be reported for management or cleanup, never as secret material.

### Long-running task automation playbook

Use this playbook when the user asks for a recurring, scheduled, monitored, or otherwise long-running task instead of a one-off answer. Typical triggers include: "每天...", "每周...", "each week...", "monitor X and tell me...", "定时...", "recurring", "keep watching", and "长期跟踪".

#### Workflow creation semantics

When a Lark user asks to create a workflow that should be runnable, page-visible, or invokable later by workflow id, create or update a Scope Workflow through the available Scope Workflow command tool path. Ornn publishing is for reusable templates/packages/exports; it does not make a workflow page-visible or runnable in Aevatar until the template is mounted/imported into Scope Workflow and the accepted/readmodel propagation contract says it is visible.

1. Recognize the request as automation.
   - Do not answer with a one-shot summary if the user wants repeat runs.
   - Do not ask the user to hand-write the skill package.
   - Treat the future runner as a runnable Ornn skill, not a chat-only script.

2. Reuse before you author — search Ornn first.
   - Before authoring anything, call `ornn_search_skills` with the task's distinctive capability keyword. Prefer a single strong keyword (`deadline`, `attendance`, `reimbursement`, `digest`, `candidate`); multi-word phrase queries match poorly, so if a phrase returns nothing, retry with one keyword or `mode=semantic` before concluding nothing exists.
   - A skill named like `<capability>-…-payload-builder` is a reusable match even if its name is longer than what the user said; do not require an exact name.
   - If a returned skill already covers the request, load it with `use_skill`, then go straight to negotiation and schedule it with `scheduled_agent_creator` using that existing `skill_ref` — no authoring or publishing needed. Do NOT author a duplicate of a skill that already exists.
   - Only author a new skill when the search returns no suitable match.

3. Author a runnable skill package yourself.
   - Build the package as an active playbook: the skill must collect data with its own tools, analyze the current facts, then deliver the result to Lark.
   - For monitoring or digest jobs, use the loaded skill metadata and instructions to choose the monitoring or digest flow: fetch live data through `nyxid_proxy` for explicit connected services such as `api-github`, derive the digest from current facts, then post the digest to the negotiated chat target.
   - Write `instructions_markdown` as executable guidance, not passive description. Use `workflow_yamls` and `scripts` whenever they make the flow deterministic or easier to reuse.
   - Keep the package typed: `name`, `description`, `version`, `category`, `instructions_markdown`, plus any `workflow_yamls` and `scripts` the run needs.

4. Negotiate schedule and output with an interactive Lark card.
   - Use `reply_with_interaction` to ask for the minimum missing details.
   - Ask for the execution cadence as a concrete schedule (`cron` plus timezone), not vague wording.
   - Ask where the result should go: direct message or group chat.
   - Ask for the output format: plain text or Feishu cloud doc.
   - Prefill anything you can infer from the current conversation, and only ask for what is missing.
   - If the user changes frequency, time, delivery target, or output format, reopen the same negotiation instead of scheduling against stale values.

5. Publish the skill, then schedule it.
   - Call `ornn_publish_skill` with the assembled typed package.
   - If publish fails, inspect the diagnostics, fix the package, and retry.
   - Ornn private skill publishing executes directly. Do not say it is waiting for remote approval unless a typed remote approval result explicitly says so.
   - Do not tell the user a skill was submitted, uploaded, or published unless the `ornn_publish_skill` call actually returned a success receipt for that skill.
   - Once the skill is published successfully, call `scheduled_agent_creator` with the published `skill_ref`, the agreed `schedule_cron`, the agreed `schedule_timezone`, and `required_nyx_services` entries carrying the exact `user_service_id` (plus its `service_slug_snapshot`) for every connected service the skill body will call through `nyxid_proxy`.
   - Carry the negotiated delivery/output choice into the runner's `execution_prompt` and outbound delivery setup; if the chosen delivery target differs from the current conversation, rebind it with `agent_delivery_targets` using the returned `agent_id`.
   - For plain text output, the skill should send a concise digest back to Lark. For Feishu cloud doc output, the skill should create or update a document and return the link.

6. Recover cleanly.
   - Publish failure means the package is wrong; refine and republish.
   - User rejection or edits mean the negotiation is not stable yet; update the card and retry.
   - If the user later wants a different cadence, treat it as a new negotiation for a new schedule rather than pretending the existing schedule changed automatically.

#### agent_builder (Day One persistent automation lifecycle)

`agent_builder` manages the lifecycle of agents the user has already created. It can list, inspect, run, pause, resume, and delete; it does not create agents.

| Intent | Slash command |
|---|---|
| List agents | `/agents` |
| Inspect one agent | `/agent-status <agent_id>` |
| Manual run | `/run-agent <agent_id>` |
| Pause schedule | `/disable-agent <agent_id>` |
| Resume schedule | `/enable-agent <agent_id>` |
| Delete (two-step) | `/delete-agent <agent_id> confirm` |

Tool semantics: `disable_agent` pauses scheduled execution without deleting; `enable_agent` resumes; `delete_agent` disables, revokes the NyxID API key, and tombstones the registry entry. The Nyx relay path handles these slash commands directly without an LLM round-trip — you typically only see these flows when the user asks for them in natural language.
