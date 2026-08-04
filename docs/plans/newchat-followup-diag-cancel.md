# Diagnosis: the canceled `chatc-…` GET on a brand-new chat

Scope: who issues the transcript GET observed as `(canceled)` in DevTools on a
fresh chat's first turn, whether the cancel is benign, and why a read fired at
all. Companion to two sibling diagnoses (A: why the turn printed nothing;
B: what the UI should show) — neither is re-litigated here.

All file:line references are against this worktree, which is byte-identical to
prod `main` @ `67f6f374` for the three files involved
(`git diff origin/main -- use-assistant.ts aevatar-transport.ts assistant.tsx`
is empty).

## VERDICT

**EXPECTED-BUT-HARMFUL.** The GET is the W4 background reconciler's first
transcript observation — issued exactly where the plan says it should be. The
cancel is the designed "pause on last-waiter release", triggered by the
placeholder→canonical query-key migration. The harm is a race the design did
not close: the same React commit that releases the placeholder waiter (abort)
immediately re-acquires under the canonical id, the resume no-ops because the
aborted observation is still flagged `running`, and the aborted observation's
exit path never reschedules — **the reconcile entry is orphaned forever**. From
that moment this tab can never materialize, time out, or tombstone the
conversation: `awaitingProjection` stays `true`, the "Syncing conversation
history..." banner is permanent, and the stalled→Retry affordance (the designed
escape hatch) is unreachable because it requires the dead loop to settle
`timed_out`.

## Issuer identification

The canceled request is the **reconciler's transcript observation**, not the
`useConversation` query's fetch and not the pump's projection read:

- `runReconcileObservation` issues
  `GET ${ASSISTANT_PREFIX}/conversations/${entry.conversationId}` **with an
  abort signal** — `assistantApi.get(..., controller.signal)`
  (`frontend/src/lib/assistant/aevatar-transport.ts:2076-2078`), where
  `controller` is a scope controller (`:2049`). `assistantApi.get` forwards the
  signal into `apiClient` (`aevatar-transport.ts:196-207`), which lives in the
  shared api-client chunk — matching the observed `auth-store-*.js:3`
  initiator.
- The alternatives are ruled out structurally:
  - `useConversation`'s `queryFn` (`frontend/src/hooks/use-assistant.ts:329`)
    ignores TanStack's `signal`, and the wire read it can reach —
    `loadHistory` (`aevatar-transport.ts:3123-3130`) — calls
    `assistantApi.get` **with no signal at all**. A request that carries no
    abort signal cannot appear as client-`(canceled)`; a TanStack "cancel" on
    key change only detaches the observer.
  - The pump's `projectTransportState` reads (`use-assistant.ts:121-174`) go
    through the same signal-less `getHistory`/`loadHistory` path — and on this
    conversation they never touch the network anyway (see next section).
  - `fetchRawIndexMembership` does take a signal (`:1864-1873`) but its URL is
    the bare `/conversations` index; the DevTools entry is named by the
    conversation id, i.e. the transcript route.
- Timing corroborates: the reconciler's first observation runs with **zero
  delay** (`reconcileProjection` → `resumeReconcileEntry` →
  `runReconcileObservation`, `:1979/:2003-2017` — the backoff timer only gates
  attempts ≥ 1), so the GET fires in the same beat as the turn's terminal —
  right after `workflow-chat` closes at ~4.6s, exactly as observed. And it is
  the **only** abortable GET on this route in the codebase.

## Why a transcript GET fired for a brand-new chat at all

By design, and the suppression held. The plan's architecture is "mirror
authoritative, transcript GET is background reconciliation" — and W4's
reconciler **is** that background reconciliation; its loop body is defined as
"transcript GET; on 404, continue" (`docs/plans/new-chat-projection-race.md`
§2.5). Sequence on this trace:

1. The turn terminal sets `projectionPending = true` + `requiredTurnId` in
   `emit` (`aevatar-transport.ts:3329-3343` — the branch matches because the
   adopted id starts with `chatc-`, `WORKFLOW_CONVERSATION_PREFIX` `:260`),
   plus `lastLocalTurnCompletedAt`. This happens for failed turns too — the
   condition is only `turn.completed` + state change + `streamDispatched`.
2. The pump's post-terminal projection (`use-assistant.ts:561-576`) calls
   `getHistory(placeholder)`; the W3 suppression branch
   (`aevatar-transport.ts:1704-1715`) serves the local mirror with
   `awaitingProjection: true` and **no network call** — the user's "hi" is in
   `turnState.messages`, so the branch applies. `projectTransportState`
   mirrors this data under both the placeholder and canonical query keys
   (`use-assistant.ts:131-160`, W6).
3. `useConversation`'s effect (`use-assistant.ts:337-361`) sees
   `awaitingProjection === true` and starts `reconcileProjection` — whose
   first observation is the GET in the trace.

So the W5 worry in the brief — a receipt-synthesized record forcing a wire read
the mirror should have answered — is **not** what happened: the record exists
with local messages, the query-path suppression worked, and the wire read is
the reconciler doing its designed job. The GET *should* fire here; it is the
only mechanism that can ever clear `projectionPending` in this tab, because the
suppression branch at `:1704` short-circuits every future `getHistory` to the
mirror while `projectionPending` is set.

## Who cancels it, and the orphaning race

The cancel is `releaseProjectionWaiter` (`aevatar-transport.ts:1983-2001`)
aborting the in-flight controller when waiters hit 0 — invoked by the
`useConversation` effect **cleanup** when its `conversationId` dep changes from
the placeholder to the canonical id. That key migration is guaranteed on every
new chat: `assistant.tsx`'s swap effect (`:212-274`) navigates
`?c=workflow-pending-…` → `?c=chatc-…` once the turn is no longer live — i.e.
immediately after the same render that started the reconciler (the reconcile
effect registers first, at the `useConversation` call `assistant.tsx:144`,
before the swap effect at `:212`, so effects flush in that order: GET fires,
then navigation).

The next commit then runs, synchronously in one task:

1. **Cleanup (old effect, placeholder id):** `releaseProjectionWaiter`
   canonicalizes through the alias, finds the entry, `waiters 1→0`, aborts the
   in-flight GET → the DevTools `(canceled)`.
2. **New effect (canonical id):** the canonical-keyed query already holds the
   mirrored `awaitingProjection: true` data (set at `use-assistant.ts:136-141`
   and again at `assistant.tsx:246-249`), so it calls
   `reconcileProjection(chatc-…)` → existing entry → `waiters 0→1` →
   `resumeReconcileEntry` (`:1941-1944`).
3. `resumeReconcileEntry` **no-ops**: `entry.running` is still `true`
   (`:2004`), because the fetch's abort rejection is delivered
   asynchronously — it cannot have propagated inside the same synchronous
   effect phase.
4. A microtask later the aborted observation unwinds:
   `if (controller.signal.aborted) return;` (`:2104`) → `finally` sets
   `entry.running = false` (`:2166`) — and **returns without reaching
   `scheduleReconcileEntry` at `:2169`**. Nothing ever calls resume again:
   the effect's deps (`conversationId`, `awaitingProjection === true`) are now
   stable, so the effect never re-fires.

End state: `{ waiters: 1, running: false, timer: none, promise pending
forever }`. Verified with a scratch state-machine model (exact transcription of
`resumeReconcileEntry`/`runReconcileObservation`-abort-path/
`releaseProjectionWaiter`, abort rejection delivered as a microtask), run under
node and deleted:

```
after start:    { running: true, waiters: 1, inFlight: true }
after swap:     { running: true, waiters: 1 }     ← resume no-opped
steady state:   { running: false, waiters: 1, timer: false, settled: false }
RESULT: ORPHANED — loop dead, promise pending forever
```

Test-coverage note: the transport suite covers single-flight with both waiters
held (`aevatar-transport.test.ts:8490-8512`) and the deadline→stalled path, but
the plan's promised test — *"pausing on last-waiter release aborts the
in-flight attempt without settling and resumes from the stored attempt"* — was
never shipped in the transport suite; `releaseProjectionWaiter` appears only in
hook-level tests against a transport double (`use-assistant.test.tsx:764`,
`use-assistant.aevatar.test.tsx:321`), which cannot exercise the race.

## Harm analysis

- **`transcriptSettling` / EmptyTurnError: disproven as an interaction.**
  `transcriptSettling` is `episodeState?.projecting === true ||
  sendMessage.isPending` (`assistant.tsx:558-560`); `projecting` counts
  in-flight `projectTransportState` calls in the pump
  (`use-assistant.ts:533, 551, 566-575`) and never references the reconciler.
  The aborted GET neither sets nor clears it. The EmptyTurnError
  (`chat-thread.tsx:535-545`) shows because the turn genuinely ended unprinted
  and the pump's projection settled instantly against the mirror — it would
  render identically with or without the cancel. (Why the turn printed nothing
  is sibling A's question.)
- **`awaitingProjection` stuck true: proven.** Only three things clear
  `projectionPending`: materialization via `applyMaterializationObservation`
  (`:3262-3264`), tombstoning, or scope reset. The first two are owned by the
  now-dead loop, and the query path can never substitute — `getHistory`
  short-circuits to the mirror at `:1704-1715` whenever `projectionPending` is
  set with local content. The "Syncing conversation history..." banner
  (`assistant.tsx:520-527`) is therefore permanent in this tab.
- **Stalled/Retry unreachable: proven.** `projectionStalled` is set only in
  `settleReconcileEntry` on `timed_out` (`:2232-2239ff`), which the orphaned
  entry never reaches. The 90s deadline passes silently; W7's designed escape
  hatch (Retry button, `assistant.tsx:504-519`) never renders.
- **Attempt/deadline accounting: no attempt consumed.** The abort returns at
  `:2104`, before `entry.attempt += 1` (`:2110`). The deadline is not
  "burned" — worse, it expires unobserved.
- **Unhandled rejection: disproven.** The abort is caught at `:2104`; the
  entry promise is resolve-only (`:1957-1960`) and simply never settles; the
  hook's `.then` chain never runs (so the terminal-outcome invalidations never
  happen either) and carries a defensive `.catch` (`use-assistant.ts:356`).
- **Rescue paths (why a reload "fixes" it):** switching to another
  conversation and back, or sending another message on this conversation
  (whose terminal flips `awaitingProjection` undefined→true again), re-fires
  the effect; with `running` now false, resume works, one observation runs,
  and the long-past deadline settles the entry on that first observation. A
  reload rebuilds everything. None of that helps the user staring at the
  banner + error after their first "hi".

## Plan-vs-shipped divergence (brand-new chat, first turn, same tab)

| Step | Plan intent (`new-chat-projection-race.md`) | Shipped behaviour | Divergence |
| --- | --- | --- | --- |
| Terminal → provenance | `projectionPending` + `requiredTurnId` set at workflow terminal (§2.1, W3) | `emit` `:3329-3343` | none |
| Post-terminal `getHistory` | mirror served, no wire GET (W3/D2) | branch `:1704-1715`, no network | none |
| First reconciler observation | transcript GET under the projection policy (§2.5) | GET issued immediately at terminal | none — this is the observed request |
| Pause on last-waiter release | abort in-flight; loop returns without settling; entry stays (§2.5 "Entry structure & pause semantics") | `releaseProjectionWaiter` + abort-return path `:2104` | none in isolation |
| **Resume** | *"Resuming (a waiter registers again): spawn a new loop continuation from the stored attempt/deadline"* — unconditional | `resumeReconcileEntry` no-ops while `entry.running` is true (`:2004`); the abort window leaves `running=true` until an async rejection, and the abort exit never reschedules | **implementation miss**: the resume contract is violated for a resume that lands inside the abort window |
| Key-migration release→re-acquire | the plan's own hook sketch keys the effect on `[conversationId, awaitingProjection]` (§2.5) and canonicalizes aliases to one entry (P3) — so a placeholder→canonical swap *necessarily* does release-then-reacquire in one commit | exactly that happens, on every new chat | **plan gap**: the plan mandated the sequence that triggers the race but never analyzed same-commit release→resume; not a deferred case — it simply wasn't seen |
| Deadline → stalled + Retry | `timed_out` → `projectionStalled` → quiet notice with Retry (§2.5, W7) | unreachable once orphaned | consequence of the above |
| Pause/resume regression test | W4 test list: "pausing on last-waiter release aborts … and resumes from the stored attempt" | not present in the transport suite | **test-plan miss** — the one test that could have caught this was dropped |

Honest summary: the plan was right about the architecture (the suppression
works; the GET is legitimate) and wrong about its own concurrency detail. The
pause/resume design assumed release and resume are temporally separated; the
plan's other components (effect keyed on `conversationId` + alias
canonicalization + the mandated key migration) guarantee they are not — on the
exact flagship path the PR existed to fix. The shipped code implemented the
ambiguous spec literally, and the planned test that encoded the resume contract
was not shipped.

## Recommended fix direction (not implemented)

1. **Honor the resume contract in the abort exit path (recommended).** In
   `runReconcileObservation`'s abort returns (`:2070`, `:2104`, and the
   scope-change returns), after the `finally` clears `running`, reschedule when
   the entry is still wanted: `if (entry.waiters > 0 && entry.scopeId ===
   this.ownerScopeId && this.reconcileEntries.has(entry.conversationId))
   this.scheduleReconcileEntry(entry)`. This is sound because an abort with
   `waiters > 0` can only mean release-then-reacquire (release with remaining
   waiters never aborts) — i.e. the abort itself is the resume signal.
   Smallest possible diff, no hook changes, and the plan's promised transport
   test (release + re-acquire in the same tick, assert a follow-up observation
   and eventual settle) becomes the regression guard. Tradeoff: none
   meaningful; a spurious extra observation after a genuine
   pause-then-quick-resume is exactly what resume is supposed to do.
2. **Alternative: stop releasing across the key migration.** Key the
   `useConversation` reconcile effect on the *canonical* id
   (`query.data?.conversation.id ?? conversationId`) so the placeholder→
   canonical swap doesn't change the dep and never releases. Tradeoff: fixes
   only this trigger, not the underlying broken resume contract (unmount-and-
   remount within the abort window — conversation switch away/back, StrictMode-
   style double effects — still orphans); leaves the transport API's stated
   semantics false. Could complement option 1 as churn reduction, but should
   not replace it.
