# Assistant Chat Wire Contract

Last verified against `fcb79b18` (2026-08-01).

This document specifies the browser-to-NyxID contract and the Aevatar request NyxID produces. It is the canonical API reference for the assistant chat surface.

## Conventions

All browser routes in this document are relative to `/api/v1/assistant`. They require a human NyxID session except `GET /api/v1/assistant/actions`, which is specified separately in [Actions registry](06-actions-registry.md).

NyxID derives the Aevatar scope from the verified `AuthUser.user_id`. A browser body never carries a user scope. All Aevatar paths below are relative to the active platform `aevatar` service row's `base_url`.

Conversation prefixes are protocol discriminators:

| Prefix | Meaning | Engine/resource family |
| --- | --- | --- |
| `nyxid-chat-` | durable typed conversation | NyxIdChat actor |
| `chatc-` | durable workflow conversation | Studio workflow and scoped Chat History |
| `workflow-pending-` | local first-turn placeholder | browser only; never sent as a durable ID |

An unknown prefix is not guessed. Family-aware browser routes return a not-found-shaped error after syntactic validation. Conversation path segments are at most 128 ASCII alphanumeric, hyphen, or underscore characters.

## Browser API

| Browser call | Purpose | Upstream resource |
| --- | --- | --- |
| `GET /conversations` | fully materialized sidebar index | `GET /api/scopes/{userId}/chat-history`, cursor-drained |
| `GET /conversations/{id}` | transcript | typed or workflow detail, selected by ID |
| `DELETE /conversations/{id}` | durable delete | typed or workflow delete, selected by ID |
| `GET /conversations/{id}/state` | typed reconnect state | `GET /api/chat/conversations/{id}/state` |
| `GET /conversations/create-recovery/{commandId}` | recover a workflow create identity | `GET /api/scopes/{userId}/chat-history/create-recovery/{commandId}` |
| `POST /chat` | typed command | `POST /api/chat` with top-level `type` |
| `POST /workflow-chat` | Studio create or continuation | `POST /api/chat` without top-level `type` |
| `POST /completions` | retained OpenAI-compatible stream | `POST /v1/chat/completions` |
| `GET /workflow-chat/ws` | retained workflow WebSocket bridge | `GET /api/ws/chat` upgrade |

The normal browser UI uses list, detail, delete, typed commands for existing typed histories, and Studio HTTP streaming for new and existing workflow histories. The completions and workflow WebSocket mounts are retained backend surfaces but are not the normal assistant transport.

## Studio workflow turns

The frontend sends a narrow `WorkflowChatTurnRequest`. `backend/src/services/assistant_service.rs:workflow_chat_body` trims and validates the prompt, enforces create-versus-continuation invariants, and constructs Aevatar's strict `HttpChatInput`. Unknown browser fields fail deserialization because the request type uses `deny_unknown_fields`.

The backend always includes the `conversation` object. Omitting it would make an Aevatar workflow turn ephemeral and exclude it from Chat History. The backend always pins `workflow` to `studio`; a browser cannot select another catalog workflow, inline YAML, LLM control, tool context, metadata, headers, or an alternate engine.

### New conversation

The browser sends exactly:

```json
{
  "prompt": "trimmed user message",
  "commandId": "1ecf9efe-480c-4c5d-a755-e32bb06de665",
  "sessionId": "0f35cf60-3a67-47f5-a661-71ed333395d1"
}
```

NyxID rebuilds the upstream body in this key order:

```json
{
  "commandId": "1ecf9efe-480c-4c5d-a755-e32bb06de665",
  "conversation": {
    "conversationId": null
  },
  "prompt": "trimmed user message",
  "sessionId": "0f35cf60-3a67-47f5-a661-71ed333395d1",
  "workflow": "studio"
}
```

`commandId` is the durable create idempotency and recovery identity. The browser uses the turn's `clientRequestId` as this value and retains it when create outcome is ambiguous. If the caller omits `commandId`, NyxID generates a UUID, but the browser transport supplies one so it can use create recovery.

`sessionId` is a per-conversation correlation handle, generated lazily and reused for later turns. It is not the conversation identity and does not replace `conversationId` plus `minimumStateVersion`. It participates in Aevatar's create replay fingerprint, so a replay with the same `commandId` must retain the same `sessionId`.

Both tokens are at most 64 ASCII alphanumeric or hyphen characters. `sessionId` may be omitted by non-browser clients of the NyxID route; NyxID then omits it upstream rather than sending `null`.

### Continuation

The browser sends exactly:

```json
{
  "prompt": "trimmed next message",
  "conversationId": "chatc-650906f30cc985fa341477281303b6de",
  "minimumStateVersion": 18,
  "sessionId": "0f35cf60-3a67-47f5-a661-71ed333395d1"
}
```

NyxID rebuilds:

```json
{
  "conversation": {
    "conversationId": "chatc-650906f30cc985fa341477281303b6de",
    "minimumStateVersion": 18
  },
  "prompt": "trimmed next message",
  "sessionId": "0f35cf60-3a67-47f5-a661-71ed333395d1",
  "workflow": "studio"
}
```

A continuation must use a `chatc-...` ID and a safe-integer state version greater than zero. `commandId` is forbidden on a continuation. A create cannot include `conversationId` or `minimumStateVersion`; a create with a fence and a continuation without a fence are rejected before Aevatar is called.

### Prompt and headers

NyxID trims the Studio prompt before validation and serialization. The result must contain 1 through 32,768 Unicode scalar values. The handler buffers at most 256 KiB for the entire JSON request.

The browser sends:

```http
Content-Type: application/json
Accept: text/event-stream
```

NyxID replaces the content type with `application/json` for the rebuilt body and forwards the caller's `Accept`. It does not synthesize `Idempotency-Key` for workflow turns; create idempotency is the body `commandId`, and continuation admission is fenced by `minimumStateVersion`.

The body matches Aevatar's own console client: create carries a create-only command identity and a null conversation ID; continuation carries the durable ID and observed Chat History fence; both pin Studio and use a stable session correlation ID.

## Typed NyxIdChat commands

`POST /chat` accepts a discriminated command allowlist. NyxID parses each command with unknown-field denial, rejects secret-shaped keys and values, validates control identities, and rebuilds the exact upstream object. It never spreads an arbitrary caller object into the Aevatar body.

Every typed command includes `clientRequestId`. NyxID copies that value into the outbound `Idempotency-Key` header. Text, action continuation, and approval resolution request `Accept: text/event-stream`. Stop, steer, retry, and skip request `Accept: application/json`.

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

The prompt must be nonblank after trimming and no longer than 32,768 characters. Unlike the Studio builder, the typed parser validates the trimmed condition but preserves the submitted prompt string in the rebuilt body. The current browser starts new conversations through Studio; typed create remains part of the backend contract and upstream actor protocol.

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

The report batch contains at most 64 entries. Action request IDs must be unique in the batch. When reports are present, the outer `originTurnId` is required and every report must match it. Allowed dispositions are `completed`, `declined`, `failed`, `cancelled`, and `expired`. The resource grammar is described in [Action cards](04-action-cards.md); the current backend requires a completed report to carry a `userService` resource.

An empty `actions` array is a typed actor wake and may omit `originTurnId`.

### `approval.resolve`

```json
{
  "type": "approval.resolve",
  "conversationId": "nyxid-chat-f8369965a444433f92ec50e67ad8ee52",
  "clientRequestId": "request-identity",
  "requestId": "approval-identity",
  "approved": true,
  "reason": "optional trimmed reason"
}
```

An empty reason is omitted. A nonempty reason is trimmed and limited to 2,048 characters.

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

`expectedStateVersion` must be nonnegative. This command returns JSON. The typed transport waits for any pending stop ordering fence before later turn or action delivery.

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

The instruction must be nonblank after trimming. The preserved wire value is not normalized. `expectedStateVersion` must be nonnegative. This command returns JSON.

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

`step.skip` has the same fields except `type` is `step.skip` and `skipRequestId` replaces `retryRequestId`. `expectedOperationGeneration` must be positive; `expectedStateVersion` must be nonnegative. Both return JSON.

Typed command parsing and reconstruction are implemented by `backend/src/services/assistant_service.rs:parse_assistant_chat_command` and `prepare_assistant_chat_command`. Header enforcement is in `backend/src/handlers/assistant.rs:typed_chat`.

## Conversation index

`GET /conversations` does not expose upstream pagination to the browser. NyxID drains Aevatar's shared scoped index:

```http
GET /api/scopes/{verifiedUserId}/chat-history
GET /api/scopes/{verifiedUserId}/chat-history?cursor={encodedCursor}
```

For each page NyxID:

1. requires an upstream success response;
2. buffers at most 4 MiB;
3. parses the `conversations` array;
4. keeps only IDs beginning `nyxid-chat-` or `chatc-`;
5. deduplicates by ID, keeping the first occurrence; and
6. follows a nonblank string `nextCursor`.

The full drain is bounded by 40 pages and an 8 MiB aggregate body budget. A repeated cursor is an internal protocol error. A non-string `nextCursor` is an internal protocol error. Reaching the page or aggregate limit returns the rows already collected; the response does not invent a continuation cursor or a truncation flag.

If a later page has malformed JSON or lacks a `conversations` array, NyxID preserves rows collected from prior pages. The same mixed-deployment posture on the first page returns an empty successful index using the first upstream response metadata. Non-success upstream status is forwarded.

After draining, NyxID sorts the retained rows newest first. It recognizes `updatedAt`, `updated_at`, `lastMessageAt`, `last_message_at`, `createdAt`, then `created_at`; parseable RFC 3339 values are compared chronologically, with raw string fallback. The browser receives:

```json
{
  "conversations": []
}
```

No upstream cursor remains. Implementation: `backend/src/handlers/assistant.rs:list_conversations` and `backend/src/services/assistant_service.rs:append_addressable_history_page`.

## Transcript detail

`GET /conversations/{id}` switches on the identifier family:

| ID | Aevatar request |
| --- | --- |
| `nyxid-chat-...` | `GET /api/chat/conversations/{id}` |
| `chatc-...` | `GET /api/scopes/{verifiedUserId}/chat-history/conversations/{id}` |

NyxID forwards the body without parsing or reshaping it. The frontend accepts two Aevatar history forms during mixed deployment:

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

The wrapped form supplies the workflow continuation fence. The legacy array does not fabricate one or reset an already observed fence. Any other transcript shape fails closed in the frontend rather than being treated as an empty conversation.

Aevatar Chat History currently materializes text messages, not every browser-local structured block. While a workflow transcript is catching up, `frontend/src/lib/assistant/aevatar-transport.ts:applyHistoryResponse` merges server text with locally retained structured action and approval messages rather than discarding those cards.

## Delete

`DELETE /conversations/{id}` reserves deletion in the browser transport so a concurrent send, action report, approval, or late recovery cannot resurrect the conversation.

| ID | Aevatar request | NyxID success response |
| --- | --- | --- |
| `nyxid-chat-...` | `DELETE /api/chat/conversations/{id}` | preserve upstream response, including `202` JSON acceptance |
| `chatc-...` | `DELETE /api/scopes/{verifiedUserId}/chat-history/conversations/{id}` | normalize every upstream success to `204` with an empty body |

Non-success responses are preserved. Workflow normalization removes `Content-Length` and `Content-Type`. The frontend uses a 15-second request deadline, tombstones the identity during deletion, aborts or waits on active work as required, and invalidates list/detail caches after success. A failed delete restores the visible conversation.

Implementation: `backend/src/handlers/assistant.rs:delete_conversation` and `frontend/src/lib/assistant/aevatar-transport.ts:deleteConversation`.

## Typed state

`GET /conversations/{id}/state` is defined only for `nyxid-chat-...`. Query parameters such as `afterStateVersion` and `turnId` are forwarded to:

```http
GET /api/chat/conversations/{id}/state
```

The upstream response uses the typed reconnect envelope with `current`, `not_modified`, `reload_required`, or `not_found` outcomes. A `chatc-...` state request returns a not-found-shaped error without calling the typed state resource. Workflow state is learned from Chat History and `aevatar.chat.context`.

Implementation: `backend/src/handlers/assistant.rs:get_state`.

## State-version fences

The workflow fence is the last observed materialized Chat History `stateVersion`. `frontend/src/lib/assistant/aevatar-transport.ts:positiveStateVersion` accepts only safe integers greater than zero.

The rules are:

- Persist positive versions only. A zero version may be valid stream context for a create but cannot authorize a continuation.
- Never fabricate a fence from message count, time, local turn count, or typed actor state.
- Never decrease the stored version. Context, history, and recovery updates take the maximum valid observed value.
- Every workflow continuation carries the last positive version in `conversation.minimumStateVersion`.
- If a durable workflow conversation has no positive fence, read Chat History before posting.
- A refresh used for retry must return a version at least as high as the currently stored fence.

The preflight reads history at delays `0`, `300`, `900`, and `1800` milliseconds. It requires a positive version and, when known, the latest assistant turn to be present. Exhaustion fails with a synchronizing error; the client does not submit an unfenced continuation.

These rules provide read-your-writes ordering across Aevatar's execution store and materialized Chat History.

## Studio continuation retry

The Studio workflow path automatically retries a continuation POST for one upstream condition only:

```text
HTTP 503
error code CHAT_HISTORY_RESERVATION_UNAVAILABLE
```

The transport waits `300` milliseconds before the first retry opportunity and `900` milliseconds before the second. Before each new POST it reads history and requires a usable `stateVersion` at least as high as the stored fence. It applies the refreshed history, rebuilds the continuation body with the resulting fence, then posts again.

It does not retry a Studio continuation for a generic network failure, another HTTP status or code, a protocol error, or an accepted stream that truncates after delivery. Once Aevatar has accepted the stream response, replay could create duplicate work and is therefore forbidden.

This is the only automatic POST retry on the Studio workflow path. Typed actor delivery has a separate idempotent replay contract described below.

Implementation: `frontend/src/lib/assistant/aevatar-transport.ts:streamWorkflowTurn`.

## Typed delivery replay

Typed `text` and `action.continue` streams have a maximum of two delivery attempts because `clientRequestId` is also the Aevatar `Idempotency-Key`. A replay is allowed for a network/header failure, an HTTP status in `408`, `425`, `429`, `500`, `502`, `503`, or `504`, or a stream outcome classified as retryable before an authoritative settlement.

The budget is one initial attempt plus one replay. Cancellation or a settled run prevents the replay. Nonretryable HTTP or protocol errors fail immediately. This actor behavior does not apply to Studio continuation, whose request identity and reservation rules are different.

Implementation: `frontend/src/lib/assistant/aevatar-transport.ts:streamTypedTextTurn` and `streamActionContinuation`.

## Create recovery

A workflow create can execute upstream even when the browser never receives enough of the response to adopt its durable identity. Re-posting as a fresh create would risk duplicate conversations. The browser instead queries by the original `commandId`:

```http
GET /api/v1/assistant/conversations/create-recovery/{commandId}
```

NyxID derives the user scope and forwards:

```http
GET /api/scopes/{verifiedUserId}/chat-history/create-recovery/{commandId}
```

A successful body must provide a valid workflow conversation ID, a safe nonnegative state version, and a valid turn ID:

```json
{
  "status": "found",
  "conversationId": "chatc-650906f30cc985fa341477281303b6de",
  "stateVersion": 18,
  "turnId": "turn-identity"
}
```

The frontend polls at `0`, `300`, `900`, and `1800` milliseconds. A `404` means not materialized yet and advances to the next delay. Other errors stop recovery.

Recovery starts at these ambiguity points:

- the create POST fails before usable response headers arrive;
- an accepted create stream ends in a retryable, truncated, or context-free state;
- the user cancels after create dispatch while the durable identity remains unknown, in which case recovery continues in the background.

A candidate is adopted only when all guards pass:

- the authenticated user scope exists and is unchanged across awaited operations;
- the local conversation still exists and is not tombstoned for deletion;
- `conversationId` matches `chatc-...`;
- the candidate does not conflict with an already adopted durable ID;
- `turnId` is valid;
- `stateVersion` is safe and nonnegative;
- a subsequent history reconciliation reaches at least `max(1, recoveredStateVersion)`; and
- that history contains the recovered assistant turn.

On success the transport aliases the local placeholder to the durable ID, takes the maximum observed fence, replaces any unrelated run-actor turn ID with the Chat History turn identity, and applies authoritative history. On exhaustion it fails closed. It retains the original create request identity so an explicit same-prompt user retry can recover or replay the same create rather than minting an unrelated duplicate.

Implementation: `backend/src/handlers/assistant.rs:get_create_recovery` and `frontend/src/lib/assistant/aevatar-transport.ts:pollCreateRecovery`, `recoverWorkflowCreate`, and `startCreateRecoveryInBackground`.

## Error and body handling

Assistant forwarding preserves upstream status and body unless this document specifies normalization. Internal database, provisioning, body-buffer, pagination, or serialization details use NyxID `AppError` handling and are not exposed as raw internals.

Browser-facing transcript parsing, stream-start errors, and terminal errors are defensive:

- malformed transcript JSON shape is a protocol error, not an empty success;
- malformed stream frames are skipped at the frame level, but missing required context or terminal structure fails the turn;
- upstream error text is converted to bounded, redacted code and message fields before display;
- a caller cannot add Authorization, secrets, engine fields, or scope by embedding them in a typed command;
- request objects with unknown fields are rejected rather than partially forwarded.

SSE decoding and terminal semantics are specified in [Stream protocol](03-stream-protocol.md).
