# Chat Parity Audit — NyxID vs Aevatar Console (origin/dev)

Owner: Calvin · Lead/final review: Fable · Implementer: GPT SOL · Adversaries: GPT SOL (findings), Opus (implementation)
Date: 2026-07-31 · Status: findings + required changes (spec follows adversarial review)

Goal (Calvin's directive): NyxID FE → NyxID BE → Aevatar must execute chat **the same way
Aevatar's own frontend does** — same endpoint, same parameters, for starting a new chat and
continuing a conversation. Not their UI, not their state management: the network contract.

Sources of truth read for this audit (all verified today):

- Aevatar console FE: `aevatarAI/aevatar` **origin/dev** @ `bbd906eb5` (2026-07-31 18:07 +0800),
  `apps/aevatar-console-web/src/pages/chat/{chatApi,chatHistoryApi,chatTypes,index}.ts[x]`,
  `src/shared/auth/fetch.ts`, `src/shared/agui/sseFrameNormalizer.ts`, plus `chatApi.test.ts`.
- Aevatar backend (same commit): `src/Aevatar.Mainnet.Host.Api/Chat/MainnetChatEndpoints.cs`,
  `src/workflow/.../ChatCapabilityModels.cs` (`HttpChatInput`), `ChatRunRequestNormalizer.cs`,
  `WorkflowChatRunInteractionService.cs`, `ChatRunStartErrorMapper.cs`,
  `src/Aevatar.Studio.Hosting/Controllers/ChatHistoryController.cs`,
  `agents/Aevatar.GAgents.NyxidChat/NyxIdChatEndpoints.cs`, `StudioWorkspaceGAgent.cs`.
- NyxID: this repo @ origin/main `2ffeb54e` (rollup-chat-20260731),
  `frontend/src/lib/assistant/aevatar-transport.ts`, `chat-stream.worker.ts`,
  `chat-stream-worker-client.ts`, `backend/src/handlers/assistant.rs`,
  `backend/src/services/assistant_service.rs`, and the rollup docs
  (`docs/chat-canonical-api-migration.md`, `docs/assistant-network-flows.md`).

---

## 1. Observed behavior — how the Aevatar console FE talks to its backend

### 1.1 One command endpoint: `POST /api/chat`

Headers: `Content-Type: application/json`, `Accept: text/event-stream`,
`Authorization: Bearer <access token>`. Response is an SSE stream (`data:` lines of JSON;
a `[DONE]` sentinel is ignored; malformed frames are skipped, not fatal).

**New chat** (`chatApi.ts startChatStream`, asserted by `chatApi.test.ts`):

```json
{
  "commandId": "<uuid>",
  "conversation": { "conversationId": null },
  "prompt": "<trimmed>",
  "sessionId": "<uuid>",
  "workflow": "studio"
}
```

**Continuation:**

```json
{
  "conversation": { "conversationId": "<id>", "minimumStateVersion": N },
  "prompt": "<trimmed>",
  "sessionId": "<uuid>",
  "workflow": "studio"
}
```

Rules the console enforces client-side:

- `workflow: "studio"` always; **no `type` member ever** (see §1.4 for why that matters).
- `commandId` is sent **only on creates**. It is the create's idempotency identity: a retry
  of the same prompt reuses the same `commandId`; a different prompt after a failed create
  mints a new one (`index.tsx runChat`).
- `conversation` is exactly `{conversationId: null}` XOR
  `{conversationId, minimumStateVersion (integer > 0)}` — extra keys are rejected before
  sending (`INVALID_CONVERSATION_INPUT`).
- `sessionId` is a client UUID; fresh per draft conversation and re-minted when a
  conversation is reopened from history. Stable across delivery retries of one create.
- All strings trimmed; undefined members omitted (`compactObject`).

### 1.2 Conversation identity comes back in-stream

The first meaningful frame of a turn is `custom` name `aevatar.chat.context` with payload
`@type = type.googleapis.com/aevatar.workflow.runs.WorkflowChatContextPayload` carrying
`{conversationId, scopeId, stateVersion, turnId}`. The console:

- adopts `conversationId` from it on creates,
- tracks `stateVersion` from it (input to the next turn's `minimumStateVersion`),
- rejects mid-stream identity changes, cross-scope contexts, and (on continuations) a
  conversation id different from the one requested.

### 1.3 Resource family: `/api/scopes/{scopeId}/chat-history/**` (`chatHistoryApi.ts`)

| Purpose | Call |
|---|---|
| List conversations (sidebar) | `GET /api/scopes/{scopeId}/chat-history` (cursor paging via `?cursor=`) |
| Open conversation (transcript + fence) | `GET /api/scopes/{scopeId}/chat-history/conversations/{id}` → `{messages, stateVersion}` |
| Delete | `DELETE /api/scopes/{scopeId}/chat-history/conversations/{id}` |
| Create recovery | `GET /api/scopes/{scopeId}/chat-history/create-recovery/{commandId}` → `{conversationId, stateVersion, status, turnId}` |

`minimumStateVersion` for a continuation is the `stateVersion` from the transcript read or
the last stream context — the console refuses to send a continuation until it has a real
watermark ("history is still synchronizing" + reconcile), never a made-up floor.

### 1.4 Error recovery behaviors

- **Reservation retry** (`startChatStreamWithHistoryRefreshRetry`): a continuation failing
  with HTTP 503 + code `CHAT_HISTORY_RESERVATION_UNAVAILABLE` is retried after 300 ms /
  900 ms; before each retry the console re-reads the conversation detail and raises
  `minimumStateVersion` to `max(current, refreshed)`, retrying only once the projection
  catches up. Creates are never retried this way.
- **Create recovery**: if a create stream completes without ever delivering the
  `aevatar.chat.context` frame, the console calls the create-recovery endpoint with the
  `commandId` to recover `{conversationId, turnId, stateVersion}` instead of failing.

### 1.5 Upstream ground truth (Aevatar C# on the same commit)

- `POST /api/chat` classifies on **presence of `type`** (`MainnetChatEndpoints`):
  `type` ∈ {`text`, `action.continue`, `approval.resolve`, `task.stop`, `task.steer`,
  `step.retry`, `step.skip`} → typed NyxIdChat assistant; **no `type`** → workflow engine
  (the only branch that reads `workflow`/`conversation`/`commandId`/`sessionId`);
  anything else → 400. The two surfaces are mutually exclusive per turn.
- `HttpChatInput` is `JsonUnmappedMemberHandling.Disallow` — unknown members are rejected.
  `CommandId` is documented as "idempotency identity for retryable HTTP chat **create**
  requests"; a body `scopeId` is ignored (trusted caller scope wins).
- The create-replay fingerprint (`WorkflowChatCreateRequestFingerprint`) applies to
  **creates only**; continuation delivery is idempotency-keyed on
  `chat-history-delivery-{actorId}-{commandId}` with an **empty** fingerprint
  (`WorkflowChatRunInteractionService`). A continuation retry may therefore change
  `minimumStateVersion` without risking the 409 `IdempotencyConflict`.
- The reservation fence fails when the projection's `currentVersion <
  minimumStateVersion` (`StudioWorkspaceGAgent`) → 503 `CHAT_HISTORY_RESERVATION_UNAVAILABLE`.
  A *stale-low* watermark passes; a watermark ahead of the projection blocks until the
  projection catches up.
- Resource routes that exist upstream (mounted in the Mainnet host via
  `AddStudioCapability()` / NyxIdChat endpoints):
  - `/api/scopes/{scopeId}/chat-history` + `/conversations/{id}` (GET/DELETE) +
    `/create-recovery/{commandId}` — the studio/workflow family (shared read model; it is
    also where typed `nyxid-chat-…` transcripts were always read from).
  - `/api/scopes/{scopeId}/nyxid-chat/conversations` (POST create, GET list,
    `{actorId}:stream`, `{actorId}/state`, `{actorId}:approve`, DELETE) — the typed family.
- **`/api/chat/conversations/**` does not exist.** Searched every remote branch of
  `aevatarAI/aevatar` (including `dev` and `feature/integrate`): zero route definitions.

---

## 2. Observed behavior — NyxID today (origin/main `2ffeb54e`)

### 2.1 What already matches (keep)

- **Turn execution for new chats matches the console.** Since #1301, a new chat posts
  FE → `POST /api/v1/assistant/workflow-chat` with
  `{prompt, conversationId?, minimumStateVersion?, commandId, sessionId}`; the backend
  rebuilds the strict upstream body
  `{commandId, conversation, prompt, workflow:"studio", sessionId}` with `workflow` pinned
  server-side and `conversation` always present (`assistant_service::workflow_chat_body`).
  Same body family the console sends; `deny_unknown_fields` both directions.
- Headers match: FE fetch sends `Accept: text/event-stream` + JSON content type
  (`chat-stream.worker.ts` / worker client); auth is the session cookie → identity/delegation
  headers injected at the pass-through (equivalent to the console's Bearer; Aevatar derives
  scope from the trusted identity either way).
- SSE consumption is equivalent: `data:`-line parsing, JSON frames, oneof/typed/flat frame
  tolerance, `aevatar.chat.context` handling with the same identity guards (#1301 restored
  fail-closed create adoption + replay-mismatch rejection).
- `sessionId` is a stable per-conversation UUID (minted lazily, reused across turns). This
  satisfies the real upstream constraint (stable within one create's retries — it is part of
  the create fingerprint). The console re-mints per reopen; ours persists — same fingerprint
  behavior where it matters, deliberate small deviation (§3.7).
- Typed NyxIdChat conversations (`nyxid-chat-…`, action/connect cards) still speak
  `POST /api/chat` with the 7 `type:` commands — that family exists upstream and is disjoint
  from the studio surface by design. Out of scope for console parity (the console cannot
  express it), unaffected by the changes below except its resource reads (§3.2).

### 2.2 Where we diverge from the console — and from upstream reality

**D1 (breaks chat): conversation resources target a phantom family.**
The rollup remapped list/transcript/state/delete to `/api/chat/conversations/**`
("canonical", copied from the eanz17/nyxid-chat reference client). That family exists in
no branch of the Aevatar repo. The earlier "verified" prod 401s do not establish existence
(auth gates reject before routing). Against a dev-shaped deployment:

- `GET /api/v1/assistant/conversations` → upstream 404/401 → handler returns the error
  (the legacy `chat-history` merge only runs after a *successful* canonical response), so
  the sidebar is empty/erroring.
- `GET /api/v1/assistant/conversations/{id}` (transcript), `DELETE .../{id}`,
  `GET .../{id}/state` → phantom path, no fallback → **reopening a conversation never
  yields a transcript or a `stateVersion`, so continuations of reopened conversations are
  impossible** (the FE refuses or falls to the floor hack, §D4).
- Guard tests on both sides (`canonical-command-guard.test.ts`; the
  `assistant_service.rs` legacy-marker test) currently **enforce** the phantom family —
  the guards encode the wrong invariant and would fight the fix.

**D2 (console behavior we lack): no create recovery.**
A create stream that ends without `aevatar.chat.context` fails closed (#1301). The console
recovers identity via `GET .../chat-history/create-recovery/{commandId}`. Our delivery
retries (same frozen body + `commandId`) do reuse the server-side replay receipt, but the
"stream completed, context never arrived" case ends as a user-visible error where the
console recovers silently. The upstream endpoint exists and is keyed exactly for this.

**D3 (console behavior we lack): reservation-unavailable retry never refreshes the fence.**
On 503 we blindly replay the identical frozen body (503 is merely in
`RETRYABLE_STREAM_STATUSES`). The console re-reads the conversation and waits for the
projection to reach the watermark before retrying (300/900 ms). Upstream analysis (§1.5)
shows the frozen-body constraint is real **only for creates**; continuations may refresh
freely. Blind replay converges only by luck of retry spacing, retries only twice with no
projection re-check, and burns delivery attempts shared with network failures.

**D4 (correctness): fabricated watermark floor.**
`workflowTurnBody` sends `minimumStateVersion: 1` when no watermark was observed. That
neuters the read-your-writes fence (any projection state ≥ 1 passes) — the turn can be
built on a projection that lacks the user's previous turns. The console never fabricates a
watermark; it blocks the send and reconciles first. (Directly tied to D1: the transcript
read that should supply the real watermark is the broken phantom route.)

**D5 (exactness): continuation bodies carry `commandId`; console creates-only.**
Upstream treats a continuation `commandId` as a delivery-idempotency key (valid, unused by
the console). Keeping it is a functional superset that protects against duplicated turns on
delivery retries; dropping it is exact console parity. Recommendation: **keep**, and record
the deviation here (adversarial reviewers: challenge this).

**D6 (exactness): prompt sent untrimmed.**
`workflow_chat_body` validates the trimmed prompt but forwards `request.prompt` verbatim;
the console trims. Upstream trims inside the create fingerprint, so replay is safe either
way — align by trimming at the backend rebuild for byte-parity.

---

## 3. Changes NEEDED

Ordered by severity. N1 is the defect; the rest are console-parity behaviors.

**N1. Re-point conversation resources at the real upstream family.**
Backend (`assistant_service.rs` path builders + `handlers/assistant.rs`):
- List: `GET api/scopes/{user_id}/chat-history` as the primary source (it is the shared
  read model covering both `chatc-…` and `nyxid-chat-…` rows — exactly what pre-#1279 main
  consumed and what the console consumes). Drop the canonical-first fetch + merge machinery.
- Transcript: `GET api/scopes/{user_id}/chat-history/conversations/{id}` (both families).
- Delete: `DELETE api/scopes/{user_id}/chat-history/conversations/{id}` for `chatc-…` rows
  (console single-delete); typed `nyxid-chat-…` rows restore the actor delete +
  history-row dual-delete that #1279-era main had (the actor is a live resource the
  history DELETE does not reap).
- State: `chatc-…` rows have **no** state route upstream — the reconnect story is the
  transcript read (console behavior). Typed rows keep a state read only via the real
  `api/scopes/{user_id}/nyxid-chat/conversations/{actorId}/state`.
- Rewrite both guard tests to enforce THIS mapping (fail on `api/chat/conversations`
  reappearing; allow the scopes families they currently forbid).
- FE: no URL changes (NyxID-relative URLs stay); update transport/mock fixtures and tests
  that assert upstream echo paths.

**N2. Create recovery (console parity).**
Backend: passthrough `GET /api/v1/assistant/conversations/create-recovery/{commandId}` →
`api/scopes/{user_id}/chat-history/create-recovery/{commandId}` (control-identity
validation on `commandId`). FE: when a workflow create stream completes without a context
frame, call it once before failing closed; adopt `{conversationId, stateVersion, turnId}`
on success with the same identity guards (#1301's fail-closed stays for the
recovery-also-missing case).

**N3. Reservation-aware continuation retry (console parity).**
FE transport: on 503 whose body carries `CHAT_HISTORY_RESERVATION_UNAVAILABLE` for a
**continuation**, re-read the transcript (N1's real route) and retry once the returned
`stateVersion` ≥ the fence, with the console's 300/900 ms pacing, raising
`minimumStateVersion` to the refreshed value. Creates keep the frozen-body replay
(fingerprint constraint). Requires the BE error passthrough to preserve the upstream error
`code` on stream-start failures (verify; the FE currently surfaces status only).

**N4. No fabricated watermark.**
Remove the `minimumStateVersion` floor-of-1. A continuation without an observed watermark
first reads the transcript (N1) to obtain `stateVersion`; if that read fails, surface the
console's "history is still synchronizing" state rather than sending a fake fence.

**N5. Byte-parity trims.**
Trim `prompt` in `workflow_chat_body` (and keep FE trims as-is). Decision on D5
(`commandId` on continuations): keep + document, unless adversarial review overturns.

**N6. Documentation corrections.**
`docs/chat-canonical-api-migration.md` + `docs/assistant-network-flows.md` present
`/api/chat/conversations/**` as verified-canonical. Add a correction note pointing here
(do not rewrite history); this file records the verified dev-branch contract.

Explicitly NOT in scope: copying console UI/state logic; the typed card surface's command
envelope (already correct against upstream); the workflow WS twin (unused by the page);
`llmModel`/`llmRoute` (console no longer sends them on chat bodies).

## 4. Verification plan (for the implementation round)

- BE tests: every assistant resource/command maps to an upstream path that exists in
  `aevatarAI/aevatar` origin/dev (`/api/chat`, `api/scopes/{uid}/chat-history/**`,
  `api/scopes/{uid}/nyxid-chat/conversations/**`); rebuilt workflow bodies byte-match the
  console shape (create + continuation fixtures from `chatApi.test.ts`); guard tests
  updated per N1.
- FE tests: transport emits console-shaped FE bodies; create-recovery path; reservation
  retry with watermark refresh; no-fabricated-fence behavior; existing round-1 card
  semantics untouched (`use-assistant.aevatar.test.tsx` suite stays green).
- Suites: `cargo test`; frontend `npm run build` + `npx vitest run` + `npm run lint`;
  e2e specs (`npm run test:e2e`) unaffected or updated.
- Live capture against a real Aevatar deployment remains blocked on a fresh login
  (tokens rotated-dead) — explicitly listed as an open item in the PR.
