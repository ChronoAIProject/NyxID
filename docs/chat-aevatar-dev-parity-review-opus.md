# Adversarial implementation review — chat parity W1–W8

Reviewer: Opus (implementation adversary) · Date: 2026-08-01
Range under review: `6f3136f5..HEAD` (`c56a8efc`, `243e7518`, `047ed4f0`, `973c1e93`)
Binding spec: `docs/chat-aevatar-dev-parity-spec.md`
Upstream reference re-derived from `~/Desktop/aelf-frontend-work/aevatar` @ `origin/dev bbd906eb5` (read-only, `git show` only).

**Final verdict: REWORK-REQUIRED** (4 required fixes; details at the end).

The work is substantially correct and the test suite is far better than the one it
replaces. The failures below are concentrated in W4 (create recovery trigger
conditions and identity comparison) plus one guard test that asserts nothing.

---

## Per-work-item verdicts

### W1 — Family-aware resources + full pagination (backend)

**W1.1–W1.5: PASS-WITH-NITS. W1.6: FAIL.**

Verified correct:

- Path builders `history_conversation_path` / `history_create_recovery_path`
  reuse `validate_conversation_id` / `validate_client_token`
  (`backend/src/services/assistant_service.rs:176-190`), and both validators
  reject any separator that could escape the path segment
  (`assistant_service.rs:66-77`, `1086-1094`).
- Family routing in `get_history` / `delete_conversation` / `get_state`
  (`backend/src/handlers/assistant.rs:513-521`, `543-552`, `588-594`).
  `get_state` for `chatc-…` returns `AppError::NotFound` **before** the echo
  collector, so no upstream call is made — pinned by
  `proxy.rs:8265-8281` (`calls_before_workflow_state` equality assertion).
- The FE genuinely never calls `/state`: `grep "/state"` over
  `aevatar-transport.ts` + `use-assistant.ts` returns only a doc comment.
- Cursor drain (`handlers/assistant.rs:418-491`): loop guard on repeated
  cursors → `AppError::Internal`, 40-page cap documented in code
  (`assistant.rs:42-45`), per-family filter + first-wins dedupe + newest-first
  sort. `MAX_HISTORY_INDEX_PAGES` is honoured.
- Workflow DELETE normalization to 204 with `Content-Length`/`Content-Type`
  stripped (`assistant.rs:562-572`); typed DELETE keeps 202 + JSON
  (`proxy.rs:8231-8248`).
- New route registered at `routes.rs:1306-1309`; the arity differs from
  `/conversations/{id}` and the static `create-recovery` segment out-prioritises
  `{conversation_id}` in matchit. `build_router` is actually constructed and
  driven by `oneshot` in the test (`proxy.rs:8340-8362`), so a router conflict
  would panic the test.
- `conversation_index_includes_workflow`, `merge_workflow_history_rows`, and
  `filter_addressable_conversation_index` are all deleted with no callers left
  (FI-007 satisfied).

The pagination test is strong and does what the spec asked: 53 rows over two
cursor pages, 52 kept, foreign `voicec-` kind dropped, duplicate `nyxid-chat-…`
row losing to its first occurrence, newest-first ordering asserted by explicit
index, exact upstream call list including `?cursor=page-2`, cursor-loop → error,
and a 40-page cap proven by `captured.len() == 40`
(`proxy.rs:8828-8904`).

**W1.6 FAIL — the rewritten guard test asserts nothing.**
`backend/src/services/assistant_service.rs:1379-1384`:

```rust
for suffix in [":stream", "/approve", "/stop", "/steer", "/retry", "/skip"] {
    assert!(!SOURCE.contains(&["conversations/", suffix].concat()), ...);
}
```

`["conversations/", "/approve"].concat()` is `"conversations//approve"` — a
double slash that cannot occur in any path builder. `["conversations/", ":stream"]`
is `"conversations/:stream"`, whereas a real builder emits
`conversations/{conversation_id}:stream`. I verified all six needles are absent
from the file *and* structurally unreachable, so every assertion is
unconditionally true. Meanwhile the old
`!SOURCE.contains("/chat-history/conversations")` assertion was necessarily
removed. Net effect: the per-conversation-command guard the spec asked to keep
now guards nothing, and a canonical `api/chat/conversations/{id}/approve`
regression would pass CI. The family-mapping assertions the spec asked for live
in `builds_family_aware_resource_paths` (`:1310-1341`), which is fine, but the
guard half of W1.6 is not delivered.

Nits:

- **Shape tolerance regressed.** `append_addressable_history_page` returns
  `AppError::Internal` when a page omits `conversations`
  (`assistant_service.rs:101-108`), and `list_conversations` now maps a
  non-JSON page body to `Internal` (`assistant.rs:466-468`). The deleted
  code documented the opposite posture ("an index that is not
  `{"conversations": [...]}` is returned untouched … deploy-independence"). One
  malformed page now blanks the entire sidebar instead of degrading.
- **Unknown-prefix ids now 400 locally** (`conversation_resource_family`,
  `assistant_service.rs:104-110`), where they previously forwarded and got an
  upstream 404. `useConversation`'s retry predicate short-circuits only 404 /
  `AssistantConversationNotFoundError` (`use-assistant.ts:301-305`), so a
  `workflow-pending-…` id reaching `getHistory` now costs three retries plus an
  error toast rather than the local-mirror fallback. Reachable only for an
  *empty* placeholder (`aevatar-transport.ts:1338-1350`), so narrow — but it is
  untested and undocumented.
- **Drain memory ceiling.** 40 pages × `MAX_CONVERSATION_INDEX_RESPONSE_BYTES`
  (4 MiB) = up to 160 MiB of accumulated rows per list request. The pre-change
  path was bounded at ~8 MiB. There is no aggregate budget across pages.
- **The list no longer forwards the caller's request.** Every page is a
  `synthetic_request`, so W7.1's `Accept: application/json` reaches NyxID but
  not Aevatar for the list, unlike transcript/delete/recovery which forward the
  caller request. The console sends it on *every* history call
  (`chatHistoryApi.ts:13-15`). Billing is unaffected —
  `synthetic_request` inserts `BillingRoutePolicy` itself (`assistant.rs:220-224`).

### W2 — Console-exact workflow turn body (backend) — **PASS**

- Trim first, validate the trimmed value, serialize the trimmed value
  (`assistant_service.rs:1104-1109`, `1173-1176`). Confirmed against the
  console's `prompt: request.prompt.trim()` at `chatApi.ts:381-385`.
- `commandId` is create-only; `conversationId` + `commandId` → `BadRequest`
  (`assistant_service.rs:1111-1115`, `1144-1154`).
- Byte-exact fixtures for create and continuation (`:1806-1832`). These are
  meaningful: `serde_json` resolves with `preserve_order` in this workspace
  (`Cargo.lock` shows the `indexmap` dependency), so the `serde_json::Map`
  insertion order in `workflow_chat_body` is what is serialized, and the
  continuation fixture pins the exact key set `{conversation, prompt, sessionId,
  workflow}` with no `commandId`.
- Trim boundary test present (`:1863-1875`).

Nit: the boundary test only covers the *accepting* side (32768 + whitespace →
ok). There is no negative case proving 32769 trimmed chars is rejected, which is
the half that would catch a re-regression to `request.prompt.chars().count()`.

### W3 — Console-exact turn body (frontend) — **PASS**

`workflowTurnBody` continuation is exactly `{prompt, conversationId,
minimumStateVersion, sessionId}` (`aevatar-transport.ts:2444-2450`); the
floor-of-1 is gone and a missing watermark now throws rather than fabricating.
Create body unchanged. Pinned by an exact `Object.keys` equality assertion
(`aevatar-transport.test.ts` — "continues with the observed stateVersion and no
client commandId") and by a full `toEqual` on the continuation body in the
create-context-zero test.

### W4 — Console-parity retry + recovery (frontend) — **FAIL**

Correct:

- The blanket `RETRYABLE_STREAM_STATUSES` replay loop is gone for workflow runs;
  workflow now takes `streamWorkflowTurn` and the typed/actor path keeps
  `STREAM_DELIVERY_ATTEMPTS` verbatim (`aevatar-transport.ts:2849-2875`). Hard
  constraint "typed/actor path unchanged" holds.
- Reservation retry matches the console's shape: only continuations, only
  503 + `CHAT_HISTORY_RESERVATION_UNAVAILABLE`, pacing `[300, 900]`, refresh
  before each retry, wait (consume an attempt) when the refresh is below the
  fence, raise to `max(fence, refreshed)`
  (`aevatar-transport.ts:2755-2801`). Re-derived against
  `chatApi.ts:477-568` — behaviourally equivalent.
- Create `commandId` reuse for the same prompt / new id for a changed prompt
  (`aevatar-transport.ts:2281-2290`) matches `index.tsx:1232-1237`.
- The abort-path recovery is genuinely abort-independent: a fresh
  `AbortController` is used, not `run.controller.signal`
  (`aevatar-transport.ts:2622-2628`), and the deletion tombstone is re-checked
  before and after reconciliation (`:2554`, `:2581`). Both
  `IMPLEMENTATION_NOTES.md` claims verified.

**FAIL 1 — recovery never runs when a create stream ends normally without a
context frame.** `streamWorkflowTurn` triggers recovery only on
`result.kind === "retryable"` (`aevatar-transport.ts:2814`) or a headers-level
network error (`:2730`). A create stream that emits `RUN_STARTED` and then
`RUN_FINISHED` with no `aevatar.chat.context` frame sets `run.deliveryTerminal`,
so `consumeTurnStream` returns `{kind: "settled"}` (`:3054-3057`) and
`streamWorkflowTurn` returns at `:2812` — the run **completes** with the
conversation still on its local `workflow-pending-…` id. Nothing fails closed:
the existing `applyWorkflowChatContext` guard only fires when a context frame
*arrives* malformed (`:3672-3684`), which is a different case (and its test,
"fails closed when a create turn never names its server conversation", sends a
context frame with a missing `conversationId`).

Upstream does the opposite: after the frame loop completes normally the console
runs `recoverCreateIdentity` whenever `!receivedChatHistoryContext &&
createCommandId`, and then hard-throws "Chat completed without a conversation
context" if recovery is empty
(`origin/dev:apps/aevatar-console-web/src/pages/chat/index.tsx:1415-1443`).
Consequence in NyxID: the next send takes the create branch again and mints a
**second** upstream conversation — exactly the failure mode #1301's fail-closed
guard exists to prevent. Spec W4.2 lists "normal EOF" first among the three
recovery entry points; it is the one case not implemented.

**FAIL 2 — the recovery gate keys on the wrong flag, and the identity guard
compares two different id spaces.** Both inline call sites gate on
`isCreate && !run.deliveryStarted` (`:2730`, `:2814`). For workflow runs
`run.deliveryStarted` is set by **`RUN_STARTED`**, not only by the context frame
(`:3239-3274` — the pre-start-CUSTOM allowance means `RUN_STARTED` reaching the
handler first legitimately sets it). So a create whose stream emits
`RUN_STARTED` and then truncates is `retryable` with `deliveryStarted === true`
→ recovery is skipped. The console's predicate is `!receivedChatHistoryContext`,
i.e. "no conversation identity adopted". Notably
`startCreateRecoveryInBackground` uses the *correct* predicate
(`stored.conversation.id.startsWith(WORKFLOW_CONVERSATION_PREFIX)`, `:2614-2620`) —
the three call sites disagree with each other.

The same `RUN_STARTED` path also assigns `run.turnId` from
`frame.runStarted.runId` (`:3261-3274`), which is a run-actor id, whereas
create-recovery returns the Chat History `turnId`. `recoverWorkflowCreate`'s
guard `run.turnId !== null && run.turnId !== recovery.turnId` (`:2565`) will
therefore raise a spurious "Chat create recovery changed the conversation
identity" on the abort path (where there is no `deliveryStarted` gate), and it
is swallowed by `.catch(() => undefined)` at `:2627`. The background-abort test
never exercises this because its stubbed stream delivers no frames at all.

**FAIL 3 — dead `history_refresh_failed` branch; refresh failures surface the
wrong error.** `aevatar-transport.ts:2786-2800` builds
`finalFailure = {code: "history_refresh_failed", …}` and breaks; control then
reaches `:2811-2812` which unconditionally overwrites it with the 503 reservation
error. The branch is unreachable in effect. The console instead throws
`ChatRetryPreparationError(refreshError)` for a non-retryable refresh failure
(`chatApi.ts:499-509`), so the user sees the refresh error, not the reservation
error. The new test `does not replay when reservation refresh remains HTTP %i`
asserts only `status: "failed"` and therefore pins neither behaviour.

**Scope expansion worth an explicit decision (not a spec violation, but new
production risk):** `applyWorkflowChatContext` gained a hard scope guard
(`:3638-3650`) rejecting any context frame whose `payload.scopeId` is not a
string equal to `useAuthStore.getState().user?.id`, **and** rejecting when the
auth store holds no user. Spec W4.2 refers to this as one of the "existing
context-frame guards" — it did not exist before
(`git show 6f3136f5:frontend/src/lib/assistant/aevatar-transport.ts` has no
`scopeId` comparison). It mirrors the console (`index.tsx:1326`), and NyxID's BE
does derive the Aevatar scope from the NyxID user id
(`api/scopes/{user_id}/…`), so it is *probably* correct — but it is a new
fail-closed dependency on `auth-store.user.id` being byte-identical to Aevatar's
`scopeId`, and it already forced `use-assistant.aevatar.test.tsx` to seed a user.
Any render path where the assistant mounts before the auth store hydrates now
fails **every** workflow turn with `stream_protocol_error`. Given the memory note
that live tokens are rotated-dead and production composition is UNPROVEN, this
needs either a live capture or a softer treatment when the local scope is unknown.

### W5 — No fabricated watermark; bounded pre-send reconciliation — **PASS**

- Floor removed; a continuation without a positive observed watermark throws
  (`:2440-2446`).
- The preflight runs **before** the optimistic append: `sendMessage` computes
  `needsWorkflowPreflight` and skips `appendOptimisticUserMessage`
  (`:2292-2299`); `streamWorkflowTurn` reconciles first and appends only after
  (`:2641-2661`). `run.optimisticMessageAppended` makes the append idempotent.
- Bounded `0/300/900/1800`, positive `stateVersion` required, previous
  assistant turn required when known (`:2459-2490`, `:2426-2438`). Exhaustion →
  retryable `history_synchronizing` failure.
- The keep-max early return in `applyHistoryResponse` (`:2270-2273`) protects the
  optimistic user message from being wiped by a mid-turn reservation refresh, and
  the new in-place `existing.stateVersion = Math.max(...)` mutation (`:2256-2261`)
  is what makes the raised fence visible through that early return. Correct and
  non-obvious; worth a comment.
- Create-context-zero window covered and tested end-to-end.
- The `preflights a missing watermark before one optimistic append` test
  genuinely checks message-count invariance across the failed attempt and asserts
  exactly one user message after the successful retry.

### W6 — Session id parity — **PASS-WITH-NITS**

Re-mint on restore lives in `getHistory` (`:1359-1364`), gated on the workflow
family and on `activeConversationId !== conversationId`; `activeConversationId`
is maintained through `createConversation` (`:1198`), context adoption
(`:3692-3694`), and recovery adoption (`:2597-2599`), so a post-turn history
refresh of the active conversation correctly does *not* remint. Matches
`index.tsx:868-899`.

Nit: the new test asserts only `sessionIds[1] !== sessionIds[0]`. There is no
assertion that the session stays stable when the *same* conversation is re-read
without an intervening switch, and none for the "within-conversation retry"
stability the spec calls out (the pre-existing "reuses one sessionId for every
turn" test covers turns, not retries).

### W7 — Resource papercuts — **PASS**

- `Accept: application/json` added locally in `assistantApi.get`
  (`:92-101`); `lib/api-client.ts` untouched (confirmed — not in the diffstat).
  Pinned by "requests assistant resources as JSON".
- 204 delete tolerated: `apiClient` short-circuits 204 before `response.json()`
  (`api-client.ts:145-148`); test uses a real bodyless `Response(null, {status: 204})`.
- Aevatar-shaped `{code, message}` and empty 404 bodies both covered by the
  parameterized replacement for the deleted `types a 404 …` test — this is a
  strengthening, not a weakening. `parseErrorResponse` degrades an
  unparseable body to a synthetic envelope while preserving `status`
  (`api-client.ts:83-95`), which is what `pollCreateRecovery`'s 404 check
  depends on.

### W8 — Documentation — **PASS**

`docs/assistant-network-flows.md` §4 now splits typed/workflow per resource,
documents the drain + 40-page cap, the 204 normalization, the not-found state
row, the create-recovery row, and the absence of a continuation `commandId`.
`docs/chat-canonical-api-migration.md` gets a prepended correction note and the
historical brief is untouched. Both match what the code actually does.

---

## Hard-constraint compliance

| Constraint | Verdict | Evidence |
|---|---|---|
| Typed card surface command envelope / card semantics untouched | PASS | typed branch of `streamTurn` unchanged apart from comments + prettier reflow (`aevatar-transport.ts:2852-2865`) |
| `POST /api/v1/assistant/chat` typed facade untouched | PASS | only the `task.stop` call site reindented (`:4671-4690`); body identical |
| Workflow WS twin untouched | PASS | `workflow_chat_ws_path` and its handlers unchanged |
| Admin surfaces untouched | PASS | not in diffstat |
| No proxy code outside assistant handlers/service | PASS | all `proxy.rs` hunks fall in lines 7975–8904; `#[cfg(test)] mod proxy_resolution_integration_tests` spans 6876–9025 |
| Global `frontend/src/lib/api-client.ts` untouched | PASS | absent from diffstat |
| `AppError`/`AppResult` conventions | PASS | new errors are `BadRequest`/`NotFound`/`Internal`; no upstream detail leaks in messages |
| No new deps | PASS | no `Cargo.toml` / `Cargo.lock` / `package.json` / lockfile diff |
| No `console.log` in FE production code | PASS | `git diff … \| grep '^+.*console\.log'` empty |
| Conventional commits, branch-local, not pushed | PASS | 4 `fix:`/`docs:` commits; no `origin/verify-aevatar-chat-calls` ref exists |
| Tree clean | FAIL (minor) | `IMPLEMENTATION_NOTES.md` is untracked at the worktree root (Acceptance §3) |

---

## Test-quality findings

1. **The W1.6 guard loop is vacuous** — see W1 above. Highest-value test defect
   in the change: it reads as a regression barrier and is not one.
2. **No test covers the FAIL-1 case** (create stream completes normally with a
   terminal frame but no context frame). Every recovery test drives either an
   empty stream (`{}`), an explicit truncation, or a headers network error.
3. **`does not replay when reservation refresh remains HTTP %i` under-asserts** —
   only `status: "failed"`; it would pass with either the dead
   `history_refresh_failed` code or the console's behaviour, so it cannot detect
   the FAIL-3 divergence.
4. **BE trim boundary is one-sided** — no 32769-trimmed-chars rejection case.
5. **BE create-recovery has no 404 passthrough test.** The FE polling contract
   depends on a 404 status surviving the pass-through; the BE test only covers a
   200 body and a malformed command id.
6. **W6 stability is under-asserted** — inequality on remint is checked; same-id
   re-read stability and within-conversation retry stability are not.
7. Deletions reviewed and cleared: the removed `types a 404 …` test was replaced
   by a stronger parameterized version; the removed
   `deletes a conversation upstream …` test was replaced by the 204 case (typed
   202+JSON passthrough is now covered on the BE instead); the removed
   `merge_*` / `conversation_index_includes_workflow` unit tests correspond to
   deleted code and their coverage moved to the drain tests. No silent weakening
   found beyond item 1.
8. Genuinely strong new tests worth keeping as-is: the BE 53-row / 2-page /
   loop-guard / 40-page-cap drain test with an exact upstream call list; the
   byte-exact turn-body fixtures; the exact `Object.keys` continuation assertion;
   the "preflights a missing watermark before one optimistic append" invariance
   checks; the `it.each` no-replay matrix (unrelated 503 / 500 / empty body /
   malformed body / network error).

---

## Gate results (rerun by me, this worktree)

```
$ cargo test --workspace
test result: FAILED. 4816 passed; 94 failed; 0 ignored
```

The 94 failures are **environment-only and pre-existing**. Reproduced one in
isolation:

```
$ cargo test --workspace org_invite_service::tests::duplicate_key
panicked at backend/src/services/org_invite_service.rs:551:14:
local MongoDB required for org_invite_service tests
```

`backend/src/test_utils.rs:106-111` probes `127.0.0.1:27018` (credentialed) then
`127.0.0.1:27017`; only the docker-compose 27018 instance is up here and its
credentials do not match the probe. No assistant, proxy, or routing test is in
the failure set. This matches `IMPLEMENTATION_NOTES.md`'s account exactly (same
count, same cause).

Scoped rerun of everything this change touches:

```
$ cargo test --workspace assistant
test result: ok. 45 passed; 0 failed
  …including assistant_chat_handlers_rebuild_bodies_for_the_admin_service,
  assistant_list_drains_mixed_history_pages_and_captures_every_upstream_call,
  assistant_deleted_scoped_command_routes_are_unroutable,
  workflow_body_matches_the_reference_{create,continuation}_payload,
  workflow_body_rejects_a_command_id_on_continuation,
  workflow_body_trims_before_boundary_validation_and_serialization,
  builds_family_aware_resource_paths
```

```
$ cd frontend && npm run build
✓ built in 673ms  (app)   ✓ built in 46ms  (credential-accept)   — 0 errors

$ npx vitest run
Test Files  194 passed (194)
Tests  2320 passed (2320)          # first run, no flakes, no --no-file-parallelism needed

$ npm run lint
✖ 23 problems (0 errors, 23 warnings)   # all pre-existing react-refresh / exhaustive-deps

$ cargo test -p nyxid-cli --test wizard_bundle_freshness
test wizard_bundle_is_fresh ... ok
```

`IMPLEMENTATION_NOTES.md` claims verified: create-recovery scope validation uses
the authenticated NyxID user id (confirmed — and consistent with the BE's
`api/scopes/{user_id}/…` derivation); recovery is abort-independent with a
tombstone check (confirmed at `:2622-2628`, `:2554`, `:2581`); reservation retry
is workflow-only (confirmed — actor path untouched); W7 is local to the assistant
helper (confirmed — `api-client.ts` not in the diff); the wizard claim is correct
and the freshness gate passes against the committed bundle.

---

## FINAL VERDICT: REWORK-REQUIRED

1. **W4.2 — run create recovery after a *normally completed* create stream that
   carried no `aevatar.chat.context` frame, and fail closed if recovery is
   empty.** Today `consumeTurnStream` returning `{kind:"settled"}`
   (`aevatar-transport.ts:3054-3057`) lets the run complete on the
   `workflow-pending-…` placeholder, so the next send mints a second upstream
   conversation. Mirror `index.tsx:1415-1443`. Add the missing test.
2. **W4.2 — replace the `!run.deliveryStarted` recovery gate with "no server
   conversation id adopted"** at `aevatar-transport.ts:2730` and `:2814`, matching
   the predicate `startCreateRecoveryInBackground` already uses (`:2614-2620`).
   `RUN_STARTED` sets `deliveryStarted` for workflow runs (`:3239-3274`), so a
   create that truncates after `RUN_STARTED` currently gets no recovery. In the
   same pass, stop comparing `run.turnId` (a run-actor id when it came from
   `RUN_STARTED`) against the Chat History `turnId` returned by create-recovery
   (`:2565`) — that guard raises a spurious identity error on the abort path.
3. **W1.6 — fix the vacuous guard assertions** at
   `backend/src/services/assistant_service.rs:1379-1384`. `"conversations//approve"`
   and `"conversations/:stream"` cannot occur in any real path builder, so all six
   assertions are unconditionally true. Assert against the shapes the code would
   actually emit (e.g. `/approve"` / `:stream"` suffixes on a `conversations/{`
   format string), and restore an equivalent of the deleted
   `/chat-history/conversations` guard scoped to the typed family.
4. **W4.1 — resolve the dead `history_refresh_failed` branch**
   (`aevatar-transport.ts:2786-2812`): either surface it (console parity —
   `chatApi.ts:499-509` throws the refresh error) or delete it, and extend
   `does not replay when reservation refresh remains HTTP %i` to assert the
   resulting `error.code` so the choice is pinned.

Recommended before merge but not blocking: get an explicit decision (and ideally
a live capture) on the **new hard scope guard** in `applyWorkflowChatContext`
(`:3638-3650`) — it is console-parity but was not an existing guard, it hard-fails
every workflow turn when `auth-store.user.id` is absent or differs from Aevatar's
`scopeId`, and production composition is still UNPROVEN per the audit. Also worth
addressing: restore shape tolerance in the list drain, add the negative trim
boundary and BE create-recovery 404 tests, bound the aggregate drain buffer, and
either commit or remove `IMPLEMENTATION_NOTES.md` so the tree is clean.

---
---

# Verification round (2026-08-01)

Range verified: `973c1e93..HEAD` — `1735134d fix(assistant): harden history parity
after review`, `ddd2b0be fix(assistant): close workflow recovery gaps after review`.
Five files touched (`handlers/assistant.rs`, `handlers/proxy.rs`,
`services/assistant_service.rs`, `aevatar-transport.ts`,
`aevatar-transport.test.ts`); +380 / −66.

**Round-2 verdict: APPROVE.** All four required fixes landed and are correct in
substance, not merely present. Lead decisions 5–10 landed. Four non-blocking nits
are recorded at the end as follow-ups.

## Required fix 1 — normal-EOF create recovery — **VERIFIED**

The trigger sits exactly on the site I identified. `consumeTurnStream`
(`aevatar-transport.ts:3063-3076`) now intercepts the `run.deliveryTerminal`
branch *before* `settleDeliveryTerminal` and returns
`{kind:"retryable", error:{code:"stream_protocol_error", message:"Chat completed
without a conversation context."}}` when `run.protocol === "workflow" &&
workflowCreateNeedsRecovery(conversationId)`. `streamWorkflowTurn:2817-2824` then
runs recovery; on empty recovery it `break`s to the tail and calls
`finishTurn(…, "failed", finalFailure)` carrying that message, and
`recoverWorkflowCreate` returns `false` before mutating anything, so the
conversation stays on `workflow-pending-…`. Fail-closed as required.

The new test genuinely drives a terminal frame, not an empty stream or a
truncation: frames are `[{runStarted…}, ...WORKFLOW_TAIL]`, and `WORKFLOW_TAIL`
contains `runFinished` (`aevatar-transport.test.ts:5623-5637`). It asserts
`recoveryAttempts === 4` (proving the full 0/300/900/1800 poll), `streams`
called exactly once (no POST replay), and
`getHistory(conversation.id).conversation.id` matching `/^workflow-pending-/`.

Ordering re-checked: `deliveryProtocolError` is still evaluated first
(`:3060-3062`), so a scope-mismatched or malformed context frame remains a
definitive protocol error with no recovery — correct, and the pre-existing
"fails closed when a create turn never names its server conversation" test still
passes.

## Required fix 2 — recovery gate + identity guard — **VERIFIED**

- `workflowCreateNeedsRecovery` (`:2674-2680`) tests the *stored* conversation id
  against `PENDING_WORKFLOW_CONVERSATION_PREFIX`. It is now used at **all four**
  call sites — `startCreateRecoveryInBackground:2664`, headers network error
  `:2733`, terminal path `:3065`, post-consume retryable `:2819`. `grep
  deliveryStarted` confirms no occurrence remains as a recovery gate.
- The predicate is the right one: `isWorkflowConversationId` (`:163-168`) admits
  only `chatc-` and `workflow-pending-`, and `createConversation:1187` mints
  `workflow-pending-<uuid>`, so the check is exactly "workflow create not yet
  adopted".
- The `run.turnId !== recovery.turnId` comparison is deleted and `run.turnId =
  recovery.turnId` is now unconditional with an explanatory comment
  (`:2612-2650`). Removing it loses no guard: if a context frame had been
  accepted, the stored id would already be `chatc-` and recovery would not run at
  all. The remaining guards are intact — `chatc-` shape via
  `WORKFLOW_SERVER_CONVERSATION_ID_PATTERN` in `decodeCreateRecovery:2508`,
  prior-id replay mismatch at `:2610-2616`, scope compared before and after
  reconciliation at `:2548`, `:2553`, `:2580`, and the deletion tombstone at
  `:2554`/`:2581`.
- Both previously-broken cases are exercised with stubs that really deliver
  `RUN_STARTED`: "recovers a create truncated after RUN_STARTED" sends
  `frames:[{runStarted…}]` plus a `stream_closed` completion and asserts
  `completed` + adoption of `WORKFLOW_CONVERSATION`; "adopts abort-path recovery
  after RUN_STARTED supplied a run actor id" now pushes `RUN_STARTED` through
  `request.onFrames`, awaits delivery, then cancels, and asserts `cancelled` +
  adoption — i.e. the spurious identity error is gone.

Collateral change worth recording: `cancelTurn` was restructured so
`run.protocol === "workflow"` is the first branch (`:4579-4590`). At the baseline
`6f3136f5` a workflow run holding a `turnId` fell into `if (run.turnId) { abort;
requestServerStop }` and posted a typed `task.stop` for a workflow conversation
(the old comment claiming workflow "takes this path unconditionally" was simply
wrong — that comment sat in the `else if`). That is now impossible, which is a
genuine correction and a prerequisite for this fix. The actor branches are
semantically identical to before (`run.turnId` → stop; `!run.streamDispatched` →
local abort; else → `stopPendingStart` fence), so the typed/actor hard constraint
holds. No test pins the absence of the stray `task.stop`, because `stubFetch`
answers unmatched routes with a soft 404 (`aevatar-transport.test.ts:186-201`).

## Required fix 3 — W1.6 guard test — **VERIFIED (falsifiability proved independently)**

The guard (`assistant_service.rs:1376-1408`) now (a) splits the production source
at `"#[cfg(test)]\nmod tests"` before scanning — which also fixes the test's own
literals tripping it, (b) checks
`format!("conversations/{{conversation_id}}{suffix}")`, i.e. the fragment a real
builder emits, and (c) restores the deleted `/chat-history/conversations`-style
guard *behaviorally*: it resolves `conversation_resource_family(CONV)`, builds the
resulting detail path, and asserts it does not contain
`/chat-history/conversations`. That is stronger than the literal it replaced,
because it exercises the router rather than the file text.

I proved falsifiability myself rather than accepting the notes:

```
# probe A — canonical builder style, injected into the production section
+ pub fn review_probe_path(conversation_id: &str) -> String {
+     format!("api/chat/conversations/{conversation_id}/approve")
+ }

$ cargo test --workspace migration_guard_keeps_scoped
test ...migration_guard_keeps_scoped_typed_commands_and_per_conversation_commands_out ... FAILED
panicked at backend/src/services/assistant_service.rs:1404:13:
per-conversation command route /approve must not return
```

matching the implementer's account exactly. A second probe exposes the residual
limit:

```
# probe B — same forbidden route, binding renamed
+     let id = conversation_id;
+     format!("api/chat/conversations/{id}/approve")

$ cargo test --workspace migration_guard_keeps_scoped
test ...migration_guard... ok
```

So the guard is falsifiable for builders that name the binding `conversation_id`
(which every current builder does) but is name-coupled. Nit, not a blocker.

Both probes were reverted with `git checkout --`; `git diff --stat` is empty for
tracked files and `git status --short` shows only the two untracked markdown
files (`.impl-review-opus.md`, `IMPLEMENTATION_NOTES.md`).

## Required fix 4 — refresh-failure surfacing — **VERIFIED (one unclaimed divergence)**

A `refreshFailed` flag (`:2767`, `:2806`) plus `if (refreshFailed) break;`
(`:2812`) is placed before the unconditional `finalFailure = error`, so the
`history_refresh_failed` branch is now reachable rather than dead.

I re-derived the console rather than accepting the prose
(`chatApi.ts:477-568` plus `isRetryableHistoryRefreshError`): a non-retryable
refresh error throws `ChatRetryPreparationError(error)` so the *refresh* error
surfaces; a retryable 5xx `continue`s, and when attempts exhaust the loop throws
`lastReservationError` — the original 503. NyxID now matches on both HTTP-status
classes, and the test pins each code explicitly:
`error.code === status === 404 ? "history_refresh_failed" :
"CHAT_HISTORY_RESERVATION_UNAVAILABLE"` (`aevatar-transport.test.ts:6555-6566`),
with `streams` called once in both.

Not mentioned in the notes: the console treats an error carrying **no `status`**
(a raw fetch network failure) as *retryable* (`return true` at the tail of
`isRetryableHistoryRefreshError`), whereas NyxID's condition is `refreshError
instanceof ApiError && 500 <= status < 600`, so a transient network blip during
the refresh now surfaces `history_refresh_failed` immediately instead of
consuming the remaining attempt. `AssistantConversationNotFoundError` (tombstone
raised inside `applyHistoryResponse`) lands in the same branch. Both are
defensible fail-fast choices, but "matching the console retry loop" is accurate
only for the HTTP-status cases, and neither is covered by a test.

## Lead decisions 5–10

| # | Decision | Verdict | Evidence |
|---|---|---|---|
| 5 | Scope guard enforce-when-known | **VERIFIED** | `:3658-3671` — `activeScopeId && (not-a-string \|\| mismatch)` → hard fail; unhydrated skips. Comment present and accurate. New test "accepts workflow context when the local auth user is not hydrated" nulls the store, asserts `completed` **and** that no create-recovery fetch fired, so it proves acceptance rather than mere non-crash. The mismatch test is retained. |
| 6 | Drain degrades, loop guard stays hard | **VERIFIED** | Non-JSON page → `break` preserving rows (`assistant.rs:474-479`); missing `conversations` → `Ok(None)` ends the drain (`assistant_service.rs:118-124`); repeated cursor still `AppError::Internal` (`assistant.rs:491-495`) and still asserted. Drain test modes 3 and 4 each return 1 row / 2 calls. |
| 7 | Aggregate ~8 MiB budget that actually stops the drain | **VERIFIED** | `MAX_HISTORY_INDEX_AGGREGATE_BYTES = 8 * 1024 * 1024` documented at `assistant.rs:42-45`; `checked_add` + `break` **before** the page is processed (`:469-475`). Test mode 5 serves ~3 MiB pages: the handler makes **3** calls and returns **2** rows — the budget, not the 40-page cap, is what stops it. |
| 8 | Unknown prefix → not-found-shaped | **VERIFIED** | `AppError::NotFound` (`assistant_service.rs:108`), which maps to 404 (`errors/mod.rs:956-957`), so `getHistory`'s `noServerTranscriptYet` path applies and `useConversation`'s retry predicate short-circuits. BE test `unknown_conversation_families_are_not_found_shaped` (`:1346-1352`). |
| 9 | `Accept: application/json` on synthetic page requests | **VERIFIED end-to-end** | Set at `assistant.rs:436-438`; the drain test captures the header **as observed by the downstream server** (`proxy.rs:8726-8730`) and asserts it on every call in all five modes (2, 2, 40, 2, 3 calls). |
| 10 | Three test gaps closed | **VERIFIED** | Negative trim boundary: 32769 trimmed chars → `BadRequest` (`assistant_service.rs:1896-1903`). BE create-recovery 404 passthrough: stub at `proxy.rs:8026-8032`, assertion at `:8323-8338`, and the path appears in the exact captured-call list at `:8409-8411`. sessionId: a `getHistory` inserted between turns in "reuses one sessionId for every turn" pins same-conversation re-read; `bodies[1].sessionId === bodies[0].sessionId` added to the reservation-retry test pins within-conversation retry. |

## Regression re-check on previously-PASSed work

- **W2** (`workflow_chat_body`), **W3** (`workflowTurnBody`), **W5** (preflight +
  optimistic-append ordering), **W7** (`assistantApi.get`, 204 delete), **W8**
  (docs) — none appear in this range's diff; only new tests were added around W2.
- **Hard constraints**, re-verified for `973c1e93..HEAD`: all `proxy.rs` hunks
  fall in 8025–8903, inside `#[cfg(test)] mod proxy_resolution_integration_tests`
  (now 6876–9126); exactly five files touched with `lib/api-client.ts` absent; no
  `Cargo.toml`/`Cargo.lock`/`package.json`/lockfile diff; zero added
  `console.log`; typed/actor `streamTurn` path untouched; workflow WS twin and
  admin surfaces untouched; branch still unpushed (no
  `origin/verify-aevatar-chat-calls`).
- **Under-assertion check**: the new tests are materially stronger than round 1 —
  they pin poll counts, error codes, call counts, the *unchanged* conversation id
  on fail-closed, and the downstream-observed `Accept` header. Nothing I flagged
  as under-asserting in round 1 remains so.

## Gate results (rerun by me, this worktree)

```
$ cargo test --workspace
test result: FAILED. 4817 passed; 94 failed; 0 ignored
```

Same environment-only failure set as round 1 (4816 → 4817 passed = the one new BE
test). I classified all 94: they fall in exactly nine DB-backed modules
(`handlers::node_{admin,agent,ws}`, `services::{node_fanout_resolver,
node_pending_credential_service, oauth_service, org_invite_service,
rci_audit_service, social_token_exchange_service}`) and **zero** touch assistant,
proxy, or routes. Reproduced the cause again in isolation:
`panicked at backend/src/services/org_invite_service.rs:551: local MongoDB
required for org_invite_service tests` — `test_utils.rs:106-111` probes
credentialed `127.0.0.1:27018` then bare `127.0.0.1:27017`; only the
docker-compose 27018 instance is up here and its credentials do not match the
probe. I could not stand up a disposable Mongo in this environment, so the
implementer's "full `cargo test` passed on 27017" claim is **corroborated but not
independently reproduced**; the failure signature is byte-identical to the
pre-change baseline.

```
$ cargo test --workspace assistant
test result: ok. 46 passed; 0 failed          # 45 → 46, +unknown_conversation_families_are_not_found_shaped
  …including assistant_list_drains_mixed_history_pages_and_captures_every_upstream_call,
  assistant_chat_handlers_rebuild_bodies_for_the_admin_service,
  migration_guard_keeps_scoped_typed_commands_and_per_conversation_commands_out,
  workflow_body_trims_before_boundary_validation_and_serialization

$ cd frontend && npm run build
✓ built in 44ms  — 0 errors

$ npx vitest run
Test Files  194 passed (194)
Tests  2323 passed (2323)     # 2320 → 2323; first run, no flakes

$ npm run lint
✖ 23 problems (0 errors, 23 warnings)   # unchanged, all pre-existing

$ cargo test -p nyxid-cli --test wizard_bundle_freshness
test wizard_bundle_is_fresh ... ok
```

The notes' gate figures match what I observed (46 / 194 files / 2323 tests / 0
lint errors / wizard fresh).

## Non-blocking follow-ups

1. **Terminal kind is discarded on the new recovery path.** Bypassing
   `settleDeliveryTerminal` (`:4364-4412`) drops its `error` / `stopped` /
   `finished+blocked` discrimination. A workflow create that lacks a context
   frame but emits an **error** terminal (or `finished` with `status:"blocked"`)
   and whose recovery then succeeds is reported `completed`; on the failure path
   the upstream terminal's own error message is replaced by the generic "Chat
   completed without a conversation context." The console preserves the error
   state through `buildSessionFromAccumulator(…, accumulator.errorText …)`.
   Anomaly-on-anomaly (needs a committed reservation with no context frame), so
   not blocking, but worth a follow-up.
2. **Guard is name-coupled** — probe B above passes. Consider matching the
   suffix against any `conversations/{…}` interpolation rather than the literal
   binding name.
3. **Network-class refresh failures diverge from the console** (fix 4 section);
   either align with `isRetryableHistoryRefreshError`'s `return true` default or
   document the deliberate fail-fast, and pin it with a test.
4. **Untested edges**: a malformed *first* drain page (now yields
   `200 {"conversations": []}` because `response_parts` is captured before
   parsing); the absence of a typed `task.stop` on the workflow cancel path.
   Also `IMPLEMENTATION_NOTES.md` remains untracked at the worktree root —
   commit or delete it before handoff (spec Acceptance §3, "tree clean").

---

**FINAL VERDICT (round 2): APPROVE.** No remaining required fixes. Four
non-blocking follow-ups are listed above.
