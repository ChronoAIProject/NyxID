# Chrono Ephemeral Agent POC — Extension Plan (SSOT)

Status: **PLAN + AS-SHIPPED DELTA — the extension was implemented on this branch in a reduced, read-only scope; §0 records exactly what shipped and corrects the stale sections below**
Date: 2026-08-14 (plan authored and implemented the same day; branch rebased to `origin/main` `9f3d38a6` before implementation)
Branch: `feat/assistant-ephemeral-agent-poc` (plan originally written against `0016d657`; `95af8911 feat(assistant): add ephemeral direct-agent POC` is in history via merge `62add35b`).
Supersedes nothing; extends `docs/chat/chrono-llm-ephemeral-agent-plan.md` (the shipped v0 SSOT). Where the two disagree about *future* work, this document wins; the v0 document remains the authority on what v0 shipped. The adversarial reviews that reduced this plan's scope are preserved verbatim in `chrono-ephemeral-agent-poc-extension-plan.review-sol.md` (plan review, verdict BLOCKED on the mutation/account/Aevatar-fallback extensions) and `chrono-ephemeral-agent-poc-implementation.review-opus.md` (live implementation review).

## 0. As-shipped POC delta (2026-08-14)

The implementation merged from this branch differs from the plan below. Where this section and a later section disagree, this section and the code win.

**Shipped:**
- **Skills pair** `nyx_search_skills` / `nyx_get_skill` replacing `ornn_*`: bundled skills matched by slug/label only; Ornn results admitted only with strictly canonical UUID ids; `nyx_get_skill{source:"ornn"}` requires an id observed by a search in the same run, one Ornn fetch per run. Skill delivery is a structured document with `source`, `id`, `version`, `content_sha256` (full body), `delivered_sha256`, `content_bytes_total/delivered`, `content_truncated`, and an 8 KiB delivered-content cap (`MAX_SKILL_CONTENT_BYTES`); the same provenance (digests, never content) is written to the per-tool audit event. The `nyx_search_skills` result reports per-source status including `not_queried` (bundled rows filled the limit) vs `not_connected`.
- **`tool.completed` gained `result_preview`**: a bounded (≤2 KiB, `MAX_TOOL_RESULT_PREVIEW_BYTES`) preview derived from the already-scrubbed model body — never from arguments. The browser may now receive this scrubbed preview; it never receives raw unsanitized results or raw tool arguments. Frontend parsing is backward-compatible: the strict schema marks `result_preview` optional and a missing preview normalizes to `null` (regression-tested), so older streams parse. `RunCard` renders it as an escaped collapsible `<pre>`; the run-step type carries `result_preview?: string | null`.
- **Scrubbing extended to values**: in addition to the credential-key denylist, secret-shaped string *values* (`Bearer`/`Basic` prefixes, PEM blocks, `ghp_`/`github_pat_`, `sk-`-family, `nyx_*` tokens, three-segment JWTs) are replaced with `[REDACTED]` before model context and preview. The precise guarantee: NyxID-injected credentials are structurally absent; downstream-returned secret-shaped strings are best-effort redacted.
- **Structural phase walls**: Plan and Report request bodies declare no `tools`/`tool_choice`/`parallel_tool_calls` at all; a Plan hop that still emits a tool call (or any non-`stop` Plan finish) fails the run before any dispatch; natural and forced Report hops reject undeclared tool calls identically. The Report hop is always a separate disabled-tools hop; empty assistant content is serialized as JSON `null` via one shared helper.
- **Context compaction with Report reserve**: before every Execute/Report hop the serialized body is compacted to `MAX_UPSTREAM_BODY_BYTES − 64 KiB` headroom by rewriting the oldest complete tool exchange (ids preserved, arguments emptied, bodies compacted); `context_overflow` only when nothing compactable remains. Tool-call arguments over 32 KiB (`MAX_TOOL_ARGUMENT_BYTES`) become synthetic `invalid_args` without dispatch.
- **Internal platform services are excluded** from the registry (`service_category == "internal"` filtered at listing, advertising, resolution, and the single execute-time predicate), alongside the existing generic-proxy exclusion.
- **Prompt protocol** restated as Understand preflight → Plan → Execute → Report (stage id `final` renders as "Report" in the UI); scripted-hop tests drive the real `execute()` loop end-to-end, and the billing route smoke exercises the real HTTP route with the always-separate Report hop.

**Deferred / not shipped** (per the preserved plan review; the sections below describing them are historical design, not shipped behavior):
- `nyx_whoami` and `nyx_account_call` (§6, §9) — not built.
- The mutation allowlist (§8) — not built; the registry remains read-only and `ApprovalVerb::Read`-gated with no write path.
- Aevatar fallback descriptors and any Aevatar-specific code path (§11/E5) — not built. Aevatar participates only if the canonical platform catalog publishes typed operations for it (test-pinned); it is never runtime, orchestrator, or fallback.
- Budget raises (§7) — not applied; `MAX_LLM_CALLS = 8`, `MAX_TOOL_CALLS = 8`, `WALL_DEADLINE = 180 s` stand.

**Removal checklist correction (§14 deleteability):** deleting the POC now also requires removing the `result_preview` field from the run-step type in `frontend/src/types/assistant.ts` and its render block in `frontend/src/components/assistant/blocks/run-card.tsx` (both shared with the Aevatar engine's RunCard; the field is optional, so removal is a two-line revert).

**Known POC residuals (accepted):** downstream response bytes are still fully buffered inside `mcp_service::execute_tool` before the post-hoc 16 KiB model cap (v0 R9); value-side redaction is pattern-based best effort; `result_bytes` reports the pre-truncation size when `truncated` is true.

**Verdict: PROCEED.** Every primitive the extension needs already exists in main and was re-verified at file/line level. The extension is additive inside the existing `assistant_direct_agent_poc` deletion boundary: two new in-process tools, a merged skills tool pair, a one-entry mutation allowlist with defense-in-depth re-checks, and Aevatar strictly as a downstream typed service. No new routes, collections, migrations, flags, workflows, or persistence. Two items need a human decision before the demo (see Blockers, §14).

---

## 1. Binding POC contract

The user decision this plan implements, verbatim in effect:

1. **No Aevatar runtime.** Aevatar appears only as an ordinary downstream NyxID-managed service, callable through the same typed-operation registry as any other connected service. NyxID's own ephemeral, in-request agent runtime (Chrono LLM native tool calling) is the orchestrator.
2. **Ephemeral only.** One spawned task per request; run state lives in `RunContext` (`backend/src/services/assistant_direct_agent_poc.rs:142-161`); reload/completion destroys it. No workflows, actors, durable runs, background jobs, scheduling, recovery.
3. **Disposable runtime, reusable artifacts.** The runtime and UI may be deleted wholesale; system prompts, skills, tool schemas/adapters, result envelopes, and evaluation scenarios are written transport-neutrally for reuse by a later managed Codex agent (§5).
4. **Chrono never sees credentials.** NyxID executes every tool in-process. Chrono hops go through `execute_admin_proxy` to `chrono-llm-public`, whose live policy is `identity_propagation_mode:"none"`, no access-token forwarding, no delegation injection (v0 SSOT §13, live-verified 2026-08-13). Tool credentials are resolved inside `mcp_service::execute_tool` *after* model context is built; results are scrubbed before re-entering model context (`tools.rs:816-924`).
5. **Minimum walls preserved:** feature flag `experimental:direct-chat-engine` (`backend/src/services/feature_flag_service.rs:111`), browser-session-only auth (`handlers/assistant_direct_agent_poc.rs:34-38`), bounded run (budgets §7), allowlisted mutations only (§8), no arbitrary URLs (parameters only via `build_proxy_args`), no admin/secret delivery (projection rules §9), redacted results (`scrub_credentials`, `tools.rs:881-924`), metadata-only audit (`assistant_direct_agent_poc.rs:1190-1209`).

### Non-goals (unchanged from v0 unless noted)

- No approval cards / interactive waits; deny rules still enforced.
- No generic proxy, no Destructive (DELETE) operations, no streaming/binary tool responses.
- No runtime provisioning; no NyxID **account-state** mutations (the one demo mutation targets a *downstream* service).
- No new AppError variants/codes, no new collections, no admin config surface, no CLI changes.
- No bounded-drain refactor of `execute_tool` and no full tool-cancellation propagation (v0 R9/R10 stand).
- No production hardening beyond the stated floor; internal trial only.

## 2. Current baseline (evidence, re-verified on this branch)

The merged POC (`95af8911`) provides, at exact locations:

| Primitive | Where (file:line) |
| --- | --- |
| Flag + session-only + billing-classified + rate-limited SSE handler | `backend/src/handlers/assistant_direct_agent_poc.rs:28-102` (flag :33, `AuthMethod::Session` :34-38, billing egress permit :40-43, permit :49-51, body caps :54-70, SSE + heartbeat :92-135) |
| Bounded run loop: understand → plan → execute → final, forced-final, cancellation, wall deadline | `backend/src/services/assistant_direct_agent_poc.rs:199-401` (budgets :31-38, planning transition :1071-1078, force-final :433-436) |
| Chrono hops via `execute_admin_proxy` to `chrono-llm-public`, strict hop-drain decoder | `assistant_direct_agent_poc.rs:446-583` (dispatch :488-496); `sse_decode.rs:7-10` caps; chrono row resolved by `assistant_service::resolve_admin_service_by_slug` (`services/assistant_service.rs:38-45`) |
| Five tools: `nyx_list_services`, `nyx_search_tools`, `nyx_call_tool`, `ornn_search_skills`, `ornn_get_skill` | definitions `tools.rs:44-104`; dispatch `assistant_direct_agent_poc.rs:701-789` |
| Read-only eligibility predicate (advertise-time + execute-time, same fn) | `tools.rs:363-383` (`is_poc_operation_eligible`: not generic proxy, `derive_verb_from_method == Read`, `binary_artifact == Some(false)`, textual content types, supported schema) |
| Registry over canonical per-user catalog + connected-services view | `ReadOnlyRegistry` `tools.rs:112-238`; loads at `assistant_direct_agent_poc.rs:278-315` (`mcp_service::load_operation_catalog` `services/mcp_service.rs:471-557`, `load_user_tools_all_scoped` `:428-435`, both `Unrestricted` scopes — correct for session callers). Catalog publish rule: a service with zero publishable endpoint rows is **dropped entirely**, not degraded (`operation_set_is_publishable`, `mcp_service.rs:559-572`); typed-vs-generic precedence for catalog-backed user services is instance `openapi_spec_url` > template `ServiceEndpoint` rows > generic proxy (`:974-1034`) |
| Deny-rule enforcement with catalog-identity targeting | `enforce_deny_only` `assistant_direct_agent_poc.rs:1032-1056`; `mcp_approval::approval_target_for_tool` `services/mcp_approval.rs:20` (struct :7); `approval_service::evaluate_deny_only` `services/approval_service.rs:184-200` |
| In-process execution with credential injection, identity propagation, billing settlement | `execute_endpoint` `assistant_direct_agent_poc.rs:791-854` → `mcp_service::execute_tool` (call :823-841) with `McpExecContext{api_key_id: None, allow_all_nodes: true}` :814 |
| Result envelope: scrub + 16 KiB model-context cap + truncation marker | `ModelToolResult` `tools.rs:794-879`; `scrub_credentials`/`is_credential_key` :881-924 |
| Restricted Ornn reads: fixed GET descriptors, exact-GUID allowlist, one fetch/run, provenance fence, projection | `tools.rs:385-468` (descriptors), `:926-1035` (fence, version token, search projection); one-fetch flag `assistant_direct_agent_poc.rs:156, 771-786` |
| Metadata-only audit: run start/finish + per-tool | `assistant_direct_agent_poc.rs:265-273, 1003-1019, 1190-1209` |
| Bundled skills registry (compile-time) | `services/assistant_direct.rs:60-85` (`DIRECT_SKILLS`: `nyxid`, `github-via-nyxid`, `firecrawl-via-nyxid`; ≤64 KiB const asserts :78-85); prompt injection `assistant_direct_agent_poc/prompt.rs:35-47` |
| Frontend: four-stage RunCard UI, strict zod frame schemas, toggle, cancel, 11 tests + 6 fixtures | `frontend/src/lib/assistant/direct-agent-poc.ts` (schemas :20-107, stream :236-332), branch `direct-transport.ts:480-495`, toggle `direct-chat-controls.tsx:167-183`, flag via `useFeature` (`feature-flags.ts:22`, `use-feature-flag.ts:18-22`, `pages/assistant.tsx:174-181`) |

**Newer main-side subsystem that affects this design (PR #1443, after #1444):** every catalog master-credential decrypt — including the MCP executor path this POC uses — now routes through the `authorize_master_credential*` gate in `proxy_service` (design `docs/assistant/TRAVEL_BOOKING.md` §9/§A.1; verification `docs/assistant/PR_A_VERIFICATION.md`). Consequences: (a) user-credentialed services are unaffected; (b) public catalog rows resolve as before; (c) a **private** platform-credentialed catalog row is callable only with OAuth-app consent — a private row with no `developer_app_ids` is unreachable by anyone; (d) rows with `auth_method:"none"` (the expected live `aevatar` shape) never reach the credential gate. Upstream error bodies are no longer logged. The extension inherits all of this for free; the live-validation runbook (§12) includes the PR-A readback for the demo services.

**Frontend contract constraint (verified):** all SSE frame schemas are `.strict()` (`direct-agent-poc.ts:20-107`). `tool.started.tool` and `target.{service_slug,endpoint}` are free strings, and `run.started.limits` values are plain ints — so **new tool names and changed budget values need zero frontend changes**, but **no field may be added to any existing frame** without a lockstep schema update. This plan adds no frame fields.

## 3. Target contract → design mapping

| Target item | Design |
| --- | --- |
| 1 `nyx_whoami` | New in-process tool, POC-owned account module (§9) |
| 2 `nyx_list_services` w/ active/readiness/ownership/tool counts | Enrich existing projection with ownership + credential-source data from `user_service_service::list_user_services_with_sources` (§6) |
| 3 `nyx_search_tools` | Unchanged (adds `mutation:true` marker on allowlisted write ops) |
| 4 `nyx_call_tool` + mutation allowlist | Existing tool; gate widened by explicit const allowlist with re-check (§8) |
| 5 `nyx_account_call` | New in-process tool, three pure read operations (§9) |
| 6 `nyx_search_skills` / `nyx_get_skill` | Merge bundled + Ornn under two tools; `ornn_*` tool names deleted (§10) |
| 7 Aevatar as downstream only | Via the same registry/`execute_tool` path; catalog-first with a bounded descriptor fallback (§11) |
| 8 Prompt: Understand→Plan→Execute→Report | Update `backend/prompts/direct/agent-poc.md` + phase instructions; stage ids stay `understand/plan/execute/final` (FE enum is fixed, `direct-agent-poc.ts:20`); "Report" is the final-phase language |
| 9 Four-stage UI | Already shipped; new tools render as RunCard steps automatically (`direct-agent-poc.ts:595-645`) |
| 10 Test scenario | §12 demo script |
| 11 Ephemeral | Unchanged (§1.2) |

Final tool inventory: **seven logical tools** — `nyx_whoami`, `nyx_list_services`, `nyx_search_tools`, `nyx_call_tool`, `nyx_account_call`, `nyx_search_skills`, `nyx_get_skill`. The `ornn_search_skills`/`ornn_get_skill` names are deleted (prefer deletion, FI-007; the POC is disposable and has no compatibility consumers).

## 4. Architecture

```
Browser (session cookie, flag on)
  └─ POST /api/v1/assistant/direct/agent        [unchanged route; no new routes]
       handlers/assistant_direct_agent_poc.rs    [unchanged except budget constants surface]
         services/assistant_direct_agent_poc.rs  [run loop; dispatch gains 3 arms, loses 2]
           ├─ LLM hops ──────────────────────── execute_admin_proxy → chrono-llm-public
           │                                     (identity none; no tokens ever forwarded)
           ├─ in-process account tools ──────── assistant_direct_agent_poc/account.rs [NEW]
           │     nyx_whoami · nyx_account_call     └─ existing service layer (org_service,
           │                                          key_service, node_service, approval_service)
           ├─ typed downstream tools ─────────── tools.rs registry → enforce_deny_only
           │     nyx_call_tool (reads + 1-entry     → mcp_service::execute_tool
           │     mutation allowlist)                  [credentials injected here; identity
           │       ├─ user services (GitHub, …)        propagation + delegation-token injection
           │       ├─ aevatar (downstream ONLY)        per the service row's own config —
           │       └─ chrono-sandbox etc.              e.g. aevatar inject_delegation_token]
           └─ skills ─────────────────────────── bundled DIRECT_SKILLS (in-process)
                 nyx_search_skills · nyx_get_skill    + Ornn fixed GET descriptors (bounded)
```

**Auth/delegation, precisely:**
- The handler admits only `AuthMethod::Session` (`handlers/assistant_direct_agent_poc.rs:34-38`); the assistant mount's reject layers additionally block API-key/SA/delegated/relay callers (`routes.rs:1448-1449, 1501-1503`).
- In-process account tools use `AuthUser.user_id` (as `&str`) against the service layer directly — no HTTP, no tokens minted. This is safe *because* of the session gate: `AuthUser.allowed_service_ids`/`allow_all_*` are middleware/handler concerns not enforced by service fns, and session users hold full account authority anyway. If this surface ever admits API keys, those allowlists must be enforced explicitly first (recorded as a hard precondition, not done here).
- UserService/catalog tools go through `mcp_service::execute_tool` (`services/mcp_service.rs:3044-3590`), which owns credential resolution and per-row identity semantics. **Verified nuance:** `execute_tool` propagates *identity* only — `X-NyxID-Identity-Token` / identity headers / RBAC headers when the row's `identity_propagation_mode != "none"` (`:3261-3345`) — and **never mints `X-NyxID-Delegation-Token` and never forwards the caller's bearer** (zero references in `mcp_service.rs`; delegation minting lives exclusively on the REST-proxy/assistant paths, `handlers/proxy.rs:2105-2129` gated on `inject_delegation_token`, TTL `MCP_DELEGATION_TOKEN_TTL_SECS` = 300 s at `crypto/jwt.rs:763`). So a downstream that authenticates NyxID calls must accept the identity JWT/headers on this path; no token of any kind ever goes to Chrono, the model, the browser, or audit. `execute_tool` also imposes **no verb allowlist** of its own — the HTTP method comes from `endpoint.method` via `build_proxy_args`/`parse_proxy_method` (`:2718-2723`, `:3648-3661`) — which is exactly why the POC's own advertise+execute predicate (§8) is the only method wall and must stay the single shared function.
- Session callers bypass interactive approval waits platform-wide; deny rules still apply via `enforce_deny_only`. Mutations get no special approval flow in this POC — the wall is the allowlist plus deny rules (§8).

## 5. Transport-neutral reusable artifacts (POC-disposable vs keep)

**Reusable later by a managed Codex agent (design for reuse now):**
1. **System prompt + phase instructions** — `backend/prompts/direct/agent-poc.md` + the Plan/Execute/Final(Report) instruction strings (`prompt.rs:12-33`). Pure text; no transport assumptions.
2. **Bundled skills** — `backend/prompts/direct/*.md` with the `DIRECT_SKILLS` const table (slug/label/body). Provenance = `bundled@` + `env!("CARGO_PKG_VERSION")` (§10).
3. **Tool JSON Schemas** — `agent_tool_definitions()` output (`tools.rs:44-104` extended per §6-§10): plain OpenAI-function JSON, engine-agnostic.
4. **Result envelope** — `ModelToolResult` shape `{status, body, truncated, bytes}` + scrub rules + 16 KiB cap + synthetic `{"executed":false,"error":…}` failure contract (`tools.rs:794-924`). Documented here as the stable envelope.
5. **Eligibility + allowlist predicates** — `is_poc_operation_eligible` (`tools.rs:363-383`) and the new mutation-allowlist predicate (§8): pure functions over `McpToolEndpoint`.
6. **Ornn adapters** — fixed descriptors, search projection, SKILL.md extraction + provenance fence (`tools.rs:385-468, 926-1035`).
7. **Account projections** — the new `account.rs` response structs (§9): the same shapes serve any future runtime.
8. **Evaluation scenario** — §12's demo script and its acceptance assertions, written against tool names + envelopes only.

**POC-only (delete with the POC):** the run loop/state machine, SSE dialect + heartbeats, `sse_decode.rs`, the FE `direct-agent-poc.ts` transport/UI, the toggle, fixtures. Deletion boundary is unchanged: `backend/src/{handlers,services}/assistant_direct_agent_poc*`, `backend/prompts/direct/agent-poc.md`, `frontend/src/lib/assistant/direct-agent-poc*.ts`, one route tuple, one FE branch + control.

## 6. Tool contract — exact schemas

All tools are OpenAI function declarations; every result is a `ModelToolResult` (scrubbed, ≤16 KiB, `{status, body, truncated, bytes}` serialized as the `role:"tool"` content). Argument validation stays two-layer: closed key sets in `validate_tool_arguments` (`tools.rs:470-517`) plus per-operation schema validation (`tools.rs:522-529`).

```jsonc
// 1. nyx_whoami — in-process; no arguments
{"type":"object","properties":{},"additionalProperties":false}
// result body:
{"user_id":"…","display_name":"…","email":"…","auth":"session",
 "orgs":[{"org_user_id":"…","org_name":"…","role":"admin|member|viewer"}]}

// 2. nyx_list_services — unchanged args {query?}; row gains ownership/readiness:
{"service_id":"…","name":"…","slug":"…","description":null,"category":"…",
 "source":"platform|user_service","executable":true,"tool_count":3,
 "owner":"personal" /* or "org:<org_name>" */,"org_role":null /* or role */,
 "is_active":true /* active-only view; stated, not queried */}

// 3. nyx_search_tools — unchanged args {query}; match rows gain:
{"name":"aevatar__get_api_agents","description":"…","input_schema":{…},"mutation":false}

// 4. nyx_call_tool — unchanged: {"tool_name":"<slug>__<endpoint>","arguments":{…}}

// 5. nyx_account_call
{"type":"object","properties":{
   "operation":{"type":"string","enum":["list_api_keys","list_nodes","list_approvals"]},
   "limit":{"type":"integer","minimum":1,"maximum":20}},
 "required":["operation"],"additionalProperties":false}

// 6. nyx_search_skills
{"type":"object","properties":{
   "query":{"type":"string","minLength":1,"maxLength":200},
   "limit":{"type":"integer","minimum":1,"maximum":10}},
 "required":["query"],"additionalProperties":false}
// result: {"matches":[{"source":"bundled","id":"nyxid","name":"NyxID","version":"bundled@<pkg>"},
//                     {"source":"ornn","id":"<guid>","name":"…","description":"…", …v0 projection}],"count":n}

// 7. nyx_get_skill
{"type":"object","properties":{
   "source":{"type":"string","enum":["bundled","ornn"]},
   "id":{"type":"string","minLength":1,"maxLength":128}},
 "required":["source","id"],"additionalProperties":false}
// result: provenance-fenced untrusted text (§10), ≤16 KiB
```

`nyx_list_services` ownership comes from joining the existing connected-services view with `user_service_service::list_user_services_with_sources` (`services/user_service_service.rs:259` — the **active-only** variant; per CLAUDE.md rule 8 the `_including_disabled` variant at `:277` backs `/keys` only and must not be used here). `CredentialSource` (`:229`) supplies `Personal | Org{org_name, role, allowed}`. Deliberately omitted: `connection_status`/`credential_missing` — they require `unified_key_service::list_keys` (`services/unified_key_service.rs:1993`), which needs `EncryptionKeys` and whose sibling `GET /keys` path triggers the `auto_provision_no_auth_services` write (`:1474`); a read tool must not take that on. `executable` remains the readiness signal.

`tool_identity` (`assistant_direct_agent_poc.rs:1147-1170`) and `safe_tool_name` (`:1179-1188`) gain the new names; frames show `nyxid · whoami`, `nyxid · account:<operation>`, `skills · search`, `skills · <source>:<id>` — never raw model-controlled strings.

## 7. Budgets

Demo needs ~7-8 tool calls (whoami, list, search, skill search, skill get, 1-2 Aevatar reads, 1 mutation) ⇒ ~10 hops with `parallel_tool_calls:false`. New values (constants only; same enforcement code, `assistant_direct_agent_poc.rs:31-38`):

`MAX_LLM_CALLS 8→12` · `MAX_TOOL_CALLS 8→10` · `WALL_DEADLINE 180s→300s` · unchanged: `FIRST_BYTE_TIMEOUT 30s`, `HOP_IDLE_TIMEOUT 60s`, `TOOL_TIMEOUT 60s`, `MAX_TOOL_RESULT_BYTES 16 KiB`, `MAX_UPSTREAM_BODY_BYTES 448 KiB`, `MAX_AGENT_CONTENT_BYTES 128 KiB`, heartbeat 10 s. New: `MAX_MUTATION_CALLS_PER_RUN = 1`, `MAX_ORNN_SKILL_FETCHES_PER_RUN = 1` (renames the existing `ornn_skill_fetched` flag), `MAX_BUNDLED_SKILL_FETCHES_PER_RUN = 2`. Preflight arithmetic re-check: 12 hops × (16 KiB + ~1 KiB) ≈ 204 KiB tool exchange + 67 KiB prompt/skill + 128 KiB transcript + overhead ≈ 430 KiB — still under 448 KiB, but tight; the preflight cap already fails closed (`:459-462`), and the forced-final path absorbs overflow honestly. `run.started.limits` carries the new values (ints; no FE schema change).

## 8. Mutation allowlist (smallest safe set) + defense in depth

**Mechanism** (all in `tools.rs`, POC-owned):

```rust
/// (service_slug, endpoint_name). Compile-time; reviewed in PR diff; empty ⇒ read-only POC.
pub const POC_MUTATION_ALLOWLIST: &[(&str, &str)] = &[
    ("aevatar", "<PINNED-AT-LIVE-VALIDATION>"),   // exactly one entry for v1 (Blocker B1)
];
```

An operation is **mutation-eligible** iff ALL hold: (1) `(service_slug, endpoint_name)` is in the table; (2) `derive_verb_from_method(method) == ApprovalVerb::Write` — POST/PUT/PATCH only; DELETE stays Destructive and structurally excluded; (3) every *other* clause of `is_poc_operation_eligible` holds (not generic proxy, `binary_artifact == Some(false)`, textual content types, supported schema). Read eligibility is untouched. The combined predicate `is_poc_operation_admitted(service_slug, endpoint) = read_eligible || mutation_eligible` is **one function used at advertise time and re-checked immediately before `execute_tool`** (same single-predicate discipline as today, `assistant_direct_agent_poc.rs:799`).

**Defense in depth per mutation call:** allowlist membership → verb re-derivation from the endpoint's own `method` at execute time (not from the table) → `validate_operation_arguments` against the typed schema → `enforce_deny_only` (deny rules can veto; `:805-812`) → per-run budget `MAX_MUTATION_CALLS_PER_RUN = 1` (second attempt returns synthetic `mutation_budget_exhausted`, outcome `skipped`) → audit event gains `"mutation": true` → SSE unchanged (`tool.completed` outcome enum already covers ok/failed/denied/skipped; no frame-shape change). Search results mark the op `"mutation": true` so the model must knowingly choose it; the prompt (§10) requires an explicit user instruction before any mutation call and honest reporting of its result.

**Why one entry:** the demo needs exactly one safe internal-test write ("execute one safe test operation" against Aevatar). Candidates, smallest blast radius first, to be pinned during live validation: create a workspace directory; create a workflow *draft*; post a test chat conversation. All are visible, deletable via the Aevatar UI, and non-cascading. NyxID **account** mutations remain excluded (§9) — the account-state-unchanged ephemerality beat survives.

## 9. Internal account tools — layering

New file `backend/src/services/assistant_direct_agent_poc/account.rs` (inside the deletion boundary). It is a *service-layer* module: it calls existing service functions and reads models directly where the service layer already does — it never invokes handlers, never self-calls HTTP, and never serializes model structs into results (dedicated projection structs, mirroring the handlers' response-struct rule).

| Operation | Backing (verified) | Projection (all fields explicit; nothing else) |
| --- | --- | --- |
| `nyx_whoami` | `users` `find_one` by `_id` (the pattern `handlers/users.rs:85-96` uses; no service-layer whoami exists today, so the POC owns one) + `org_service::list_memberships_for_member` (`services/org_service.rs:860`) + `org_service::get_org_user` (`:113`) for names | `user_id, display_name, email, auth:"session", orgs[{org_user_id, org_name, role}]` — no password hash, no MFA secrets, no capabilities dump |
| `list_api_keys` | `key_service::list_api_keys` (`services/key_service.rs:539`; active-only, metadata-only by construction — model stores `key_prefix`/`key_hash`, `models/api_key.rs:23-25`) | `id, name, key_prefix, platform, allow_all_services, allowed_service_count, last_used_at, created_at` |
| `list_nodes` | `node_service::list_user_nodes` (`services/node_service.rs:432`; unions org nodes via memberships) | `id, name, owner{kind, display_name}, status, is_connected, last_heartbeat_at` — **raw `Node` carries token/secret material; the projection is mandatory** |
| `list_approvals` | `approval_service::list_requests(db, user_id, &[], statuses, 1, limit)` (`services/approval_service.rs:1344`; personal branches only) | `id, service_name, action, status, created_at, decided_at` + `total` |

Excluded and why: `get_or_create_channel` (notification settings) inserts on first read (`services/notification_service.rs:757-770`) — not a pure read; `GET /keys` semantics — `auto_provision_no_auth_services` writes; `approval_service::get_request` (`:1391`) has no ownership filter — no single-row fetch is exposed; API-key create/rotate return raw bearer tokens (`key_service.rs:24-45, 244, 590`) — never model-reachable; connect-link create returns a raw `nyx_clk_` token (`connect_link_service.rs:145, 48`) — excluded. If account mutations are ever added, reuse the receipted, replay-safe `assistant_action_execution_service` pattern (`services/assistant_action_execution_service.rs:385`) — recorded as the precedent, not built now.

All results still pass through `ModelToolResult::from_response(200, …)` — uniform scrub + cap. Dispatch arms live beside the existing ones in `execute_call_inner` (`assistant_direct_agent_poc.rs:701-789`); in-process ops skip `enforce_deny_only` (they are not downstream operations) but are audited per-call like every tool.

## 10. Skills: sources, versioning, provenance, Ornn bounds

`nyx_search_skills` merges two sources; `nyx_get_skill` fetches by `{source, id}`:

- **Bundled** (`DIRECT_SKILLS`, `services/assistant_direct.rs:60-85`): search is substring over slug/label; fetch by exact slug. Provenance/version: `bundled@` + `env!("CARGO_PKG_VERSION")` in both the search row and the fence header — deterministic, build-pinned, no runtime state. A bundled body >16 KiB truncates honestly at the envelope cap (const-asserted ≤64 KiB at build; the lossless path remains prompt injection via the existing `skill_slug` request field, which is unchanged).
- **Ornn** (bounded, visibility-safe): search reuses the fixed `GET /api/v1/skill-search` descriptor + projection (`tools.rs:385-404, 956-981`) through the user's own connected `ornn-api` UserService (`resolve_ornn_service`, `:274-287` — ambiguity fails closed), so Ornn's own ACL applies via the row's identity propagation. Fetch widens v0's single-GUID constant to a **run-local observed set**: `nyx_get_skill{source:"ornn"}` accepts only GUIDs returned by an earlier `nyx_search_skills` call *in this run* (no blind GUID probing), still max **one Ornn fetch per run**, still exact-path SKILL.md extraction (`:1037-1061`), still fenced: `--- BEGIN untrusted skill content (ornn, id=…, version=…, fetched …) ---`. Fetched content remains untrusted reference data — the fence is hygiene; the walls are the registry, the allowlist, and the deny checks (v0 R4 stands; the hostile-skill fixtures are extended to try to trigger the mutation entry).

Both fences share one format so a later runtime can parse provenance uniformly: `source`, `id`, `version`, `fetched_at`.

Prompt updates (`backend/prompts/direct/agent-poc.md` + `prompt.rs` phase strings): tools list refreshed; rule 5 (read-only registry) becomes "mutations are possible only for operations explicitly marked `mutation:true`; call one only when the user's request requires it, at most once per run; for anything else give the `nyxid` CLI command"; Final phase instruction retitled in text as the **Report** phase (stage id remains `final`): lead with verified results, cite producing tool calls, name every failed/denied/skipped check, report mutation outcomes with the actual downstream identifiers.

## 11. Aevatar as downstream (and only downstream)

**Verified repo facts:** `aevatar` is **not seeded anywhere** — no overlay in `backend/specs/catalog/` (22 files, none aevatar), no `catalog_spec_registry` entry (`SLUG_TO_SPEC_KEY`, `services/catalog_spec_registry.rs:117-144`), no row in `provider_service::seed_default_services` (`services/provider_service.rs:3452`). The live row is admin-created; an admin-created `DownstreamService` inherits serde defaults `identity_propagation_mode:"none"`, `forward_access_token:false`, `inject_delegation_token:false`, `visibility:"public"` (`models/downstream_service.rs:161-162, 224-250, 357-371`) unless explicitly configured. The slug is the code constant `AEVATAR_SLUG` (`services/assistant_service.rs:31`).

Primary path — **zero code**: platform catalog discovery admits any active http row with `requires_user_credential:false` and category ≠ provider (`mcp_service.rs:720-782`; note there is **no visibility filter at discovery** — private rows are listed but blocked at execution by the PR-A gate). Endpoints come from `ServiceEndpoint` rows; with zero rows the service is dropped from the operation catalog entirely (`operation_set_is_publishable`, `:559-572`). The concrete no-code remedy exists in main: once an admin sets `openapi_spec_url` on the live row, `catalog_spec_sync::sync_spec_backed_service_endpoints` (`services/catalog_spec_sync.rs:87-103`, plus `spawn_spec_endpoint_sync` on admin update `:108-131`) auto-materializes `ServiceEndpoint` rows for active http services with a spec URL and zero rows. Then `nyx_search_tools` finds `aevatar__…` reads and the one allowlisted mutation, and `nyx_call_tool` executes them.

**Authentication on this path (corrected by verification):** `execute_tool` mints no delegation token (§4). Typed Aevatar calls therefore authenticate only via identity propagation — the live row must set `identity_propagation_mode` to `jwt` or `both` (audience via `identity_jwt_audience`, e.g. `urn:aevatar:api`) for Aevatar to receive `X-NyxID-Identity-Token`. The chat path's `inject_delegation_token` flag is irrelevant here. If the live row is identity-`none`, typed calls arrive unauthenticated and Aevatar will reject them — this is a config readback item (B2), not code.

Fallback — **only if catalog metadata is missing**: a POC-owned fixed-descriptor set in `tools.rs`, exactly the Ornn pattern (`ornn_search_endpoint()`, `:385-404`), limited to 2 typed reads (e.g. list agents; list workflows) + the single allowlisted test mutation, resolved against the user's authentic connected `aevatar` UserService the way `resolve_ornn_service` works, engaged **only when the canonical catalog yields zero `aevatar` operations**. Under no interpretation does Aevatar schedule, resume, or own the run — it only answers typed HTTP calls.

PR-A interaction (exact): the `Platform` branch of `execute_tool` resolves through `resolve_proxy_target`/`_lenient` (`mcp_service.rs:3226-3241`), which now run `authorize_master_credential` (`services/proxy_service.rs:151-213`) — a **private** credential-bearing row 404s without an OAuth-app consent; `auth_method:"none"` rows never reach the gate. The Chrono hop path (`execute_admin_proxy` → `resolve_admin_proxy_target` → `authorize_master_credential_server_chosen`, `proxy_service.rs:217-244, 751`) hard-requires `visibility:"public"` for credential-bearing rows; `auth_method:"none"` early-returns (`:736-748`). The §12 runbook includes the PR-A readback from `docs/assistant/PR_A_VERIFICATION.md` §6 for `aevatar`, `ornn-api`, and `chrono-llm-public`.

## 12. Tests, live validation, demo

**Backend (unit/handler, CI-safe, stub-only)** — extend the existing suites in place:
- Registry: allowlisted mutation admitted; same op absent from table rejected; DELETE in table still rejected (verb re-derivation wins); read eligibility unchanged; `mutation:true` marker present in search output only for the entry.
- Budget: second mutation in one run → synthetic `mutation_budget_exhausted`, outcome `skipped`, no dispatch (observer count 0, pattern `assistant_direct_agent_poc.rs:1437-1470`).
- Account: each operation returns only projected fields (assert absence of `key_hash`, node token material, email of other users); `whoami` org join; limits clamped; results scrubbed + capped.
- Skills: bundled search/fetch; ornn fetch rejected without prior in-run search hit; observed-set accepts a searched GUID; one-ornn-fetch cap; fence format for both sources; hostile SKILL.md cannot invoke the mutation entry or any non-allowlisted op (extends the existing hostile fixtures).
- Prompt shape: new rules present; exactly one phase instruction; no `DIRECT_MODE_OVERRIDE`.
- Frames/audit: `model_controlled_tool_identity_never_reaches_frames_or_audit_metadata` extended to the new names; mutation audit carries `"mutation": true` and no arguments/bodies.
- Billing: **no new routes** ⇒ `billing_route_coverage_smoke` and the existing T8b case remain green untouched.

**Frontend:** no schema changes required (verified §2). Add two fixtures: a run exercising `nyx_whoami`/`nyx_account_call` steps, and a mutation run (step meta shows the op; outcomes unchanged) — replayed through the existing `direct-agent-poc.test.ts` harness. Optional copy tweak in `AGENT_POC_MODE_COPY` ("read-mostly tools · one allowlisted test write").

**CI commands (the exact gates):**
```bash
cargo fmt --all -- --check                       # ci.yml:165
cargo clippy --workspace --all-targets -- -D warnings   # ci.yml:183
cargo test -p nyxid billing_route_coverage_smoke -- --nocapture   # ci.yml:239
python3 scripts/check-backend-docker-embeds.py   # prompt files must stay under backend/
cargo nextest run -p nyxid --profile ci          # needs replica-set Mongo on :27017 (ci.yml:198-226)
cd frontend && npm run lint && npm run test && npm run build      # ci.yml:337-340; build = tsc -b + vite
```
Local Mongo: `docker compose up -d` (replica set on 27018; export `NYXID_TEST_DATABASE_URL` accordingly — 5000 tests failing in seconds means a connection fault, not regressions).

**Live validation runbook (pre-demo, mutable prod state):** flag on for demo user; PR-A readback for `aevatar`/`ornn-api`/`chrono-llm-public`; **read back the live `aevatar` row's `identity_propagation_mode` + `identity_jwt_audience`** — must be `jwt`/`both` for typed `execute_tool` calls to authenticate (§11; the chat path's delegation token does not exist here); confirm `aevatar` typed operations appear in the user's operation catalog (else set `openapi_spec_url` and let `catalog_spec_sync` materialize rows — B2 — or accept fallback descriptors); pin the mutation allowlist entry against the live catalog (B1) and execute it once by hand to confirm blast radius; re-run the v0 checklist items (Ornn service unique+executable, budgets, third concurrent run → 429).

**Demo script (target scenario, item 10):** ask: *"Check what I'm connected to, load the relevant NyxID skill, look at my Aevatar agents and workflows, run one safe test operation there, and report exactly what you saw."* Expected: Understand shows live counts; Plan streams; Execute steps: `nyxid · whoami` → `nyxid · connected_services` → `skills · search`/`skills · get` (fenced) → `aevatar · <read>` ×2 → `aevatar · <mutation>` (one) → Report cites actual IDs/status/errors from this run's tool results, names anything denied/failed, and states the one mutation with its downstream identifier. Reload: everything gone; audit shows metadata-only events; NyxID account state unchanged.

## 13. Task breakdown (files + acceptance criteria)

| # | Task | Files | Acceptance criteria |
| --- | --- | --- | --- |
| E1 | Budgets + registry generalization | `services/assistant_direct_agent_poc.rs` (consts, dispatch arms, `tool_identity`, `safe_tool_name`), `tools.rs` (rename gate to `is_poc_operation_admitted`, keep read predicate byte-identical) | Existing read behavior byte-identical when allowlist is empty; new budget values in `run.started`; all v0 tests green |
| E2 | Mutation allowlist | `tools.rs` (const table, mutation predicate, `mutation:true` in search rows), `assistant_direct_agent_poc.rs` (per-run budget, audit flag) | §8 defense-in-depth tests pass; empty-table build = pure read-only POC |
| E3 | Account module | new `services/assistant_direct_agent_poc/account.rs`, dispatch arms, tool definitions | §9 projections exact; secret-absence asserts; no handler imports; deletion boundary intact |
| E4 | Skills merge | `tools.rs`, `prompt.rs`, `services/assistant_direct.rs` (only if `find_skill`/`DIRECT_SKILLS` visibility needs widening to `pub(crate)`) | §10 provenance/caps/observed-set tests; `ornn_*` names gone; chat-mode prompt tests untouched |
| E5 | Aevatar fallback descriptors (conditional) | `tools.rs` | Engaged only on zero catalog `aevatar` ops; same eligibility predicate; removed by deleting one const block |
| E6 | Prompt/report language | `backend/prompts/direct/agent-poc.md`, `prompt.rs` | Prompt-shape tests; docker-embeds check green |
| E7 | Backend test suite | test mods in the three POC files | §12 list; `cargo nextest run -p nyxid --profile ci` green |
| E8 | Frontend fixtures + copy | `frontend/src/lib/assistant/__fixtures__/*`, `direct-agent-poc.test.ts`, optional `direct-chat-controls.tsx` copy | `npm run test`/`build` green; zero schema edits; replay covers mutation + account steps |
| E9 | Live runbook + allowlist pin | this doc §12 executed; one-line allowlist edit | B1/B2 resolved; demo dry-run recorded |

Estimated effort: E1-E2 ≈ 1.5 d, E3 ≈ 1 d, E4 ≈ 1 d, E5 ≈ 0.5 d, E6 ≈ 0.5 d, E7 ≈ 1.5 d, E8 ≈ 0.5-1 d, E9 ≈ 0.5 d ⇒ **≈ 7-7.5 engineering-days** (one engineer; live-validation access assumed; review time excluded).

## 14. Config/migration impact, deleteability, risks, blockers

**Config/migration:** zero migrations, zero new env vars, zero new collections, zero new routes or billing inventory rows, no flag changes (rides `experimental:direct-chat-engine`). The only environment actions are pre-demo: flag grant, optional `openapi_spec_url` on the live `aevatar` row, allowlist pin.

**Deleteability:** unchanged from v0 — delete the `assistant_direct_agent_poc*` files (now including `account.rs`), the prompt file, the FE `direct-agent-poc*` files, one route tuple, one FE branch + control. `mcp_approval.rs` stays (shared with MCP transport). Nothing new leaks outside the boundary.

**Risks:**
- R-A (med): live `aevatar` row publishes no typed operations → fallback descriptors (E5) or config fix (B2); the plan works either way, but the demo needs one of them true.
- R-B (med): mutation choice is prod-state-dependent; a wrong pick mutates something regrettable → B1 requires a human pin + one manual execution before the demo; table ships empty until pinned.
- R-C (low): budget increases raise per-run cost/latency (12 hops × Chrono) → bounded by wall deadline; internal flag-gated surface.
- R-D (low): preflight 448 KiB cap is tighter at 12 hops → fail-closed `context_overflow` already handles it; forced-final absorbs.
- R-E (low): strict FE schemas — any future frame-field addition must be lockstep; this plan adds none.
- R-F (carried from v0): prompt injection via fetched skills (structural walls, now including the mutation allowlist, hold); unbounded `execute_tool` drains (R9); non-cancellable in-flight tools (R10); session bypasses interactive approvals (deny-only enforcement).
- R-G (low): PR-A gate — a demo service in private+credentialed shape would 404 at execution (while still being *listed*, since discovery has no visibility filter); runbook readback catches it.
- R-H (med): the live `aevatar` row may be configured for the chat path (`inject_delegation_token`) but not for identity propagation — typed `execute_tool` calls would then arrive unauthenticated and fail. Config fix (identity mode `jwt`/`both`), verified in the runbook; no code.

**Blockers requiring a human decision:**
- **B1:** the exact `(aevatar, <endpoint>)` mutation allowlist entry — needs the live catalog readback and owner sign-off on the specific operation (workspace-directory create is the recommended candidate).
- **B2:** the live `aevatar` row's shape: (i) whether an admin sets `openapi_spec_url` on it (preferred — `catalog_spec_sync` then materializes `ServiceEndpoint` rows with zero code) or the POC ships the E5 fallback descriptors; and (ii) confirming/setting `identity_propagation_mode: jwt|both` + `identity_jwt_audience` so typed `execute_tool` calls authenticate (§11, R-H). Decide at live validation; both halves are config, not code.
