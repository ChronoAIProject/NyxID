# NyxID#1408 — Migrate the default chat trunk to typed NyxIdChat

Implementation plan. Author: Opus. **Revision 2, after adversarial review** (see
`1408-typed-chat-trunk.review-codex.md`, verdict REJECT on revision 1). All nine findings
verified as correct and addressed here; §7 maps finding → resolution.

## 0. Decision record

| Decision | Value | Authority |
|---|---|---|
| Legacy `chatc-*` policy | **Read-only immediately.** Listable, readable, deletable. No new turns. Workflow *send* paths deleted in the same PR that flips the trunk. | Calvin, 2026-08-11 |
| Migration owner | Calvin (product), this branch (implementation) | — |
| Cutoff | None needed — read-only takes effect at cutover; no dual-write window | Follows from the policy |
| Base branch | `origin/main` @ `04602a74` | Contains facade v4 (#1404); `consensus-rnd-integration` does **not** |
| Aevatar contract revision | `aevatarAI/aevatar` @ `0a86713671fcf551dc19ad86b1b6aa8ae6cb980b` | Pinned by #1408 |
| NyxID baseline | `ChronoAIProject/NyxID` @ `04602a740bc1c82c85b6794a5cbfecb7f4afc158` | Pinned by #1408 |
| Projection reference | `StudioAssistant/actor-state.js` (522 lines) @ `0a867136` | **Port from it. Do not design the projection from canon prose.** |

`chatc-*` inventory is captured in PR 0 and recorded here before PR 3 lands. If the inventory
shows non-trivial active usage, that is a signal to revisit the policy with Calvin — not a
reason to silently soften it.

## 1. Objective

Every **new** Assistant conversation runs end to end on Aevatar's typed `NyxIdChat` actor
contract. One conversation, one identity, one state owner. Legacy workflow conversations
become read-only history; the workflow send path is deleted rather than preserved as an empty
forwarding layer (FI-007).

### Non-goals

- Aevatar Team/Member/workflow-draft APIs. `memberId`, `workflowId`, `publishedServiceId` do
  not participate in Assistant conversation identity.
- Converting existing `chatc-*` rows to typed identities. Aevatar canon: legacy actors
  "remain read-only until an explicit migration contract creates a controller identity and
  records a real mapping." No such contract exists. Do not invent one.
- Deriving an actor id from a workflow conversation id or an action card. No prefix or
  equality rule may convert between identity families.
- Changing Aevatar's execution model. How Aevatar runs a turn is not NyxID's concern; only
  the wire contract is.

## 2. Verified baseline

All references against `04602a74`. Re-verify before editing — a plan built on a misreading is
worse than no plan.

### Backend — largely done

- `parse_assistant_chat_command` accepts 8 of 9 verbs: `text`, `input.resolve`,
  `action.continue`, `approval.resolve`, `task.stop`, `task.steer`, `step.retry`, `step.skip`
  (`backend/src/services/assistant_service.rs:734-904`).
- **`plan.resolve` is absent.** Zero hits repo-wide.
- Canonical resources mounted (`backend/src/routes.rs:1386-1408`).
- `validate_conversation_id` (`assistant_service.rs:63`) is **syntax-only**. Typed commands do
  not verify the `nyxid-chat-` family, and neither do the resource routes.

### Frontend — the actual work

- `createConversation()` mints `workflow-pending-*` (`aevatar-transport.ts:1656`);
  `sendMessage` selects `protocol = "workflow"` (`:2650`); turns post to `/workflow-chat`
  (`:4126`).
- **The typed control plane is dead code for every new conversation.** `decideApproval`
  hard-throws for `chatc-*` (`:2719`); `resolveInput` throws unless the id matches
  `TYPED_SERVER_CONVERSATION_ID_PATTERN` (`:2854`).
- Client verbs today: `text`, `input.resolve`, `approval.resolve`, `action.continue`,
  `task.stop`. **Missing: `plan.resolve`, `task.steer`, `step.retry`, `step.skip`.**
- **Zero** references to `nyxid.task.snapshot`, `nyxid.task.step.changed`,
  `nyxid.control.changed`, `nyxid.step.control.changed`. `availableActions` never read.
- `/state` is called once — a pre-flight version read (`:6520`) — never as a reload
  projection. Reload is text-only (`:570`).
- `aevatar-transport.ts` is **6,843 lines**; `rg -ic workflow` → **129**. Reproduce rather
  than trusting these numbers.

### Coupling the deletion must account for

Found by review; verified. This is the real PR 3 blast radius:

| Location | What |
|---|---|
| `backend/src/handlers/proxy.rs:8985, 9144, 9158, 9174, 9483, 9526, 9582, 9613` | **Direct calls** to `workflow_chat`, `workflow_chat_ws`, `get_create_recovery`. Deleting the handlers breaks test compilation. |
| `backend/src/handlers/proxy.rs:9092-9141` | The **only** existing proof that `chatc-*` history/delete route to scoped chat history. Must be preserved and narrowed, not deleted. |
| `backend/src/mw/auth.rs:1485` | `/api/v1/assistant/workflow-chat/ws` in the protocol-denial test. |
| `frontend/src/stores/assistant-receipt-store.ts` (295 lines) | Workflow create-receipt / recovery persistence. |
| `frontend/src/lib/assistant/scenario-intercept-transport.ts:35, 94` | Mock scenarios mint `workflow-pending-*`. |
| `frontend/src/lib/assistant/wire-replay.ts` (17 workflow refs) | Diagnostic replay of the workflow protocol. |
| `use-assistant.mock-scenarios.test.tsx:129`, `use-assistant.test.tsx:715-716`, `use-assistant.aevatar.test.tsx:798-899`, `aevatar-transport.test.ts` (workflow-create/recovery blocks) | Test expectations naming the workflow prefix. |

## 3. Contract reference

Authoritative: `aevatar/docs/canon/nyxid-chat-api.md` @ `0a867136`, and the working reference
client in `StudioAssistant/`. **On conflict, canon wins** — re-read rather than trusting this
summary.

### First turn (no `conversationId`)

```json
{ "type": "text", "clientRequestId": "<uuid>", "prompt": "..." }
```

No `workflow`, `sessionId`, `commandId`, or `conversation` object. `sessionId` is deprecated
and ignored upstream. Assistant DTOs reject unknown fields. `Idempotency-Key` is set from
`clientRequestId` (reference: `transport.js:394-423`).

**Attachments.** The reference maps an attachment to typed `inputParts` and strips
console-only `surface`/`attachment` fields. NyxID must make an explicit choice — see PR 3.

### Control commands (all `POST /api/chat` → `202 Accepted`)

| Intent | `type` | Required facts |
|---|---|---|
| Stop | `task.stop` | `conversationId`, `turnId`, `stopRequestId`, `clientRequestId`, `expectedStateVersion` |
| Steer | `task.steer` | `conversationId`, `turnId`, `steeringId`, `clientRequestId`, `instruction`, optional `inputParts`, `expectedStateVersion` |
| Retry step | `step.retry` | `conversationId`, `turnId`, `taskId`, `stepId`, `retryRequestId`, `clientRequestId`, `expectedOperationGeneration`, `expectedStateVersion` |
| Skip step | `step.skip` | `conversationId`, `turnId`, `taskId`, `stepId`, `skipRequestId`, `clientRequestId`, `expectedOperationGeneration`, `expectedStateVersion` |
| Resolve input | `input.resolve` | `conversationId`, actor `requestId`, `clientRequestId`, `answer`, `expectedStateVersion` |
| Resolve plan gate | `plan.resolve` | `conversationId`, `taskId`, `planId`, `requestId`, `clientRequestId`, `planRevision`, `confirmed`, `expectedStateVersion` |

**`202` proves dispatch acceptance only** — not commit, effect, or read-model visibility.
Observe frames or re-read `/state`.

### Committed frames

`nyxid.task.snapshot`, `nyxid.task.step.changed`, `nyxid.control.changed`,
`nyxid.continuation.changed`, `nyxid.step.control.changed`, `nyxid.action.request`,
`nyxid.input.request`, `nyxid.input.changed`, `nyxid.approval.request`,
`nyxid.approval.changed`.

`step.changed.custom.payload` is always `{taskId, planRevision, step, changeKind}` — never a
bare step; nested `step` is identical in shape to a TaskPlan step.

### `/state` snapshot — full ownership

The snapshot is **much larger than TaskPlan**. Revision 1 wrongly scoped the projection to
TaskPlan alone. It owns: `actorId`, `scopeId`, `stateVersion`, `progressSequence`,
`activeTurn`, `latestTurn`, recent turns, ordered task steps + typed sources, operation
key/generation/phase, effect evidence, `availableActions`, `pendingInput`, approval
presentation, latest safe input/approval resolution facts, typed `pendingActions`, bounded
`recentActions`, control fences, continuation admission, actor-authored attention
(`attentionKind`, `attentionSince`), `activeStepSummary`, `latestStepControlResult`, and
bounded `recentStepControlResults`.

Envelope statuses: `current` | `not_modified` | `reload_required` | `not_found`. Monotonic
overwrite: newer replaces older; byte-equal same-version idempotent; same-version conflict
fails; older never overwrites newer.

### Terminals

`succeeded` → `RUN_FINISHED`/`completed`; `blocked`/`stopped` → `RUN_FINISHED`/`blocked`;
`failed` → `RUN_ERROR`; inconsistent committed states → fail closed with
`NYXID_CHAT_TERMINAL_STATE_CONFLICT`.

## 4. PR sequence

**Ordering rule (revision 2):** the trunk flip lands only after the projection is *wired and
exercised*, not merely present. Revision 1's "PR 2 is inert, PR 3 flips" split was incoherent:
it would have shipped command builders with no source for the facts they must send.

---

### PR 0 — Live smoke + synthetic fixtures + inventory *(spike; gates the rest)*

**Deliverables:**

1. Authenticated end-to-end run against the pinned Aevatar deployment through the NyxID
   facade, using a **dedicated disposable test account** and **deterministic non-sensitive
   prompts only**. Exercise: first turn, follow-up, approval, input question, tool call,
   reload mid-turn, and a step control.
2. **Minimized structural fixtures**, committed under
   `frontend/src/lib/assistant/__fixtures__/typed/`. **Not raw captures.** Replace every id,
   prompt, response text, timestamp, and URL with synthetic values; delete unknown fields
   rather than retaining them. Add a fixture-sanitizer test that fails on credential-shaped
   strings, real user identifiers, and non-allowlisted keys. Raw evidence stays local and
   ephemeral — NyxID's own wire-log panel warns raw captures carry sensitive payloads
   verbatim (`assistant-wire-log-panel.tsx:534-540`).
3. `chatc-*` production inventory: count, newest `last_message_at`, distinct users → §0.
4. **Two written answers:**
   - **Q1 — plan gate `auto` or `confirm`?** If `confirm`, the plan-gate slice
     (decode + render + submit + reload) is **PR 2 scope**, and PR 3 cannot flip until it
     lands. A command builder alone does not unblock a pending gate — the required
     `taskId`/`planId`/`requestId`/`planRevision`/`expectedStateVersion` come from the
     projection, not from `RUN_STARTED`.
   - **Q2 — is the typed path a capability regression?** Aevatar admits one tool call per LLM
     operation (`AllowMultipleToolCalls = false`); Studio has no such limit. If normal
     multi-step requests degrade, **Calvin decides whether to flip** — we do not absorb it.

**Acceptance:** sanitized fixtures committed; Q1/Q2 answered in writing; inventory recorded.
No production code changes.

---

### PR 1 — Backend: complete and harden the typed surface

Independent; can land in parallel with PR 0.

- Add `plan.resolve` — `PlanResolveCommand { conversation_id, task_id, plan_id, request_id,
  client_request_id, plan_revision, confirmed, expected_state_version }` — to the enum,
  parser, and body builder. `confirmed` is a **required explicit bool**; absent → 400,
  mirroring `approval.resolve`. Response kind `Json`.
- **One strict typed-ID validator, applied everywhere a typed id selects an upstream
  canonical path** — all nine `/chat` parse arms *and* `get_history`, `delete_conversation`,
  `get_state` (`handlers/assistant.rs:836-843`, `:866-929`). Revision 1 covered only `/chat`,
  which left the resource routes failing open: a malformed `nyxid-chat-*` id reached upstream
  instead of being rejected locally. Keep the separate `chatc-*` validator for legacy history.

**Tests:** parse + body shape per verb (nine); `plan.resolve` without `confirmed` → 400;
`chatc-*` id on each typed verb → 400; malformed typed id on GET/DELETE/state → 400 with no
upstream call; unknown `type` → 400 and never falls through to workflow.

**Not in scope:** deleting workflow routes (PR 3), so reverting PR 3 alone restores a working
system.

---

### PR 2 — Frontend: the actor-state projection, wired to the typed protocol

The largest PR. **Port from `StudioAssistant/actor-state.js`** — it already solves this. Read
it before designing anything.

**Not inert.** Typed conversations already exist (`nyxid-chat-*` rows from before #1301), so
this PR wires the projection into the live typed path and is exercised on real data before the
trunk flips. That is what makes PR 3 a rewire rather than a leap.

**New files:**

- `task-plan.ts` — Zod schemas + **one** decoder each for `NyxIdChatTaskPlan`, step, and the
  `step.changed` envelope.
  - Assert `schemaVersion`. Unknown *additive* fields tolerated; unknown *step kind*,
    *source kind*, or *action verb* fail closed.
  - Closed source union: `llm`, `tool`, `browserAction`, `postcondition`, `input`, `approval`,
    reserved `web`.
  - **Browser-safe integer guard**: `operationGeneration` and `latestProgressSequence` outside
    `Number.isSafeInteger` **fail closed** (canon `:219-227` — an explicit wire rule; silent
    precision loss would corrupt a retry fence).
  - Never rename fields; never infer identities. **No untyped `Record` after decode.**
- `actor-state.ts` — the full projection, not just TaskPlan. Covers everything in §3's
  `/state` inventory: task plan, pending input, pending approval, pending/recent actions,
  control fences, continuation admission, step-control results, attention, `activeTurn`/
  `latestTurn`, `progressSequence`.
  - **Envelope protocol checks**, per reference `actor-state.js:116-211`: `scopeId`, actor
    identity, envelope-vs-snapshot version equality, `progressSequence` safety, and each of
    `current`/`not_modified`/`reload_required`/`not_found`. These are protocol checks, not
    card data — a misleading reload must not be treated as trusted state.
  - Reducers: `applySnapshot`, `applyStepChanged`, `applyControlChanged`,
    `applyStepControlChanged`, `applyContinuationChanged`, `applyInputRequest/Changed`,
    `applyApprovalRequest/Changed`, `applyActionRequest`.
  - Monotonic `stateVersion`; lower dropped, same-version conflict fails closed, `actorId`
    mismatch fails closed.

**Two thin wire adapters, one model** (the shape the `proportional-containment` verdict asked
for — not a shared abstraction over the two representations): SSE `custom` frame → decode →
reduce; `/state` snapshot → same decode → reduce.

**`RUN_STARTED` consistency rule** — specify and test before aliasing: outer `actorId`/`turnId`
required; nested `runStarted.threadId`/`runId` must match when present; exactly one adoption
per stream; mismatch → terminal protocol error.

**State the retirement explicitly:** which existing card reducer and which part of
`preserveLocalStructuredMessages` the projection replaces.

**If PR 0 answered Q1 = `confirm`, the plan gate ships here** — decode, render, submit,
reload — not in PR 4.

**Tests:** decode every PR 0 fixture; **convergence tests** (live SSE replay vs `/state`
snapshot deep-equal) for pending input, approval, action, plan gate, action continuation,
retry/skip result, and each `/state` status; fail-closed test per guard; `RUN_STARTED`
negative matrix.

---

### PR 3 — Flip the trunk + legacy read-only

**Routing:** `createConversation()` mints a local `draft-` key, never sent upstream, never
matched by any legacy guard (do not reuse `nyxid-pending-`, reserved for the stale-URL
not-found guard). First turn posts `{type:"text", prompt, clientRequestId}` to
`/api/v1/assistant/chat`. Adopt authoritative identity from `RUN_STARTED` under the PR 2 rule;
alias the draft in place. Follow-ups send the adopted `conversationId` + new `clientRequestId`.

**Attachments:** make the explicit call — either reject at the UI boundary with a visible
limitation, or add an audited typed `inputParts` schema + forwarder with tests. Do not leave
it undefined.

**Legacy read-only:** `sendMessage` on `chatc-*` throws a typed read-only error with no
network call; composer renders a disabled archived state. History read/list/delete keep
working — `get_history` and `delete_conversation` dispatch `chatc-*` to the scoped history
resource independently of the create-recovery endpoint (verified: `handlers/assistant.rs:715-817`,
`:819-897`, `:931-954`). **Do not touch those paths.**

**Deletions (FI-007), with their test consequences:**

- Backend routes/handlers: `/workflow-chat`, `/workflow-chat/ws`,
  `/conversations/create-recovery/{commandId}`; `workflow_chat`, `workflow_chat_ws`,
  `get_create_recovery`; `workflow_chat_body`, `WorkflowChatTurnRequest`,
  `workflow_chat_ws_path`, `WORKFLOW_CHAT_WORKFLOW`, `history_create_recovery_path`.
- **Required companion task — the plan does not compile without it:** rewrite the eight call
  sites in `backend/src/handlers/proxy.rs` and the route entry in `backend/src/mw/auth.rs:1485`.
  **Preserve and narrow** `proxy.rs:9092-9141` — it is the only regression proof that `chatc-*`
  history and delete still route correctly. Do not delete the block wholesale.
- Frontend: `PENDING_WORKFLOW_CONVERSATION_PREFIX`, `streamWorkflowTurn`, `workflowTurnBody`,
  `recoverWorkflowCreate`, `startCreateRecoveryInBackground`, `workflowCreateNeedsRecovery`,
  `settleRecoveredWorkflowCreate`, `reconcileWorkflowHistory`, and the `protocol` branch.
- `stores/assistant-receipt-store.ts` — delete only after proving no generic delete behavior
  depends on it.
- `scenario-intercept-transport.ts:35, 94` — convert mock scenarios to `draft-*`, and update
  `use-assistant.mock-scenarios.test.tsx:129`. Otherwise "a fresh chat can never select
  `workflow-pending-*`" is false in scenario runs.
- `wire-replay.ts` — **explicit product decision**: keep as a clearly-labelled historical
  diagnostic with its own fixtures, or remove the workflow protocol and update the inspector.
  Do not leave it ambiguous.
- **Keep:** `WORKFLOW_CONVERSATION_PREFIX`, `conversation_resource_family` — still needed for
  legacy reads and deletes.

**Tests — absence, not just non-use:**

- **Router integration:** `/workflow-chat`, `/workflow-chat/ws`, and facade create-recovery
  return 404 after this PR. Assert the server-side echoed path and body, not only a mocked
  frontend URL.
- A typed first turn and follow-up reach **only** upstream `/api/chat` with the discriminated
  body containing none of `workflow`, `sessionId`, `commandId`, `conversation`.
- **No fallback:** a typed upstream 4xx or network error cannot retry through workflow.
- An unknown/malformed typed discriminator neither forwards nor reaches workflow (canon `:687-693`).
- Fresh chat can never select `workflow-pending-*`, `chatc-*`, or `/workflow-chat`.
- Mixed-history: typed and legacy ids with deliberately similar shapes; no prefix or equality
  rule converts between families.
- `sendMessage` on a legacy id → read-only error, zero network calls.

---

### PR 4 — Remaining controls and task UI

- Render task/step progress from the PR 2 projection; new blocks under
  `components/assistant/blocks/`.
- **Controls render only from `step.availableActions`.** Canon: *"The actor computes `retry`,
  `skip`, and `stop` availability… UI code must not derive these actions independently."*
  Unknown verb → not rendered.
- Implement `task.steer`, `step.retry`, `step.skip` (and `plan.resolve` if Q1 left it here).
- **Steering affordance:** an active turn already fails closed locally
  (`AssistantTurnActiveError`) and upstream (`ACTIVE_TURN_REQUIRES_STEERING`). The composer
  offers steer instead of only blocking.
- Retire the text-only `preserveLocalStructuredMessages` grace once the projection owns card
  state.

**Tests:** exact body shape and fences per control; stale-version rejection; a control absent
from `availableActions` is not rendered; reload mid-turn reproduces the live projection.

---

### PR 5 — Documentation

Rewrite `docs/chat/01-architecture.md` (its "Two mutually exclusive engines" section currently
documents the dual-engine model as normal); update `02`, `03`, `04`, `07`. Pin both revisions
(`04602a74`, `0a867136`). Fold this decision record into permanent docs; mark this plan
superseded.

## 5. Risks

| Risk | Mitigation |
|---|---|
| Typed path is a capability regression (one tool call per operation) | PR 0 Q2 before any flip. Escalate to Calvin; do not absorb. |
| Plan gate arrives `pending` | PR 0 Q1. If pending, the full gate slice is PR 2 scope and PR 3 waits. |
| Users lose in-flight conversations | Accepted by the read-only decision; PR 0 inventory sizes it. |
| PR 3 is a large deletion in a 6,843-line file plus 8 backend test call sites | PR 2 lands and exercises the replacement first; test migration is an explicit PR 3 task, not a surprise. |
| `/state` and SSE drift into two models | Convergence fixtures in PR 2 across all pending-fact kinds, re-asserted in PR 4. |
| Aevatar contract moves | Both revisions pinned in tests; drift fails a test rather than production. |
| Fixture leak of production data | Synthetic-only fixtures + sanitizer test + disposable account (PR 0). |

## 6. Verification gates

Per PR: `cargo test`, `npm run build` (CI gate is `build` — `tsc -b` with
`noUncheckedIndexedAccess`; `tsc --noEmit` passes anyway and misses it), `npm run test`,
`npm run lint`.

Before PR 3 merges: authenticated end-to-end smoke against the pinned Aevatar deployment,
re-run rather than reusing PR 0's transcript.

Frontend dep/lockfile changes trip the CLI Wizard Bundle Freshness check — none expected, but
rebuild with `npm --prefix frontend run build:wizard` if one appears.

## 7. Review findings → resolution

| # | Finding | Resolution |
|---|---|---|
| 1 | PR 3 deletion breaks compilation (8 `proxy.rs` call sites, `auth.rs:1485`); would erase the only `chatc-*` routing proof | PR 3 gains an explicit test-migration task; `proxy.rs:9092-9141` preserved and narrowed |
| 2 | `plan.resolve` conditional strands pending gates — a builder without its fact pipeline | Ordering rule rewritten; full gate slice moves to PR 2 when Q1 = `confirm`; PR 3 waits |
| 3 | PR 2 omitted most of the canonical `/state` model | §3 now inventories the full snapshot; PR 2 scope covers input/approval/action/control/continuation + envelope checks; ported from `actor-state.js` |
| 4 | Typed-ID hardening failed open on resource routes and `RUN_STARTED` | One strict validator across `/chat` **and** resource routes; explicit `RUN_STARTED` consistency rule + negative matrix |
| 5 | Raw production fixtures unsafe | Synthetic minimized fixtures, disposable account, sanitizer test; raw evidence stays local |
| 6 | Deletion inventory missed receipt store, scenario transport, wire-replay, test suites | §2 coupling table + per-item PR 3 tasks, including an explicit wire-replay product decision |
| 7 | Missed reference-client rules (attachments/`inputParts`, `Idempotency-Key`, envelope checks) | Attachment decision required in PR 3; envelope checks lifted into PR 2; no untyped `Record` after decode |
| 8 | Tests proved non-use, not absence | PR 3 gains router 404 tests, no-fallback test, unknown-discriminator test, server-side echo assertions |
| 9 | Line count stale (6,117 → 6,843) | Corrected; reproduce with `wc -l` / `rg -ic workflow` rather than trusting the number |
