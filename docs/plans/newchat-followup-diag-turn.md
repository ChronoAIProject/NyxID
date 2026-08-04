VERDICT: UNDETERMINED

# New-chat empty-turn diagnosis

## Executive conclusion

The cancelled transcript `GET` did not create the assistant error. The error is the UI's representation of a closed stream episode for which the turn-event pump saw no printable event. `POST /workflow-chat` being 200 proves only that response headers arrived; it says nothing about whether the SSE body contained an answer.

The evidence available in the screenshot and Network request list cannot distinguish among these wire-level outcomes:

1. HTTP 200 followed by clean EOF with zero SSE frames;
2. HTTP 200 followed by context/usage/keepalive and possibly a terminal, but no printable frame or terminal `result.output`;
3. HTTP 200 followed by a body-read/network failure; or
4. printable data in an unexpected or malformed wire shape that the parser ignored.

The first three are upstream/proxy delivery failures or an upstream run that genuinely produced no answer. The fourth would be a client/upstream contract mismatch. No supplied artifact contains the response body, so selecting `UPSTREAM` rather than `UNDETERMINED` would overstate the evidence. The strongest supported attribution is nevertheless **upstream/nonprinting stream, not today's client change**: the 4.0-4.6 second lifetime rules out both client deadlines, the new projection reconciler has a separate controller and live-turn interlocks, and the cancelled `GET` is downstream of turn settlement.

The `EmptyTurnError` condition is semantically correct: the client must not invent an assistant reply when the episode printed nothing. Its generic copy is not the best error presentation because the transport already has a sanitized `stream_closed`, `network_error`, `RUN_ERROR`, or protocol error that could be shown in that row.

## Evidence chain

### 1. Send, stream dispatch, and header/body separation

1. `useSendMessage` creates a per-episode event pump before calling the transport, registers the returned handle, and projects the transport mirror independently (`frontend/src/hooks/use-assistant.ts:698-728`). Cache warming cannot abort sending: its history and list reads use `Promise.allSettled`, and their failures are deliberately nonfatal (`frontend/src/hooks/use-assistant.ts:105-165`).
2. `sendMessage` creates a `RunningTurn` with its own `AbortController`, records first-create provenance, inserts the run into `running`, and starts `streamTurn` (`frontend/src/lib/assistant/aevatar-transport.ts:2286-2350`; controller initialization is at `frontend/src/lib/assistant/aevatar-transport.ts:3280-3309`).
3. A workflow turn calls `startChatStream`, awaits `stream.headers`, and only then awaits body completion through `consumeTurnStream` (`frontend/src/lib/assistant/aevatar-transport.ts:3679-3689`, `frontend/src/lib/assistant/aevatar-transport.ts:3785-3817`).
4. `startChatStream` passes the run's signal to `chatStreamClient.start`; parsed frames alone enter `handleAgUiFrame` (`frontend/src/lib/assistant/aevatar-transport.ts:4097-4142`). There is no transcript-read signal in this call.
5. `ChatStreamWorkerClient.start` chooses the module worker in production and the behavior-equivalent inline fallback otherwise (`frontend/src/lib/assistant/chat-stream-worker-client.ts:137-175`). In the worker path, the run signal only sends `stream.cancel` for that stream request (`frontend/src/lib/assistant/chat-stream-worker-client.ts:178-240`).
6. The worker performs `fetch`, then posts `stream.response` immediately after successful headers, before reading the body (`frontend/src/lib/assistant/chat-stream.worker.ts:278-333`). Therefore DevTools showing 200 does not prove that even one SSE byte or frame arrived.
7. The worker reads the body, incrementally parses SSE, flushes the parser at EOF, and only then posts completion (`frontend/src/lib/assistant/chat-stream.worker.ts:352-361`). `ChatStreamParser` accepts JSON objects only; malformed JSON, scalars, arrays, comments, and records with no accepted `data` object yield no frame (`frontend/src/lib/assistant/chat-stream-parser.ts:4-23`, `frontend/src/lib/assistant/chat-stream-parser.ts:26-43`).
8. The worker client resolves headers and completion separately (`frontend/src/lib/assistant/chat-stream-worker-client.ts:244-307`, `frontend/src/lib/assistant/chat-stream-worker-client.ts:379-390`). Thus the POST can remain 200 even if body completion later reports `network_error`.

### 2. What can print

1. Workflow `aevatar.chat.context` adopts the durable `chatc-...` identity and aliases the placeholder (`frontend/src/lib/assistant/aevatar-transport.ts:4663-4733`). It then emits `turn.status/running`, not content (`frontend/src/lib/assistant/aevatar-transport.ts:4745-4780`). This can explain the durable production URL without any assistant text.
2. Correlation context, reasoning, keepalive, usage, and state/projection frames are nonprinting (`frontend/src/lib/assistant/aevatar-transport.ts:4550-4560`, `frontend/src/lib/assistant/aevatar-transport.ts:4629-4644`; usage handling is at `frontend/src/lib/assistant/aevatar-transport.ts:4424-4426`).
3. Text prints only when a `TEXT_MESSAGE_CONTENT` delta arrives after an open text block (`frontend/src/lib/assistant/aevatar-transport.ts:4357-4394`). Structured blocks also print because their block events carry non-text content.
4. A workflow `RUN_FINISHED` can still print without streamed text, but only when `runFinished.result.output` is a nonblank string; otherwise it records only a terminal (`frontend/src/lib/assistant/aevatar-transport.ts:4446-4476`). `RUN_ERROR` similarly records a terminal error rather than assistant content (`frontend/src/lib/assistant/aevatar-transport.ts:4432-4444`).
5. The pump's exact printable predicate accepts nonblank `block.delta`, a non-null decision patch, a non-text started/completed block, or a nonblank text started/completed block. Status, message lifecycle, usage, and `turn.completed` do not print (`frontend/src/hooks/use-assistant.ts:486-507`).

### 3. Terminal settlement and `turnPrinted`

1. `consumeTurnStream` arms the transport watchdog and awaits body completion, not merely headers (`frontend/src/lib/assistant/aevatar-transport.ts:4016-4024`).
2. A worker/body failure is converted to a retryable transport error (`frontend/src/lib/assistant/aevatar-transport.ts:4032-4034`, `frontend/src/lib/assistant/aevatar-transport.ts:4175-4188`). A clean EOF with no AG-UI terminal is explicitly `stream_closed`, not success (`frontend/src/lib/assistant/aevatar-transport.ts:4068-4079`).
3. A recorded `RUN_FINISHED`, `RUN_ERROR`, or `RUN_STOPPED` is settled according to that terminal (`frontend/src/lib/assistant/aevatar-transport.ts:4035-4053`, `frontend/src/lib/assistant/aevatar-transport.ts:5380-5428`). A successful terminal with no output is therefore a completed but empty turn.
4. On the normal workflow path, an unsettled result closes any partial local blocks and emits a failed terminal (`frontend/src/lib/assistant/aevatar-transport.ts:3785-3823`). `finishTurn` always emits `turn.completed` (`frontend/src/lib/assistant/aevatar-transport.ts:5563-5577`).
5. The pump initializes `printed = false` and immediately publishes the episode, so a live episode has an explicit `false`, not `undefined` (`frontend/src/hooks/use-assistant.ts:517-553`, `frontend/src/hooks/use-assistant.ts:596-597`). Every event is checked by `eventPrintsContent`; `turn.completed` closes the episode without changing `printed` (`frontend/src/hooks/use-assistant.ts:623-657`).
6. The page defines `turnEnded` from that closed episode, excluding user cancellation, and passes `episodeState.printed` to the thread (`frontend/src/pages/assistant.tsx:439-455`, `frontend/src/pages/assistant.tsx:548-560`).
7. Because an explicit `false` is authoritative, `ChatThread` does not fall back to tail transcript content. After the 700 ms grace and after projection activity ends, it renders `EmptyTurnError` (`frontend/src/components/assistant/chat-thread.tsx:384-387`, `frontend/src/components/assistant/chat-thread.tsx:524-545`, visible string at `frontend/src/components/assistant/chat-thread.tsx:342-356`).

This is why the rendered error means “the current stream episode produced no printable turn event,” not “the transcript GET was cancelled.”

## What a 200 POST plus empty or nonprinting body does today

### Clean 200, body object present, zero bytes, clean EOF

1. Fetch resolves and the worker reports 200 headers.
2. The reader immediately returns `done`; parser flush returns `[]`.
3. The worker reports `complete`, because the byte stream itself ended cleanly.
4. The transport sees no delivery terminal and returns retryable `stream_closed`.
5. For an already durable `chatc-...` conversation, `streamWorkflowTurn` emits failed `turn.completed(error.code = "stream_closed")`.
6. The pump receives only that terminal event, closes with `printed: false`, and the thread renders the generic empty-turn error after its grace.

If `response.body` is literally `null`, the worker reports `stream_closed` directly (`frontend/src/lib/assistant/chat-stream.worker.ts:335-349`). The inline fallback has the same result (`frontend/src/lib/assistant/chat-stream-worker-client.ts:558-572`).

### Clean 200 with only nonprinting frames

Context, usage, state, keepalive, and correlation frames can establish the conversation and/or turn without printing. If a valid `RUN_FINISHED` follows with blank or absent `result.output`, the transport emits completed `turn.completed`; the pump still closes with `printed: false`. If no AG-UI terminal follows, clean EOF becomes failed `stream_closed`. A `RUN_ERROR` becomes failed with its sanitized code, also with `printed: false` unless earlier content printed.

### 200 whose body dies mid-read

The worker catches the read exception and posts `stream.network_error` unless its controller was deliberately aborted (`frontend/src/lib/assistant/chat-stream.worker.ts:362-376`). The inline path matches this (`frontend/src/lib/assistant/chat-stream-worker-client.ts:592-601`). The transport emits failed `turn.completed(error.code = "network_error")`; with no prior printable block, the pump reports `printed: false`.

### Special first-create recovery path

If the stream closes before it adopts a `chatc-...` context, the first-create path polls create recovery and requires a History response containing the recovered assistant turn (`frontend/src/lib/assistant/aevatar-transport.ts:3540-3579`). If recovery fails, the original `stream_closed`/`network_error` is emitted as a failed turn. If recovery succeeds, `settleRecoveredWorkflowCreate` treats the reconciled History row as authoritative completion (`frontend/src/lib/assistant/aevatar-transport.ts:5432-5443`).

There is a pre-existing presentation hole here: recovered History is applied directly to the transport mirror, but it is not replayed as block events through the pump. Recovery can therefore find a persisted assistant reply while the episode remains `printed: false`; the explicit false then overrides transcript fallback in `ChatThread`. Existing coverage confirms truncated-create recovery settles `completed` after History recovery (`frontend/src/lib/assistant/aevatar-transport.test.ts:7142-7193`). That hole was not introduced by today's provenance/reconciler work, and it does not match the supplied screenshot's statement that no assistant reply exists, but a live wire/history capture should check it.

## Watchdogs: not this incident's timing

There are two client deadlines:

1. The hook first-event deadline is 30 seconds (`frontend/src/hooks/use-assistant.ts:51-55`). It cancels only when the pump has received no event; any event, including a nonprinting status or terminal, permanently clears it (`frontend/src/hooks/use-assistant.ts:599-626`).
2. The transport progress watchdog is 120 seconds (`frontend/src/lib/assistant/aevatar-transport.ts:281-285`). It is armed only during body consumption, rearmed by meaningful frames, and cleared when consumption exits (`frontend/src/lib/assistant/aevatar-transport.ts:4016-4024`, `frontend/src/lib/assistant/aevatar-transport.ts:4080-4082`, `frontend/src/lib/assistant/aevatar-transport.ts:4203-4231`).

The observed workflow request ended after roughly 4.0-4.6 seconds. Neither 30 seconds nor 120 seconds can produce that boundary.

This is not the exact recurrence of the PR #1321 client bug. Commit `d418c74a` records the prior signature as every turn being cancelled at exactly 8.00 seconds with zero response bytes; the fix changed `STREAM_START_DEADLINE_MS` from 8 seconds to the current 30 seconds. Today's trace ends around half of the former deadline and far below the current deadline. It may be the same upstream symptom (slow or absent first useful output), but it is not the same client watchdog cancellation mechanism.

## Whether today's shipped change contributed

### What it did contribute

Today's code records `identityPending` while the first create lacks a durable identity and `projectionPending` after a workflow terminal (`frontend/src/lib/assistant/aevatar-transport.ts:2323-2333`, `frontend/src/lib/assistant/aevatar-transport.ts:3317-3343`). `historyFromStored` exposes that provenance as `awaitingProjection` only after the live turn is no longer active (`frontend/src/lib/assistant/aevatar-transport.ts:1823-1839`). The page maps it to “Syncing conversation history...” (`frontend/src/pages/assistant.tsx:520-526`).

Therefore today's change explains the banner and the subsequent transcript reconciliation request. It does not explain why the stream episode printed nothing.

### Scope reset does not match the observed terminal

The new account boundary subscribes to auth-store changes and calls `resetScope` only when the user ID changes (`frontend/src/lib/assistant/aevatar-transport.ts:1286-1304`). `resetScope` does abort every live run and every scope-owned background controller (`frontend/src/lib/assistant/aevatar-transport.ts:1307-1336`), so it is a real abort path and deserves scrutiny.

However, it does not synthesize `turn.completed`; it aborts and clears the transport maps. `consumeTurnStream` sees an aborted run as already settled and returns without emitting a terminal (`frontend/src/lib/assistant/aevatar-transport.ts:4023-4026`). Before any event, the hook could only close the episode at its 30-second start deadline. After a context/status event, that deadline is cleared, so a scope reset would leave an open/stale episode rather than produce this 4-5 second closed-empty signature. Assistant API calls also preserve auth state on downstream 401s (`frontend/src/lib/assistant/aevatar-transport.ts:189-220`), so the cancelled transcript request cannot itself clear the user and trigger `resetScope`.

### Reconciler cancellation is separate from stream cancellation

Each reconciliation observation creates its own `scopeController` and stores it on the reconciliation entry (`frontend/src/lib/assistant/aevatar-transport.ts:2031-2051`). Releasing the React effect's last waiter aborts only `entry.controller` (`frontend/src/lib/assistant/aevatar-transport.ts:1983-2001`); the turn stream uses `run.controller.signal` (`frontend/src/lib/assistant/aevatar-transport.ts:4125-4135`). These are different `AbortController` instances.

The reconciler also refuses to read over a live turn (`frontend/src/lib/assistant/aevatar-transport.ts:2031-2047`) and rechecks after an already-started transcript read returns so a newly live turn is not overwritten (`frontend/src/lib/assistant/aevatar-transport.ts:2075-2102`). Normal history projection likewise serves the local mirror while a turn is in flight (`frontend/src/lib/assistant/aevatar-transport.ts:1693-1703`).

The cancelled `GET /conversations/chatc-...` is thus consistent with reconciliation waiter cleanup, canonical-route/query-key transition, or component unmount. It cannot cancel or starve the separate workflow stream, and it cannot change previously emitted printable block events into `printed: false`.

### Can today's code make printed content appear unprinted?

No new provenance or reconciliation branch writes the episode's `printed` field. Only the event pump owns it (`frontend/src/hooks/use-assistant.ts:517-553`, `frontend/src/hooks/use-assistant.ts:623-631`). Canonical-ID projection copies the existing episode to the canonical key rather than recreating its printed state (`frontend/src/hooks/use-assistant.ts:136-159`). Live-turn interlocks preserve the transport mirror rather than replacing it.

The one known way for a persisted answer to coexist with `printed: false` is the older create-recovery path described above: it applies History directly and emits only settlement. That is not evidence for this incident because the screenshot reportedly contains no reply, and the supplied request list does not show the create-recovery poll that a context-free first turn normally requires.

## Scratch reproduction

A temporary Vitest test and config were created under `/tmp`, imported the real `AevatarAssistantTransport` and unmocked `ChatStreamWorkerClient`, and replaced only `fetch` with controlled `Response` objects. The test used the same `eventPrintsContent` predicate as the hook. It passed three cases:

```text
200-empty-clean-eof
events:   ["turn.completed"]
terminal: failed, error.code="stream_closed"
printed:  false

200-body-read-dies
events:   ["turn.completed"]
terminal: failed, error.code="network_error"
printed:  false

200-nonprinting-terminal
events:   ["turn.status", "turn.completed"]
terminal: completed, error=null
printed:  false
```

Vitest result: 1 file passed, 3 tests passed. Both scratch files were deleted afterward; no test or config was added to the repository.

## What would settle the production verdict

Capture one failing `workflow-chat` exchange with all of the following:

- response `Content-Type` and raw SSE records or the Assistant wire-log line capture;
- total body bytes and wire end outcome (`complete`, `network_error`, `cancelled`, or `protocol_cancel`);
- whether `aevatar.chat.context`, `TEXT_MESSAGE_*`, `aevatar.raw.observed`, `RUN_FINISHED.result.output`, `RUN_ERROR`, or no terminal arrived;
- the client `turn.completed` status/error code; and
- the first materialized History response for the same `turnId`.

The built-in Assistant wire log is preferable to a request-list screenshot because it records SSE lines and the stream end outcome. A HAR with response/event-stream content plus the matching NyxID/Aevatar upstream wire log is also sufficient. If printable, contract-valid content is present on the wire but absent from pump events, the verdict becomes `CLIENT REGRESSION` with the failing frame adapter/parser branch. If the body is empty, nonprinting, terminal-only with blank output, or interrupted, the verdict becomes `UPSTREAM` (including proxy delivery between Aevatar and the browser).

## Recommended fix direction

### Option 1: Preserve the semantic error, render the actual terminal cause

Keep `EmptyTurnError` as the no-content fallback, but render the sanitized `ActiveTurn.error` in that assistant row and offer Retry. Add frame-count, printable-event-count, first-frame latency, last-frame latency, body-byte count, and end-outcome telemetry. This is the lowest-risk option: it does not fabricate content or replay an accepted workflow turn, and it immediately separates `RUN_ERROR`, `stream_closed`, `network_error`, protocol mismatch, and deadline failures. The tradeoff is that a server reply materializing shortly afterward can initially show an error until reconciliation updates the transcript.

### Option 2: Reconcile empty terminals before final empty-turn presentation

For a noncancelled terminal with `printed: false`, run a short, bounded History check keyed by the required `turnId`. If a printable assistant row materializes, project it and make episode answer detection agree with the transcript; otherwise show the exact terminal error. This also closes the existing create-recovery hole where recovered History contains an answer but the pump never sees block events. The tradeoff is extra History traffic and delayed error presentation, so it must remain turn-ID/fence-gated and must not suppress a real failure indefinitely.

Do not treat HTTP 200 as a successful turn, and do not remove `EmptyTurnError`: both would return the UI to a silent blank response when the upstream stream genuinely produces nothing.
