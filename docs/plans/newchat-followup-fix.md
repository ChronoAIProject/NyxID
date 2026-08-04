# Plan: new-chat empty-turn follow-up — orphaned reconciler, outcome-blind provenance, turn-outcome precedence (rev 2)

Branch `fix-new-chat-empty-turn-ui`, base `main` @ `67f6f374`. Frontend-only.
No new dependencies, no lockfile change, no wizard-manifest files touched.
Rev 2 incorporates the GPT-Sol adversarial review
(`docs/plans/newchat-followup-fix.review-gpt.md`, verdict REWORK); the
`## Review response` section maps every finding to its resolution.

## Direction change (2026-08-04, during implementation)

New production evidence from the user: on the failing new chat, **the stream
stops, the error shows, and a refresh reveals the assistant's reply already in
the conversation.** The turn succeeds upstream; `EmptyTurnError` is a false
negative, and the review's P1-2 coexistence case (error rendered beside an
answer that materializes) is the *actual* production behavior, not a
theoretical seam. User direction: do not display the error (log the detection
instead), remove the syncing banner entirely, and make the materialized reply
render into the open thread without a refresh. UI refinement comes later.

Consequences for this plan:

- **D3 and D4 are superseded for this PR.** No failure row, no catch-up
  state, no stalled banner: projection provenance and empty-turn detection
  render *nothing*. The empty-turn condition is kept as an invisible
  `data-empty-turn-detected` attribute on the thread root (detection
  preserved for tests/DOM inspection); the durable record is W6's
  transport-side telemetry (`transportOutcome` + printable-event counts).
- **W3, W4, W5 are DEFERRED, not deleted.** The honest failure row with the
  sanitized cause + `Try again` (W3/W4) and the in-thread cold states +
  DESIGN.md reshape (W5) remain the refinement backlog; their D3/D4 design
  text and ledger rows L7-L18 stay in this document as the spec for that
  later PR. The turn-correlated `answerProven` suppression (D3a) is
  prerequisite work for any future failure row — the new evidence makes it
  mandatory, since the answer routinely arrives after the terminal.
- **W1 and W2 are unchanged and more load-bearing than before:** W1's
  un-orphaned reconciler is what delivers the late reply into the open thread
  (the refresh the user performs manually today is exactly the invalidation
  the orphaned entry never fires), and W2's preserved turn fence for
  `stream_closed`/`network_error` is what stops the reconciler settling on a
  transcript that predates the reply.
- Shipped in the UI layer instead of W3-W5: removal of both status strips and
  the error render sites, the detection attribute, and a real-transport
  integration test (`use-assistant.aevatar.test.tsx`, "empty-stream late
  materialization") pinning that a late-materializing answer renders into the
  mounted conversation through the reconciler's own invalidation — across the
  placeholder→canonical swap — with no refetch call and no cache write after
  the swap.
- Ledger status for this PR: L7-L18 deferred with W3-W5. The shipped UI tests
  are: the no-refresh integration test (fails on `main`: the orphaned entry
  never invalidates), the no-error-rendered and no-banner tests (fail on
  `main`: the alert and strips render there), and the detection-semantics
  conversions in `chat-thread.test.tsx` (three retitled `[guard]`).
- The stream truncation itself remains out of scope and unexplained
  client-side; W6's capture runbook is how it gets settled.

Inputs (all file:line refs verified against this worktree, byte-identical to
`main` @ `67f6f374` for every file named):

- `docs/plans/newchat-followup-diag-cancel.md` — same-commit release→re-acquire
  during the placeholder→canonical key migration orphans the reconcile entry;
  the round-one pause/resume transport test was never shipped.
- `docs/plans/newchat-followup-diag-ux.md` — `projectionPending` set on every
  workflow terminal (`aevatar-transport.ts:3329-3344`); for a content-free
  turn, materialization is unsatisfiable and the stalled Retry re-arms an
  impossible 90s wait. Precedence rule + case table + DESIGN.md UI proposal.
- `docs/plans/newchat-followup-diag-turn.md` — why the turn printed nothing is
  UNDETERMINED without the body; provably not today's client change;
  `EmptyTurnError` semantically correct, copy discards the sanitized cause;
  capture list; fix options 1/2.

## Scope

**In:** the orphaned reconcile entry; the reconciler's first-observation
schedule (the guaranteed-doomed terminal read the review confirmed as P1);
outcome-contract-aware `requiredTurnId`; precedence between turn outcome and
transcript state; rendering the real terminal error immediately with a working
retry; suppressing the failure row when a turn-correlated answer exists in the
transcript; the missing pause/resume transport tests; capture runbook +
metadata-only stream telemetry.

**Out:** making the turn succeed (upstream, unproven — the UI must not imply
success); Opus case (d) new absent-conversation surface (the confirmed-stale
redirect at `assistant.tsx:233-239` covers it); `transcriptSettling` general
rework beyond the declared-failure bypass (see D3 — the outcome-blind
`projections` decrement and `waitWithDeadline` pass-as-settle,
`use-assistant.ts:91-103,127-130,571-575`, stay as the bounded suppressor for
the *inferred* case only, filed as a follow-up note in W6's doc); the
foreground preflight ladder (unchanged round-one deferral); partial-print
failure presentation (turn printed, then failed → toast, as today).

**Non-bug — do not "fix":** both siblings proved the canceled GET does not
un-suppress the empty-turn error (`transcriptSettling` never observes the
reconciler). No work item may wire the reconciler into `transcriptSettling`.

---

## Decisions

### D1 — `requiredTurnId` keyed to the server-side outcome contract, not to
### whether this browser printed

The review is right that rev 1's content predicate was wrong twice over
(verified independently):

- `stream_closed` explicitly documents that the server-side run may still
  finish and surface on a later reload (`aevatar-transport.ts:4068-4079` —
  "the server-side run may still finish, in which case the full reply
  surfaces on the next history reload"), yet rev 1 required exactly that case
  to clear on the first fence-current read without the turn.
- Workflow cancellation aborts only the browser stream and cannot prove the
  create/turn was rejected (`:5612-5619` starts create recovery for that
  reason) — same ambiguity.
- `settleRecoveredWorkflowCreate` applies recovered History directly and never
  sets `sawText`/`currentMessageId`/`activityMessageId`/`openCards`
  (`:3568-3613`); rev 1's claim that the recovered mirror made the run
  predicate true was false, and the terminal emission would have overwritten
  recovery's exact fence with `null`.
- The member set diverged from the print contract: `sawText` is set by any
  non-empty delta including whitespace (`:4380-4383`, no trim) while the pump
  requires `trim().length > 0` (`use-assistant.ts:491-503`);
  `currentMessageId` is cleared in `settleDeliveryTerminal` before terminal
  emission (`:5380-5386` → `closeOpenMessage`); `openCards` is cleared during
  finalization (`:5552`).

**Corrected rule.** `projectionPending` is still set on every workflow
terminal (unchanged from rev 1 — it suppresses doomed foreground reads and is
required for recovery and ambiguous delivery). `requiredTurnId` is keyed to
whether a later server-side assistant row is still possible, represented by
**two explicit `RunningTurn` fields set at emission/settlement sites, never
reconstructed from UI-ish state**:

- `run.assistantContentObserved: boolean` (default `false`) — set `true` at
  exactly the sites that emit pump-grade printable content: a
  `TEXT_MESSAGE_CONTENT` delta with `delta.trim().length > 0` (`:4379-4390`),
  `emitStaticText` (`:5340-5359`), and the structured/card/activity block
  emission sites (block/card events carry non-text content — same semantic
  predicate as the pump's `eventPrintsContent`). This field is also the
  printable-turn-event counter's predicate in W6.
- `run.serverAnswerExpectation: "possible" | "none"` (default `"possible"`,
  conservative) — set to `"none"` at exactly one site:
  `settleDeliveryTerminal` for `deliveryTerminal.kind === "finished"`
  (`completed`/`blocked`, `:722`) when `run.assistantContentObserved` is
  false. An authoritative RUN_FINISHED with nothing printable is the only
  terminal whose contract rules out a later assistant row.

Emit gate (`:3335-3343`):
`requiredTurnId = run.serverAnswerExpectation === "none" ? null : run.turnId`.

#### Outcome → fence/liveness matrix (P2-4 requirement)

| Terminal outcome | `requiredTurnId` | `projectionPending` clears via | Bounded by | User-visible (after D4 gate) |
| --- | --- | --- | --- | --- |
| RUN_FINISHED, printed content | `run.turnId` | fence-current read containing the turn | 90s `PROJECTION_BACKOFF_POLICY` | nothing (case b) |
| RUN_FINISHED, no printable content | `null` | first fence-current read (`historyIncludesAssistantTurn(entries, null)` vacuously true, `:820-829`; fence check still rejects pre-fence reads, `:3247-3265`) | one observation, else 90s if the row never materializes | nothing (failure/empty row owns the screen) |
| RUN_ERROR | `run.turnId` (conservative — contract on whether a row may still commit is unknown; cost of preserving is silent bounded polling, cost of `null` is premature settlement) | turn-bearing read, else timeout | 90s, silent | failure row with cause |
| `stream_closed` / `network_error` | `run.turnId` (server may still finish, `:4070-4071`) | turn-bearing read → answer projected → failure row suppressed reactively (D3), else timeout | 90s, silent | failure row with cause; disappears if the answer lands |
| Cancelled (Stop) | `run.turnId` (`:5612-5619`) | turn-bearing read, else timeout | 90s, silent | nothing (cancelled is excluded from `turnEnded`, `assistant.tsx:452-455`) |
| Create-recovery settled | `recovery.turnId` — recovery sets `stored.requiredTurnId` (`:3587`) and `run.turnId = recovery.turnId` (`:3607`); expectation stays `"possible"` (default), so the emit gate re-derives the same id, never `null` | already materialized by recovery's own `applyHistoryResponse` (`:3608-3612`) | n/a | the recovered answer (D3 suppresses the failure row) |

No outcome-specific shorter policy: the review allowed one "if needed"; it is
not — every ambiguous outcome is silent under D4's gate and bounded by the
existing 90s deadline, and one fewer policy object is one fewer thing to
review. Stated limitation (unchanged): an answer materializing after the 90s
deadline is picked up by reload or the next turn's preflight, not by the
settled reconciler.

### D2 — reconciler lifecycle: origin-scheduled first observation, timer-aware
### pause/resume, abort-exit reschedule

The review confirmed (P1) that attempt 0 bypasses the backoff floor entirely —
`reconcileProjection` inserts the entry and calls `resumeReconcileEntry`
(`:1957-1980`), which invokes `runReconcileObservation` directly
(`:2003-2017`); only attempts ≥ 1 reach `scheduleReconcileEntry`
(`:2163-2195`). On the flagship same-tab path this reintroduces the exact
doomed-terminal-read pattern round one existed to remove, one call site over.
Accepted in full. There is no projection-ready signal in the browser contract
(context `stateVersion` is a fence; raw index membership proves existence, not
materialization, `:1864-1893`), so deferral uses the existing jittered policy
— **no fixed sleep is introduced**.

**Entry fields added:** `origin: "post_terminal" | "cold_observed" |
"identity_recovery" | "explicit_retry"` and `nextAttemptAt: number |
undefined` (the retained due time of a scheduled observation).

**Origin derivation at entry creation** (transport-internal; no new public
parameters):

| Condition at `reconcileProjection` | Origin | First observation |
| --- | --- | --- |
| `stored.identityPending` | `identity_recovery` | immediate — different endpoint (`create-recovery/{commandId}`, `:2062-2064`), own policy; the create may have committed before reload |
| `stored.projectionStalledAt !== undefined` (the path `:1947-1950` already special-cases) | `explicit_retry` | immediate — the user asked |
| `stored.lastWireObservationAt` within the current task/beat — a new stored field stamped by `getHistory`'s cold-evidence 404 paths (`:1744-1766`, `:1771-1787`), which have *just consumed* a real transcript read | `cold_observed` | seed `attempt = 1`, `nextAttemptAt = now + nextBackoffDelay(policy, 0, random)`; the entry inherits the observation instead of duplicating the just-failed GET |
| otherwise (same-tab post-terminal) | `post_terminal` | `attempt 0` scheduled through `PROJECTION_BACKOFF_POLICY` — `nextAttemptAt = now + nextBackoffDelay(policy, 0, random)`, timer set, **no GET in the terminal task** |

`deadlineAt` stays anchored at entry creation; a deferred attempt 0 runs
inside the same 90s budget.

**Pause/resume becomes timer-aware.** `releaseProjectionWaiter` on the last
waiter clears a pending timer (existing `:1992-1995`) but **retains
`nextAttemptAt`**; `resumeReconcileEntry`, when not running and no timer, must
check `nextAttemptAt`: still in the future → set a timer for the *remaining*
delay; past (or unset, for immediate-origin entries) → observe now. This is
what makes the same-commit timer release/re-acquire (the key-migration
sequence on the fresh path, which after the origin change holds a timer, not a
fetch) restore the remaining delay instead of firing the doomed GET
immediately.

**Abort-exit reschedule (unchanged from rev 1, wording hardened per P2):** the
in-flight abort returns at `:2070` and `:2104` become explicitly
`pausedByAbort = true; return;` — **the `return` is load-bearing**; falling
through after setting the flag would continue attempt/deadline work under an
aborted signal and double-schedule via the outer `.catch` (`:2013-2016`). The
`finally` (`:2163-2168`), after `entry.running = false`, runs:
`if (pausedByAbort && entry.waiters > 0 && entry.scopeId ===
this.ownerScopeId && this.reconcileEntries.has(entry.conversationId))
this.scheduleReconcileEntry(entry)`. Scope-mismatch returns (`:2020`, `:2066`,
`:2080`, `:2124`) never reschedule. `settleReconcileEntry` deletes both map
keys before its abort (`:2236-2244`), so a settled entry cannot reschedule; a
tombstoned-mid-observation entry reschedules once and settles `absent` at the
deleted-check (`:2024-2029`). This path remains necessary for in-flight
attempts ≥ 1, cold/Retry immediate observations, tombstones, and second-tab
overlap.

Why abort-exit reschedule over resume-side probing (unchanged rationale): a
resume that starts a second loop while the first unwinds races both
`entry.running` and `entry.controller` — two owners for one transition. The
observation that held the controller decides what happens after it dies.
Genuine pause vs re-acquire stays distinguishable **by state**
(`entry.waiters` at unwind / at release), never by timing.

### D3 — Option 1, with turn-correlated answer suppression and a genuinely
### immediate declared-failure path

Two review P1s land here; both accepted.

**(a) A recovered or late-materialized answer must suppress the failure row.**
The pump alone owns `printed` (`use-assistant.ts:517-553`), no reconciler or
recovery path updates the episode, and `ChatThread` treats explicit
`turnPrinted === false` as authoritative (`chat-thread.tsx:529-545`) — so on
the create-recovery path the page would render the recovered answer *and*
append "The assistant didn't reply" after the 700 ms grace. Fix: the
empty-turn presentation gains a **turn-correlated override**, not an
uncorrelated tail check (approval continuations append to an earlier
assistant group — the documented reason `turnPrinted` exists):

- `ChatThread` receives `turnId: string | null` (from `turn.data?.turnId`,
  already in `ActiveTurn`, `types/assistant.ts:265`).
- `answerProven = messages.some(m => m.role === "assistant" &&
  safeTurnId(m.turnId) === turnId && hasPrintableContent([m]))` — History-
  projected messages carry `turnId` (`types/assistant.ts:147`;
  `latestAssistantTurnId` reads it, `aevatar-transport.ts:3423-3434`);
  live-streamed messages may not, but a live-streamed answer sets
  `printed: true` anyway.
- The failure/empty row renders only when `!tailAnswered && !answerProven`.
  Because the predicate is derived from `messages` on every render, it is
  **reactive**: a failure row already on screen (e.g. immediate
  `stream_closed`) disappears when the reconciler later materializes the
  correlated answer — which is the coexistence case that actually reaches the
  immediate path (see Review response, P1-2 pushback).

This also makes D1's preserved fences useful end-to-end: interrupted-delivery
turns whose answer lands within the 90s window go error → answer, replacing
rev 1's "error forever beside a silent mirror".

**(b) Declared failures bypass `transcriptSettling`, not just the grace.**
The pump publishes `projecting: true` on every terminal before its projection
settles (`use-assistant.ts:561-580`), and the page feeds that straight into
`transcriptSettling` (`assistant.tsx:558-560`) — so rev 1's "immediate" was
still suppressed for up to `PROJECTION_DEADLINE_MS`. Fix: **two predicates**
in `ChatThread`:

- **Declared failure** — `turnEnded && turnError != null && !tailAnswered &&
  !answerProven`: renders immediately, independent of both `useSettled` and
  `transcriptSettling`.
- **Inferred empty** — `turnEnded && !turnError && !tailAnswered &&
  !answerProven && !transcriptSettling`, behind
  `useSettled(…, EMPTY_TURN_GRACE_MS)` — unchanged 700 ms, unchanged settling
  guard (that guard's job — don't call a slow projection an error — is
  exactly the inferred case).

Option 2 (foreground turn-gated probe) stays rejected: D1's preserved fences
+ the reconciler are the bounded background check, and (a) closes the
presentation seam Option 2 was for — without a second read path racing the
reconciler, and without delaying declared failures behind a probe window.

### D4 — UI: precedence gate, in-thread cold states, coexistence table

Unchanged from rev 1 in intent (`DESIGN.md` anchors: l.13-14, l.41-42, l.65,
l.276-278, l.350-362 `ErrorBanner`, l.375-376 banner shapes, l.409, l.474,
l.49 `AmbientStatusLine` rule), with the P2 gaps closed:

- **Precedence gate** (unchanged): projection surfaces render only when
  `episodeState === undefined && messages.length === 0`.
- **Pre-400 ms cold window specified** (P2): the raw cold-sync condition and
  its settled visibility are **separate facts**. The page passes
  `coldSync: "none" | "pending" | "stalled"` (raw, un-delayed) to
  `ChatThread`; a non-`"none"` value suppresses the "Start a new
  conversation" early return (`chat-thread.tsx:551-580`) **immediately**;
  only the catch-up copy + `StreamingDots` appear behind the ~400 ms
  `useSettled`. During the window the thread renders the empty gutter —
  never the false empty-chat CTA, never a flash of catch-up for a fast
  materialization.
- Failure row copy/retry, catch-up copy ("Catching up on this
  conversation…"), stalled amber `rounded-xl` in-thread banner + Retry →
  `useRetryConversationProjection`: all as rev 1.
- **Out** (explicit, unchanged): case (d) surface; `transcriptSettling`
  internals; partial-print presentation.

#### UI coexistence table (P2-4: in this plan, not by reference)

States on the chat surface and the single winner per combination. "Cold" =
`episodeState === undefined`; "live" = episode exists (open or closed).

| # | Condition | Winner (only voice) | Test |
| --- | --- | --- | --- |
| 1 | history query loading, no messages | "Loading conversation..." (`assistant.tsx:543-546`, unchanged) | existing |
| 2 | `history.isError` | non-blocking error strip (`:534-542`, unchanged) — coexists with thread, by design | existing |
| 3 | live episode open (thinking/streaming) | thread + thinking/streaming affordances | existing |
| 4 | live episode closed, printed | answer on screen; projection state silent | L11 |
| 5 | live episode closed, unprinted, declared error | failure row with cause, immediate | L7, L8, L12 |
| 6 | live episode closed, unprinted, no error | failure row after 700 ms grace + settling guard | L9 [guard] |
| 7 | live episode closed, unprinted, transcript later gains the correlated answer | answer; failure row suppressed/removed | L10, L13 |
| 8 | cold, messages present (second tab / reload after materialization) | thread only; projection state silent | L17 |
| 9 | cold, empty, `awaitingProjection`, < 400 ms | empty gutter — no CTA, no catch-up copy | L15 |
| 10 | cold, empty, `awaitingProjection`, ≥ 400 ms | in-thread catch-up state | L14 |
| 11 | cold, empty, `projectionStalled` | in-thread amber banner + Retry | L16 |
| 12 | cold, empty, no pending provenance | "Start a new conversation" CTA (unchanged) | existing |
| 13 | switch-away/switch-back, cached closed episode under the conversation key (`use-assistant.ts:393-416`) | same as rows 4-7 — the cached episode keeps the turn outcome authoritative | L18 |
| 14 | conversation confirmed absent | existing confirmed-stale redirect (`assistant.tsx:233-239`) | existing (`assistant.test.tsx`) |

---

## Ordered work items

W1 and W2 are transport; W3 plumbs facts; W4/W5 consume them; W6 documents;
W7 gates. Ledger rows (L*) in the table at the end; every unmarked row must
fail on `main` @ `67f6f374`; **[guard]** rows pass on `main` and are labeled
regression guards, not new coverage (Opus verifies labels against pre-fix
source).

### W1 — Reconciler lifecycle: origin schedule, timer-aware pause/resume,
### abort-exit reschedule

- **Files:** `frontend/src/lib/assistant/aevatar-transport.ts`
  (`reconcileProjection` `:1907-1981`, `releaseProjectionWaiter`
  `:1983-2001`, `resumeReconcileEntry` `:2003-2017`,
  `runReconcileObservation` `:2019-2170`, `scheduleReconcileEntry`
  `:2172-2195`, `getHistory` cold-evidence stamps `:1744-1787`, entry type),
  `frontend/src/lib/assistant/aevatar-transport.test.ts`.
- **Change:** D2 in full — `origin` + `nextAttemptAt` entry fields, the
  origin derivation table, post-terminal attempt 0 through the policy, cold
  entries seeded as already-consumed observations
  (`stored.lastWireObservationAt` stamped in the evidence paths), immediate
  start only for `identity_recovery`/`explicit_retry`, timer-retaining
  release + remaining-delay resume, and the `pausedByAbort = true; return;`
  abort-exit reschedule with its guards.
- **Tests** (real transport, `stubFetch`, fake timers, injected
  `now`/`random` — pattern at `aevatar-transport.test.ts:8514-8536`):
  L1-L6, L19-L21.
- **Failure modes closed:** the orphaned entry (permanent syncing state; dead
  `timed_out`/`absent` transitions); the guaranteed-doomed transcript GET in
  the terminal task (wire-log noise that made this incident look like a
  transcript failure; cross-tab synchronized pre-jitter work); the duplicate
  immediate GET after a cold evidence read; the latent
  tombstone-mid-observation orphan.

### W2 — Outcome-contract `requiredTurnId`

- **Files:** `frontend/src/lib/assistant/aevatar-transport.ts` (`RunningTurn`
  `:3276-3310` new fields, `TEXT_MESSAGE_CONTENT` `:4379-4390`,
  `emitStaticText` `:5340-5359`, structured/card emission sites,
  `settleDeliveryTerminal` `:5380+`, emit gate `:3329-3344`),
  `frontend/src/lib/assistant/aevatar-transport.test.ts`.
- **Change:** D1 — `run.assistantContentObserved` (trim-consistent, set at
  printable-emission sites), `run.serverAnswerExpectation` (default
  `"possible"`, `"none"` only at finished-without-content), emit gate derives
  `requiredTurnId` from the expectation. No change to
  `settleRecoveredWorkflowCreate` or `applyMaterializationObservation`.
- **Tests:** L22-L27 (incl. the review-required cases: delayed post-EOF
  materialization, workflow cancellation, context-free create recovery,
  whitespace-only text, structured-only output).
- **Failure modes closed:** the unsatisfiable materialization criterion for
  authoritative-empty turns (90s burn, impossible-Retry loop at the source);
  rev 1's premature settlement for `stream_closed`/cancelled turns whose
  answer can still land; rev 1's recovery-fence overwrite.

### W3 — Plumb declared terminal outcome + turn id to the thread

- **Files:** `frontend/src/pages/assistant.tsx`,
  `frontend/src/components/assistant/chat-thread.tsx`.
- **Change:** `ChatThread` gains `turnError?: { code: string; message:
  string } | null` and `turnId?: string | null`, both sourced from
  `turn.data` (carried by `turnFromEvent`, `use-assistant.ts:176-188`;
  sanitized by the transport). Pass-through only; no new derivation in the
  page.
- **Tests:** covered by W4's; the props alone are not behavior.

### W4 — Honest failure row: immediate declared cause, correlated-answer
### suppression, `Try again`

- **Files:** `frontend/src/components/assistant/chat-thread.tsx`
  (`EmptyTurnError` `:347-358`, predicates `:529-545`, render sites `:691`,
  `:701-708`), `frontend/src/pages/assistant.tsx` (retry wiring),
  `frontend/src/components/assistant/chat-thread.test.tsx`,
  `frontend/src/pages/assistant.test.tsx`,
  `frontend/src/hooks/use-assistant.aevatar.test.tsx` (real-transport
  integration).
- **Change:** D3 — the two predicates (declared: independent of `useSettled`
  AND `transcriptSettling`; inferred: both guards unchanged), the
  `answerProven` turn-correlated override on both, the D4 copy (headline "The
  assistant didn't reply.", detail = sanitized `turnError.message` else "The
  reply ended before anything was sent."), and `Try again` (ghost, `size="sm"`,
  `text-[12px]`) → `onRetryTurn` → page resends the last user message through
  the existing send path; hidden while a turn is active or no user message
  exists.
- **Tests:** L7-L13.
- **Failure modes closed:** filler copy discarding a known cause; declared
  failures hidden behind grace and behind `transcriptSettling` (up to
  `PROJECTION_DEADLINE_MS`); the failure row rendered beside a recovered or
  late-materialized answer (the create-recovery presentation hole GPT-Sol's
  diagnosis flagged); recovery-by-retyping.

### W5 — Precedence gate + in-thread cold states

- **Files:** `frontend/src/pages/assistant.tsx` (`:504-527` strips deleted,
  gate + `coldSync` derivation), `frontend/src/components/assistant/chat-thread.tsx`
  (`coldSync` prop, empty-state suppression `:551-580`, catch-up + stalled
  rendering), `frontend/src/pages/assistant.test.tsx`.
- **Change:** D4 — gate `episodeState === undefined && messages.length === 0`;
  raw `coldSync` suppresses the empty-state CTA immediately; copy/dots behind
  ~400 ms `useSettled`; stalled amber `rounded-xl` in-thread banner with
  Retry → existing `useRetryConversationProjection` (`use-assistant.ts:365-391`,
  unchanged); `history.isError` strip unchanged.
- **Tests:** L11-L12, L14-L18.
- **Failure modes closed:** two contradictory voices; the syncing banner
  asserting an impossible sync; stalled+Retry over a live-episode thread;
  page-chrome status for a conversation-level fact; the false
  "Start a new conversation" flash in the pre-400 ms window; projection
  furniture over a populated second-tab thread.

### W6 — Capture runbook + metadata-only stream telemetry

- **Files:** new `docs/chat/08-wire-capture-runbook.md`; pointer from
  `docs/chat/07-testing-and-gaps.md`; `frontend/src/lib/assistant/aevatar-transport.ts`
  (counter attachment); `frontend/src/schemas/assistant-wire-log.ts`
  (`:105-111` outcome enum extended with the new fields — schema change,
  Critical Rule 4 lives here); the wire-log store/record path
  (`frontend/src/lib/assistant/assistant-wire-log-transport.ts` /
  `wire-body-capture.ts` as discovered); matching schema + transport tests.
- **Change:**
  1. **Runbook** — steps to settle the next UNDETERMINED: enable
     `experimental:aevatar-chat-wire-log` (`feature-flags.ts:21`; document
     the real enable path per `assistant-wire-log-panel.test.tsx:198-215` —
     account `enabled_features`), open the panel, reproduce one failing send,
     export. Required artifacts (verbatim from the GPT-Sol diagnosis):
     `Content-Type` + raw SSE records; body bytes + wire end outcome; which
     of `aevatar.chat.context` / `TEXT_MESSAGE_*` /
     `RUN_FINISHED.result.output` / `RUN_ERROR` / no terminal arrived; client
     `turn.completed` status/error; first materialized History for the turn.
     Decision rule: printable contract-valid content on the wire absent from
     pump events ⇒ client regression; empty/nonprinting/interrupted ⇒
     upstream. Also carries the deferred-follow-up note on
     `transcriptSettling` outcome-awareness (Scope/Out).
  2. **Telemetry — two distinct outcome fields** (P2 accepted: wire
     completion ≠ transport settlement): `wireOutcome` stays the existing
     enum `complete | cancelled | network_error | worker_error |
     protocol_cancel` (`chat-stream-worker-protocol.ts:10-15`, unchanged);
     new `transportOutcome` records the client settlement (e.g. `completed`,
     `stream_closed`, `network_error`, `RUN_ERROR` code, `cancelled`).
     Counters: frames seen, **printable turn events (D1's
     `assistantContentObserved`-grade predicate, not raw AG-UI frames)**,
     first-frame ms, last-frame ms. Metadata only, never payload; flag-gated;
     no `console.log`.
- **Tests:** L28-L29 (clean EOF ⇒ `wireOutcome: "complete"` +
  `transportOutcome: "stream_closed"`; dying body ⇒ `network_error` /
  `network_error`).
- **Failure mode closed:** re-diagnosis from screenshots; the four
  indistinguishable wire outcomes; conflating "the bytes ended cleanly" with
  "the turn succeeded".

### W7 — Gates

From `frontend/`: `npm run lint`, `npm run test`, `npm run test:coverage`
(15% line threshold), `npm run build` (tsc -b with
`noUncheckedIndexedAccess`; `tsc --noEmit` is not the gate). Verify
`git diff --name-only` ∩ `cli/src/wizard/bundle-meta/index.manifest` = ∅ and
the manifest untouched. No `package.json`/lockfile changes. Do not commit
(repo owner lands).

---

## Test ledger

Every unmarked row must be demonstrated failing on `main` @ `67f6f374` before
the fix commit; the PR body maps each row to its test file + name; dropping or
weakening any row requires a written PR-body note. Rows live against the real
transport unless the location says otherwise.

| # | Where | Test | On `main` |
| --- | --- | --- | --- |
| L1 | transport | post-terminal entry issues **no transcript GET in the terminal task**; one GET after the policy delay fires | fails (immediate GET) |
| L2 | transport | timer release + same-commit re-acquire restores the **remaining** delay — no immediate GET, fires at the original due time, exactly one timer | fails |
| L3 | transport | in-flight abort + same-tick re-acquire keeps the loop alive and settles; exactly one retry timer/second observation is created | fails (orphan) |
| L4 | transport | tombstone mid-observation settles `absent` for a still-registered waiter | fails (orphan) |
| L5 | transport | [guard] releasing the last waiter parks the entry without settling; a later waiter resumes from stored attempt/deadline | passes |
| L6 | transport | cold-evidence entry does **not** duplicate the just-consumed read (fetch count over the evidence path + first reconcile beat) | fails |
| L19 | transport | identity-recovery entry polls `create-recovery/{commandId}` immediately, not the transcript route | fails (transcript-first schedule differs) — verify on `main`; if it passes there, relabel [guard] with a PR-body note |
| L20 | transport | explicit Retry from stalled observes immediately | fails (new `nextAttemptAt` semantics) — same verification note as L19 |
| L21 | transport | scope change during a paused/deferred entry never reschedules or fires | fails |
| L22 | transport | authoritative-empty terminal (RUN_FINISHED, no content) materializes on the first fence-current read without its turn | fails |
| L23 | transport | `stream_closed` terminal **retains** its turn fence; a fence-current read without the turn does not settle; a later read containing the turn materializes and projects the answer (delayed post-EOF materialization) | fails |
| L24 | transport | workflow cancellation retains its turn fence | fails |
| L25 | transport | context-free create recovery keeps `recovery.turnId` through the terminal emission (no `null` overwrite) | fails under rev 1's design; on `main` verify — recovery fence survives today, so expect [guard] with note |
| L26 | transport | whitespace-only text does not count as assistant content (expectation `"none"`) | fails |
| L27 | transport | structured-only output (card/activity, no text) counts as content (fence retained) | fails |
| L7 | chat-thread | declared failure renders its sanitized cause immediately with `transcriptSettling={true}` | fails |
| L8 | chat-thread | `Try again` reports the retry intent | fails |
| L9 | chat-thread | [guard] inferred-empty keeps the 700 ms grace and the settling guard | passes |
| L10 | chat-thread | a turn-correlated printable assistant message suppresses the failure row despite `turnPrinted === false`; an **uncorrelated** assistant tail does not | fails |
| L11 | page | no projection surface over a live episode (closed, unprinted) | fails |
| L12 | page | declared failure visible immediately with episode `{open:false, printed:false, projecting:true}` | fails |
| L13 | hooks (real transport, `use-assistant.aevatar.test.tsx`) | create-recovery integration: recovered answer renders, failure row never appears | fails |
| L14 | page | cold empty thread shows in-thread catch-up state after the 400 ms settle | fails |
| L15 | page | pre-400 ms cold window: no "Start a new conversation" CTA, no catch-up copy | fails |
| L16 | page | cold stalled shows the amber in-thread banner; Retry calls the projection retry | fails (new copy/placement) |
| L17 | page | second tab with materialized messages: no projection furniture | fails |
| L18 | page | switch-away/switch-back with cached closed episode keeps the turn outcome authoritative | verify on `main`; expected [guard] with note (cached-episode behavior exists today) |
| L28 | transport | clean EOF records `wireOutcome: "complete"` + `transportOutcome: "stream_closed"` + zero printable events | fails (fields absent) |
| L29 | transport | dying body records `network_error` / `network_error` | fails (fields absent) |

Three rows (L19, L20, L25) plus L18 carry explicit verify-on-main
instructions: their fails-on-main status depends on behavior the plan changes
adjacent to behavior that already exists; the implementer must run them on
`main` and set the label from the result, in the PR body — a wrong guess in
this table must surface as a label correction, not silently.

---

## Review response

| Finding | Disposition | What changed |
| --- | --- | --- |
| P1-1 fence cleared for outcomes that can still materialize | **Accepted** — all cited evidence independently verified (`:4068-4079`, `:5612-5619`, `:3568-3613`, `:4380-4383` vs `use-assistant.ts:491-503`, `:5380-5386`, `:5552`) | D1 rewritten: outcome-contract rule, two explicit `RunningTurn` fields set at settlement/emission sites, `"none"` only for finished-without-content, outcome/liveness matrix added; W2 + L22-L27 replace rev 1's row 5 |
| P1-2 not Option 2; error beside recovered answer | **Accepted, with one mechanism correction** (below) | D3(a): turn-correlated `answerProven` override, reactive on `messages`; real-transport integration test L13; L10 pins correlated-vs-uncorrelated |
| P1-3 reconciler repeats the doomed terminal read | **Accepted in full** — this is the load-bearing finding; it reintroduced round one's removed pattern at a new call site | D2 rewritten: origin-scheduled first observation, `nextAttemptAt`, timer-aware pause/resume; W1 + L1-L2, L6, L19-L21; ledger split into timer-release (L2) and in-flight-abort (L3) tests as demanded |
| P1-4 declared failure blocked by `transcriptSettling` | **Accepted** — `use-assistant.ts:561-580` publishes `projecting: true` on every terminal; rev 1's test defaulted the prop | D3(b): two predicates, declared path independent of both guards; L7 (`transcriptSettling={true}`) and L12 (`projecting: true` at page level) |
| P2 pre-400 ms cold state underspecified | **Accepted** | D4: raw `coldSync` separated from settled visibility; suppresses the empty-state CTA immediately; L15, L17, L18 added |
| P2 abort return must stay a return | **Accepted** | D2 wording: `pausedByAbort = true; return;` explicit, single-scheduler assertion folded into L3 |
| P2 wire completion vs transport settlement conflated | **Accepted** — enum verified at `chat-stream-worker-protocol.ts:10-15` | W6: two fields (`wireOutcome` unchanged enum + new `transportOutcome`), schema/store/transport files named, printable counting uses D1's predicate; L28-L29 |
| P2 verification/post-mortem gates incomplete | **Accepted** | W7 adds standalone `npm run test`; ledger extended to every P1 seam; outcome/liveness matrix and UI coexistence table now live in this plan with row→test mapping; Post-mortem gains item 5 |

**Pushback (one, scoped):** P1-2 states "For a preserved `RUN_ERROR`, W4 can
do so immediately" as part of the create-recovery failure narrative. On the
recovery path specifically, that immediate variant is unreachable: create
recovery is entered only for pre-context/missing-terminal closures
(`streamWorkflowTurn` recovery trigger `:3540-3579`; a delivered `RUN_ERROR`
settles via `settleDeliveryTerminal` `:4035-4053` and never enters recovery),
and successful recovery emits a **completed** terminal
(`settleRecoveredWorkflowCreate` → `finishTurn(..., "completed")`), so the
recovery-coexistence case runs through the 700 ms inferred path, not the
immediate declared path. The immediate coexistence case that *is* reachable is
different: a `stream_closed`/`RUN_ERROR` failure row rendered immediately,
followed by a reconciler-materialized correlated answer (enabled by D1's
preserved fences). The correction stands unchanged — the reactive
`answerProven` override covers both mechanisms — but the test design follows
the corrected mechanism: L13 exercises recovery through the 700 ms path, L23
exercises the late-materialization path, and L10 pins the presentation
predicate for both.

---

## Post-mortem

This is the second attempt at this bug; rev 2 of the second attempt. What
round one got wrong, and what changes:

1. **The promised test that would have caught the shipped bug was dropped,
   silently.** Round one's W4 list included the pause/resume transport test;
   it was never written in the transport suite — only hook-level tests
   against a double, which structurally cannot exercise transport
   concurrency — and the orphan lived in that untested seam. **Change:**
   numbered ledger; implementer maps every row to a shipped test in the PR
   body; any drop is written down; Opus verifies labels against pre-fix
   source.
2. **Pause/resume was specified in prose and reviewed by prose.** The design
   never said who may spawn the continuation while the previous observation
   unwinds from an abort, and the plan's own hook sketch guaranteed the
   same-commit release→re-acquire. **Change:** lifecycle state machines get
   explicit commit-boundary interleaving analysis and a single owner per
   transition (D2 does both, now for timers as well as fetches).
3. **Provenance was designed outcome-blind — twice.** Round one never asked
   "what if the terminal produced nothing to project"; rev 1 of this plan
   then swapped in a *browser*-outcome predicate for a *server*-outcome
   question and got `stream_closed`, cancellation, and create-recovery wrong.
   **Change:** every pending-state must carry a liveness matrix — for each
   outcome, what clears it and what bounds it (D1's table); predicates about
   server state must be derived from the server-side contract
   (`deliveryTerminal`), not from UI-adjacent fields.
4. **New UI states were added without a precedence rule.** **Change:** the
   coexistence table for every state on the touched screen lives in the plan
   itself, each load-bearing row mapped to a ledger test (D4's table).
5. **A fix reintroduced the pattern it removed, at a different call site —
   and three review rounds missed it.** Round one's core premise was "never
   read the transcript at the terminal; the mirror is authoritative"; the
   reconciler it introduced then performed exactly that read, immediately, on
   every terminal — attempt 0 simply bypassed the backoff policy the same
   plan specified. **Change:** every plan that removes a pattern must end
   with an explicit sweep item: list the call sites the fix *adds*, and check
   each against the invariants the plan itself states (here: D2's origin
   table is that sweep's output; W7 inherits the check for future revisions —
   before gating, re-read the plan's own stated invariants against the new
   call sites in the diff).
