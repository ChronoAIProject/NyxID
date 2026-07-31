# Chat Canonical `/api/chat` Migration — Round-2 Implementation Brief

Owner: Calvin · PM/final review: Fable · Implementer: Codex · Adversary: Opus
Date: 2026-07-31 · Status: ready for implementation
Prereq reading: `docs/chat-action-envelope-v4-audit.md` (round 1, all landed on this branch).

## 0. Why round 2

Rich cards still don't work properly on latest main. Root causes, established by audit:

1. Main's #1279 devloop routes only the **first turn of a new chat** through Aevatar's
   canonical `POST /api/chat` (backend `typed_chat_path()`, FE `TYPED_CHAT_URL`,
   `typed-create` protocol, RUN_STARTED actor adoption). **Every subsequent turn,
   action continuation, approval, control, state read, transcript read, and delete
   still uses the deprecated scoped `api/scopes/{user_id}/nyxid-chat/**` +
   `chat-history` family.** The reference client (eanz17/nyxid-chat) migrated 100%
   of assistant traffic to `POST /api/chat` + `/api/chat/conversations/**` on
   2026-07-29 and demonstrably gets rich cards on prod through that path.
   Confirmed: prod Aevatar answers 401 (auth-gated, deployed) on both canonical
   routes.
2. None of round 1's envelope hardening is on main (this branch is unpushed): main
   still parses `nyxid:connect` fences from prose, silently rewrites cards on
   re-emission, shows success theater, and can send `completed` without a
   `userService` resource.
3. Main moved 10 commits under us (#1279, #1281 Web-Worker stream transport,
   #1285 reconcile, #1287 canonical-id continuity) touching 12 of our 21 files.

Round 2 = **Phase A** reconcile this branch with origin/main, then **Phase B**
finish the canonical migration so our whole assistant surface speaks `/api/chat`
exactly like the reference.

## 1. Facts about main you must respect (verified 2026-07-31)

- Backend: `routes.rs:1336` mounts `POST /api/v1/assistant/chat` →
  `handlers::assistant::typed_chat` → `assistant_service::typed_chat_path()` =
  `"api/chat"`. `TypedChatTurnRequest` is `{type:"text", prompt, clientRequestId}`
  only, `deny_unknown_fields`, 256 KiB cap, control-identity charset on
  `clientRequestId`, prompt ≤ 32768 chars. Proxy test asserts downstream
  `/api/chat`, no caller Authorization, `x-nyxid-identity-token` +
  `x-nyxid-delegation-token` present.
- FE: `createConversation()` mints local `nyxid-pending-<uuid>` (no server call).
  `sendMessage` picks protocol: `workflow` (`chatc-…`) | `typed-create`
  (`nyxid-pending-…` → posts `TYPED_CHAT_URL`) | `actor` (posts the old
  per-conversation `/stream`). RUN_STARTED adoption validates
  `/^nyxid-chat-[A-Za-z0-9_-]{1,117}$/`, re-keys the conversation, and registers
  an alias; #1287 adds canonical-id URL continuity, `AssistantConversationNotFoundError`,
  cancel/delete/stop fence addressing via aliases.
- Main added `wakeActions` (`actions: []` wake envelope, `actionWakeBodySchema`,
  required on the `AssistantTransport` interface) + a producer-contract smoke
  script (`npm run test:producer-contract`).
- #1281 replaced inline SSE consumption with a module Worker:
  `chat-stream.worker.ts`, `chat-stream-worker-client.ts`, `chat-stream-parser.ts`;
  `consumeTurnStream(stream: ChatStreamRequestHandle)`; `handleAgUiFrame` takes a
  parsed `ChatStreamFrame`. `decideApproval` / `streamActionContinuation` use
  `startChatStream`.
- Main did NOT touch `handleCustomFrame`/`addActionCard`/`action-registry`/
  `blocks/action-card.tsx`, and `connect-fence.ts` is still alive + imported there.
- Wizard: assistant frontend sources are wizard sources — #1279 shipped a wizard
  bundle rebuild. Our branch MUST run `npm --prefix frontend run build:wizard` and
  commit the `cli/src/wizard/` output once, in the final commit of this round
  (do not blanket-rebuild anything else).

## 2. Phase A — reconcile with origin/main

Merge `origin/main` into this branch (do not rebase; preserve the reviewed round-1
commits). Known collision map (from a hunk-level audit — verify while resolving):

- `aevatar-transport.ts`: 3 textual conflicts (import block; `createConversation`
  region; not-found error region). Ours deletes the `connect-fence` import and its
  `textToBlocks` usage — main still has both; our deletion wins and
  `connect-fence.ts` + `connect-fence.test.ts` stay deleted. Main's three big
  stream-plumbing hunk replacements (`streamTurn`, `streamActionContinuation`,
  `consumeTurnStream` → Worker client) do not textually overlap our hunks — take
  main's plumbing wholesale and re-verify our behavior on top of it.
- `types/assistant.ts`: both sides added required transport methods — keep BOTH
  `wakeActions` (main) and `blockActionCard` (ours); both transports implement both.
- `schemas/assistant-actions.ts`: main inserted `actionWakeBodySchema` /
  `buildActionWakeBody` where we inserted the secret-scan/lookup helpers — keep both.
- `chat-thread.tsx` / `pages/assistant.tsx`: main rewrote regions our small hunks
  sit inside (`groupMessages`/`ChatThread` bodies; the page wiring) — re-apply our
  `onBlockAction` threading and callback wiring into main's rewritten bodies by hand.
- Test files (`aevatar-transport.test.ts` ±1431 ours vs +557 main's,
  `assistant-actions.test.ts`, `chat-thread.test.tsx`, `pages/assistant.test.tsx`):
  re-apply, don't line-merge. Our round-1 behavioral tests are the contract —
  every one of them must exist and pass after reconciliation, ported to main's
  Worker-based stream doubles (`ChatStreamRequestHandle` with `.headers` /
  `.completion` / `.cancel`) where they previously mocked `fetch`/`Response`.
- Semantic checks after merge: card fingerprints must survive the typed-create
  RUN_STARTED re-key (the adoption re-keys the same stored object — verify with a
  test: card emitted on a `typed-create` first turn, then conflict detection still
  works after adoption); `wakeActions` must be unaffected by card state (it carries
  no reports); re-arm/`blocked`/`conflicted` semantics unchanged.

Phase A exit: full frontend suite green (`npm run build`, `npm run test`,
`npm run lint`), backend untouched so far, all round-1 acceptance tests passing on
top of main's plumbing. Commit Phase A separately (`merge` + any follow-up
`test(assistant): port round-1 suites to worker stream transport`).

## 3. Phase B — full canonical migration (mirror the reference)

Target: for typed NyxIdChat conversations, 100% of traffic uses
`POST {aevatar}/api/chat` and `{aevatar}/api/chat/conversations/**` through our
existing authenticated pass-through (identity/delegation injection unchanged).
The Workflow surface (`chatc-…`, `workflow-chat`, its ws) stays exactly as-is —
same boundary the reference kept.

### B1. Backend — generalize `typed_chat` into the single command facade

`assistant_service.rs` / `handlers/assistant.rs`:

1. Replace the text-only `TypedChatTurnRequest` with a discriminated
   `AssistantChatCommand` (serde `deny_unknown_fields` per variant, tag = `type`):
   - `text`: `prompt` (or future inputParts; keep prompt-required for now),
     `clientRequestId`, optional `conversationId`.
   - `action.continue`: `clientRequestId`, optional `originTurnId` (required when
     `actions` non-empty; absent allowed for the `actions: []` wake),
     `conversationId` required, `actions` array (validate: ≤ some sane cap, per-report
     allowlist `{actionRequestId, originTurnId, disposition, resource?}`, the five
     dispositions, six single-variant resources, per-report originTurnId equality
     when non-empty, duplicate actionRequestId rejection — mirror the FE zod rules;
     reject secret-shaped values with the same regex family used in
     `frontend/src/schemas/assistant-actions.ts`).
   - `approval.resolve`: `conversationId`, `clientRequestId`, `requestId`,
     `approved: bool`, optional `reason` ≤ 2048.
   - `task.stop` | `task.steer` | `step.retry` | `step.skip`: forward the exact
     identity/version facts (`conversationId`, `turnId`, `stopRequestId`/`steeringId`/
     `retryRequestId`/`skipRequestId`, `clientRequestId`, `instruction` for steer,
     `taskId`/`stepId`/`expectedOperationGeneration` for step controls,
     `expectedStateVersion`).
   All identities validated with the existing control-identity rule. The upstream
   body is REBUILT from the validated struct (explicit fields, never echo the raw
   caller body). Set `Idempotency-Key: clientRequestId` on the upstream request.
2. Dispatch: `text` / `action.continue` / `approval.resolve` forward as SSE
   streams; the four controls forward as JSON (202 receipt passthrough) — exactly
   the reference's split (`server.mjs handleChat`).
3. Conversation resources, typed family only (`nyxid-chat-…` ids):
   - `GET  /api/v1/assistant/conversations/{id}` → `api/chat/conversations/{id}`
   - `GET  /api/v1/assistant/conversations/{id}/state[?afterStateVersion&turnId]`
     → `api/chat/conversations/{id}/state` (cursor passthrough)
   - `DELETE /api/v1/assistant/conversations/{id}` → single
     `api/chat/conversations/{id}` DELETE — **remove the dual-delete** (nyxid-chat
     actor + chat-history row) for typed ids.
   - `GET /api/v1/assistant/conversations` (list) → fetch canonical
     `api/chat/conversations` for typed rows; keep the legacy filtered
     `chat-history` fetch ONLY to preserve `chatc-…` workflow rows; merge
     newest-first. If the canonical list also returns workflow rows, drop the
     legacy fetch entirely (verify against the live response shape in tests with
     both fixtures).
   - Legacy scoped paths (`conversations_path`, `stream/approve/stop/steer/state/
     retry/skip` scoped builders, chat-history transcript/dual-delete, the
     creation **index-polling** helper) — delete the ones no callers use after
     this migration (FI-007: prefer deletion; `chatc-` keeps only what workflow
     still calls). `POST /conversations` (create) becomes typed-unused: keep the
     route returning the local-create contract the FE expects, or delete it if the
     FE no longer calls it at all — check `frontend` usage and pick deletion if dead.
4. Per-conversation COMMAND routes are DELETED, not remapped (Calvin directive
   2026-07-31: "ensure 5 is not used … exactly as ean has done, do not reinvent"):
   remove `/{id}/stream`, `/{id}/approve`, `/{id}/stop`, `/{id}/steer`, and
   `/{id}/turns/{turn}/steps/{step}/retry|skip` from `routes.rs` along with their
   handlers and scoped path builders. They are typed-only surfaces (workflow uses
   `/workflow-chat`; the standalone approvals feature uses `/api/v1/approvals`,
   unrelated) and every command now flows through the single
   `POST /api/v1/assistant/chat` facade with `conversationId` in the body —
   mirroring the reference exactly (`POST /api/demo/chat` = commands;
   `/api/demo/conversations/**` = resources). RESOURCE routes stay (list, detail,
   state, delete) with canonical upstream mapping per item 3.
5. Port the reference's migration guard (their `protocol.test.mjs` repository
   guard): add tests that read the runtime sources and fail on reintroduced
   legacy markers — frontend `src/lib/assistant/**` must not contain a
   per-conversation stream/approve/stop path construction (e.g. the substrings
   `}/stream` / `}/approve` / `}/stop` in URL templates); backend
   `assistant_service.rs` must not contain `nyxid-chat/` scoped path strings.
   `chat-history` remains allowed ONLY in the workflow (`chatc-`) list-merge code
   path.
5. Backend tests: extend the proxy handler test to assert, for each command and
   resource: downstream path (`/api/chat` or `/api/chat/conversations/...`),
   exact rebuilt body, `Idempotency-Key`, identity headers, no caller
   Authorization; and that no typed-conversation request ever produces a
   `api/scopes/…/nyxid-chat` or `chat-history` upstream path. `cargo test` green.

### B2. Frontend — collapse the split-brain protocol

1. `actor`-protocol turns move off `/conversations/{id}/stream` to
   `POST /api/v1/assistant/chat` with `{type:"text", conversationId, prompt,
   clientRequestId}` — one URL for `typed-create` (no conversationId) and `actor`
   (with conversationId). Workflow stays on `WORKFLOW_CHAT_URL`.
2. `streamActionContinuation` (reports + wake) posts the same
   `/api/v1/assistant/chat` with `conversationId` in the body (extend
   `actionContinueBodySchema`/`actionWakeBodySchema` + builders; still explicit
   literals, still frozen retry bodies with a stable `clientRequestId`).
3. `decideApproval` for typed conversations posts `{type:"approval.resolve",
   conversationId, clientRequestId, requestId, approved}` to
   `/api/v1/assistant/chat` (workflow approvals unchanged). Keep the JSON-ack vs
   SSE sniffing behavior on the response.
4. Stop for typed conversations sends the typed `task.stop` command (real
   `expectedStateVersion` is round-3 scope — keep sending the current value, but
   route it through the new command). History/state/delete/list hooks keep their
   NyxID URLs (backend remaps them).
5. Update mock transport + fixtures accordingly; every round-1 behavioral test
   still passes; new tests assert the exact outbound URL+body per command
   (explicit key-set assertions, unknown-member paranoia — Aevatar rejects
   unknown members).

### B3. Verification

- Gates: frontend `npm run build` / `test` / `lint`; backend `cargo test`;
  wizard rebuild committed once (see §1 last bullet).
- The producer-contract smoke (`npm run test:producer-contract`) stays working
  against the new URL surface (update the script if it hardcodes the old
  per-conversation stream path — it currently posts
  `/api/v1/assistant/conversations/{id}/stream`; point it at the equivalent
  canonical-command call).
- Mock end-to-end: new chat → first turn (typed-create) → card → journey →
  `action.continue` with `conversationId` → continuation stream renders. This is
  the acceptance demo.

## 4. Hard rules

Round-1 §5 rules stay binding, with these changes: backend changes are now IN
scope (`backend/src/services/assistant_service.rs`, `backend/src/handlers/
assistant.rs`, `routes.rs`, and their tests only — no other backend surface);
the wizard rebuild commit is REQUIRED this round; still no new deps, no
console.log, conventional commits, local only, no push. Reference repos remain
read-only. Do not touch the Workflow surface's behavior. Backend error handling
follows `AppError`/`AppResult` conventions; internal upstream details never leak
to clients (Critical Rule 3).

## 5. Acceptance

1. With the mock: full rich-card round trip works (§B3 demo), and all round-1
   acceptance bullets still hold.
2. Backend tests prove: every typed-conversation request maps to
   `/api/chat`-family upstream paths only; commands are rebuilt with exact
   allowlists; controls JSON-forward; streams SSE-forward; `Idempotency-Key` set.
   The per-conversation command routes no longer exist (requests to them 404/405),
   and the migration-guard tests (§B1.5) pass on both sides.
3. FE tests prove: one command URL for typed turns/continuations/approvals with
   exact bodies; workflow untouched; RUN_STARTED adoption + aliasing + round-1
   card semantics (conflict, re-arm, blocked, fingerprints) all still pass on the
   Worker transport.
4. `git log` shows: merge commit, ported-tests commit (if separate), backend
   migration commit(s), FE migration commit(s), wizard rebuild commit. Tree clean.
