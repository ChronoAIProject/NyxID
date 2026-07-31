# Assistant Page — Complete Network Flow

Owner: Calvin · Author: Fable (PM) · Date: 2026-07-31
Companion docs: `docs/chat-canonical-api-migration.md` (round-2 brief),
`docs/chat-action-envelope-v4-audit.md` (round-1 envelope audit),
`docs/ASSISTANT_STREAM_ALIGNMENT.md` (frame-level alignment).

Three views: the reference client's flow (the proven-working model), our flow
**before** this branch (origin/main — why cards break), and our flow **after**
this branch (the implemented target). Verified from code; a live prod capture
additionally requires a fresh login (all saved refresh tokens are rotated-dead).

## 1. Authentication chain (identical in every view of ours)

```text
Browser (HttpOnly session cookie; assistant routes are human-only —
         API keys / service accounts / delegated & relay tokens rejected)
  -> NyxID backend /api/v1/assistant/** (AuthUser from session)
     -> admin-managed `aevatar` service row (catalog config:
        identity_propagation_mode=jwt, aud urn:aevatar:api,
        inject_delegation_token=true, forward_access_token=false)
        -> upstream request carries X-NyxID-Identity-Token +
           X-NyxID-Delegation-Token; caller Authorization is never forwarded
           -> Aevatar derives the scope from the injected identity
```

The reference does the same through the public proxy
(`/api/v1/proxy/s/aevatar`); we do it through the assistant pass-through. Same
injection, same Aevatar-side scope derivation.

## 2. The reference flow (eanz17/nyxid-chat @ f1807da — the model)

Commands: ONE endpoint. Resources: one family. Nothing else.

| Browser → BFF | BFF → upstream (via NyxID proxy) | Notes |
|---|---|---|
| `POST /api/demo/chat` | `POST /api/chat` | 7 typed commands: `text`, `action.continue`, `approval.resolve`, `task.stop`, `task.steer`, `step.retry`, `step.skip`. SSE-forward for the first three; JSON 202 for controls. `Idempotency-Key: clientRequestId`. |
| `GET /api/demo/conversations` | `GET /api/chat/conversations` | list |
| `GET /api/demo/conversations/{id}` | `GET /api/chat/conversations/{id}` | transcript |
| `GET /api/demo/conversations/{id}/state` | `GET /api/chat/conversations/{id}/state` | `afterStateVersion`/`turnId` cursors |
| `DELETE /api/demo/conversations/{id}` | `DELETE /api/chat/conversations/{id}` | single delete |

Start/continue semantics (we now follow these exactly):
- First turn: `{type:"text", prompt, clientRequestId}` — **no** `conversationId`,
  no pre-create call, no polling. The stream's `RUN_STARTED` returns the
  authoritative `conversationId`/`turnId`; the client adopts it.
- Later turns: same body **plus** `conversationId`.
- Card journey: `nyxid.action.request` CUSTOM frame → validated card → journey →
  `{type:"action.continue", conversationId, clientRequestId, originTurnId,
  actions:[…]}` on the same command endpoint → new continuation turn streams.
- Wake: same envelope with `actions: []` and no `originTurnId`.
- A repo guard test fails their build if legacy scoped-path strings reappear.

## 3. Our flow BEFORE this branch (origin/main `b0ad31a2`) — why cards broke

| # | Page action | Browser → NyxID | Upstream | Family |
|---|---|---|---|---|
| 1 | List sidebar | `GET /api/v1/assistant/conversations` | `GET api/scopes/{uid}/chat-history` (filtered) | legacy |
| 2 | Open conversation | `GET …/conversations/{id}` | `GET api/scopes/{uid}/chat-history/conversations/{id}` | legacy |
| 3 | New chat | *(local `nyxid-pending-…` id; no network)* | — | — |
| 4 | First turn, new chat | `POST …/assistant/chat` `{type:"text",…}` | `POST /api/chat` (SSE) | **canonical** (#1279 only) |
| 5 | Every later turn | `POST …/conversations/{id}/stream` | `POST api/scopes/{uid}/nyxid-chat/conversations/{id}:stream` | **deprecated scoped** |
| 6 | Card continuation / wake | same `/stream` | same scoped `:stream` | deprecated |
| 7 | Tool approval | `POST …/{id}/approve` | scoped `:approve` | deprecated |
| 8 | Stop | `POST …/{id}/stop` | scoped `:stop` | deprecated |
| 9 | Delete | `DELETE …/conversations/{id}` | scoped actor delete **+** chat-history row (dual-delete) | deprecated |
| 10 | State | *(route existed; FE never called it)* | scoped `/state` | deprecated |
| W | Workflow chat (`chatc-…`) | `POST …/workflow-chat` (+ ws) | `POST /api/chat` (workflow body) | separate surface |

The split-brain: a card could arrive on turn 1 (row 4), but the entire journey
lifecycle (rows 5–8) ran on the deprecated family, and none of the round-1
envelope hardening existed on main.

## 4. Our flow AFTER this branch (implemented) — ean-parity

| # | Page action | Browser → NyxID | Upstream | Body |
|---|---|---|---|---|
| 1 | List sidebar | `GET /api/v1/assistant/conversations` | `GET /api/chat/conversations` (+ legacy `chat-history` restricted to `chatc-` workflow rows, merged) | — |
| 2 | Open conversation | `GET …/conversations/{id}` | `GET /api/chat/conversations/{id}` | — |
| 3 | New chat | *(local `nyxid-pending-…` id; no network)* | — | — |
| 4 | First turn | `POST /api/v1/assistant/chat` | `POST /api/chat` (SSE) | `{type:"text", prompt, clientRequestId}`; identity adopted from `RUN_STARTED` |
| 5 | Every later turn | **same** `POST …/assistant/chat` | **same** `POST /api/chat` (SSE) | `{type:"text", conversationId, prompt, clientRequestId}` |
| 6 | Card continuation | same | same (SSE) | `{type:"action.continue", conversationId, clientRequestId, originTurnId, actions:[…]}` |
| 6b | Wake | same | same (SSE) | `{type:"action.continue", conversationId, clientRequestId, actions:[]}` |
| 7 | Tool approval | same | same (SSE) | `{type:"approval.resolve", conversationId, clientRequestId, requestId, approved[, reason]}` |
| 8 | Stop (and future steer/retry/skip) | same | same (**JSON 202**) | `{type:"task.stop", conversationId, turnId, stopRequestId, clientRequestId, expectedStateVersion}` |
| 9 | Delete | `DELETE …/conversations/{id}` | `DELETE /api/chat/conversations/{id}` (single) | — |
| 10 | State | `GET …/conversations/{id}/state` | `GET /api/chat/conversations/{id}/state` | cursor passthrough |
| W | Workflow chat | unchanged | unchanged | unchanged |

Deleted outright (per Calvin 2026-07-31, matching ean's migration): the
per-conversation command routes `/{id}/stream`, `/{id}/approve`, `/{id}/stop`,
`/{id}/steer`, `/{id}/turns/{turn}/steps/{step}/retry|skip` — routes, handlers,
and every scoped `nyxid-chat/` upstream builder. Guard tests on both sides fail
the build if they come back:
- FE: `frontend/src/lib/assistant/canonical-command-guard.test.ts`
- BE: proxy-handler tests assert `/api/chat`-family upstream paths only and
  deleted-route unroutability.

`Idempotency-Key: clientRequestId` is set on all command forwards (required a
one-line addition to the proxy forward-header allowlist).

## 5. The frames that come back (unchanged by the migration)

The SSE stream vocabulary and card behavior are the round-1 surface:
`RUN_STARTED` (identity) → `TEXT_MESSAGE_*` (prose, always inert markdown) →
`CUSTOM nyxid.action.request` (schema-v4 validated → action card; fail-closed) →
terminal (`RUN_FINISHED` `completed|blocked` / `RUN_ERROR` / `RUN_STOPPED`).
Cards: dedupe + conflict-vs-first-commit by `actionRequestId`, re-arm of
`blocked` cards on matching re-emission, `completed` requires
`resource.userService.userServiceId`, post-report copy is "Reported — awaiting
assistant verification". Full detail: `docs/ASSISTANT_STREAM_ALIGNMENT.md`.

## 6. Open items after this branch

- Live prod network capture (needs one fresh login; tokens rotated-dead).
- Actor postcondition verification + card rehydration via `/state`
  (round-1 audit Phase 2).
- Real `expectedStateVersion` on stop + steer/retry/skip UI (availableActions
  gating).
