# NyxID#1408 — Migrate the default chat trunk to typed NyxIdChat

Implementation plan. Author: Opus. Status: **draft, pending adversarial review**.

## 0. Decision record

| Decision | Value | Authority |
|---|---|---|
| Legacy `chatc-*` policy | **Read-only immediately.** Listable, readable, deletable. No new turns. Workflow *send* paths deleted in the same PR that flips the trunk. | Calvin, 2026-08-11 |
| Migration owner | Calvin (product), this branch (implementation) | — |
| Cutoff | None needed — read-only takes effect at cutover; there is no dual-write window | Follows from the policy above |
| Base branch | `origin/main` @ `04602a74` | Contains facade v4 (#1404); `consensus-rnd-integration` does **not** |
| Aevatar contract revision | `aevatarAI/aevatar` @ `0a86713671fcf551dc19ad86b1b6aa8ae6cb980b` | Pinned by #1408 |
| NyxID baseline | `ChronoAIProject/NyxID` @ `04602a740bc1c82c85b6794a5cbfecb7f4afc158` | Pinned by #1408 |

`chatc-*` inventory is captured in PR 0 and recorded here before PR 3 lands. If the inventory
shows non-trivial active usage, that is a signal to revisit the policy with Calvin — not a
reason to silently soften it.

## 1. Objective

Every **new** Assistant conversation runs end to end on Aevatar's typed `NyxIdChat` actor
contract. One conversation, one identity, one state owner. Legacy workflow conversations
become read-only history; the workflow send path is deleted rather than preserved as an
empty forwarding layer (FI-007).

### Non-goals

- Aevatar Team/Member/workflow-draft APIs. `memberId`, `workflowId`, `publishedServiceId`
  do not participate in Assistant conversation identity.
- Converting existing `chatc-*` rows to typed identities. Aevatar canon: legacy actors
  "remain read-only until an explicit migration contract creates a controller identity and
  records a real mapping." No such contract exists. Do not invent one.
- Deriving an actor id from a workflow conversation id or an action card. No prefix or
  equality rule may convert between identity families.
- Changing Aevatar's execution model. How Aevatar runs a turn is not NyxID's concern; only
  the wire contract is.

## 2. Verified baseline

Read before writing code. All line references are against `04602a74`.

### Backend — largely done

Facade v4 (#1404) shipped the typed command surface:

- `parse_assistant_chat_command` accepts 8 of 9 verbs: `text`, `input.resolve`,
  `action.continue`, `approval.resolve`, `task.stop`, `task.steer`, `step.retry`,
  `step.skip` (`backend/src/services/assistant_service.rs:734-904`).
- **`plan.resolve` is absent.** Zero hits repo-wide.
- Canonical resources are mounted (`backend/src/routes.rs:1386-1408`): `GET /conversations`,
  `GET|DELETE /conversations/{id}`, `GET /conversations/{id}/state`, `POST /chat`.
- `validate_conversation_id` (`assistant_service.rs:63`) is **syntax-only** — it accepts any
  alphanumeric/`-`/`_` string up to 128 chars. Typed commands do not verify the
  `nyxid-chat-` family.

### Frontend — the actual work

- `createConversation()` mints `workflow-pending-*` (`aevatar-transport.ts:1656`);
  `sendMessage` selects `protocol = "workflow"` from that prefix (`:2650`); turns post to
  `/workflow-chat` (`:4126`).
- **Consequence: the typed control plane is dead code for every new conversation.**
  `decideApproval` hard-throws for `chatc-*` (`:2719`); `resolveInput` throws unless the id
  matches `TYPED_SERVER_CONVERSATION_ID_PATTERN` (`:2854`).
- Verbs with client code today: `text`, `input.resolve`, `approval.resolve`,
  `action.continue`, `task.stop`. **Missing: `plan.resolve`, `task.steer`, `step.retry`,
  `step.skip`.**
- **Zero** references to `nyxid.task.snapshot`, `nyxid.task.step.changed`,
  `nyxid.control.changed`, `nyxid.step.control.changed`. `availableActions` never read.
- `/state` is called in exactly one place — a pre-flight version read before a decision
  (`:6520`) — never as a reload projection. Reload is text-only:
  `"History v4 is text-only until /state card rehydration ships"` (`:570`).
- ~129 `workflow` references across a 6,117-line transport.

## 3. Contract reference

Authoritative source: `aevatar/docs/canon/nyxid-chat-api.md` @ `0a867136`. Reproduced here
only where implementation needs exact shapes. **On any conflict, the canon wins** — re-read
it rather than trusting this summary.

### First turn (no `conversationId`)

```json
{ "type": "text", "clientRequestId": "<uuid>", "prompt": "..." }
```

No `workflow`, no `sessionId`, no `commandId`, no `conversation` object. `sessionId` is
explicitly deprecated and ignored upstream. Assistant DTOs reject unknown fields.

Authoritative `conversationId`/`actorId` and `turnId` arrive in the stream's `RUN_STARTED`
transport context. Follow-ups send that exact `conversationId` plus a **new** stable
`clientRequestId`.

### Control commands (all `POST /api/chat`, all → `202 Accepted`)

| Intent | `type` | Required facts |
|---|---|---|
| Stop | `task.stop` | `conversationId`, `turnId`, `stopRequestId`, `clientRequestId`, `expectedStateVersion` |
| Steer | `task.steer` | `conversationId`, `turnId`, `steeringId`, `clientRequestId`, `instruction`, optional `inputParts`, `expectedStateVersion` |
| Retry step | `step.retry` | `conversationId`, `turnId`, `taskId`, `stepId`, `retryRequestId`, `clientRequestId`, `expectedOperationGeneration`, `expectedStateVersion` |
| Skip step | `step.skip` | `conversationId`, `turnId`, `taskId`, `stepId`, `skipRequestId`, `clientRequestId`, `expectedOperationGeneration`, `expectedStateVersion` |
| Resolve input | `input.resolve` | `conversationId`, actor `requestId`, `clientRequestId`, `answer`, `expectedStateVersion` |
| Resolve plan gate | `plan.resolve` | `conversationId`, `taskId`, `planId`, `requestId`, `clientRequestId`, `planRevision`, `confirmed`, `expectedStateVersion` |

**`202` proves dispatch acceptance only** — not commit, not effect, not read-model
visibility. Observe `nyxid.task.snapshot` / `nyxid.task.step.changed` or re-read `/state`.

### Committed frames

`nyxid.task.snapshot`, `nyxid.task.step.changed`, `nyxid.control.changed`,
`nyxid.continuation.changed`, `nyxid.step.control.changed`, `nyxid.action.request`,
`nyxid.input.request`, `nyxid.input.changed`, `nyxid.approval.request`,
`nyxid.approval.changed`.

`nyxid.task.snapshot.custom.payload` is the complete public `NyxIdChatTaskPlan`.
`nyxid.task.step.changed.custom.payload` is always the complete envelope
`{taskId, planRevision, step, changeKind}` — never a bare step; the nested `step` uses the
identical shape to a TaskPlan step.

**Canon, verbatim:** *"Live TaskPlan payloads and current-state `snapshot.activeTask` are
the same contract, not two browser models. Clients must use one TaskPlan decoder and one
step decoder for initial SSE, reconnect/reload, and step-change reduction. They must not
rename fields, infer identities, or maintain a second lifecycle model."*

### Conditional state read

`GET /api/v1/assistant/conversations/{id}/state?afterStateVersion={v}&turnId={t}` →
`current` | `not_modified` | `reload_required` | `not_found`.

Monotonic overwrite: newer replaces older; byte-equal same-version is idempotent;
same-version conflict fails; older never overwrites newer.

### Terminals

`succeeded` → `RUN_FINISHED` status `completed`; `blocked`/`stopped` → `RUN_FINISHED` status
`blocked`; `failed` → `RUN_ERROR`; inconsistent committed states → fail closed with
`NYXID_CHAT_TERMINAL_STATE_CONFLICT`.

## 4. PR sequence

Six PRs. Each lands green on its own. PR 0 is a spike whose output gates the rest.

---

### PR 0 — Live smoke + golden fixtures *(spike; gates everything after)*

**Why first.** Two facts cannot be established from the NyxID repo, and both change the
plan if they come back wrong.

**Deliverables:**

1. Authenticated end-to-end run against the pinned Aevatar deployment through the NyxID
   facade — a real typed conversation: first turn, follow-up, an approval, an input
   question, a tool call, a reload mid-turn.
2. **Captured SSE streams and `/state` responses committed as fixtures** under
   `frontend/src/lib/assistant/__fixtures__/typed/`. These become the golden test data for
   PR 2 and the convergence proof for PR 4. Redact tokens; the canon's secret boundary
   applies to fixtures too.
3. `chatc-*` production inventory: count, newest `last_message_at`, distinct users. Recorded
   in §0 above.
4. **Two explicit answers**, written into this document:
   - **Q1 — Is the plan gate `auto` or `confirm` in our deployment?** If gates arrive
     `pending`, `plan.resolve` is a **PR 3 blocker**, not PR 4 scope: a conversation that
     hits a pending gate with no client resolver hangs forever. Move it forward.
   - **Q2 — Does the typed path do the job?** Aevatar admits at most one tool call per LLM
     operation (`AllowMultipleToolCalls = false`). Studio has no such limit. Does a normal
     multi-step request still complete well? This is Aevatar's design, not ours to fix —
     but if it is a visible regression, Calvin decides whether to flip, not us.

**Acceptance:** fixtures committed; Q1 and Q2 answered in writing; inventory recorded.

**No production code changes in this PR.**

---

### PR 1 — Backend: complete the typed command surface

Independent of everything else. No user-visible change. Can land in parallel with PR 0.

**Scope:**

- Add `plan.resolve` — `PlanResolveCommand { conversation_id, task_id, plan_id, request_id,
  client_request_id, plan_revision, confirmed, expected_state_version }` — to
  `AssistantChatCommand`, `parse_assistant_chat_command`, and
  `prepare_assistant_chat_command`. `confirmed` is a **required explicit bool**; absent must
  be a 400, mirroring the existing `approval.resolve` / `APPROVAL_DECISION_REQUIRED`
  treatment. Response kind `Json` (202 receipt), matching the other controls.
- **Typed-family validation.** Every command that carries a `conversationId` must verify the
  `nyxid-chat-` family, not just syntax. Add
  `validate_typed_conversation_id` alongside the existing `validate_conversation_id` and
  wire it into all nine parse arms. A `chatc-*` id posted to `/chat` must 400 locally, not
  produce an upstream `ACTOR_NOT_FOUND`.

**Tests:** one parse + one body-shape test per verb (nine total); `plan.resolve` without
`confirmed` → 400; `chatc-*` id on each typed verb → 400; unknown `type` → 400 and never
falls through to workflow.

**Explicitly not in scope:** deleting workflow routes. That happens in PR 3, so a revert of
PR 3 alone restores a working system.

---

### PR 2 — Frontend: TaskPlan decoder + projection model *(pure addition, no routing change)*

The largest new-code PR, and deliberately inert: nothing calls it until PR 3/PR 4. This is
what makes the trunk flip reviewable instead of a 3,000-line big bang.

**New files:**

- `frontend/src/lib/assistant/task-plan.ts` — Zod schemas + decoders for `NyxIdChatTaskPlan`,
  step, and the `step.changed` envelope. **One** decoder each, per canon.
  - Assert `schemaVersion`; unknown *additive* fields tolerated, unknown *step kind* /
    *source kind* / *action verb* fail closed.
  - Closed source union: `llm`, `tool`, `browserAction`, `postcondition`, `input`,
    `approval`, reserved `web`.
  - **Browser-safe integer guard**: `operationGeneration` and `latestProgressSequence`
    outside `Number.isSafeInteger` **fail closed** — canon makes this an explicit wire rule,
    and silent precision loss here would corrupt a retry fence.
  - Never rename fields; never infer identities.
- `frontend/src/lib/assistant/task-projection.ts` — pure reducer over decoded facts:
  `applySnapshot`, `applyStepChanged`, `applyControlChanged`, `applyStepControlChanged`,
  `applyContinuationChanged`. Monotonic `stateVersion`; a lower version is dropped, a
  same-version conflict fails closed, an `actorId` mismatch fails closed.

**Two wire adapters, one model** — this is the shape the consensus `proportional-containment`
verdict asked for. Do **not** build a shared abstraction over the two representations; build
two thin adapters that both produce the canonical decoded type:

- SSE adapter: `custom` frame → decode → reduce.
- State adapter: `/state` `snapshot.activeTask` → same decode → reduce.

**Tests:** decode every fixture from PR 0; **convergence test** — replay a captured SSE
stream and the `/state` snapshot for the same conversation and assert the resulting
projection is deep-equal. Fail-closed tests for each guard above.

---

### PR 3 — Flip the trunk + legacy read-only *(the core acceptance criteria)*

**Routing:**

- `createConversation()` no longer mints `workflow-pending-*`. Use a local draft key with a
  **new** prefix (`draft-`) that is never sent upstream and never matched by any legacy
  guard. Do not reuse `nyxid-pending-` (reserved for the stale-URL not-found guard) and do
  not reuse any workflow prefix.
- First turn posts `{type:"text", prompt, clientRequestId}` to `/api/v1/assistant/chat`.
- Adopt authoritative `conversationId`/`turnId` from `RUN_STARTED`; alias the draft key in
  place; a stream may never change identity after adoption.
- Follow-ups send the adopted `conversationId` + a new `clientRequestId`.

**Legacy read-only:**

- `sendMessage` against a `chatc-*` id throws a typed read-only error; the composer renders
  a disabled state explaining the conversation is archived.
- History read, list, and delete for `chatc-*` keep working — `get_history` is already
  family-aware and reads the workflow chat-history resource. **Do not touch that path.**

**Deletions (FI-007 — no empty forwarding layers):**

- Backend: `/workflow-chat`, `/workflow-chat/ws`, `/conversations/create-recovery/{commandId}`
  routes; `workflow_chat`, `workflow_chat_ws`, `get_create_recovery` handlers;
  `workflow_chat_body`, `WorkflowChatTurnRequest`, `workflow_chat_ws_path`,
  `WORKFLOW_CHAT_WORKFLOW`, `history_create_recovery_path`.
- Frontend: `PENDING_WORKFLOW_CONVERSATION_PREFIX`, `streamWorkflowTurn`,
  `workflowTurnBody`, `recoverWorkflowCreate`, `startCreateRecoveryInBackground`,
  `workflowCreateNeedsRecovery`, `settleRecoveredWorkflowCreate`,
  `reconcileWorkflowHistory`, the create-receipt machinery, and the `protocol` branch itself.
- **Keep:** `WORKFLOW_CONVERSATION_PREFIX` and `conversation_resource_family` — still needed
  to route legacy reads and deletes.

**Regression tests (these are the acceptance criteria, written as tests):**

- A fresh chat can never select `workflow-pending-*`, `chatc-*`, or `/workflow-chat`.
- First upstream body contains none of `workflow`, `sessionId`, `commandId`, `conversation`.
- New-chat, continuation, transcript, delete, and reload all use one identity.
- Mixed-history test: a typed and a legacy conversation with deliberately similar ids; assert
  no prefix or equality rule converts between families.
- `sendMessage` on a legacy id → read-only error, no network call.

**Conditional:** if PR 0 answered Q1 as `confirm/pending`, `plan.resolve` (client side) moves
into this PR.

---

### PR 4 — Controls driven by actor state

Wire the projection from PR 2 into the UI.

- Render task/step progress from the projection. New components under
  `frontend/src/components/assistant/blocks/` following the existing card pattern.
- **Controls render only from `step.availableActions`.** Canon, verbatim: *"The actor
  computes `retry`, `skip`, and `stop` availability… UI code must not derive these actions
  independently."* No client-side inference of task status, effect truth, or retry/skip/stop
  availability. Unknown verb in `availableActions` → not rendered.
- Implement the missing client commands: `task.steer`, `step.retry`, `step.skip`, and
  `plan.resolve` (if not already pulled into PR 3).
- **Steering affordance.** An active turn already rejects a second text turn locally
  (`AssistantTurnActiveError`) and upstream (`ACTIVE_TURN_REQUIRES_STEERING`). The composer
  should offer "steer" instead of just blocking — this is the user-visible half of the
  contract.
- `/state` rehydration on reload: conditional read with `afterStateVersion` + `turnId`,
  handling `not_modified` / `reload_required` / `not_found`. Retire the text-only
  `preserveLocalStructuredMessages` grace once the projection owns card state.

**Tests:** each control's exact body shape and fences; stale-version rejection; a control
absent from `availableActions` is not rendered; reload mid-turn reproduces the live
projection (reuse the PR 2 convergence fixture).

---

### PR 5 — Documentation

- Rewrite `docs/chat/01-architecture.md` — the "Two mutually exclusive engines" section
  currently documents the dual-engine model as the normal browser contract. Typed becomes the
  normal path; the workflow adapter is labeled legacy read-only compatibility.
- Update `02-wire-contract.md`, `03-stream-protocol.md`, `04-action-cards.md`,
  `07-testing-and-gaps.md`.
- Pin both verified revisions (`04602a74`, `0a867136`) in docs and tests.
- Fold this plan's decision record into the permanent docs; delete the plan file or mark it
  superseded.

## 5. Risks

| Risk | Mitigation |
|---|---|
| Typed path is a capability regression vs Studio (one tool call per operation) | PR 0 Q2 answers it before any flip. Escalate to Calvin, don't absorb silently. |
| Plan gate arrives `pending` and hangs conversations | PR 0 Q1. If pending, `plan.resolve` moves into PR 3. |
| Users lose in-flight conversations at cutover | Accepted by the read-only decision. PR 0 inventory sizes the blast radius; revisit with Calvin if it is large. |
| PR 3 is a large deletion in a 6,117-line file | PR 2 lands the replacement first, so PR 3 is delete + rewire, not build + delete. |
| `/state` and SSE drift into two models | Convergence fixture test in PR 2, re-asserted in PR 4. |
| Aevatar contract moves under us | Both revisions pinned in tests; drift shows as a test failure, not a production surprise. |

## 6. Verification gates

Per PR: `cargo test`, `npm run build` (the CI gate is `build`, not `tsc --noEmit` — the
former runs `tsc -b` with `noUncheckedIndexedAccess`), `npm run test`, `npm run lint`.

Before PR 3 merges: authenticated end-to-end smoke against the pinned Aevatar deployment,
re-run rather than reusing PR 0's transcript.

Frontend dep/lockfile changes trip the CLI Wizard Bundle Freshness check — none expected
here, but rebuild with `npm --prefix frontend run build:wizard` if one appears.
