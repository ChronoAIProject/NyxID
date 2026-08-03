# Assistant Stream Protocol

Last verified against `f608b33c` (2026-08-01).

The assistant transport consumes Aevatar's AG-UI events over Server-Sent Events and projects them into NyxID transcript messages and content blocks. The stream is an ordered delivery protocol, not an unstructured token feed: it establishes a turn identity, emits zero or more renderable events, and settles exactly once.

Implementation anchors are `frontend/src/lib/assistant/sse.ts`, `chat-stream-parser.ts`, `chat-stream-worker-client.ts`, and `aevatar-transport.ts:handleAgUiFrame`.

## SSE framing

The browser incrementally decodes UTF-8 and preserves partial code units across network chunks. SSE records are separated by a blank line. The framer accepts all legal newline forms:

- CRLF (`\r\n`);
- LF (`\n`); and
- bare CR (`\r`).

A CR at the end of a network chunk is held until the next chunk so a split CRLF cannot create a false record boundary. On end of stream, the parser flushes a final nonblank record even when the server omitted the trailing blank line. This matters because the final unterminated record may be `RUN_FINISHED`.

Within one record, every line beginning with `data:` contributes its value. The optional single space after the colon is removed, and multiple data lines are joined with `\n` before JSON parsing. Comment lines and non-data fields do not contribute a payload. A record with no data is ignored.

Examples with equivalent payload extraction:

```text
data: {"type":"RUN_FINISHED"}

```

```text
event: message
data: {"type":"TEXT_MESSAGE_CONTENT",
data: "delta":"hello"}

```

The second record joins its data values with a newline; it is usable only if the result is valid JSON.

The conventional OpenAI sentinel `data: [DONE]` is not special-cased. It is not JSON, so the frame parser ignores it. A malformed JSON payload, JSON scalar, or JSON array is also ignored. Only JSON objects become stream frames. One malformed frame does not poison later valid records.

`frontend/src/lib/assistant/sse.ts:drainSseBuffer` and `flushSseBuffer` define framing. `frontend/src/lib/assistant/chat-stream-parser.ts:ChatStreamParser` defines JSON acceptance.

### Anti-buffering

Every reverse proxy or CDN in front of `/api/v1/assistant/**` must pass SSE without response buffering. NyxID's response-header middleware identifies `text/event-stream` case-insensitively and with parameters, then overwrites `X-Accel-Buffering` with `no`. On HTTP/1.1, a healthy live response exposes `x-accel-buffering: no` and `transfer-encoding: chunked`, and each upstream frame is flushed incrementally instead of the response arriving as one blob after the terminal frame. HTTP/2 and HTTP/3 do not use the HTTP/1.1 `Transfer-Encoding` header but must preserve the same incremental delivery.

Implementation: `backend/src/mw/security_headers.rs:security_headers_middleware`; coverage: `is_sse_media_type_ignores_case_and_parameters`, `duplicate_content_type_with_one_sse_value_is_marked`, and `with_response_headers_marks_sse` in that file.

## Frame type detection

The adapter accepts both normal AG-UI `type` tags and body-keyed forms. A tag is compared case-insensitively by converting it to uppercase. When `type` is absent, these object properties infer the same event family:

| Property | Event type |
| --- | --- |
| `runStarted` | `RUN_STARTED` |
| `runFinished` | `RUN_FINISHED` |
| `runStopped` | `RUN_STOPPED` |
| `runError` | `RUN_ERROR` |
| `stepStarted` | `STEP_STARTED` |
| `stepFinished` | `STEP_FINISHED` |
| `textMessageStart` | `TEXT_MESSAGE_START` |
| `textMessageContent` | `TEXT_MESSAGE_CONTENT` |
| `textMessageEnd` | `TEXT_MESSAGE_END` |
| `toolCallStart` | `TOOL_CALL_START` |
| `toolCallEnd` | `TOOL_CALL_END` |
| `toolApprovalRequest` | `TOOL_APPROVAL_REQUEST` |
| `authorizationRequired` | `AUTHORIZATION_REQUIRED` |
| `usage` | `USAGE` |
| `mediaContent` | `MEDIA_CONTENT` |
| `stateSnapshot` | `STATE_SNAPSHOT` |
| `custom` | `CUSTOM` |

Unknown event types are skipped for forward compatibility. They do not settle or fail the turn.

## Consumed event vocabulary

### Run lifecycle

`RUN_STARTED` establishes the typed actor delivery and authoritative turn ID. The ID is selected from the top-level `turnId`, `runStarted.turnId`, then `runStarted.runId`. It must pass the control-identity guard. A second start is a protocol error, as is a replay that changes the already adopted turn ID.

Workflow delivery usually starts earlier with `CUSTOM aevatar.chat.context`. Its later `RUN_STARTED` identifies a run actor used for run-level control and does not replace the Chat History turn identity.

`RUN_FINISHED` accepts an omitted status or the explicit status `completed` or `blocked`. Another status is a protocol error. A workflow terminal may carry `result.output`; when no text or observed completion has supplied printable content, the adapter emits that output as the assistant response.

`RUN_ERROR` records a failed terminal with a bounded, sanitized error code and message. `RUN_STOPPED` records a cancelled terminal; it is not classified as a truncated stream.

### Text

`TEXT_MESSAGE_START` opens an assistant message and its first text block. The supplied `messageId` is used when present; otherwise the browser creates a local ID.

`TEXT_MESSAGE_CONTENT` appends a nonempty `delta` to the open text block and marks the turn as having printable content. A content frame without an open block is ignored rather than creating a structurally ambiguous message.

`TEXT_MESSAGE_END` closes the open block and emits message completion. Terminal settlement also closes a still-open message.

### Tools and steps

`TOOL_CALL_START` adds or updates a step in the turn's run ledger using `toolCallId` and `toolName`, with bounded fallbacks when either is absent. `TOOL_CALL_END` completes the corresponding step and projects a safe summary rather than raw arbitrary tool output.

`STEP_STARTED` and `STEP_FINISHED` provide equivalent workflow-step activity. A failed step is marked failed; a successful step is marked complete.

Custom `aevatar.step.request` and `aevatar.step.completed` map to the same run ledger. Custom `aevatar.workflow.waiting_signal` marks the turn as waiting without settling it.

### Approvals and authorization

`TOOL_APPROVAL_REQUEST`, `CUSTOM aevatar.tool_approval.pending`, and `CUSTOM aevatar.human_input.request` create an approval block and place the turn at a human gate.

The generic `AUTHORIZATION_REQUIRED` frame is intentionally ignored because it does not carry NyxID's credential classification contract. Only `CUSTOM nyxid.authorization.required` can create a connection-recovery card. Its payload must pass `parseAuthorizationBlocker` before it is projected.

`CUSTOM nyxid.action.request` creates a v4 action card. Its complete lifecycle is specified in [Action cards](04-action-cards.md).

### Usage and media

`USAGE` updates per-conversation usage metadata. It does not create a visible transcript message by itself.

`MEDIA_CONTENT` becomes an artifact block. Inline base64 media longer than 8,000,000 characters is summarized as text instead of embedded in a data URL. This limits browser memory and DOM payload growth.

### Telemetry-only frames

`STATE_SNAPSHOT` is workflow projection state and is never rendered.

`CUSTOM aevatar.llm.reasoning` is deliberately neither rendered nor copied into a transcript block. Reasoning content is not exposed by this UI.

`CUSTOM aevatar.nyxid_chat.keepalive` proves connection liveness but not execution progress. It does not reset the 120-second progress watchdog.

`CUSTOM aevatar.run.context` and `demo.conversation.context` carry correlation identities only and do not render.

`CUSTOM aevatar.raw.observed` is an engine envelope. The envelope, workflow definition, system prompt, kernel state, and reasoning remain hidden. When the payload type is `RoleChatSessionCompletedEvent`, the adapter may extract only presentation-safe completion data:

- tool calls and matching receipts into the run ledger;
- unmatched receipts into defensive ledger entries;
- final content as fallback text when no text streamed;
- model and usage metadata.

Tool receipt output is bounded and summarized. `reasoningContent` is not read.

## Workflow chat context

The first meaningful Studio workflow frame is normally:

```text
CUSTOM name=aevatar.chat.context
```

Its payload establishes the Chat History reservation identity:

```json
{
  "scopeId": "verified NyxID user UUID",
  "conversationId": "chatc-650906f30cc985fa341477281303b6de",
  "stateVersion": 18,
  "turnId": "turn-identity"
}
```

The context is processed only for a workflow run. It may appear before `RUN_STARTED`; this ordering is valid and starts delivery.

### Scope guard

When the authenticated user ID is present in the frontend auth store, `scopeId` must be a string equal to that ID. A mismatch or omission is a protocol error. If the auth store has not hydrated yet, the current implementation cannot compare and applies the rest of the guards. Tightening this to require scope unconditionally depends on live deployment capture; see [Testing and gaps](07-testing-and-gaps.md).

The scope is also checked around create recovery awaits. An auth-user switch prevents adoption of a recovered conversation.

### Conversation guard

`conversationId`, when present, must match the workflow `chatc-...` form. A create context without a valid server conversation ID fails closed: retaining a `workflow-pending-...` placeholder would make the next send look like another create.

Once a durable workflow ID has been adopted, a replay cannot change it. A change is a protocol error rather than a query-cache rekey. On the first valid create context, the browser:

- replaces the stored placeholder identity;
- removes the retained create request marker;
- stores the conversation under the durable ID;
- records an alias from the placeholder to the durable ID; and
- updates the active URL/query identity when the placeholder is open.

### State-version guard

`stateVersion` may be a number or a numeric string. After conversion it must be a safe integer greater than or equal to zero. A negative, fractional, unsafe, or nonnumeric value is a protocol error.

Zero is accepted as context but not persisted as a continuation fence. A positive version updates the stored fence monotonically with `max(previous, observed)`. The browser never lowers a fence.

### Turn guard

`turnId` must be a valid nonblank control identity. It starts workflow delivery and becomes the authoritative Chat History turn ID. If a replay already established a turn ID, the new value must match. A mid-stream change is a protocol error.

After adoption, the adapter emits a `turn.status` event with `running` exactly once.

Implementation: `frontend/src/lib/assistant/aevatar-transport.ts:applyWorkflowChatContext`.

## Ordering and protocol errors

Typed delivery requires `RUN_STARTED` before ordinary content. A non-keepalive typed frame before start fails the delivery. Workflow permits custom context frames before `RUN_STARTED`, because `aevatar.chat.context` starts the workflow delivery.

Exactly one terminal frame is allowed. A second `RUN_FINISHED`, `RUN_ERROR`, or `RUN_STOPPED` is a protocol error, even if it repeats the first outcome.

After a terminal:

- typed actor nonterminal data is a protocol error;
- typed keepalive is harmless;
- repeated terminals are a protocol error; and
- workflow trailing projection snapshots or raw observed telemetry are ignored because the workflow engine emits those after its logical terminal.

The adapter cancels stream consumption when it records a delivery protocol error. It does not apply later frames from that response.

## Progress watchdog

Every meaningful frame rearms a 120-second progress watchdog. A keepalive does not. Approval waiting suspends the watchdog because user action, not upstream progress, is required.

On expiry the adapter makes a best-effort typed server stop when it has an addressable turn, closes open text, marks activity failed, emits `upstream_progress_timeout`, and aborts the local stream. A workflow run has no equivalent server-side stop route in this mount; local cancellation and recovery behavior are described in [Testing and gaps](07-testing-and-gaps.md).

## Terminal settlement

The transport records a terminal while it is reading frames and settles after the stream completes cleanly. This allows it to detect a duplicate terminal or illegal post-terminal typed data.

| Wire outcome | Turn result | Activity result |
| --- | --- | --- |
| `RUN_FINISHED` absent/`completed` | `completed` | `done`, or `waiting` when an approval remains. A context-free Studio create is carved out below. |
| `RUN_FINISHED status=blocked` | `blocked` | `blocked` |
| `RUN_ERROR` | `failed` with sanitized error | `failed` |
| `RUN_STOPPED` | `cancelled` | `cancelled` |
| EOF while waiting for approval | `completed` pause | `waiting` |
| EOF without terminal or approval gate | retryable `stream_closed` failure | partial content retained, activity failed |

EOF at an approval gate is a successful pause because Aevatar may close an idle stream while waiting for the human. The approval card remains actionable; its decision opens a continuation stream.

EOF without a terminal is not success. The UI tells the user the partial answer may be incomplete and that the full reply can appear after history reload. Workflow create may enter bounded create recovery before settling this failure. A Studio continuation is never replayed after such an accepted stream truncation.

For a Studio create still addressed by `workflow-pending-...`, a terminal received without `aevatar.chat.context` is converted to a retryable stream outcome and routed through create recovery rather than settled against the placeholder. Empty recovery fails closed. Successful recovery preserves an observed terminal: `RUN_ERROR` remains failed and blocked `RUN_FINISHED` remains blocked. Only recovery from a header failure or truncated stream with no observed terminal settles `completed` from the reconciled history.

For an action continuation, a terminal error leaves the report batch queued because it does not prove Aevatar admitted the reports. A finished or stopped terminal accepts the batch. Details are in [Action cards](04-action-cards.md).

Implementation: `frontend/src/lib/assistant/aevatar-transport.ts:consumeTurnStream`, `recordDeliveryTerminal`, `settleDeliveryTerminal`, and `settleRecoveredWorkflowCreate`.

## Transcript projection

The adapter emits NyxID's internal turn-event vocabulary into the hook layer. `frontend/src/hooks/use-assistant.ts` batches high-frequency deltas and applies them to a normalized message/block transcript.

The principal projections are:

| AG-UI source | Transcript projection |
| --- | --- |
| text start/content/end | assistant message with a `text` block |
| tool and step events | synthetic assistant activity message with a `run` block |
| approval requests | activity message with an `approval` block |
| `nyxid.authorization.required` | activity message with a `connect` block |
| `nyxid.action.request` | activity message with an `action` block |
| media | activity message with an `artifact` block |
| usage/context/snapshots/keepalive/reasoning | metadata only or ignored |

One synthetic activity message per turn hosts the run ledger and structured cards, so tool progress remains visible even when the model emits no text. Blocks retain stable IDs and ordering. Text deltas update the open block; structured card updates replace the matching block state rather than append duplicate cards.

Block projection never renders raw workflow telemetry, prompts, kernel state, delegated credentials, authorization headers, secret action parameters, or model reasoning. History reconciliation preserves local structured blocks until the upstream history contract can represent them durably.
