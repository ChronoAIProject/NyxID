# Assistant Stream Protocol

The Aevatar chat contract is pinned in `tests/fixtures/assistant/aevatar-chat-contract-pin.json`. See the pin section of [README.md](README.md).

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

`SsePayloadDecoder` owns framing. `normalizeBackendSseFrame` converts backend
oneof/camel-case frames to AG-UI events, `RuntimeEventAccumulator` builds the
visible message, and the actor-state reducer independently applies committed
`nyxid.*` facts. The transcript and actor projection therefore share the same
wire observation but never infer state from each other.

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
- `nyxid.action.request`; and
- `nyxid.authorization.required`.

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
records a bounded, sanitised failure. `RUN_STOPPED` and a local reader abort
settle the NyxID session as `stopped`, which is intentionally quiet in the UI.
Exactly one terminal is permitted. Nonterminal typed data after a terminal is a
protocol error; a keepalive is harmless but never proves execution progress.

## Presentation events

`TEXT_MESSAGE_START`, `TEXT_MESSAGE_CONTENT`, and `TEXT_MESSAGE_END` project
normal assistant text. A content delta without an open text block is ignored;
terminal settlement closes an open block.

`TOOL_CALL_START` and `TOOL_CALL_END` contribute a safe activity summary.
`USAGE` is metadata only. The NyxID `mediaContent` oneof normalizes to
`MEDIA_CONTENT` and appends an artifact block. Inline media is capped at
8,000,000 characters. An oversize item becomes a bounded notice rather than
being retained. History does not currently carry artifacts, so live media is
not reconstructed after reload. Reasoning, raw provider envelopes, prompts, kernel
state, delegated credentials, Authorization headers, secret action parameters,
and unknown opaque telemetry are never rendered or retained in the transcript.

The actor projection, not the text transcript, owns pending input, approval,
action, stop/steer control, task status, authorization recovery, and
continuation state. A typed authorization blocker becomes a connect card. A
strict readiness-shaped `TOOL_CALL_END` result may do the same; arbitrary tool
results never do. History rendering must not independently mutate or replace
the actor projection.

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

The client does not perform an automatic transcript reconciliation after every
terminal. The local runtime message remains authoritative until a later route
selection or reload reads history, matching the console orchestration.

## Delivery deadlines and watchdog

Typed commands use `clientRequestId` as the upstream `Idempotency-Key`. The
browser does not automatically replay a failed command. An HTTP failure before
the stream starts rejects the send and restores the prior composer state.

A 30-second start deadline is armed before `POST /assistant/chat`. If neither
the response nor a meaningful frame arrives, the session gets the visible error
`The assistant did not start replying in time. Try again.` and the composer is
freed without duplicating the submitted text.

After the first meaningful frame, every meaningful frame rearms the 120-second
progress watchdog; keepalives do not. On expiry the orchestrator makes a
best-effort typed `task.stop` when it has a
valid actor and turn identity, marks the activity failed, and aborts the local
stream. Local Stop is never state-version-fenced; it always aborts the reader.
The optional server `task.stop` is sent only when a positive current state
version is available.

## Workflow exclusion

The browser no longer processes `aevatar.chat.context`, workflow state
snapshots, raw workflow observations, workflow waiting signals, or a Workflow
create-recovery response. It does not allocate `workflow-pending-*`, alias a
workflow identity, or replay workflow wire captures. Legacy `chatc-*` history
may be displayed or deleted, but it never becomes a stream or control target.
