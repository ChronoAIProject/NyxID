# Assistant Stream Protocol

Last verified against Aevatar `0a86713671fcf551dc19ad86b1b6aa8ae6cb980b` and the
production typed-chat probe (2026-08-11).

The Assistant consumes Aevatar AG-UI Server-Sent Events only from the typed
`NyxIdChat` route. The stream is an ordered actor-observation protocol, not an
unstructured token feed. It establishes one actor and turn identity, projects
committed actor facts, emits presentation events, and settles exactly once.

## Framing

The browser incrementally decodes UTF-8 and preserves partial code units across
network chunks. SSE records are separated by a blank line and may use CRLF, LF,
or bare CR. A trailing CR is held until the next chunk, so a split CRLF cannot
create a false record boundary. The final nonblank record is flushed at EOF.

Every `data:` line contributes to a record; its optional single leading space is
removed and multiple values are joined with a newline before JSON parsing.
Comments and non-data fields do not contribute. `data: [DONE]`, malformed JSON,
scalars, and arrays are ignored as individual frames. A malformed frame does not
poison later valid frames.

Every proxy in front of `/api/v1/assistant/**` must disable buffering for
`text/event-stream`. NyxID marks SSE responses with `X-Accel-Buffering: no`.

## Identity and lifecycle

`RUN_STARTED` is the first required meaningful typed frame. It must contain a
valid top-level `actorId` and `turnId`. When nested `runStarted.threadId` and
`runStarted.runId` are present, they must exactly match the outer actor and turn
respectively. The browser adopts those identities once; a duplicate start or a
missing, changed, or contradictory identity is a terminal protocol error.

Observed typed streams carry `nyxid.task.snapshot` followed by one or more
`nyxid.task.step.changed` custom frames. The browser routes every recognised
custom frame to one authoritative ActorProjection reducer:

- `nyxid.task.snapshot` and `nyxid.task.step.changed`;
- `nyxid.control.changed` and `nyxid.step.control.changed`;
- `nyxid.continuation.changed`;
- `nyxid.input.request` and `nyxid.input.changed`;
- `nyxid.approval.request` and `nyxid.approval.changed`; and
- `nyxid.action.request`.

Task snapshots and state reads use one TaskPlan decoder and one step decoder.
The reducer never infers task, plan, request, actor, turn, or operation identity
from a URL, transcript, event ordering, or card. An operation key is valid only
when all actor-authored components agree:

```text
actorId + turnId + taskId + stepId + operationId + operationGeneration
```

`stateVersion`, `progressSequence`, and `operationGeneration` must be
browser-safe integers. A stale, fractional, unsafe, or identity-conflicting
frame cannot advance the projection.

`RUN_FINISHED` accepts absent/`completed` or `blocked` status. `RUN_ERROR`
records a bounded, sanitised failure and `RUN_STOPPED` records cancellation.
Exactly one terminal is permitted. Nonterminal typed data after a terminal is a
protocol error; a keepalive is harmless but never proves execution progress.

## Presentation events

`TEXT_MESSAGE_START`, `TEXT_MESSAGE_CONTENT`, and `TEXT_MESSAGE_END` project
normal assistant text. A content delta without an open text block is ignored;
terminal settlement closes an open block.

`TOOL_CALL_START` and `TOOL_CALL_END` contribute a safe activity summary.
`USAGE` is metadata only. `MEDIA_CONTENT` is an artifact block, subject to the
existing inline-size limit. Reasoning, raw provider envelopes, prompts, kernel
state, delegated credentials, Authorization headers, secret action parameters,
and unknown opaque telemetry are never rendered or retained in the transcript.

The actor projection, not the text transcript, owns pending input, approval,
action, stop/steer control, task status, and continuation state. Existing
TurnEvent/card reducers and history reconciliation may render the projection but
must not independently mutate or replace it for a typed conversation.

## Current-state convergence

The browser requests the typed state resource on conversation mount, reconnect,
and relevant terminal or control acknowledgement. It sends
`afterStateVersion=<current>` only when it has a valid current version.

The state envelope has four outcomes:

| Outcome | Browser action |
| --- | --- |
| `current` | validate the envelope and atomically apply the newer actor snapshot |
| `not_modified` | retain the projection unchanged only when its safe envelope version equals the stored version |
| `reload_required` | discard the conditional cursor and reload current state |
| `not_found` | do not adopt state; present the resource result without legacy fallback |

For `current`, the envelope state version must equal `snapshot.stateVersion`;
the snapshot actor and scope must agree with the conversation, and state/progress
versions must be monotonic safe integers.
Recognised snapshot fields are reduced by typed schema; unknown additive fields
are ignored. This is intentional: production currently returns
`canaryEffectFault`, which the pinned canon does not list. Forward compatibility
for additive state is not permission to accept an unknown control verb or
identity conflict.

History is a separate, eventually consistent text projection. A transcript load
may update visible persisted text but cannot overwrite ActorProjection facts. A
reload test must cover a mixed transcript plus pending typed input, approval, or
action and demonstrate that the actor-owned card remains actionable.

## Delivery retry and timeout

Typed `text` and `action.continue` use `clientRequestId` as the upstream
`Idempotency-Key`. They allow one replay after a network/header failure, an HTTP
`408`, `425`, `429`, `500`, `502`, `503`, or `504`, or a retryable pre-settlement
stream result. A cancellation, terminal settlement, nonretryable HTTP response,
or protocol failure forbids replay. No failure class retries through Workflow.

Every meaningful frame rearms the 120-second progress watchdog; keepalives do
not. On expiry the transport makes a best-effort typed `task.stop` when it has a
valid actor and turn identity, marks the activity failed, and aborts the local
stream. A waiting input or approval suspends the watchdog because the next
action belongs to the user.

## Workflow exclusion

The browser no longer processes `aevatar.chat.context`, workflow state
snapshots, raw workflow observations, workflow waiting signals, or a Workflow
create-recovery response. It does not allocate `workflow-pending-*`, alias a
workflow identity, or replay workflow wire captures. Legacy `chatc-*` history
may be displayed or deleted, but it never becomes a stream or control target.
