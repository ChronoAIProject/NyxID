# Adversarial review — Chrono-LLM Ephemeral Agent plan (Opus lens)

Reviewer: Opus · 2026-08-13 · worktree `zen-fox`, branch `chat-chronollm-direct-effort`
Target: `docs/chat/chrono-llm-ephemeral-agent-plan.md` (Draft v1)
Lens: architecture correctness · state-machine completeness · user-visible behavior · trust boundaries ·
prompt/skill semantics · effort estimate · frontend event rendering · cancellation · error terminality.

Method: every load-bearing claim was re-derived from source on this branch. Nothing below is taken from the
plan's own citations without opening the file. Probe results P1–P6 (§4) are **live-only** and were not
re-executed here; findings that depend on them are labelled as such. Prod DB state (`chrono-llm-public`,
`ornn-api` rows) is likewise unverifiable locally and labelled.

**Verdict: BUILD-WITH-FIXES.** 2 BLOCKER · 10 MAJOR · 11 MINOR · 4 overengineering removals.
Both blockers are localized design edits, not approach failures. The core bet — reuse `execute_tool`,
reuse `execute_admin_proxy`, reuse `RunCard`, persist nothing — survives attack. What does not survive is
the deny-rule wiring, the Ornn endpoint story, three frontend terminal paths, and the effort estimate.

---

## BLOCKER

### B1 — The deny-rule floor evaluates the wrong service id, so it is a silent no-op

**Claim attacked.** §5.3: "Per tool call: `approval_service::evaluate_deny_only` (`approval_service.rs:184`)
runs before execution — the exact parity of what a session user gets on the REST proxy today." §7's
`nyx_call_tool` row: `resolve_tool_call` (`:2534`) → `build_mcp_operation_descriptor` (`:2731`) →
`evaluate_deny_only` → `execute_tool` (`:3039`).

**Evidence.**
- `evaluate_deny_only` takes **five** arguments — `(db, actor_user_id, service_owner_user_id, service_id,
  descriptor)` — `backend/src/services/approval_service.rs:184-190`. The plan's chain names no source for
  `service_owner_user_id` and implies `service_id` comes from the resolved `McpToolService`.
- For user-managed rows, `McpToolService.service_id` **is** `UserService.id`:
  `backend/src/services/mcp_service.rs:1045` (`service_id: us.id.clone()`) and `:1054-1055`
  (`source: UserManaged { user_service_id: us.id.clone(), … }`).
- The MCP transport — the precedent the plan invokes — never passes that id to approval evaluation. It goes
  through `approval_target_for_tool`, whose doc comment states the trap verbatim:
  `backend/src/handlers/mcp_transport.rs:1424-1428` — *"MCP activation uses `UserService.id`, while
  catalog-backed approval policies use `catalog_service_id`; those identities must not be conflated."*
  It resolves via `proxy_service::find_approval_resolution_hint_by_user_service_id`
  (`backend/src/services/proxy_service.rs:1666-1672`), which additionally runs the org ACL
  (`org_service::resolve_owner_access`, `:1697-1711`) and returns `{service_id, service_owner_id}`.
- The REST-proxy precedent the plan cites (`handlers/proxy.rs:1376`) sits *inside* a function that already
  resolved a hint the same way (`proxy.rs:1334-1370`) — it is not a two-argument shortcut.

**Consequence for the demo.** Deny rules are keyed on the catalog service id; the agent would evaluate them
against a `UserService` UUID that no approval config references. Every deny rule misses, silently. The only
pre-execution authorization gate in a design whose §5.3 is titled "Authorization posture" does nothing, and
org-owned services additionally lose the owner-access check the hint performs. T4 AC3 ("a locally-inserted
deny rule yields `denied_by_policy`") does not pin this: it passes or fails purely on which id the fixture
writer chose. Residual real-world risk is bounded (session-equivalent reach, the user's own services), but
the plan asserts a boundary it would not have.

**Minimal fix.** Lift `approval_target_for_tool` out of `handlers/mcp_transport.rs:1428` into
`services/mcp_service.rs` (or a small `mcp_approval` helper), call it from T4, and pass its output:
`evaluate_deny_only(db, &actor, &target.service_owner_user_id, &target.service_id, &descriptor)`. Change T4
AC3 to require the deny rule be inserted against `catalog_service_id`, and add a second case for an
org-owned service.

---

### B2 — The Ornn tool path has no verified endpoint source and no specified fallback

**Claim attacked.** §8 step 2: "Resolve the two pinned endpoints from the loaded catalog by
`service_slug == "ornn-api"` + method + path-template match … If absent → typed `ornn_not_connected`."
§17 lists "Dynamically resolve relevant skills from Ornn" as **MUST**; §16 step 5 scripts it on stage.

**Evidence.**
- For a catalog-backed `UserService` with no instance `openapi_spec_url`, published endpoints come from the
  catalog template `ServiceEndpoint` rows, and the no-rows case yields **empty endpoints with
  `is_generic_proxy = false`**: `backend/src/services/mcp_service.rs:995-1020`, specifically the
  `None => (template_rows, false, false)` arm at `:1018` where `template_rows` defaults to
  `endpoints: Vec::new()` (`:993-997`).
- There is no Ornn overlay: `backend/specs/catalog/` contains 22 files (anthropic … twitter), none for ornn.
- There is no `ornn-api` catalog seed in `backend/src` — the only occurrence is a test helper,
  `backend/src/handlers/oauth.rs:4484`.
- The only in-repo description of the Ornn surface is prose:
  `skills/nyxid-service-skill-authoring/SKILL.md:28` (`/api/v1/skills/<name>/json`), `:103`
  (`/api/v1/skill-search?query=…&mode=semantic&scope=mixed&pageSize=20`), `:107`.
- §19 flags *auto-provisioning* of the row (item 2) but never flags **endpoint publication**, which is the
  thing the pinning scheme depends on.

**Consequence for the demo.** Unless prod's `ornn-api` catalog row happens to carry an `openapi_spec_url`
that the auto-discovery sweep already expanded into `ServiceEndpoint` rows, both Ornn tools return
`ornn_not_connected` at run time. The MUST is unmet and beat 5 of the demo script dies — and this is only
discoverable at Stage 0, i.e. after the plan is approved and Stages 1–3 are budgeted.

**Minimal fix (two parts).**
1. Make "the demo user's `ornn-api` publishes ≥2 typed endpoints matching `GET …/skill-search` and
   `GET …/skills/{…}/json`" an explicit **go/no-go gate** in Stage 0, not an implicit assumption.
2. Specify the fallback the plan currently forbids: when no typed endpoint matches, execute against the
   service's generic-proxy endpoint with a **server-built** descriptor
   `{method:"GET", path:"/api/v1/skill-search", query:…}`. This is still zero model-supplied path, and it is
   already safe — `build_generic_proxy_args` runs `validate_requested_proxy_path`
   (`mcp_service.rs:3588-3639`, `proxy_service.rs:396-402`). Note that this fallback requires the service to
   actually expose a generic-proxy endpoint, which the empty-template arm does **not** produce
   (`is_generic_proxy = false`), so budget the deterministic option too: add `ornn.openapi.json` under
   `backend/specs/catalog/` (Docker-embed-safe, drift-guard-listed) so the endpoints exist by construction.

---

## MAJOR

### M1 — `billing_route_coverage_smoke` goes red: every mounted metered route must be exercised end-to-end

**Claim attacked.** T1 AC3: "billing inventory smoke green with the new entry, red without it."

**Evidence.** `assert_mounted_routes_are_exercised` (`backend/src/billing_integration_tests.rs:2176-2188`)
asserts `exercised_routes == every mounted Metered route`, with the message *"every mounted metered route
must cross its real route boundary in the smoke test"*. The existing direct-chat case is ~50 lines: catalog
row with `platform_metric: Tokens`, flag override, real `build_router`, a live call through
`call_mounted_route`, `exercised_routes.insert(…)`, and `assert_direct_reported_usage`
(`:283-333`).

**Consequence.** Adding `/api/v1/assistant/direct/agent` as any `Metered(_)` route leaves the suite failing
until a full agent run is driven through the real router with a stub Chrono emitting a P3-shaped tool_call
stream **and** a stub tool downstream. That is the hardest test in the plan and it is unbudgeted — T1's AC
describes only inventory-table parity.

**Minimal fix.** Add an explicit task (T8b) and AC: "one single-tool agent run crosses
`/api/v1/assistant/direct/agent` inside `billing_route_coverage_smoke` and inserts it into
`exercised_routes`." Budget ~0.5–1 d.

---

### M2 — The route declares `Mcp` and then fabricates `Metered(Proxy)` for its own hops

**Claim attacked.** T2: route registered `Metered(BillingIngress::Mcp)`; each synthesized LLM-hop request
gets `.extensions_mut().insert(BillingRoutePolicy::Metered(BillingIngress::Proxy))`. Framed as an "honest
compromise" about billing fidelity.

**Evidence.** The mechanics work — but the compromise is not about fidelity, it is about the invariant.
`enforce_billing_egress_classification` (`backend/src/services/billing/route_inventory.rs:48-70`) exists
specifically so a handler cannot forward downstream without a **route-declared** policy; `None` is a hard
`Internal` error (`:66-69`). `assert_http_egress_classification_is_fail_closed`
(`billing_integration_tests.rs:2118-2141`) pins that fail-closed behavior. A handler that writes the policy
into a request it constructed itself converts a router-level guarantee into handler self-attestation — and it
does so for the *expensive* egress (every LLM hop), leaving only the cheap `Requests`-metered tool calls
router-attested.

**Consequence.** Not a demo failure; a precedent failure. After this lands, "handlers may declare their own
billing class" is the pattern of record, and the fail-closed test no longer covers the newest streaming
route's dominant spend.

**Minimal fix (cheaper and more honest).** Declare the route `Metered(BillingIngress::Proxy)` — truthful for
its dominant egress — so hops carry the router-attested policy verbatim with no fabrication. Then mint the
tool-call permit through **one** named, unit-tested function added inside `route_inventory.rs` (the permit's
field is private, `:43-46`, so it can live nowhere else), e.g. `assistant_agent_tool_permit(policy)`
accepting `Metered(Proxy)` from this route only. Net effect: one auditable, tested widening instead of
per-hop fabrication.

---

### M3 — Cancellation and every failure path leave the run ledger permanently spinning

**Claim attacked.** §9 TIMED_OUT/CANCELLED rows; §10 mapping rows for `error` and user cancel; §20's own
question "the ledger must show `active`→terminal for every started tool".

**Evidence.**
- `RunCard` renders any step with `status === "active"` as `<Loader2 … animate-spin />`
  (`frontend/src/components/assistant/blocks/run-card.tsx:23-26`, rendered at `:96-100`) **regardless** of
  `block.state`; terminal state only changes the header (`:40, 55-67`).
- The one helper that settles run steps is `toTerminalBlock` (`frontend/src/lib/assistant/stream.ts:19-40`),
  which maps `active`/`waiting` → `skipped` and state → `cancelled`.
- It is called from exactly one place: `cancelRun` (`direct-transport.ts:848`), and only for the open **text**
  block (`:838-850`). It is **not** called from the failure paths:
  `streamTurn`'s catch → `closeOpenMessage` + `finishUi("failed")` (`:692-708`); the idle/first-byte timeout
  rejection (`:558-581`); the `[DONE]`-without-terminal branch (`:680-690`); the HTTP-error branches
  (`:605-639`).

**Consequence.** On `error{deadline_exceeded}`, FE idle timeout, network drop, or truncated stream, the user
sees a card headed "failed" with a tool step spinning forever — on stage, in a demo about honest state.

**Minimal fix.** F2 adds `settleRun()` emitting
`block.completed(run, {...toTerminalBlock(runBlock), state: "failed" | "cancelled"})`, called from every
terminal path (not just the `error` frame). F4 gains fixtures for FE idle-timeout and mid-stream network
error, not only the server-sent `error` frame.

---

### M4 — Pre-seeded `waiting` steps render the literal text "waiting for approval"

**Claim attacked.** §10 mapping: `run.started` → `block.started(RunContentBlock{…, steps:[Understand·active,
Plan·waiting, Answer·waiting]})`, justified by "the run-ledger idiom `blocks/run-card.tsx` already renders".

**Evidence.** `run-card.tsx:110-114` renders, for **any** step with `status === "waiting"`, a hardcoded
sub-line `waiting for approval`; `StepIcon` gives it the amber `Clock3` (`:27-28`).

**Consequence.** The very first painted frame of the demo claims two of the four stages are "waiting for
approval", in a design whose §5.3 explicitly has **no** approval waits and whose pitch is bounded, honest
capability. This is the most visible defect in the plan and it is invisible from the plan's own text.

**Minimal fix.** Never emit `waiting`. Append each stage/tool step only when it starts, with status `active`;
`steps_total` grows as the plan already describes for tool steps. (Also: `RunContentBlock` has no top-level
`meta` field — `frontend/src/types/assistant.ts:249-265` — so §10's "`meta := detail`" is valid only on a
step object.)

---

### M5 — No heartbeat; the frontend's 120 s idle timeout is structurally reachable mid-run

**Claim attacked.** F1 AC4: "first-byte/idle timeouts (`:37-38`) apply unchanged."

**Evidence.** `IDLE_TIMEOUT_MS = 120_000` (`direct-transport.ts:38`), applied around **each**
`reader.read()` (`:645-654`), aborting the controller on fire (`:569-573`). In EXECUTING, a hop that returns
only `tool_calls` emits no frame at all between `tool.completed(k)` and `tool.started(k+1)` — one complete
model round trip with zero bytes on the wire. The request grammar the agent reuses accepts
`reasoning_effort` up to `"max"` (`backend/src/services/assistant_direct.rs:130-151`), and the plan's own
server deadline is 180 s, i.e. it explicitly anticipates hops long enough to matter.

**Consequence.** The browser aborts a perfectly healthy run with "The direct model stream stopped
responding" while the server keeps burning the hop — a failure mode indistinguishable, on stage, from the
engine being broken.

**Minimal fix.** Emit an SSE comment heartbeat (`: ping\n\n`) every ~10 s from the run task. This needs
**zero** frontend change: `drainSseBuffer` keeps only `data:`-prefixed lines
(`frontend/src/lib/assistant/sse.ts:32-38`), so a comment is discarded as a payload while still delivering
bytes that reset the read timer. Add it to T6.

---

### M6 — `MAX_LLM_CALLS` accounting is self-contradictory, and the contradiction is shipped to the browser

**Claim attacked.** §9: PLANNING is "LLM call 1"; EXECUTING exits on "last LLM slot" → FINALIZING;
"Total upstream calls ≤ `1 + MAX_LLM_CALLS`". §10 ships `limits.max_llm_calls: 6` in `run.started`.

**Evidence.** Internal to the plan. If planning consumes slot 1 and finalizing consumes the last slot, the
total is `MAX_LLM_CALLS` (6), not `1 + MAX_LLM_CALLS` (7). If planning is extra, the exit condition "last LLM
slot" refers to a counter that does not include it. The three sentences cannot all be true.

**Consequence.** An off-by-one in the only bound that terminates the loop; a user-visible `limits` frame that
cannot be reconciled with observed behavior; and T3 AC2 ("last LLM slot forces FINALIZING with
`tool_choice:"none"`") is untestable as written because the slot is undefined.

**Minimal fix.** Define `MAX_LLM_CALLS` as *total upstream calls, including plan and finalize*; delete the
"Total upstream calls ≤ 1 + `MAX_LLM_CALLS`" sentence; state that FINALIZING consumes the final slot.

---

### M7 — `finish_reason` values other than `stop`/`tool_calls` have no defined transition

**Claim attacked.** §9 PLANNING and EXECUTING rows; §20's own request to "walk every `finish_reason` ×
budget-edge combination".

**Evidence.** PLANNING handles `stop` and "tool_calls anyway"; EXECUTING handles `tool_calls` and `stop`.
Nothing handles `length`, `content_filter`, or a stream that closes with no `finish_reason` at all. `length`
is reachable: no `max_tokens`/`max_completion_tokens` is sent anywhere on this surface today
(`build_upstream_body` emits exactly `model`, `stream`, `stream_options`, `messages` [+ `reasoning_effort`] —
`backend/src/services/assistant_direct.rs:250-262`, pinned by the 4-key/5-key assertions at `:398, 406, 454`),
so the model's own output cap is the only ceiling and it is hit by long grounded answers. The
no-`finish_reason` case is real enough that the shipped frontend has a dedicated code for it
(`truncated_stream`, `direct-transport.ts:682-689`).

**Consequence.** Undefined behavior at exactly the moment the answer is being written. The most likely
realization is a loop that neither advances nor terminates until the 180 s watchdog converts a nearly
complete answer into `error{deadline_exceeded}`.

**Minimal fix.** One rule, pinned in T3 AC: any terminal `finish_reason` other than `tool_calls` ends the run
as DONE, emitting `done{status:"completed", finish_reason}` so the UI can label a truncated answer; a stream
that closes with neither a `finish_reason` nor `[DONE]` is `error{upstream_failed}`.

---

### M8 — Plan text re-enters the transcript on every later turn; OQ3's stated answer contradicts shipped code and no task owns the change

**Claim attacked.** OQ3: "whether plan-stage text re-enters the client transcript on later turns (plan:
display-only; only final text is resent)." F3 AC3: "mode switch mid-conversation is safe (client transcript
is text-only in both modes)."

**Evidence.** `toDirectMessages` (`direct-transport.ts:128-142`) rebuilds the outgoing transcript from
**every** text block of each assistant message — `.filter(block => block.type === "text")` … `.join("\n\n")`
— and `toBoundedDirectMessages` (`:144-165`) only truncates it. §10 produces two text blocks per agent turn
(`-plan` and `-final`). Therefore turn N+1 sends `plan\n\nfinal` as a single assistant message. No task in
F1–F4 changes `toDirectMessages`.

**Consequence.** From turn 2 onward the model is shown its own scratch plan as if it were part of its
answer — the "I checked X, then I called Y" narration that §6.2's grounding rules exist to suppress, fed back
as ground truth. It also roughly doubles transcript growth against `MAX_OUTGOING_CONTENT_BYTES`
(`:34`), pulling the trim in earlier.

**Minimal fix.** This is not an open question — it is unowned work. Assign to F2 with an AC: tag the plan
block (id suffix `-plan`) and exclude it in `toDirectMessages`, or carry the plan on the run block instead of
a text block. Either way, pin it with a fixture-replay test asserting turn 2's request body contains only the
final text.

---

### M9 — Disconnect does not cancel the in-flight hop; synthesizing hop requests severs the proxy's own cancellation

**Claim attacked.** §9 CANCELLED: "client disconnect ⇒ channel send fails ⇒ CancellationToken stops the loop
before the next hop/tool". T6 AC1: "drop the response while a stub tool sleeps → no subsequent hop reaches
the stub."

**Evidence.** `execute_proxy_inner` derives its downstream cancellation from `request_cancellation(&request)`
(`backend/src/handlers/proxy.rs:1416`), which reads the `ClientConnectionCancellation` extension and
otherwise returns `CancellationToken::default()` — *"Requests constructed without the production listener
receive an independent token"* (`backend/src/downstream_disconnect.rs:42-51`). Hops built by the agent
handler carry no such extension, so the proxy's `until_client_disconnect` / `CancelOnDropStream` machinery
(`:61-107`) is inert for them. Separately, detection is send-driven: an `mpsc` send fails only after the
receiver drops, so a run parked inside a 60 s `TOOL_TIMEOUT` or a silent hop observes nothing.

**Consequence.** The CANCELLED row overstates reality. A closed tab leaves a Chrono stream running (and
metered) until it completes or the 180 s watchdog fires. T6 AC1 only tests the *next* hop, so it would pass
against the defect.

**Minimal fix (~5 lines).** Capture `request_cancellation(&request)` at handler entry, drive the run task's
`CancellationToken` from it, and clone the inbound `ClientConnectionCancellation` value into each synthesized
hop's extensions — the type is `pub(crate)` and `Clone` (`downstream_disconnect.rs:29-40`), so the handler can
copy it even though it cannot construct one. Extend T6 AC1 to assert the **in-flight** hop aborts.

---

### M10 — The effort estimate contradicts the only comparable in-repo actual by ~4–5×

**Claim attacked.** §14: "Total ≈ **9–10.5 dev-days** for one engineer … ~6–7 calendar days with two people.
Fits the review pipeline (Fable plans / Sol reviews+implements / Opus reviews) that shipped the direct
engine."

**Evidence.** That pipeline shipped the entire direct engine — new route + handler + server-owned request
contract + prompt composition + rate limiter + billing metering + FE transport + engine router + fixtures +
two full review-fix rounds + the effort picker — across **two calendar days**, 2026-08-11 → 2026-08-12:
`5a1e6532` (BE-1..3), `6f64136c` (BE-6..8), `58f4d5d9` (FE-1,2,5), `beab037e`, `1e1e7f52`, `43e2bed9`,
`8cfec9c8`, `6689cc7a`, `bb10b257`, `1f774cff`. The agent plan's scope is comparable, minus the engine-router
work, plus a tool loop.

**Consequence.** A 9–10.5 day price tag on an internal demo is a plausible reason to not fund it, and the
number is not derived from the pipeline's demonstrated throughput on the adjacent feature. Estimate risk cuts
both ways: it also hides where the *real* new cost sits.

**Minimal fix.** Restate the estimate in the same unit as the actuals (agent-pipeline calendar days) and
itemize the two genuinely novel cost centers instead of a flat per-task day count: the billing coverage smoke
case (M1) and Stage-0 prod verification of `ornn-api` endpoint publication (B2) — both of which are serial and
cannot be parallelized by adding a second implementer.

---

## MINOR

- **m1 — §3 overstates reuse.** Of the three functions in the "Server-owned request contract + prompt
  composition + bounds" row, only `validate_direct_request` is reusable. `compose_system_prompt`
  (`assistant_direct.rs:265`) and `find_skill` (`:276`) are private, and `build_upstream_body` consumes a
  `DirectChatRequest` whose messages are `user|assistant` only (`:153-165`) and emits a fixed 4-key body with
  no `tools`/`tool_choice` (`:250-262`). Agent mode needs its own body builder. Say so; it is estimate input.
- **m2 — "no credential-status metadata leaks into model context" is false as written.** §7 returns
  `executable`, documented as *"whether the service can currently execute requests with its configured
  credential and routing state"* (`mcp_service.rs:167-169`). Harmless (a boolean), but the sentence is wrong.
  Also `list_connected_services` actually emits `service_id`, `description`, and `source` too
  (`mcp_service.rs:3748-3757`) — §7's "Result to model" column omits all three.
- **m3 — "Chrono never receives … NyxID JWTs" is configuration-dependent, not structural.** Identity headers
  are injected whenever `identity_propagation_mode != "none"` (`mcp_service.rs:3255-3257`, and the
  equivalent proxy path). The `chrono-llm-public` row's propagation config is DB state and is not in §19's
  three flagged unknowns. Add it as item 6 (the answer is probably "none", since the shipped chat path uses
  the same row — but that is an assumption, not a verification).
- **m4 — the `[DONE]` reuse anchor is the wrong one.** `handlePayload` treats `[DONE]` as *success*
  (`direct-transport.ts:716-721`), and the post-loop branch completes on `sawFinishReason || sawDone`
  (`:682-689`). F1 AC2 requires the opposite for agent mode ("`[DONE]` without `done`/`error` →
  `truncated_stream`"). State that the agent branch needs its own flags rather than "reuse `:680-690`".
- **m5 — the zero-tool execute stage is unspecified.** §10's ordering rule "`execute completed` after the
  last `tool.completed`" is undefined when the model answers without calling anything — the likely outcome
  when a `tool_choice:"none"` plan hop is followed by a chatty model. Say whether `stage execute` still emits
  its started/completed pair.
- **m6 — `done` → `block.completed(final text)` fires unconditionally.** If the model produced no final text,
  no such block exists; `replaceBlock` → `updateBlock` is a silent no-op for unknown ids
  (`stream.ts:64-90, 165-170`), so nothing crashes, but the mapping should be conditional and the turn should
  still complete.
- **m7 — text-block indices unspecified.** `block.started` splices at `Math.min(event.index, blocks.length)`
  (`stream.ts:127-135`); the plan/final text blocks must use indices 1 and 2 or ordering is incidental.
- **m8 — the §10 step literal will not compile.** `RunContentBlock.steps[]` entries require `index`,
  `artifact_id`, and `approval_request_id` (`types/assistant.ts:255-264`); the plan's
  `{label, status, service_slug, meta}` omits all three. The FE gate is `npm run build` (tsc -b), so this is a
  compile error, not a lint nit — the plan itself notes the gate (§13).
- **m9 — the "no loop anywhere" citation is incomplete.** `tool_calls` also appears 32× in
  `backend/src/services/chatgpt_translator.rs`, not only in `llm_gateway_service.rs`. The conclusion still
  holds (both are translators, neither executes), but the evidence should name both.
- **m10 — OQ1 understates its coupling.** `docs/chat/direct-chronollm-endpoints-addendum.md:24-33` proposes
  **deleting** `DIRECT_CHAT_ENGINE_FLAG` outright and re-gating on the wire-log flag, renaming
  `require_direct_chat_enabled` → `require_advanced_chat_enabled`. The plan builds ~10 days of work directly
  on the flag the addendum removes. Name the ordering constraint (agent lands first, or the addendum lands
  first and the agent adopts the new gate), not just "a fork exists".
- **m11 — one §20 checklist item is already answerable.** `execute_admin_proxy` → `execute_proxy_inner` runs
  `check_agent_rate_limit` (`proxy.rs:1424-1440`), which is a **no-op** for session callers: it requires both
  `api_key_id` and `rate_limit_per_second` (`mw/rate_limit.rs:483-495`), and a session `AuthUser` has neither.
  Record it as verified rather than leaving it open for Sol.

---

## Overengineering to remove from a demo plan

- **O1 — collapse the two discovery tools (highest value).** `nyx_list_services` and `nyx_search_tools` are
  both thin synchronous views over a catalog the server already loaded at run start
  (`load_operation_catalog`, `mcp_service.rs:471`), yet **each call costs one of the ≤6 LLM hops** — the
  scarcest resource in the design. Inline the inventory (slug · tool name · one-line description) into the
  agent system prompt and drop both tools. Result: 3 tools instead of 5, ~2 fewer round trips per run, lower
  latency on stage, and strictly better grounding because the model can never "fail to discover" a tool it was
  handed. The `understand` stage's `detail` already advertises the counts, so the visible narrative is
  unchanged. Keep a search tool only if the demo account exceeds a few hundred operations.
- **O2 — delete the 384 KiB body cap + deterministic trim + its test (T3 AC5).** With `MAX_TOOL_CALLS = 8` ×
  16 KiB results, the trim can only fire on a transcript already near its own 256 KiB cap; `FAILED
  (context_overflow)` is already a defined terminal state. Removing it deletes a mechanism, a test, and a
  determinism obligation. While removing it, note the arithmetic is wrong anyway: 256 KiB transcript
  (`MAX_DIRECT_CONTENT_BYTES`, `assistant_direct.rs:11`) + up to 64 KiB bundled skill (`:12`) + 32 KiB Ornn
  skill + two 16 KiB results already exceeds 384 KiB, so as specified the trim engages on ordinary long
  threads and the run dies afterward regardless.
- **O3 — FINALIZING is not a state.** It is "the final hop runs with `tool_choice:"none"` plus a
  budget-exhausted notice". Make it a flag on the last EXECUTING hop: the state, its transition, and the
  "cannot loop" proof obligation all disappear, and M6's accounting ambiguity resolves itself.
- **O4 — the fallback ladder (§4) is correctly already deleted.** Keep it deleted. The one sentence that
  remains ("a P1/P2 regression is a stop-ship, not a silent downgrade") is the right amount of design.

---

## Confirmed invariants (claims that survived attack)

1. **The client grammar cannot smuggle tool or system roles into a later turn.** `DirectMessageRole` is
   `User|Assistant` only, and both `DirectMessage` and `DirectChatRequest` are `#[serde(deny_unknown_fields)]`
   (`backend/src/services/assistant_direct.rs:153-174`), pinned by a test that rejects `role:"system"`, an
   extra `name` field, and an extra top-level field (`:317-328`). §9's INIT claim — "tool exchanges
   structurally cannot re-enter later turns" — holds exactly, and `validate_direct_request` genuinely needs no
   schema change. (The *internal* message history is a different object and must be built fresh; see m1.)
2. **Extension layering does what T2 says.** `register_billing_routes` applies `Extension(policy)` to the
   method router (`routes.rs:18-21`); `.route_layer` wraps outside it (`routes.rs:1467-1472`). The per-route
   policy is therefore inserted last and wins at the handler. M2 objects to the honesty of the arrangement,
   not to its mechanics.
3. **`BillingEgressPermit` cannot be forged outside `route_inventory.rs`.** It is `Copy` with a private
   `_private: ()` field (`route_inventory.rs:43-46`), so "one permit per `execute_tool` call, no new public
   constructors" (T2 AC3) is enforced by the type system, not by discipline.
4. **`block.updated` is a shallow merge preserving `type` and `block_id`** (`stream.ts:151-163`), so the
   plan's rule that every steps patch must carry the full array is both necessary and correctly stated.
5. **`RunCard` reuse is real at the dispatch level.** `"run"` dispatches with no renderer change
   (`chat-thread.tsx:95-96`), and a non-text `block.started`/`block.completed` counts as printed content
   (`use-assistant.ts:670-681`), so spinner and episode logic work untouched. M3/M4 are about which *states*
   the plan feeds that renderer, not about whether the reuse is possible.
6. **No model-controlled string reaches URL construction outside the generic-proxy schema, and that path is
   validated.** `build_generic_proxy_args` calls `validate_requested_proxy_path` before use
   (`mcp_service.rs:3622`), which rejects raw and nested percent-encoded path breakers
   (`proxy_service.rs:396-402`, traversal test `:5579-5582`), and `build_forward_path` validates again
   (`:423`). §5.3's "no free-form URLs" claim stands.
7. **`evaluate_deny_only` is the right function.** For session callers it is outcome-identical to
   `evaluate_and_check(…, bypass_approval_flow = true)`: both return Denied on `ApprovalEffect::Deny` and
   Allowed otherwise (`approval_service.rs:184-199` vs `:201-231`). Only the arguments are wrong (B1).
8. **Long hops are not capped by the HTTP client.** The shared `reqwest::Client` sets `connect_timeout` only,
   with an explicit comment that a global timeout would break SSE (`backend/src/main.rs:598-602`), so
   multi-minute reasoning hops and a 180 s run deadline are viable.
9. **SSE comments are safe on both sides**, which is what makes M5's fix free: the backend parser ignores `:`
   lines (`sse_parser.rs:24-26, 44-48`) and `drainSseBuffer` keeps only `data:` lines (`sse.ts:32-38`).
10. **The §18 evidence anchors are accurate.** ~25 were spot-checked across
    `assistant_direct.rs`, `handlers/assistant_direct.rs`, `routes.rs`, `mcp_service.rs`,
    `route_inventory.rs`, `approval_service.rs`, `unified_key_service.rs`, `keys.rs`, `direct-transport.ts`,
    `stream.ts`, `types/assistant.ts`, `chat-thread.tsx`, `use-assistant.ts`; all landed within a few lines of
    the cited target. The `resp_`-prefixed id claim (§4 note 3) is confirmed by the committed fixture
    `frontend/src/lib/assistant/__fixtures__/chrono-llm-direct-stream.sse:1`.

---

## Verified · assumed · live-only

**Verified against code on this branch:** everything cited above with a `file:line`.

**Assumptions the plan makes that are reasonable but unproven here:** that `execute_admin_proxy` forwards a
synthesized request body opaquely (highly likely — it is the same call the shipped handler makes, and the
handler already rewrites body, headers, and URI at `handlers/assistant_direct.rs:127-149`); that the SSE
usage observer tolerates a `tools`-bearing body (nothing in `should_capture_llm_usage`,
`proxy.rs:3608-3611`, inspects request shape).

**Live-only, not re-executed here:** all of §4 (P1–P6). The parser consequences pinned in §4's notes 1–2 —
per-`index` reassembly, empty-string `name` on continuations, `delta.content:""` on the `tool_calls` finish
chunk — are the load-bearing ones; T3 AC1 correctly makes them fixtures. **Commit the raw P3 capture in
Stage 0 before Stage 1 starts**; without it, the single most brittle piece of the design has no ground truth
in the repo.

**Prod DB state, unverifiable locally:** §19's four items, plus the two this review adds — `ornn-api`
endpoint publication (B2) and `chrono-llm-public` identity-propagation config (m3).

---

## Prioritized fix list

| # | Fix | Where | Cost |
| --- | --- | --- | --- |
| 1 | Resolve the approval target through `approval_target_for_tool` before `evaluate_deny_only`; re-key T4 AC3 on `catalog_service_id` | B1 · §5.3, §7, T4 | S |
| 2 | Make Ornn endpoint publication a Stage-0 go/no-go gate **and** specify the server-built generic-proxy fallback (or budget an `ornn.openapi.json` overlay) | B2 · §8, §14, §19 | M |
| 3 | Never emit `waiting` steps — append steps only when they start | M4 · §10 | S |
| 4 | Settle the run block on **every** terminal path via `toTerminalBlock`; add idle-timeout and network-error fixtures | M3 · F2, F4 | S |
| 5 | Add a `: ping` heartbeat every ~10 s | M5 · T6 | XS |
| 6 | Define `MAX_LLM_CALLS` as total-including-plan-and-finalize; delete the "1 +" sentence; fold FINALIZING into the last hop | M6 + O3 · §9 | XS |
| 7 | One rule for every non-`tool_calls` terminal `finish_reason`; one rule for a stream with none | M7 · §9, T3 | XS |
| 8 | Assign the plan-text transcript filter to F2 with an AC; delete OQ3 | M8 · §12, §15 | S |
| 9 | Add the billing coverage smoke case as a first-class task with its own AC | M1 · T1/T8 | M |
| 10 | Declare the route `Metered(Proxy)`; add one named tool-permit constructor in `route_inventory.rs` | M2 · T2 | S |
| 11 | Thread `ClientConnectionCancellation` into hop requests; tighten T6 AC1 to the in-flight hop | M9 · T6 | XS |
| 12 | Drop `nyx_list_services` + `nyx_search_tools`; inline the inventory into the prompt | O1 · §7, T4 | S (negative) |
| 13 | Delete the 384 KiB trim and its test | O2 · §9, T3 | S (negative) |
| 14 | Restate the estimate in agent-pipeline calendar days against the direct-engine actuals | M10 · §14 | XS |
| 15 | Correct §3's reuse row, §5.2's two absolute claims, §7's result column, and the four FE mapping details (m1–m9) | MINOR · §3, §5.2, §7, §10 | S |

Items 1–8 are pre-implementation edits to the plan and cost roughly a day of doc work in total. Items 9–11
are implementation-shaped and belong in the task ACs. Items 12–13 *reduce* scope. Nothing here changes the
architecture: server-owned loop, in-process `execute_tool`, ephemeral run task, existing `RunCard`.
