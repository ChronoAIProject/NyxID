# Chat Parity Implementation Spec — console-exact studio chat

Owner: Calvin · Lead/final review: Fable · Implementer: GPT SOL · Implementation
adversary: Opus · Date: 2026-07-31
Basis: `docs/chat-aevatar-dev-parity-audit.md` (v2) + adversarial review
`docs/chat-aevatar-dev-parity-audit-review-gpt-sol.md`. Read both before coding.
Pinned upstream reference: `~/Desktop/aelf-frontend-work/aevatar` origin/dev @
`bbd906eb5` — **read-only** (use `git show origin/dev:<path>`; never modify or
check out).

Objective: make NyxID FE → NyxID BE → Aevatar behave, on the wire, exactly like
Aevatar's console FE (`apps/aevatar-console-web/src/pages/chat/`) for starting
and continuing studio chats, and make conversation resources family-correct.
NyxID-relative browser URLs stay as they are; all changes are in what the FE
sends to the BE and what the BE sends upstream.

## Hard constraints

- Do not touch: the typed card surface's command envelope or card semantics,
  `POST /api/v1/assistant/chat` (typed facade), the workflow WS twin, admin
  surfaces, any proxy code outside the assistant handlers/service.
- Backend: `AppError`/`AppResult` conventions; upstream details never leak to
  clients; no new deps; no `console.log` in FE production code.
- Conventional commits; work only on this branch; **do not push** (Fable pushes).
- Every work item lands with its tests. Full gates before handoff (§Gates).

## Work items

### W1 — Family-aware conversation resources + full pagination (backend)

`backend/src/services/assistant_service.rs`, `backend/src/handlers/assistant.rs`,
`backend/src/routes.rs`.

1. Path builders: keep `canonical_*` (typed family). Add scoped-history builders:
   `history_conversation_path(user_id, conversation_id)` →
   `api/scopes/{user_id}/chat-history/conversations/{id}` and
   `history_create_recovery_path(user_id, command_id)` →
   `api/scopes/{user_id}/chat-history/create-recovery/{command_id}`
   (reuse `validate_conversation_id` / `validate_client_token`).
2. Route by id family in the handlers: `chatc-…` → scoped history for
   transcript + delete; `nyxid-chat-…` → canonical (unchanged). `get_state` for
   a `chatc-…` id returns a not-found-shaped `AppError` (no upstream state
   route for workflow rows; verify the FE never calls it for them and remove
   any such call).
3. `list_conversations`: replace canonical-first + single merge with a full
   drain of `GET api/scopes/{user_id}/chat-history` — follow `nextCursor`
   until absent, reject repeated cursors (loop guard), cap at 40 pages and log
   nothing to the client beyond the rows (document the cap in code), filter
   rows to the two supported id families, dedupe by id, sort newest-first by
   `updatedAt` (keep the existing sort-key helper). Drop
   `conversation_index_includes_workflow` / `merge_workflow_history_rows`
   machinery if no callers remain (FI-007).
4. Workflow delete normalization: scoped history DELETE answers bare 200 with
   an empty body — normalize the BE response to 204 so `apiClient` (which
   JSON-parses any non-204 success) does not fabricate a failure. Typed
   canonical DELETE keeps its 202 + JSON passthrough.
5. New route: `GET /api/v1/assistant/conversations/create-recovery/{commandId}`
   → scoped create-recovery passthrough (different segment arity than
   `/conversations/{id}`, so no route conflict — add a routes test anyway).
6. Rewrite the BE guard test (currently asserts canonical-only and forbids
   scoped strings): assert the family mapping above; keep forbidding
   `nyxid-chat/` scoped *command* strings and per-conversation command routes.

### W2 — Console-exact workflow turn body (backend)

`assistant_service::workflow_chat_body` + `WorkflowChatTurnRequest`:

1. Trim the prompt **first**, validate 1..=32768 on the trimmed value, and
   serialize the trimmed prompt.
2. `commandId` becomes create-only: reject (`BadRequest`) a request carrying
   both `conversationId` and `commandId`; stop generating one for
   continuations (continuation upstream body has **no** `commandId` member —
   upstream generates its own). Creates keep caller-supplied-or-generated
   `commandId` exactly as today.
3. Update the body unit tests to the console fixtures (§Tests).

### W3 — Console-exact turn body (frontend transport)

`frontend/src/lib/assistant/aevatar-transport.ts` `workflowTurnBody`:
continuation body = `{prompt, conversationId, minimumStateVersion, sessionId}`
(no `commandId`); create body unchanged
(`{prompt, commandId, sessionId}`). Remove the `minimumStateVersion` floor-of-1
(see W5).

### W4 — Console-parity retry + recovery (frontend transport, workflow protocol)

Scope: `protocol === "workflow"` runs only; the typed/actor path keeps its
current behavior.

1. **Continuations**: the only automatic POST retry is HTTP 503 whose parsed
   body code is `CHAT_HISTORY_RESERVATION_UNAVAILABLE` (reuse
   `streamStartError`'s envelope parsing). On it: re-read the transcript
   (BE scoped route from W1), wait until the returned `stateVersion` reaches
   the current fence, then retry with `minimumStateVersion` raised to
   `max(fence, refreshed)`; pacing 300/900 ms; two retries max; abort-aware.
   Any other status, network error, or post-acceptance truncation: **no
   auto-replay** — settle the run as failed with the parsed message and leave
   retry to the user (fresh turn). Delete the blanket
   `RETRYABLE_STREAM_STATUSES` replay loop for workflow runs.
2. **Creates**: a definitive pre-acceptance rejection (non-503-reservation
   HTTP error) fails the run, no auto-replay; a user retry of the same prompt
   reuses the same `commandId` (existing behavior — keep). When a create
   stream ends without a context frame (normal EOF, network truncation, or
   abort), poll the create-recovery route with 0/300/900/1800 ms backoff
   (404 → keep polling; other errors → stop), validate the returned identity
   with the existing context-frame guards (`chatc-` shape, scope, replay
   mismatch), adopt `{conversationId, stateVersion, turnId}`, and reconcile:
   trust the conversation only after a transcript read shows the turn's
   assistant message and a positive, non-regressing `stateVersion`. Recovery
   exhausted → fail closed exactly as today.

### W5 — No fabricated watermark; bounded pre-send reconciliation (frontend)

1. Remove the floor-of-1. A continuation may only be sent with a positive
   observed watermark (stream context or transcript read).
2. If a continuation is requested without one: **before** the optimistic
   message append, run a bounded transcript reconciliation (0/300/900/1800 ms;
   positive `stateVersion` required; when the previous turn id is known,
   require its assistant message present). Success → send; exhausted → surface
   a retryable "history is still synchronizing" failure. Retrying must not
   duplicate the local user message (append only after preflight passes).
3. Cover the create-context-zero window: context frames carry
   `stateVersion: 0` on creates (normal); first continuation right after a
   create must reconcile against the transcript rather than deadlock.

### W6 — Session id parity (frontend)

Re-mint `sessionId` (fresh UUID) whenever a conversation is materialized or
restored from server history (sidebar open, page reload); keep it stable across
turns and within-conversation retries while the conversation stays active —
matching the console. Update the lazy-mint helper accordingly.

### W7 — Resource papercuts (frontend)

1. Assistant resource GETs send `Accept: application/json` (add in the
   assistant API helper in `aevatar-transport.ts`, not the global `apiClient`).
2. Delete flow tolerates the W1 204 (no body decode).
3. Resource-path error tests include Aevatar-shaped `{code, message}` bodies
   and empty 404 bodies, not only NyxID envelopes (fix handling where a test
   exposes loss of the symbolic code that a flow depends on; do not build a
   general error-translation layer).

### W8 — Documentation

1. `docs/assistant-network-flows.md`: correct the "after" table — resources
   are family-mapped (typed → canonical, workflow → scoped history), list is a
   drained shared index, delete semantics differ per family; add the
   create-recovery row; note continuation bodies carry no `commandId`.
2. `docs/chat-canonical-api-migration.md`: prepend a short correction note
   (workflow-row resources + wizard-graph claim) pointing to the audit v2. Do
   not rewrite the historical brief.

## Tests (land with their work items)

Backend (`cargo test`):
- Handler-level upstream-stub tests per id family: list drains 51+ mixed rows
  across 2+ cursor pages (assert both families present, order, dedupe, loop
  guard, page cap), transcript paths, state (typed ok; `chatc-` → not-found
  without any upstream call), delete (typed 202 JSON passthrough; workflow
  bare-200-empty → 204), create-recovery passthrough + command-id validation.
- `workflow_chat_body`: byte-exact create and continuation fixtures derived
  from the console's `chatApi.test.ts` (create includes `commandId`;
  continuation has exactly `{conversation{conversationId,minimumStateVersion},
  prompt, sessionId, workflow}` and **no other keys**); trim-then-validate
  boundary case (32768 chars + surrounding whitespace); `commandId` +
  `conversationId` rejected.
- Rewritten guard test per W1.6.

Frontend (`npx vitest run`):
- Turn-body assertions: exact JSON key sets for create and continuation
  (unknown-member paranoia); no `commandId` on continuation.
- Stream-start matrix for workflow runs: named reservation 503 (refresh →
  raised fence → success), reservation 503 with refresh returning
  below-fence (waits, then succeeds), other 503, 500, empty body, malformed
  body, refresh 404/5xx, network error (no replay), abort mid-retry.
- Create recovery: normal EOF without context, truncation, abort → polling
  (404 then success), identity-mismatch rejection, turn-not-yet-in-transcript
  reconciliation, recovery exhausted → fail closed.
- Watermark: floor removed; pre-send reconciliation (success, exhausted,
  no duplicate optimistic message on retry); create-context-zero window.
- Session remint on restore; stability across turns.
- Delete accepts 204; resource errors with Aevatar-shaped and empty bodies.
- Existing suites stay green — in particular
  `use-assistant.aevatar.test.tsx`, the audit probes, and the canonical
  command guard. Update fixtures/mocks (`transport.ts` mock, e2e helpers) only
  as far as the contract changes require.

## Gates (run all before handoff)

```bash
cargo test
cd frontend && npm run build && npx vitest run && npm run lint
cargo test -p nyxid-cli --test wizard_bundle_freshness
```

Wizard: `frontend/src/lib/assistant/**` is not in the committed wizard graph —
no rebuild for transport-only changes. If you touch a manifest member
(`frontend/src/lib/api-client.ts`, assistant stores, `schemas/assistant-wire-log.ts`),
run `npm --prefix frontend run build:wizard` once and commit `cli/src/wizard/`
in the final commit. Do not touch `lib/api-client.ts` unless W7 forces it —
prefer local changes in the transport helper.

## Acceptance

1. BE tests prove every assistant request maps to an upstream route that exists
   on Aevatar origin/dev @ `bbd906eb5`, with family-correct paths and the exact
   console turn bodies.
2. FE tests prove console-parity behaviors: single named-503 retry with fence
   refresh, create-recovery polling + reconciliation, no fabricated watermark,
   no continuation `commandId`, session remint, pagination-driven list intact.
3. All gates green; tree clean; conventional commits telling the W1–W8 story.
