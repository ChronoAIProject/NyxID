# Chat Parity Audit v2 — NyxID vs Aevatar Console (origin/dev)

Owner: Calvin · Lead/final review: Fable · Findings adversary: GPT SOL (review:
`docs/chat-aevatar-dev-parity-audit-review-gpt-sol.md`) · Date: 2026-07-31
Status: verified findings. Implementation spec: `docs/chat-aevatar-dev-parity-spec.md`.

v1 of this audit claimed `/api/chat/conversations/**` exists on no Aevatar branch
("phantom family"). **That was wrong** — the routes live in
`agents/Aevatar.GAgents.NyxidChat/NyxIdChatPublicEndpoints.cs` (v1 searched only
`src/`) and are mounted in the Mainnet host. The GPT SOL adversarial review caught
it; every correction below was independently re-verified by Fable against the
pinned revisions. The defect is real but different: **family confusion**, not
route nonexistence.

Goal (Calvin's directive): NyxID FE → NyxID BE → Aevatar must execute chat the
same way Aevatar's own frontend does — same endpoints, same parameters, for new
chats and continuations. Not their UI or state logic: the network contract.

Pinned sources: Aevatar `aevatarAI/aevatar` origin/dev @ `bbd906eb5`
(2026-07-31); NyxID origin/main @ `2ffeb54e` (rollup-chat-20260731).

---

## 1. Upstream ground truth (Aevatar origin/dev)

### 1.1 One command endpoint, two engines

`POST /api/chat` (`MainnetChatEndpoints.cs`) classifies on the **presence of
`type`**: a JSON body whose `type` is one of `text`, `action.continue`,
`approval.resolve`, `task.stop`, `task.steer`, `step.retry`, `step.skip` goes to
the typed NyxIdChat assistant; a body with **no `type`** goes to the workflow
engine (the only branch that reads `workflow`/`conversation`/`commandId`/
`sessionId`); anything else is a 400. Multipart always selects workflow. The two
surfaces are mutually exclusive per turn. `HttpChatInput` rejects unknown members
(`JsonUnmappedMemberHandling.Disallow`); a body `scopeId` is ignored (trusted
scope wins).

### 1.2 Resource families that exist upstream (all verified mounted)

| Family | Routes | Serves |
|---|---|---|
| Canonical typed (`NyxIdChatPublicEndpoints.cs`) | `GET /api/chat/conversations`, `GET /api/chat/conversations/{id}`, `GET .../{id}/state`, `DELETE .../{id}` | `nyxid-chat-…` rows only. List reads the **shared** history index then filters to the typed `ServiceKind` (returning the raw page's cursor — pages can arrive sparse). Detail admission-checks the id as a typed actor, then returns `{messages, stateVersion}`. DELETE runs the composite lifecycle (actor + history) and answers **202 + JSON**. |
| Scoped studio (`ChatHistoryController.cs`, mounted via `AddStudioCapability()`) | `GET /api/scopes/{scopeId}/chat-history` (`?pageSize&cursor`), `GET .../conversations/{id}`, `DELETE .../conversations/{id}`, `GET .../create-recovery/{commandId}` | The shared read model — **all** service kinds, both `chatc-…` workflow rows and `nyxid-chat-…` typed rows (typed writers initialize/save into it: `NyxIdChatConversationGAgent`, `NyxIdChatGAgent`). Detail returns `{messages, stateVersion}`. DELETE removes only the history row and answers **bare 200**. |
| Scoped typed (`NyxIdChatEndpoints.cs`) | `/api/scopes/{scopeId}/nyxid-chat/conversations/**` (create, list, `:stream`, `/state`, `:approve`, DELETE) | Legacy typed surface; not needed for this parity work. |

Consequences that matter for us:
- A `chatc-…` id passed to the canonical typed detail/state/delete fails
  admission (typed actor registry lookup → not found). Family routing must be
  id-prefix-aware.
- Typed canonical DELETE must not be replaced by manual actor+history dual
  deletes — the composite lifecycle endpoint owns that.
- Both list surfaces default to 50 rows and page by cursor.

### 1.3 Workflow-turn mechanics (creates and continuations)

- The create-replay fingerprint (`WorkflowChatCreateRequestFingerprint`: prompt,
  conversation, session, scope, inputs, …) applies to **creates only**;
  continuation reservations use an empty fingerprint keyed
  `chat-history-delivery-{actorId}-{commandId}`. If the client omits
  `commandId`, the server generates one.
- Continuation replay is **not** safe to assume: every continuation reservation
  mints a fresh `TurnId`, and a delivery actor accepts a repeated reservation
  only when all fields including `TurnId` match
  (`ChatTurnHistoryDeliveryGAgent`). A replayed continuation after an
  accepted-but-truncated stream can conflict rather than deduplicate.
- Continuation admission (`ProjectionChatConversationContinuationAdmissionReader`):
  rejects a nonpositive `minimumStateVersion`; not-ready when the projected
  `StateVersion < minimumStateVersion` **or the projection has zero messages**.
  Not-ready → HTTP 503 body `{code: "CHAT_HISTORY_RESERVATION_UNAVAILABLE", …}`
  (`ChatRunStartErrorMapper`). A stale-low watermark passes; a watermark ahead
  of the projection blocks until the projection catches up.
- A create's in-stream context frame carries `stateVersion: 0` (no prior
  context) — zero is a normal value, positivity is only required when
  *continuing*.

## 2. Observed behavior — how the console FE actually calls chat

From `apps/aevatar-console-web/src/pages/chat/` (`chatApi.ts`,
`chatHistoryApi.ts`, `index.tsx`), confirmed against `chatApi.test.ts`.

### 2.1 Turn bodies

`POST /api/chat`, headers `Content-Type: application/json`,
`Accept: text/event-stream`, `Authorization: Bearer <token>`.

- New chat: `{commandId, conversation: {conversationId: null}, prompt,
  sessionId, workflow: "studio"}`.
- Continuation: `{conversation: {conversationId, minimumStateVersion > 0},
  prompt, sessionId, workflow: "studio"}` — **no `commandId`**.
- All strings trimmed; undefined members omitted; `conversation` is strictly one
  of the two shapes (extra keys rejected client-side); no `type` member ever;
  `llmModel`/`llmRoute` are not sent by the chat page (Studio-only).
- `commandId` is the create's idempotency identity: a retry of the same failed
  create prompt reuses it; a different prompt mints a new one.
- `sessionId` is a client UUID: fresh per draft, **re-minted each time a
  conversation is reopened from history**, stable across turns/retries while
  the conversation stays active.

### 2.2 Streams and identity

SSE `data:` frames of JSON; multi-line `data:` joined per event; CRLF tolerated;
`[DONE]` ignored; malformed frames skipped. The `custom aevatar.chat.context`
frame (`WorkflowChatContextPayload`: `conversationId, scopeId, stateVersion,
turnId`) is accepted at `stateVersion >= 0`, adopted once, and guarded: scope
mismatch, mid-stream identity change, or (on continuation) a different
conversation id are hard errors.

### 2.3 Resources

Sidebar list: `GET /api/scopes/{scopeId}/chat-history`, **following every
`nextCursor`** (loop-detecting). Transcript: `GET .../conversations/{id}` →
`{messages, stateVersion}` — the continuation fence comes from here or from
stream context; the console never fabricates one (no watermark → send blocked,
"history is still synchronizing" + bounded reconciliation). Delete:
`DELETE .../conversations/{id}`. History calls send `Accept: application/json`.

### 2.4 Recovery behaviors

- **Reservation retry**: continuation start failing with HTTP 503 + body code
  `CHAT_HISTORY_RESERVATION_UNAVAILABLE` → re-read the conversation detail,
  wait until its `stateVersion` reaches the fence, retry with
  `minimumStateVersion` raised to the max; pacing 300/900 ms; two retries.
  This is the **only** automatic POST retry the console performs.
- **Create recovery**: when a create stream ends without a context frame
  (normal EOF, ambiguous error, or abort), poll
  `GET .../chat-history/create-recovery/{commandId}` with 0/300/900/1800 ms
  backoff (404 → keep polling), adopt the returned
  `{conversationId, stateVersion, turnId}` with the same identity guards, then
  reconcile: the conversation is trusted only once the transcript shows the
  turn's assistant message and a positive, non-regressing state version.
- **No blind replays**: definitive HTTP errors surface to the user; an
  ambiguous continuation failure asks the user to reload before continuing
  (never auto-replayed — see the fresh-`TurnId` conflict mechanics in §1.3).

## 3. Observed behavior — NyxID today (origin/main 2ffeb54e)

### 3.1 What already matches (keep)

- **Turn bodies for the studio surface match.** Since #1301, new chats and
  continuations post FE → `POST /api/v1/assistant/workflow-chat`
  (`{prompt, conversationId?, minimumStateVersion?, commandId, sessionId}`);
  the BE rebuilds the strict upstream body with `workflow:"studio"` pinned and
  `conversation` always present (`assistant_service::workflow_chat_body`).
  Same family the console sends; `deny_unknown_fields` both directions.
- Content negotiation matches (`Accept: text/event-stream` + JSON). Auth is
  semantically equivalent, not byte-identical: session cookie → identity/
  delegation headers at the pass-through vs the console's Bearer; Aevatar
  derives scope from the trusted identity either way.
- SSE consumption is equivalent (including multi-line `data:`, `[DONE]`,
  malformed-frame skip) and context-frame guards match (#1301 restored
  fail-closed create adoption + replay-mismatch rejection).
- The BE **does** preserve upstream error status + body through the
  pass-through, and the FE worker client delivers non-OK bodies to
  `streamStartError`, which already parses `{code, message}` — the 503 code is
  available today (v1 wrongly listed a BE change here).
- Typed NyxIdChat conversations (`nyxid-chat-…`, cards) on `POST /api/chat`
  with the 7 `type:` commands and typed canonical resources: correct against
  upstream. The workflow WS twin is not used by the page — matches the console.

### 3.2 Divergences (verified defects)

**C1 — family confusion on conversation resources (P1, breaks new chats).**
The BE routes *every* conversation id through the canonical typed family:
transcript/state/delete build `api/chat/conversations/{id}` unconditionally
(`handlers/assistant.rs` `get_history`/`get_state`/`delete_conversation`). The
canonical family admission-rejects `chatc-…` workflow rows (§1.2) — and since
#1301 **every new conversation is a `chatc-…` row**. Reopening, continuing
after reopen, and deleting any new chat fails against a dev-shaped upstream.
The list only works because a successful canonical response triggers a legacy
merge; a non-success canonical response is returned as-is (no fallback).

**C2 — list pagination truncates (P2).** The console drains every cursor page.
Our BE issues one canonical GET (+ at most one scoped GET when merging) and the
FE ignores cursors entirely — history silently truncates at the ~50-row default,
made worse by the canonical list filtering *after* pagination (sparse pages).

**C3 — no create recovery (P2).** A create stream ending without a context
frame fails closed (#1301) where the console polls create-recovery
(0/300/900/1800 ms) across normal EOF, ambiguous failure, and abort, then
reconciles via transcript. Our substitute — frozen-body auto-replay of the
create — is not console behavior and should be replaced by recovery.

**C4 — retry policy is broader than the console and unsafe for continuations
(P2).** We blind-replay all of [408, 425, 429, 500, 502, 503, 504] plus network
errors, creates and continuations alike, with a frozen body. The console
auto-retries only the named reservation 503 for continuations (with a
refreshed fence — ours never refreshes) and never auto-replays an ambiguous
continuation (fresh-`TurnId` conflict risk, §1.3).

**C5 — fabricated watermark (P2).** `workflowTurnBody` sends
`minimumStateVersion: 1` when no watermark was observed, neutering the
read-your-writes fence. The console blocks the send and reconciles. Note the
create-context-zero window (§1.3): immediately after a create there may be no
positive watermark anywhere — the fix needs bounded pre-send reconciliation,
not just deleting the floor (else first-continuation-after-create stalls).

**C6 — continuation bodies carry `commandId`.** The console omits it; the FE
sends `run.clientRequestId` and the BE *generates one* when absent. Given §1.3
(replay of a continuation `commandId` can conflict rather than deduplicate,
and its idempotency value presumes a replay contract upstream doesn't provide),
v1's "keep it" recommendation is withdrawn: **drop `commandId` from
continuations at both layers; keep it for creates.**

**C7 — prompt trim/validation mismatch.** The BE validates the *untrimmed*
prompt's length and serializes it untrimmed; the console trims before sending.
Trim first, then validate length, then serialize the trimmed prompt.

**C8 — workflow delete response shape breaks the FE client.** The scoped
history DELETE answers bare 200 with an empty body; our `apiClient` JSON-parses
every non-204 success — deletion would succeed upstream and still surface as an
error. Normalize to 204 at the BE (or accept empty 2xx bodies in the helper).

**C9 — exactness papercuts.** History reads don't send
`Accept: application/json`; resource-path error handling loses Aevatar's
symbolic `code` (NyxID envelope assumed); `sessionId` is persisted across
reopen where the console re-mints it.

### 3.3 Not defects (v1 claims corrected by adversarial review)

- `/api/chat/conversations/**` exists and is mounted upstream (v1's central
  claim — wrong, `src/`-only search).
- No BE change needed to preserve the 503 error code (already passes through).
- `frontend/src/lib/assistant/**` is **not** in the committed wizard bundle
  graph — no wizard rebuild for transport-only changes (rebuild only if a real
  manifest member changes, e.g. `lib/api-client.ts` or assistant stores;
  `docs/chat-canonical-api-migration.md` overstates this).
- The FE canonical-command guard does not enforce the resource family (only
  forbids old per-conversation command URLs); the BE guard does and must be
  rewritten family-aware.

## 4. Changes needed

Carried into `docs/chat-aevatar-dev-parity-spec.md` as work items with tests;
summary: family-aware resource routing with full cursor drain (C1, C2);
console-parity create recovery (C3); console-parity retry policy with fence
refresh (C4); no fabricated watermark + bounded pre-send reconciliation (C5);
drop continuation `commandId` FE+BE (C6); trim-then-validate prompt (C7);
delete response normalization (C8); Accept-header/error-code/sessionId
papercuts (C9); guard-test rewrite and upstream-stub test coverage; docs
corrections. Live prod capture stays an explicit open item (tokens
rotated-dead); repo evidence is dev-shaped, production composition UNPROVEN.
