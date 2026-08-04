VERDICT: REWORK

# P1 findings

## P1 - D1 clears the turn fence for outcomes that can still materialize an answer

**Plan section:** D1 / W2 (`newchat-followup-fix.md:56-101,263-289`).

**Evidence:** `historyIncludesAssistantTurn(entries, null)` is indeed vacuously true (`frontend/src/lib/assistant/aevatar-transport.ts:820-829`), and the state-version comparison still rejects a pre-fence response (`:3247-3265`). That proves a `null` fence cannot be satisfied by an older state version. It does **not** prove that the first fence-current transcript is final. The transport explicitly says a `stream_closed` run may still finish server-side and appear on a later history reload (`:4068-4078`), yet W2 specifically requires `stream_closed` to clear on the first response without the turn (`newchat-followup-fix.md:280-281`). Workflow cancellation has the same ambiguity: it aborts only the browser stream because workflow has no stop control, and starts create recovery when needed (`aevatar-transport.ts:5611-5625`); the upstream turn may continue.

The proposed content predicate is also not authoritative. Context-free create recovery waits for History containing `recovery.turnId`, applies that History directly to the mirror, and only then emits settlement (`aevatar-transport.ts:3568-3613,5432-5443`). It never sets `run.sawText`, `currentMessageId`, `activityMessageId`, or `openCards`. The plan's claim that the mirror's recovered answer makes the run predicate true (`newchat-followup-fix.md:66-75`) is therefore false. At terminal emission, the proposed assignment overwrites the recovery fence with `null`.

Finally, `sawText` is not the pump's print fact: any nonempty whitespace delta sets it (`aevatar-transport.ts:4379-4388`), while the pump requires `trim().length > 0` (`frontend/src/hooks/use-assistant.ts:491-503`). `currentMessageId` is cleared before normal terminal settlement (`aevatar-transport.ts:5380-5386,5445-5468`), `openCards` is cleared during finalization (`:5533-5552`), and `activityMessageId` is the useful structured-content marker. The proposed member list both overstates its coverage and diverges from the real print contract.

**Concrete failure:** A workflow sends context, then the browser receives clean EOF. W2 assigns `requiredTurnId = null`; an immediately available `{messages: [user], stateVersion: current}` response settles the reconciler. Aevatar commits the assistant row moments later, but this tab has stopped observing it. The same premature settlement can occur after Stop. Separately, a context-free create recovery can already contain a printable assistant row, but the predicate still reports no run content and discards its exact turn fence.

**Required correction:** Keep `projectionPending`, but key `requiredTurnId` to whether server-side assistant materialization is still possible, not merely whether this browser printed. Preserve the exact turn fence for create-recovered History, delivery interruption (`stream_closed`/`network_error`), and workflow cancellation; use `null` only for an authoritative content-free terminal whose contract rules out a later assistant row. Represent that distinction explicitly on `RunningTurn` rather than reconstructing it from UI fields. If ambiguous outcomes need a shorter budget than 90 seconds, give them a bounded outcome-specific policy. Replace ledger row 5, and add real-transport cases for delayed post-EOF materialization, workflow cancellation, context-free create recovery, whitespace-only text, and structured-only output.

## P1 - D3 is not Option 2 and can render an error beside a recovered answer

**Plan section:** D3 and the D1 limitation (`newchat-followup-fix.md:97-101,138-162`).

**Evidence:** With a null required turn, the reconciler is a single first-fence-current observation, not a bounded turn-ID-gated check (`aevatar-transport.ts:2098-2101,3247-3265`). It has no window in which to observe an answer that materializes after that first response. If an answer is already present, `applyHistoryResponse` does project it into the mirror (`:3146-3149,3200-3244`), but that does not make the episode answered. The pump alone owns `printed` (`frontend/src/hooks/use-assistant.ts:517-553,623-631`), and `ChatThread` treats an explicit `turnPrinted === false` as authoritative over printable transcript content (`frontend/src/components/assistant/chat-thread.tsx:529-545`). No reconciler path updates the episode.

This is not speculative: the existing create-recovery path is a known case where History is applied directly and the pump sees only settlement (`aevatar-transport.ts:3568-3613,5432-5443`; `newchat-followup-diag-turn.md:113-117`). The plan acknowledges that path but reaches the opposite conclusion about its state.

**Concrete failure:** Recovery projects an assistant message, then emits a completed terminal. The page renders the recovered answer, but the episode remains `{open:false, printed:false}`. After 700 ms, `ChatThread` appends "The assistant didn't reply" to the answer's own assistant group. For a preserved `RUN_ERROR`, W4 can do so immediately. Thus D3 does not deliver Option 2's central benefit: making answer detection agree with recovered History.

**Required correction:** Choose the contract explicitly. Either implement the actual short, turn-ID/fence-gated check from Option 2 and publish a proven-answer fact that suppresses the empty-turn row, or retain Option 1 but make a turn-correlated printable History row override `printed:false` for empty-turn presentation. Do not use an uncorrelated tail check, because approval continuations can append to an earlier assistant group. Add an integration test that drives the real create-recovery transport through the hook/page and asserts that a recovered answer never coexists with the failure row. The current W2/W4 ledger does not cover this seam.

## P1 - The reconciler repeats the guaranteed-doomed terminal read under a different caller

**Plan section:** D2 / W1 and the unchanged reconciler schedule (`newchat-followup-fix.md:103-136,231-261`).

**Evidence and usefulness by entry provenance:** A new entry is inserted and passed straight to `resumeReconcileEntry` (`frontend/src/lib/assistant/aevatar-transport.ts:1957-1980`); `resumeReconcileEntry` invokes `runReconcileObservation` directly (`:2003-2017`). Only attempts after that return reach `scheduleReconcileEntry` and its jittered delay (`:2163-2195`). Thus attempt 0 ignores the policy's nonzero 250 ms floor (`frontend/src/lib/assistant/backoff.ts:8-12,22-32`). This conflicts with the round-one premise that a new `chatc-` transcript is absent until terminal plus asynchronous projection (`docs/plans/new-chat-projection-race.md:9-16,48-60`).

The three creation modes do not justify one start policy:

1. **Same-tab, post-terminal `projectionPending`: not useful.** The stream mirror already contains everything the browser knows, and the terminal is the earliest possible projection boundary. A transcript GET in that same beat is doomed by the architecture's own premise.
2. **Cold canonical entry from receipt/raw-index evidence: an immediate read is useful, but it normally already happened.** With no local record, `getHistory` first calls `loadHistory`; after its 404 it uses receipt/raw-index evidence to return an empty pending mirror (`aevatar-transport.ts:1674-1685,1728-1807`). The hook then creates the reconciler (`frontend/src/hooks/use-assistant.ts:325-361`), whose immediate attempt duplicates the just-failed read. The initial cold read should remain immediate; the reconciler should inherit that observation and schedule the *next* one. An explicit Retry from stalled state can also justify an immediate observation.
3. **`identityPending` cold create recovery: useful and a different operation.** A command-only receipt is served from the mirror (`aevatar-transport.ts:1653-1668,1704-1715`), then reconciliation calls `create-recovery/{commandId}` rather than transcript History (`:2055-2073`). The create may have committed before reload, so an immediate recovery poll is reasonable and should retain its separate policy.

There is no exact projection-ready signal in the current browser contract. The context `stateVersion` is only a fence; raw index membership proves existence, not transcript materialization (`aevatar-transport.ts:1864-1893,3247-3265`). Obtaining a newer transcript fence itself requires a transcript read. Therefore deferring the fresh attempt can still produce a later 404, but it removes a **guaranteed** 404 and gives async projection a chance. The principled client fallback is the existing jittered backoff policy, not a new fixed sleep. A server projection-ready event, status endpoint, or `Retry-After` would be the exact solution, but none exists in the scoped code.

**Concrete failure/noise:** Every new chat currently emits one transcript GET immediately after the stream closes. It adds a deterministic 404/cancelled line beside the real SSE exchange and is precisely the wire-log noise that made this incident look like a transcript failure. Across tabs it also creates synchronized unnecessary work before jitter begins.

**POST interference answer:** This GET did **not** cause the reported empty POST. On the normal path, `consumeTurnStream` waits for worker body completion before settling the terminal (`aevatar-transport.ts:4016-4053`), and only terminal projection mounts the reconciler, so attempt 0 is downstream of that POST's body. The controllers are independent. Overlap is nevertheless possible with a fast continuation started while a reconciliation GET is still in flight: the transport notices the live turn only after the GET returns and discards/reschedules that observation (`:2080-2097`), rather than aborting it. A second tab has an independent transport and can likewise reconcile while another tab streams. Under HTTP/2 these requests normally multiplex; under HTTP/1.1 the GET consumes one of the browser's per-host slots. One GET should not block an SSE POST by itself, but it can marginally contend for connection slots, bandwidth, upstream capacity, and database work, especially across tabs. It is waste, not the causal explanation for this turn printing nothing.

**Required correction:** Add initial-observation provenance (at minimum `post_terminal`, `cold_observed`, `identity_recovery`, and `explicit_retry`) plus a retained `nextAttemptAt`/last-observation fact. Fresh post-terminal entries must schedule attempt 0 through `PROJECTION_BACKOFF_POLICY`; cold `getHistory` 404s should seed the entry as an already-consumed observation and schedule the next attempt; identity recovery and explicit Retry may start immediately. Do not merely put every attempt behind one delay. Add real-transport tests asserting no transcript GET in the terminal task, one delayed attempt after the policy fires, no duplicate immediate GET after cold evidence, and immediate use of the create-recovery endpoint.

This changes the W1 interleaving and must be planned with it. On the fresh path, release now clears a pending timer rather than aborting a fetch; a same-commit re-acquire must restore the **remaining** delay from `nextAttemptAt`, not call `resumeReconcileEntry` and accidentally issue the GET immediately. W1's `pausedByAbort` repair is still required for later in-flight attempts, cold/Retry immediate observations, tombstones, continuations, and second-tab overlap. Split the ledger into a timer release/re-acquire test and the existing in-flight-abort release/re-acquire test.

## P1 - W4's "immediate" failure remains blocked by `transcriptSettling`

**Plan section:** D3 grace rules / W4 (`newchat-followup-fix.md:158-162,303-338`).

**Evidence:** The current error predicate requires `!transcriptSettling` before it enters the 700 ms settle (`chat-thread.tsx:535-545`). On every terminal, the pump immediately starts a projection, increments `projections`, and publishes `projecting:true` before the async read settles (`use-assistant.ts:561-580,623-657`). The page passes that bit directly as `transcriptSettling` (`frontend/src/pages/assistant.tsx:548-560`). W4 says only that `turnError != null` bypasses the settle delay; it never says that a declared failure bypasses `transcriptSettling`, and its component test defaults that prop to false.

**Concrete failure:** `RUN_ERROR` arrives while the terminal projection takes five seconds or reaches `PROJECTION_DEADLINE_MS`. The planned "immediate" branch is still suppressed for that whole interval even though D3 says a declared failure is not an out-of-order risk.

**Required correction:** Specify two predicates: declared failure renders immediately when the episode is closed and unprinted, independent of both `useSettled` and `transcriptSettling`; inferred empty retains the existing settling guard and 700 ms grace. Add a component test with `turnError` and `transcriptSettling={true}`, plus a page-level test with `{open:false, printed:false, projecting:true}`. Both must show the declared cause immediately.

# P2 findings

## P2 - W5 leaves the pre-400 ms cold state underspecified

**Plan section:** D4 / W5 (`newchat-followup-fix.md:173-217,340-374`).

The precedence gate itself is sound for the three requested lifecycle cases. A second tab has no episode; if History supplied messages, `messages.length > 0` correctly silences projection furniture. A reload during the first turn has no episode and can honestly show the cold catch-up state once History reports pending. On a same-tab switch back, the disabled episode query retains the pump-written closed slot under its conversation key (`use-assistant.ts:393-416,517-552`), so the turn outcome remains the winner; this is desirable for a failed empty turn.

The render transition is not sound enough to implement as written. Before the proposed 400 ms `useSettled` fires, an empty `ChatThread` with no episode still takes its ordinary "Start a new conversation" early return (`chat-thread.tsx:551-580`). Removing the page strip can therefore show a false empty-chat state for 400 ms, then replace it with catch-up.

**Correction:** Pass the raw cold-sync condition separately from its settled visibility. Raw cold sync must suppress the ordinary empty state immediately; only the copy/dots are delayed. Add ledger cases for (1) no "Start a new conversation" during the 400 ms window, (2) a second tab with messages, and (3) switch-away/switch-back with a cached closed episode. The existing row 13 checks only the post-threshold state.

## P2 - D2 is sound, but W1 must preserve the abort return

**Plan section:** D2 / W1 (`newchat-followup-fix.md:103-136,231-261`).

The chosen single-owner repair is correct for an **in-flight** observation. On abort, returning from the catch skips the post-`finally` scheduler at `aevatar-transport.ts:2169`, so the guarded scheduler in `finally` is the only one. `settleReconcileEntry` deletes both map keys before the unwind can reschedule (`:2232-2244`); an external tombstone leaves the entry mapped, so one rescheduled observation reaches the deleted/deleting check and settles `absent` (`:1896-1905,2019-2029`). `waiters === 0` parks a genuine pause, while re-acquisition either happens before unwind and is detected in `finally`, or happens afterward and the existing resume path starts it. There is no lost momentary-zero interleaving. The new P1 above requires a separate timer-pause contract; `pausedByAbort` alone cannot preserve a deferred attempt's due time.

W1's wording, however, says the abort returns "set" `pausedByAbort` "instead of returning blind." The implementation must be explicitly `pausedByAbort = true; return;`. Falling through after setting the flag would continue attempt/deadline work under an aborted signal and could combine the `finally` scheduler with the outer `.catch` scheduler. Add an assertion that only one retry timer/second observation is created in the same-tick test.

## P2 - W6 conflates wire completion with transport settlement

**Plan section:** W6 (`newchat-followup-fix.md:376-409`).

The existing wire outcome is `complete | cancelled | network_error | worker_error | protocol_cancel` (`frontend/src/lib/assistant/chat-stream-worker-protocol.ts:10-15`; schema at `frontend/src/schemas/assistant-wire-log.ts:105-111`). Clean SSE EOF emits wire outcome `complete` (`chat-stream.worker.ts:352-361`); only afterward does the transport classify missing AG-UI terminal as `stream_closed` (`aevatar-transport.ts:4068-4079`). W6's test expects a single end outcome of `stream_closed`, which is not a wire outcome, and its file list omits the schema/store contract that must carry the new metadata.

**Correction:** Define two fields and enums: the existing wire/body outcome and a client settlement outcome/error code. Name all touched files (`assistant-wire-log` schema, store, transport attachment/export path, and tests). Count printable **turn events** using the same semantic predicate as D1, not merely AG-UI frames. Test clean EOF as `wireOutcome: complete` plus `transportOutcome: stream_closed`, and test a dying body separately as `network_error`.

## P2 - The verification and post-mortem gates are incomplete

**Plan sections:** W7, Test ledger, Post-mortem (`newchat-followup-fix.md:411-488`).

The ledger's current fails-on-`main` labels are credible, and rows 1-3 correctly live in `aevatar-transport.test.ts` against the real transport. Rows 4-15 also have falsifiable assertions, with guards labelled. The problem is coverage of the plan's actual risks: the ledger omits every P1 seam above, and row 5 codifies the wrong `stream_closed` behavior. It also lacks the attempt-0 request-count/timing cases even though the old plan's nonzero backoff floor was meant to prevent bursts. That repeats the post-mortem's first failure in a new form: there is a numbered ledger, but it still does not include the known create-recovery/pump disagreement, outcome-specific liveness cases, or first-observation schedule.

W7 also omits the required standalone `npm run test`; `test:coverage` is not a substitute for stating every CI gate. Add `npm run lint`, `npm run test`, `npm run test:coverage` (15% line threshold), and `npm run build`, while retaining the no-dependency/no-lockfile and Wizard Bundle Freshness checks. Critical Rule 4 is unaffected by the planned UI work: no forms or non-text form controls are introduced, and W6 correctly forbids `console.log`.

The post-mortem remedies are only partly enforced. The numbered ledger and D2 interleaving tests enforce remedies 1-2. Remedy 3 is contradicted by D1's outcome-blind use of browser printing for ambiguous server outcomes. Remedy 4 says this plan must contain a coexistence table for every existing screen state, but the plan only points to Opus's sibling table and omits loading, history-error, pre-threshold cold sync, and cached-episode transitions. Put the corrected outcome/liveness matrix and UI coexistence table in this plan itself, then map each load-bearing row to a numbered test.

# D1 adjudication

Both diagnoses were partly right. Opus was right that a content-free authoritative terminal must not wait 90 seconds for an assistant row that cannot exist, but the literal instruction to stop setting `projectionPending` was too broad: that flag suppresses doomed foreground reads and is required for create recovery and ambiguous delivery outcomes. Fable was right to separate "background projection is pending" from "this exact assistant turn must be present," and the null helper semantics are exactly as stated. Fable was wrong to equate browser-printed content with server materialization expectation, wrong that recovered History populates the proposed run fields, and wrong that first-fence-current settlement supplies Option 2's bounded late-answer check.

The corrected rule is: keep `projectionPending`; set or retain `requiredTurnId` according to the server-side outcome contract. Clear the turn requirement only for a terminal known not to be capable of a later assistant row. Preserve it, under a bounded policy, for interrupted delivery, workflow cancellation, and create-recovered History, and make a proven turn-correlated History answer suppress the empty-turn presentation.
