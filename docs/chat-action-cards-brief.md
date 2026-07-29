# Chat Action Cards — Implementation Brief (v1)

Owner: Calvin · PM: Fable · Status: ready for implementation
Branch: `feat-chat-rich-text-cards` · Scope: **frontend only** (`frontend/src`), zero backend changes.

## 1. What we are building

The assistant chat must render **action request cards**: when Aevatar needs the user to do
something only the browser may do (connect a service, hand over a credential), it emits an
AG-UI CUSTOM frame named `nyxid.action.request`. The chat renders a rich card with a CTA;
the CTA opens the existing multistep connect journey (modal); when the user finishes or
declines, the frontend posts an `action.continue` turn back on the stream endpoint and the
assistant resumes.

Reference contract doc: Calvin's "NyxID ↔ Aevatar — Action Contract, Schema version 3"
(2026-07-24). **Where that doc and the live Aevatar implementation disagree, the live
implementation wins.** The live implementation is on the `origin/dev` branch of the repo at
`~/Desktop/aelf-frontend-work/aevatar` — the facts below were read from it on 2026-07-29
(head `020a9acd2`, includes PR #2911 `feature/integrate`). Key files if you need to
re-verify (read-only; never modify that repo):

- `agents/Aevatar.GAgents.NyxidChat/NyxIdChatEndpoints.Streaming.cs` — stream endpoint + DTOs
- `agents/Aevatar.GAgents.NyxidChat/NyxIdChatBrowserActions.cs` — action/continuation state machine
- `agents/Aevatar.GAgents.NyxidChat/NyxIdChatConversationAguiFrameBuilder.cs` — frame builder
- `agents/Aevatar.GAgents.NyxidChat/protos/nyxid_chat_task.proto` — wire payload messages
- `test/Aevatar.AI.Tests/NyxIdChatActionContinuationEndpointsTests.cs` — endpoint tests

## 2. The authoritative wire contract (verified against Aevatar origin/dev)

### 2.1 Inbound: the `nyxid.action.request` CUSTOM frame

SSE frame shape (protobuf `JsonFormatter.Default` → lowerCamelCase; fields at default
values — empty strings/arrays — are **omitted**):

```json
{
  "type": "CUSTOM",
  "sequence": 42,
  "custom": {
    "name": "nyxid.action.request",
    "payload": {
      "schemaVersion": 4,
      "actorId": "<conversation actor id>",
      "originTurnId": "turn-<hex>",
      "taskId": "task-…",
      "stepId": "step-…",
      "actionRequestId": "act-…",
      "action": "service.connect",
      "params": {
        "catalogService": {
          "serviceSlug": "api-github",
          "requestedScopes": ["repo"],
          "viaNodeId": "…",
          "targetOrgId": "…"
        }
      }
    }
  }
}
```

- `schemaVersion` is **4** (`NyxIdAssistantActionRegistry.SupportedSchemaVersion`). Treat any
  other value as unsupported (render the fallback card, §5.4).
- In schema v4 Aevatar emits **only `action: "service.connect"`**, with `params` being a
  oneof: `catalogService` (`NyxIdCatalogServiceConnectParams`) **or** `customService`
  (`NyxIdCustomServiceConnectParams { name, endpointUrl, authMethod, authKeyName, viaNodeId,
  targetOrgId }`). Design the frontend registry so more verbs slot in later, but only
  implement these two journeys.
- The same `actionRequestId` can be **re-emitted idempotently** (server `CommitRequest` is
  idempotent). Dedupe by `actionRequestId` exactly like `promptedApprovalIds` does for
  approvals.
- Aevatar also emits other new CUSTOM names (`nyxid.task.snapshot`, `nyxid.task.step.changed`,
  `nyxid.control.changed`, `nyxid.continuation.changed`, `nyxid.step.control.changed`).
  These must continue to be **silently ignored** (today's `default: return` already does
  this — do not break that).
- After emitting the action request, the server marks the origin turn **Blocked and
  terminal** — the SSE stream ends with a normal terminal frame shortly after the card
  frame. The card must remain interactive **after** the run finishes (unlike approvals,
  there is no live stream waiting on it).

### 2.2 Outbound: the discriminated `/stream` body

`POST /api/v1/assistant/conversations/{id}/stream` (NyxID backend forwards the body
byte-for-byte to Aevatar; no NyxID backend change needed). Aevatar's
`NyxIdChatStreamRequest` has `JsonUnmappedMemberHandling.Disallow` — **any member not
listed below → HTTP 400 with an empty body**. Exact members (camelCase JSON):
`type`, `prompt`, `inputParts`, `clientRequestId`, `originTurnId`, `actions`, `sessionId`
(deprecated, do not send).

Text turn (this replaces today's flat `{prompt, clientRequestId}` body — that shape now
400s against the new contract; fixing it is **in scope**):

```json
{ "type": "text", "prompt": "…", "clientRequestId": "…" }
```
Constraints for `type:"text"`: `prompt` (or `inputParts`) required; `originTurnId` must be
absent/blank; `actions` must be absent/empty.

Action continue turn:

```json
{
  "type": "action.continue",
  "clientRequestId": "<stable per submission>",
  "originTurnId": "turn-<hex of the turn that emitted the card(s)>",
  "actions": [
    {
      "actionRequestId": "act-…",
      "originTurnId": "turn-<same as top-level>",
      "disposition": "completed",
      "resource": { "userService": { "userServiceId": "<created UserService uuid>" } }
    }
  ]
}
```

Validated server-side (violations → 400 or a rejected continuation):

- `prompt` and `inputParts` must be absent for `action.continue`.
- `originTurnId` (top-level) is **required**; every report's `originTurnId` must equal it
  ordinally. A single continue can only report actions from **one** origin turn.
- `actions` must be **non-empty** (the spec doc's "trigger 2: empty actions array" is NOT
  supported by the live server — never send an empty array).
- No duplicate `actionRequestId` within one continue.
- `disposition` ∈ `completed | declined | failed | cancelled | expired`. v1 uses:
  `completed` (journey succeeded), `declined` (user explicitly said no), `failed` (journey
  errored out). Do not send `cancelled`/`expired` in v1.
- `resource` is optional; when present, **exactly one** variant:
  `userService{userServiceId}` | `key{keyId}` | `node{nodeId}` |
  `serviceAccount{serviceAccountId}` | `developerApp{clientId}` | `device{deviceId}`.
  IDs only — never a secret, token, or URL. For a completed `service.connect`, send the
  created/connected `userService` id when the journey exposes it; omit `resource` if it
  doesn't.
- All ids (`clientRequestId`, `originTurnId`, `actionRequestId`, resource ids) must satisfy
  the control-identity rule: 1–256 chars, no whitespace, no control chars, none of
  `/ \ ? #`. A UUID or the server-issued ids satisfy this.
- Idempotency: replaying the **same** `clientRequestId` with the same reports is an
  idempotent no-op/replay. A retry after a network failure must reuse the same
  `clientRequestId` (same rule the existing text-turn retry follows).
- If another turn is active server-side, the continuation is **rejected** with reason code
  `NYXID_ACTION_CONTINUATION_ACTIVE_TURN` ("Another conversation turn is active."). The
  client must therefore never fire a continue while a local turn is streaming — queue it
  and send when the active turn reaches a terminal state. Treat a rejected continuation as
  "report still unsent": keep it queued and retry once the conversation is idle (do not
  mark the card errored; do not resend while a turn is active).
- Response is SSE, same as a text turn: the continuation **starts a new turn** that streams
  the assistant's follow-up. Run it through the exact same stream-consumption path as
  `sendMessage` (`streamTurn`), including retry budget, watchdog and cursor handling.
- **Rejection signalling (corrected 2026-07-29 after adversarial review).** The admission
  reason codes (`NYXID_ACTION_CONTINUATION_{INVALID,CONFLICT,ACTIVE_TURN}`) **never appear
  on the continuation stream**: a rejected admission is published as a
  `nyxid.continuation.changed` CUSTOM frame on the *origin* turn's session
  (`NyxIdChatProjectionSession` gates `BuildContinuationChanged` on
  `admission.OriginTurnId == context.SessionId`, and the continuation stream's session is
  the continuation turn id). The client only ever observes a generic terminal
  (`STREAM_TIMEOUT`, `PROJECTION_UNAVAILABLE`, `ACTOR_NOT_FOUND`, `COMMAND_START_FAILED`)
  or a stall that trips the local watchdog. Client rule: a report batch is settled only by
  a real terminal (`RUN_FINISHED` / `RUN_STOPPED`, or reaching an approval gate — proof
  the continuation turn ran); **any** error terminal or stall requeues the batch under the
  same `clientRequestId` for retry at next idle. An earlier revision of this section
  implied the reason codes arrive as run-error codes — that was wrong.

### 2.3 Deployment note (put in the PR description verbatim)

The discriminated body (`type:"text"`) is **required** by Aevatar ≥ `feature/integrate`
(PR aevatarAI/aevatar#2911) and **rejected** (unknown member) by the currently deployed
prod Aevatar. This branch must deploy **after** the Aevatar dev contract reaches prod.
Same for the whole action-card feature.

## 3. Current frontend anatomy (verified file map)

All paths relative to `frontend/src`.

| Concern | Location |
|---|---|
| CUSTOM frame dispatch — the insertion point | `lib/assistant/aevatar-transport.ts` `handleCustomFrame` (~`:1578-1649`); unknown names hit `default: return` at `:1646` |
| Card-creation precedents | `addApprovalCard` (~`:1900`), `addConnectCard` (`:1967`), both via `appendActivityBlock` (`:1750`) + `ensureActivityMessage` |
| Dedupe precedents | `promptedApprovalIds` (Set), `promptedConnectSlugs` (Map slug→block_id with in-place `block.updated` upgrade) |
| Stream POST body | `aevatar-transport.ts` `streamTurn` (~`:1100-1182`, fetch at `:1116-1131`) — flat `{prompt, clientRequestId}` today, hand-rolled fetch (SSE), `STREAM_DELIVERY_ATTEMPTS = 2` |
| Continuation precedent (approve) | `decideApproval` flow (~`:792-952`): reserves the conversation **before** the fetch, continues cursor from `turnState.lastCursor`, sniffs `text/event-stream` vs JSON ack |
| Turn/block reducer | `lib/assistant/stream.ts:85-179`; terminal mapping `toTerminalBlock` `:19-54` — new block type needs a case or it hangs pending on cancel |
| Open-card finalization | `openCards: Map<string,"approval"\|"connect">` (`:279`) + `finalizeActivity` (`:2271`) — action cards must **NOT** be terminal-ized at turn end; they stay pending after the run finishes |
| Block type union | `types/assistant.ts:102-107` (`ContentBlock`), all-readonly interfaces with `type` + `block_id` |
| Render switch | `components/assistant/chat-thread.tsx` `renderBlock` `:42-70`; callbacks threaded from `pages/assistant.tsx` → `ChatThread` props |
| Existing card components | `components/assistant/blocks/{connect-card,approval-card,run-card,text-block,artifact-block}.tsx` |
| The multistep modal to reuse | `components/dashboard/add-key-dialog.tsx` (`AddKeyDialog`, props `open/onOpenChange/prefillSlug/reconnectKey`, `WizardStep` machine: catalog→routing→form→node_setup→oauth_credentials→oauth→device_code→verify). `blocks/connect-card.tsx:124-133` shows exactly how a chat block opens it |
| Transport selection / mock | `lib/assistant/transport.ts:202-224` (mock vs live) — extend the mock so the feature is demoable and testable without Aevatar |
| History load | `historyEntryToMessage` (`aevatar-transport.ts:382-402`) — history is text-only; cards are not persisted/replayed (accepted v1 limitation, see §7) |
| Tests | colocated vitest: `aevatar-transport.test.ts`, `stream.test.ts`, `chat-thread.test.tsx`, `blocks/*.test.tsx`, fixtures in `lib/assistant/__fixtures__/` |

## 4. Deliverables

1. **Schema + types** — `schemas/assistant-actions.ts` (zod, colocated `.test.ts`): parse
   and validate the `nyxid.action.request` payload (tolerating omitted-default fields) and
   build/validate the `action.continue` body. New `ActionCardContentBlock` in
   `types/assistant.ts` (readonly, `type: "action_card"`, `block_id`, plus: action verb,
   `action_request_id`, `origin_turn_id`, parsed params, card status:
   `pending | in_progress | completed | declined | failed | unsupported`, and a
   human-readable outcome note).
2. **Frontend action registry** — `lib/assistant/action-registry.ts`: a descriptor map
   `verb → { copy (title/body/cta builders from params), risk, journey binding }`. v1
   entries: `service.connect` (catalog variant, custom variant). Anything else — including
   unknown verbs and wrong `schemaVersion` — resolves to the unsupported descriptor. NyxID
   owns all copy; never render model-supplied text as the consent copy.
3. **Transport work** (`aevatar-transport.ts`):
   - `handleCustomFrame` case for `nyxid.action.request` → validate → dedupe by
     `actionRequestId` → `appendActivityBlock` an `action_card` block.
   - Discriminated stream bodies: `{type:"text", …}` for `sendMessage`, and a new
     `continueActions(conversationId, originTurnId, reports)` transport method posting
     `{type:"action.continue", …}` and consuming the SSE continuation exactly like a turn
     (reserve conversation, cursor continuity, retry with the same `clientRequestId`,
     content-type sniff like `decideApproval`).
   - Pending-report queue: card resolutions accumulate per `originTurnId`; a continue fires
     immediately when the conversation is idle, otherwise queues until the active turn
     terminates. Batch all unsent reports that share an `originTurnId` into one POST; never
     mix origin turns in one body; never send an empty `actions`.
   - Lifecycle: action cards survive turn end (exempt from `finalizeActivity` expiry);
     `block.updated` drives status transitions; `toTerminalBlock` gets an `action_card`
     case (a still-`pending` card stays pending — it must not be zombified by cancel).
4. **UI** — `components/assistant/blocks/action-card.tsx` (+ test): rich text card —
   title, NyxID-owned consent copy, params summary (service name/slug, requested scopes as
   chips, org/node when present), risk framing, primary CTA ("Connect GitHub…"), secondary
   **Decline**. States: pending (CTA + decline), in_progress (modal open / continue
   in-flight), completed / declined / failed (receipt row, no CTA), unsupported (explanatory
   copy + Decline only). Wire into `renderBlock` + thread the callback through
   `chat-thread.tsx` / `pages/assistant.tsx` the same way `onDecideApproval` is threaded.
   Follow DESIGN.md and match the visual language of `connect-card.tsx` / `approval-card.tsx`
   (the live app is the visual source of truth; don't invent new tokens).
5. **The multistep journey** — CTA opens `AddKeyDialog`:
   - catalog variant → `prefillSlug={serviceSlug}`; verify how the dialog reports success
     and what identifiers it exposes (the connect-card precedent + `/keys` hooks); on
     success send `completed` + `resource.userService.userServiceId` when obtainable,
     else `completed` without `resource`.
   - custom variant → the dialog's custom-endpoint path prefilled with
     `name/endpointUrl/authMethod/authKeyName`; if a param doesn't map cleanly, prefill
     what maps and leave the rest to the wizard (never silently drop the whole journey).
   - `requestedScopes`: display on the card; pass into the dialog only if it already
     supports scope input — do not build new scope plumbing in v1 (note the gap in the PR).
   - Modal dismissed without finishing → card returns to `pending` (dismiss ≠ decline).
     Decline is only the explicit card button. Journey hard-failure → `failed` with a
     retryable CTA? No — v1: `failed` is terminal on the card; the user can ask the
     assistant to try again (server re-emits idempotently). Keep it simple.
6. **Mock transport** — emit a `nyxid.action.request` frame in the mock so the card +
   modal + continue round-trip is demoable locally and testable end-to-end.
7. **Tests** (vitest, colocated; mirror existing patterns):
   - schema: payload parse (omitted defaults, both param variants, bad schemaVersion),
     continue-body construction rules (§2.2 constraints incl. control-identity charset).
   - transport: frame → block creation + dedupe + re-emission upgrade; continue POST body
     exactness (no extra members!); queue-while-active-turn; retry reuses `clientRequestId`;
     rejected continuation keeps reports queued; text body now carries `type:"text"`.
   - reducer: `action_card` terminal mapping.
   - components: card states render + CTA/decline callbacks; renderBlock dispatch.
8. **Docs** — update `docs/ASSISTANT_STREAM_ALIGNMENT.md` (or nearest assistant doc) with
   the action-card frame, the discriminated bodies, and the §2.3 deployment gate. Keep this
   brief in the repo as `docs/chat-action-cards-brief.md`.

## 5. Hard rules

1. **Never** put a secret, token, credential, or full URL-with-credentials in a card,
   a continue body, a log line, or a test fixture beyond obviously-fake values.
2. Aevatar's DTO **rejects unknown members** — construct outbound bodies from explicit
   object literals, not spreads of larger objects. Add a test asserting the exact key set.
3. Do not break existing flows: approvals, connect-card (`nyxid.authorization.required`),
   keepalive, unknown-frame tolerance. The transport test file is the regression net.
4. No `console.log`; follow `frontend/src` conventions (kebab-case files, readonly types,
   zod in `schemas/`, TanStack Query in `hooks/` if any new fetch — there shouldn't be).
5. CI gate is `npm run build` (tsc -b with `noUncheckedIndexedAccess`) — run it, plus
   `npm run test` and `npm run lint`, all from `frontend/`, all green before you're done.
6. Frontend only. No `backend/`, `cli/`, `sdk/`, `mobile/` changes. No new npm deps.
7. Conventional commits on this branch (`feat(assistant): …`); commit locally, do not push.

## 6. Acceptance walkthrough (the demo that must work against the mock)

1. User asks for something needing GitHub → assistant text streams → `nyxid.action.request`
   (catalogService `api-github`, scopes `["repo"]`) arrives → turn finishes → card renders
   with GitHub copy + scope chip + Connect CTA + Decline, and stays interactive.
2. CTA → AddKeyDialog multistep opens prefilled → user completes → card flips to
   `completed` receipt → exactly one `POST …/stream` with
   `{type:"action.continue", clientRequestId, originTurnId, actions:[{actionRequestId,
   originTurnId, disposition:"completed", …}]}` → continuation SSE streams the assistant's
   follow-up into the thread.
3. Decline instead → one continue with `disposition:"declined"`, no modal.
4. Card arrives while user is mid-turn typing/streaming → resolution queues; continue fires
   only when idle.
5. Duplicate frame re-emission → single card (no dupes).
6. Unknown verb / wrong schemaVersion → unsupported card; Decline works.

## 7. Explicit non-goals (v1)

- No backend action registry (`GET /api/v1/assistant/actions`) — separate task.
- No NyxID backend `/stream` validation changes (passthrough already forwards the body).
- No card persistence/rehydration across reload: history is text-only today. A reloaded,
  stuck conversation self-heals because the user's next text turn leads Aevatar to
  re-emit the pending action idempotently. Document this in the PR.
- No standing-grant / remember-me integration; no V2 verb surface; no `admin.open`.
- No `cancelled`/`expired` dispositions; no scope-widening UI beyond what AddKeyDialog has.
