# Assistant Chat Wire Contract

Last verified against Aevatar `0a86713671fcf551dc19ad86b1b6aa8ae6cb980b` and the
production typed-chat probe (2026-08-11), with NyxID support-contract revision
`f45febb057a7182dab2495d4c739d2bb8d7026f5`.

This document specifies the browser-to-NyxID contract and the Aevatar request NyxID produces. It is the canonical API reference for the assistant chat surface.

## Conventions

All browser routes in this document are relative to `/api/v1/assistant`. They require a human NyxID session except `GET /api/v1/assistant/actions`, which is specified separately in [Actions registry](06-actions-registry.md).

NyxID derives the Aevatar scope from the verified `AuthUser.user_id`. A browser body never carries a user scope. All Aevatar paths below are relative to the active platform `aevatar` service row's `base_url`.

Conversation prefixes are protocol discriminators:

| Prefix | Meaning | Engine/resource family |
| --- | --- | --- |
| `nyxid-chat-` | durable typed conversation | NyxIdChat actor |
| `chatc-` | legacy historical conversation | scoped Chat History read/delete only |

Typed conversation IDs are exactly `nyxid-chat-{32 lowercase hex}`. An unknown,
malformed, or wrong-family ID is not guessed. Family-aware browser routes return
a not-found-shaped error after syntactic validation and make no upstream call.

## Browser API

| Browser call | Purpose | Upstream resource |
| --- | --- | --- |
| `GET /conversations` | fully materialized sidebar index | `GET /api/scopes/{userId}/chat-history`, cursor-drained |
| `GET /conversations/{id}` | transcript | typed or legacy historical detail, selected by ID |
| `DELETE /conversations/{id}` | durable delete | typed or legacy historical delete, selected by ID |
| `GET /conversations/{id}/state` | typed reconnect state | `GET /api/chat/conversations/{id}/state` |
| `POST /chat` | typed command | `POST /api/chat` with top-level `type` |

The browser sends every new and continuing turn through `POST /chat`. It may list,
read, and delete a legacy `chatc-...` row, but it has no legacy send, create
recovery, WebSocket, completion, local-placeholder, or fallback route.

## Typed NyxIdChat commands

`POST /chat` accepts a discriminated command allowlist. NyxID parses each command with unknown-field denial, rejects secret-shaped keys and values, validates control identities, and rebuilds the exact upstream object. It never spreads an arbitrary caller object into the Aevatar body.

Every typed command includes `clientRequestId`. NyxID copies that value into the outbound `Idempotency-Key` header. Text and action continuation request `Accept: text/event-stream`. Input, approval, and plan resolution plus stop, steer, retry, and skip request `Accept: application/json`.

The only accepted discriminators are `text`, `plan.resolve`, `input.resolve`,
`action.continue`, `approval.resolve`, `task.stop`, `task.steer`, `step.retry`,
and `step.skip`. An explicit unknown discriminator is a local `400`; it never
falls back to a body without `type`.

### `text`

New typed conversation:

```json
{
  "type": "text",
  "prompt": "message",
  "clientRequestId": "request-identity"
}
```

Typed continuation:

```json
{
  "type": "text",
  "prompt": "message",
  "clientRequestId": "request-identity",
  "conversationId": "nyxid-chat-f8369965a444433f92ec50e67ad8ee52"
}
```

The prompt must be nonblank after trimming. Its 32,768-character maximum is
measured on the original untrimmed string, and the submitted string is preserved
in the rebuilt body. A new conversation omits `conversationId`; a continuation
uses the exact adopted `nyxid-chat-...` identity.

Typed text does not support attachments. The composer has no attachment
affordance, and the strict NyxID DTO rejects attachment, `surface`, and
`inputParts` fields. Aevatar's StudioAssistant reference client strips its
console-only `surface` and `attachment` fields and maps an attachment to
`inputParts`; NyxID deliberately does not adopt that behaviour. Adding
`inputParts` requires a separate feature with its own schema, secret review,
and body-shape tests.

### `plan.resolve`

```json
{
  "type": "plan.resolve",
  "conversationId": "nyxid-chat-f8369965a444433f92ec50e67ad8ee52",
  "taskId": "task-identity",
  "planId": "plan-identity",
  "requestId": "plan-gate-identity",
  "planRevision": 1,
  "clientRequestId": "request-identity",
  "expectedStateVersion": 22,
  "confirmed": true
}
```

The browser may send this only from the exact pending actor-owned gate. It
copies every identity, revision, and fence from the current projection; the
human supplies the `confirmed` decision. It does not derive actor facts from a
transcript, URL, or previously rendered card. The
deployment currently reports an automatic satisfied gate, so the normal path
does not render a decision, but a conforming decoder and UI must retain the
explicit-gate shape for deployments that return one.

### `input.resolve`

```json
{
  "type": "input.resolve",
  "conversationId": "nyxid-chat-f8369965a444433f92ec50e67ad8ee52",
  "clientRequestId": "request-identity",
  "requestId": "input-identity",
  "answer": {
    "selectedOptionIds": ["option-a", "option-b"]
  },
  "expectedStateVersion": 22
}
```

`answer` is a closed union with exactly one of `freeText` or `selectedOptionIds`. Free text is trimmed, must remain nonblank, and is limited to 32,768 characters. A selection contains 1-6 distinct control identities. `expectedStateVersion` is required and must be positive. The browser reads it from the authoritative typed current-state envelope, verifies the exact pending input identity, and never derives it from AG-UI `sequence` (which is `progressSequence`). The command returns JSON transport acceptance; the matching `nyxid.input.changed` or current-state `latestInputResolution` proves commit.

### `action.continue`

```json
{
  "type": "action.continue",
  "conversationId": "nyxid-chat-f8369965a444433f92ec50e67ad8ee52",
  "clientRequestId": "request-identity",
  "originTurnId": "turn-identity",
  "actions": [
    {
      "actionRequestId": "action-identity",
      "originTurnId": "turn-identity",
      "disposition": "completed",
      "resource": {
        "userService": {
          "userServiceId": "service-identity"
        }
      }
    }
  ]
}
```

The report batch contains at most 64 entries. Action request IDs must be unique in the batch. When reports are present, the outer `originTurnId` is required and every report must match it. Allowed dispositions are `completed`, `declined`, `failed`, `cancelled`, and `expired`. The resource grammar is described in [Action cards](04-action-cards.md); every completed report requires exactly one allowlisted safe resource reference.

An empty `actions` array is a typed actor wake and may omit `originTurnId`.

### `approval.resolve`

```json
{
  "type": "approval.resolve",
  "conversationId": "nyxid-chat-f8369965a444433f92ec50e67ad8ee52",
  "clientRequestId": "request-identity",
  "requestId": "approval-identity",
  "approved": true,
  "reason": "optional trimmed reason",
  "expectedStateVersion": 22
}
```

An empty reason is omitted. A nonempty reason is trimmed and limited to 2,048 characters. `expectedStateVersion` is required and must be positive. The browser reads it from the authoritative typed current-state envelope and verifies the exact pending approval identity. This command returns JSON transport acceptance; the matching `nyxid.approval.changed` or current-state `latestApprovalResolution` proves commit.

### `plan.resolve`

```json
{
  "type": "plan.resolve",
  "conversationId": "nyxid-chat-f8369965a444433f92ec50e67ad8ee52",
  "taskId": "task-identity",
  "planId": "plan-identity",
  "requestId": "plan-gate-identity",
  "clientRequestId": "request-identity",
  "planRevision": 3,
  "confirmed": true,
  "expectedStateVersion": 23
}
```

All four plan and gate identities are required and `planRevision` plus `expectedStateVersion` must be positive. At click time the browser re-reads authoritative current state, requires an exact `confirm` + `pending` gate match, and submits the state version from that same read. A pending Stop fence is observed before this preflight. JSON 202 is dispatch acceptance only; the browser refreshes current state once and changes the card only when the actor-owned TaskPlan gate changes.

### `task.stop`

```json
{
  "type": "task.stop",
  "conversationId": "nyxid-chat-f8369965a444433f92ec50e67ad8ee52",
  "turnId": "turn-identity",
  "stopRequestId": "stop-identity",
  "clientRequestId": "request-identity",
  "expectedStateVersion": 22
}
```

The backend accepts a nonnegative `expectedStateVersion`, but the browser never submits zero. It first reads authoritative current state, requires a positive exact version and the matching active turn, and verifies that at least one current TaskPlan step offers `availableActions.stop`. This command returns JSON. The typed transport waits for any pending stop ordering fence before later turn or action delivery.

### `task.steer`

```json
{
  "type": "task.steer",
  "conversationId": "nyxid-chat-f8369965a444433f92ec50e67ad8ee52",
  "turnId": "turn-identity",
  "steeringId": "steering-identity",
  "clientRequestId": "request-identity",
  "instruction": "new direction",
  "expectedStateVersion": 22
}
```

The instruction must be nonblank after trimming. The preserved wire value is not normalized. The browser reads the current active turn and requires a positive exact `expectedStateVersion`; it never opens a competing text turn to steer active work. This command returns JSON.

### `step.retry` and `step.skip`

```json
{
  "type": "step.retry",
  "conversationId": "nyxid-chat-f8369965a444433f92ec50e67ad8ee52",
  "turnId": "turn-identity",
  "taskId": "task-identity",
  "stepId": "step-identity",
  "retryRequestId": "retry-identity",
  "clientRequestId": "request-identity",
  "expectedOperationGeneration": 3,
  "expectedStateVersion": 22
}
```

`step.skip` has the same fields except `type` is `step.skip` and `skipRequestId` replaces `retryRequestId`. The browser submits either command only when that exact step offers the corresponding `availableActions.retry` or `availableActions.skip`. It copies the positive `expectedOperationGeneration` from the step's current operation and a positive exact `expectedStateVersion` from the same current-state read. Both return JSON.

Typed command parsing and reconstruction are implemented by `backend/src/services/assistant_service.rs:parse_assistant_chat_command` and `prepare_assistant_chat_command`. Header enforcement is in `backend/src/handlers/assistant.rs:typed_chat`.

## Conversation index

`GET /conversations` does not expose upstream pagination to the browser. NyxID
drains two independent authorities:

```http
GET /api/chat/conversations?pageSize=50
GET /api/chat/conversations?pageSize=50&cursor={encodedCursor}
GET /api/scopes/{verifiedUserId}/chat-history
GET /api/scopes/{verifiedUserId}/chat-history?cursor={encodedCursor}
```

The canonical index contributes only structurally valid `nyxid-chat-*` rows.
The scoped history index contributes only `chatc-*` rows. A wrong-family row at
either source is ignored; the legacy index cannot become typed-list authority.

For each page NyxID:

1. requires an upstream success response;
2. buffers at most 4 MiB;
3. parses the `conversations` array;
4. keeps only IDs from that source's authoritative family;
5. deduplicates by ID, keeping the first occurrence; and
6. follows a nonblank string `nextCursor`.

The full drain is bounded by 40 pages and an 8 MiB aggregate body budget. A repeated cursor is an internal protocol error. A `nextCursor` that is neither absent, `null`, nor a string is an internal protocol error. Reaching the page or aggregate limit returns the rows already collected; the response does not invent a continuation cursor or a truncation flag.

If a later page has malformed JSON or lacks a `conversations` array, NyxID preserves rows collected from prior pages. The same mixed-deployment posture on the first page returns an empty successful index using the first upstream response metadata. Non-success upstream status is forwarded.

After draining, NyxID sorts the retained rows newest first. It recognizes `updatedAt`, `updated_at`, `lastMessageAt`, `last_message_at`, `createdAt`, then `created_at`; parseable RFC 3339 values are compared chronologically, with raw string fallback. The browser receives:

```json
{
  "conversations": []
}
```

No upstream cursor remains. Implementation: `backend/src/handlers/assistant.rs:list_conversations` and `backend/src/services/assistant_service.rs:append_conversation_family_page`.

## Transcript detail

`GET /conversations/{id}` switches on the identifier family:

| ID | Aevatar request |
| --- | --- |
| `nyxid-chat-...` | `GET /api/chat/conversations/{id}` |
| `chatc-...` | `GET /api/scopes/{verifiedUserId}/chat-history/conversations/{id}` |

NyxID forwards the body without parsing or reshaping it. The frontend accepts
the supported Aevatar history forms:

```json
[
  { "id": "...", "role": "user", "content": "..." }
]
```

and:

```json
{
  "messages": [
    { "id": "...", "role": "user", "content": "..." }
  ],
  "stateVersion": 18
}
```

Any other transcript shape fails closed rather than being treated as an empty
conversation. A transcript supplies persisted messages only. For a typed
conversation it cannot replace the actor projection, including pending input,
approval, action, control, task, or continuation facts. For a `chatc-...`
conversation it is historical display data only; the composer remains disabled.

## Delete

`DELETE /conversations/{id}` reserves deletion in the browser transport so a
concurrent typed send, action report, approval, or late stream cannot resurrect
the conversation.

| ID | Aevatar request | NyxID success response |
| --- | --- | --- |
| `nyxid-chat-...` | `DELETE /api/chat/conversations/{id}` | preserve upstream response, including `202` JSON acceptance |
| `chatc-...` | `DELETE /api/scopes/{verifiedUserId}/chat-history/conversations/{id}` | normalize every upstream success to `204` with an empty body |

Non-success responses are preserved. Legacy normalization removes
`Content-Length` and `Content-Type`. The frontend uses a 15-second request
deadline for known durable identities, tombstones the identity during deletion,
aborts or waits on active work as required, and invalidates list/detail caches
after success. A failed known-ID delete restores the visible conversation.

Implementation: `backend/src/handlers/assistant.rs:delete_conversation` and `frontend/src/lib/assistant/aevatar-transport.ts:deleteConversation`.

## Typed state

`GET /conversations/{id}/state` is defined only for `nyxid-chat-...`. Query parameters such as `afterStateVersion` and `turnId` are forwarded to:

```http
GET /api/chat/conversations/{id}/state
```

The upstream response uses the typed reconnect envelope with `current`,
`not_modified`, `reload_required`, or `not_found` outcomes. A `chatc-...` state
request returns a not-found-shaped error without calling the typed state
resource.

`current` carries an actor-authored snapshot. Its recognised fields include
`activeTurn`, `latestTurn`, `recentTerminalTurns`, `activeTask`, `taskStatus`,
`pendingInput`, `pendingApproval`, `pendingActions`, `recentActions`,
`latestInputResolution`,
`latestApprovalResolution`, `latestControlResult`, `latestStepControlResult`,
`recentStepControlResults`, `controlFence`, `continuationAdmission`,
`attentionKind`, `attentionSince`, `activeStepSummary`, `scopeId`,
`stateVersion`, and `progressSequence`. `activeTask` uses the same TaskPlan and
step decoders as `nyxid.task.snapshot` and `nyxid.task.step.changed`.

The envelope and snapshot must agree with the adopted typed actor and scope.
For `current`, the envelope `stateVersion` must equal `snapshot.stateVersion`.
For `not_modified`, the safe envelope version must equal the stored version.
`stateVersion`, `progressSequence`, and every operation generation must be safe
integers. A current state may only advance the stored version and sequence;
`reload_required` is retried without a cursor, and `not_found` does not select a
legacy resource. The decoder accepts unknown additive snapshot keys, such as the
production-only `canaryEffectFault`, but never accepts an unknown command verb
or identity mismatch as forward-compatible state.

After loading a `nyxid-chat-...` transcript, the browser also reads this state resource and hydrates the current TaskPlan, input, approval, and action cards. It does not proactively hydrate `chatc-...` conversations. A state-resource `404` after a valid typed transcript preserves the richer local/history mirror for mixed deployments instead of erasing cards.

`activeTask` carries the full published TaskPlan shape. Live `nyxid.task.snapshot`, live `nyxid.task.step.changed`, and `snapshot.activeTask` all enter the same task reducer. The reducer enforces actor identity, plan revision, monotonically increasing `progressSequence` and `stateVersion`, and exact task/step relationships. Unknown additive fields are ignored. `availableActions` is a closed object containing only `retry`, `skip`, and `stop`; an unknown action verb fails closed.

Task-step approval observations retain the public decision mode, `approval_required` / `denied` receipt status, observation time, optional `rejected` / `expired` / `timed_out` terminal outcome, and optional non-sensitive `subjectKind`. The public read model deliberately reserves and omits `subjectId`; the browser does not recreate or display it.

The hydrated cards live in one stable synthetic current-state message. Each hydration removes the prior managed TaskPlan/input/approval/action projections and rebuilds one copy of each current card, so repeated reloads cannot accumulate duplicates. A partial pending-input or pending-approval snapshot may retain the matching richer live card by request identity. Matching committed input and approval resolutions retain the local terminal card while advancing its state version.

`reload_required`, actor mismatch, invalid versions, unavailable controls, and stale operation generations fail before a control POST. Stop, steer, retry, and skip always preflight this resource; the browser never fabricates `expectedStateVersion: 0`.

Implementation: `backend/src/handlers/assistant.rs:get_state`.

## Typed delivery replay

Typed `text` and `action.continue` streams have a maximum of two delivery attempts because `clientRequestId` is also the Aevatar `Idempotency-Key`. A replay is allowed for a network/header failure, an HTTP status in `408`, `425`, `429`, `500`, `502`, `503`, or `504`, or a stream outcome classified as retryable before an authoritative settlement.

The budget is one initial attempt plus one replay. Cancellation or a settled run
prevents the replay. Nonretryable HTTP or protocol errors fail immediately. A
typed failure never invokes a legacy send path.

Implementation: `frontend/src/lib/assistant/aevatar-transport.ts:streamTurn` and `streamActionContinuation`.

## Error and body handling

Assistant forwarding preserves upstream status and body unless this document specifies normalization. Internal database, provisioning, body-buffer, pagination, or serialization details use NyxID `AppError` handling and are not exposed as raw internals.

Browser-facing transcript parsing, stream-start errors, and terminal errors are defensive:

- malformed transcript JSON shape is a protocol error, not an empty success;
- malformed stream frames are skipped at the frame level, but missing required context or terminal structure fails the turn;
- upstream error text is converted to bounded, redacted code and message fields before display;
- a caller cannot add Authorization, secrets, engine fields, or scope by embedding them in a typed command;
- request objects with unknown fields are rejected rather than partially forwarded.

SSE decoding and terminal semantics are specified in [Stream protocol](03-stream-protocol.md).
