# Adversarial review — new-chat projection race (`fix-new-chat-timing`)

Reviewer: Opus 5. Base `origin/rollup-chat-2026-08-04` (== `main` @ `fde6041f`), 4 commits,
3270 insertions / 19 files. Frontend-only.

VERDICT: REWORK

One reproduced P1: the W8 keep-max relaxation, combined with the new always-on reconciler,
deletes the user's in-flight message from the transcript. This is the same class of
user-visible content loss PR #1304 fixed, reintroduced through a new path, and W8 shipped
with **zero** new tests. Everything else in the change is in good shape — the scope
boundary, the receipt/intent store, the evidence gating and the single-flight loop all hold
up under inspection, and the gates reproduce exactly.

---

## Gate results

All run from `frontend/` on a clean worktree at HEAD. Every implementer number reproduced.

**`npm run lint`** — reproduced (0 errors, 23 warnings):

```
/…/src/pages/ai-setup.tsx
  243:9  warning  The 'clients' logical expression could make the dependencies of useMemo Hook (at line 249) change on every render…

✖ 23 problems (0 errors, 23 warnings)
```

**`npm run test`** — reproduced (199 files, 2436 tests):

```
 Test Files  199 passed (199)
      Tests  2436 passed (2436)
   Start at  02:50:55
   Duration  21.83s
```

(The happy-dom `AbortError` / `catalog 401` lines in the log are pre-existing teardown noise,
not failures.)

**`npm run build`** — reproduced, green:

```
dist/assets/ssh-terminal-m7gQIClx.js   472.22 kB │ gzip: 123.58 kB
✓ built in 595ms
…
✓ built in 49ms
```

**Wizard bundle freshness** — verified independently:

```
$ cargo test -p nyxid-cli wizard_bundle_is_fresh
test wizard_bundle_is_fresh ... ok
test result: ok. 1 passed; 0 failed
```

**Diff hygiene** — verified: `git diff --stat origin/rollup-chat-2026-08-04...HEAD` restricted
to `*package.json`, `*package-lock.json`, `cli/src/wizard/**`, `backend/**` returns empty.
No backend change; the proxy stays opaque. Layering claim holds.

**No coverage padding (spot check)** — reverted `aevatar-transport.ts` to base, kept the new
modules and tests, ran the transport suite:

```
 Test Files  1 failed (1)
      Tests  7 failed | 166 passed (173)
 FAIL … > clears local mirrors on account switch but preserves same-user refreshes
 FAIL … > uses raw index membership as cold 404 evidence and then materializes
 FAIL … > single-flights concurrent projection waiters
 FAIL … > turns a deadline with continuing index membership into stalled provenance
 FAIL … > rejects a cold 404 after one raw absent confirmation
 FAIL … > deletes a dispatched unaliased create locally without a placeholder DELETE
 FAIL … > reads a pending placeholder from the local mirror without a doomed request
```

All 6 new transport tests fail on base source. No padding among the tests that exist. The
problem is the tests that **don't** exist — see P2-4. Worktree restored clean after each
experiment (`git status --porcelain` empty).

---

## P1 findings (block the PR)

### P1-1 — A fence-current shorter transcript deletes the user's in-flight message

`frontend/src/lib/assistant/aevatar-transport.ts:3112-3123` (predicate),
`:3363-3375` (`latestAssistantTurnId`), `:822-831` (`historyIncludesAssistantTurn`),
`:3068` (`applyHistoryResponse` — no active-turn guard).

**The defect.** W8's fourth clause — the one the GPT review demanded, "the latest local
assistant turn must be present in the server entries" — is **vacuous for exactly the
conversations this PR is about**. `latestAssistantTurnId` scans the local mirror for an
assistant message carrying a turn id (`:3363-3375`); messages produced by the live stream
carry none. It therefore returns `null`, and `historyIncludesAssistantTurn(entries, null)`
returns `true` unconditionally (`:826`). Clauses 1–3 (wrapped shape, fence-current, past the
15s grace) are all satisfiable during a normal projection lag, so a shorter server transcript
replaces a longer local mirror.

Separately, `applyHistoryResponse` has **no active-turn guard**, unlike its sibling
`mergeIndexEntry` (`:3015`, `if (existing && isTurnActive(…)) return;`). Before this PR that
did not matter, because nothing called `applyHistoryResponse` mid-stream. It matters now:
`getHistory`'s active-turn branch (`:1686`) returns `historyFromStored` (`:1805`), which
stamps `awaitingProjection: true` whenever `projectionPending` is set — and nothing clears
`projectionPending` when a *new* turn starts. So the `useConversation` effect
(`frontend/src/hooks/use-assistant.ts:337-359`) keeps a reconciler running straight through
the next turn, and its loop body calls `applyHistoryResponse` (`:2043`) with no interlock.

**Failure scenario.** New chat. Turn 1 completes → `projectionPending = true`,
`stateVersion = 3`, mirror `[user, assistant, assistant]` — no assistant carries a turn id.
Upstream projection lags past the 15s grace (the exact condition this PR exists to handle).
The user sends a follow-up; the optimistic user message is appended. A reconcile observation
lands in the window between that append and the assistant's first delivered block, carrying a
fence-current (`stateVersion: 4 ≥ 3`) transcript that only holds turn 1. Clause 4 passes
vacuously → replacement → **the message the user just sent disappears from the transcript
while the assistant answers it.**

**How I verified it.** Scratch test appended to the real suite (then reverted; worktree
verified clean), driving the real transport through `createConversation` → a full workflow
turn → a 20s clock advance → `sendMessage` → one `applyHistoryResponse` call with a
fence-current two-entry body:

```
"beforeMirror": ["user#-", "assistant#-", "assistant#-"],   ← no assistant turn id
"fenceBefore": 3,  "pendingBefore": true,
"midMirror":   ["user#-", "assistant#-", "assistant#-", "user#-"],   ← optimistic send
"afterMirror": ["user#T1", "assistant#T1", "assistant#-"]            ← user message GONE
```

On `main` the same input keeps the mirror: keep-max there is unconditional
(`comparableLocalMessageCount > messages.length → return existing`). This is a regression, not
a pre-existing hazard.

A control run on a *seeded* conversation (whose mirror does contain an assistant with a turn
id) kept the local mirror intact — confirming clause 4 does work when it is not vacuous, and
that the vacuity is the whole defect.

**Fix.** Both halves:

1. `applyHistoryResponse` (`:3068`) must return `existing` untouched when
   `isTurnActive(existing.turnState.activeTurn?.status)`, matching the guard `mergeIndexEntry`
   already carries at `:3015`. A background reconciliation read must never rewrite a mirror
   that a live turn is writing into.
2. `latestAssistantTurnId` (`:3363`) must fall back to `stored.requiredTurnId` when no local
   assistant message carries a turn id, so clause 4 is never vacuous for a conversation with a
   known local terminal. (Alternatively require `latestTurnId !== null` for replacement — but
   the `requiredTurnId` fallback preserves the intended convergence.)

Then add the three W8 tests the plan named in §4 and the diff omits — in particular
"a fence-current shorter read MISSING the latest local turn keeps the local mirror" written
against a *new-chat* mirror, not a seeded one, since the seeded shape hides the bug.

---

## P2 findings (should fix before merge)

### P2-1 — The reconciler runs during the whole first turn, polling create-recovery on every new chat

`aevatar-transport.ts:1686` + `:1805` (active-turn branch now stamps provenance),
`use-assistant.ts:337`, `aevatar-transport.ts:2133` (`adoptRecoveredReceipt`).

`sendMessage`'s create branch sets `identityPending = true` (`:2247`), so from the first token
of a new chat `getHistory` answers `awaitingProjection: true`, the page shows the
"Syncing conversation history…" strip (`pages/assistant.tsx:520`), and the hook effect fires
`reconcileProjection`. With `identityPending` set, the loop takes the recovery branch and polls
`create-recovery/{commandId}` on the backoff schedule **for the duration of a perfectly healthy
first turn**. On `main` create-recovery was only polled behind `workflowCreateNeedsRecovery`
(`:3560-3567`); there is no such guard here. This is a new doomed-poll loop on every new chat —
the wire-log pollution D2 set out to remove, reintroduced at a different address.

If one of those polls returns 200 mid-stream, `adoptRecoveredReceipt` (`:2133`) adopts identity
concurrently with the context frame, and unlike `recoverWorkflowCreate` (`:3543`) it does **not**
update `this.activeConversationId` — leaving the active pointer on the placeholder.

Fix: gate the recovery-mode loop behind the existing `workflowCreateNeedsRecovery` predicate (or
simply skip reconciliation while a turn is active on the conversation), and mirror the
`activeConversationId` update from `:3543` into `adoptRecoveredReceipt`.

### P2-2 — A cold load with a live receipt renders an empty chat and never reads the transcript

`aevatar-transport.ts:1666-1676` (synthesize on `findReceiptByConversation`) and `:1691-1696`
(pending short-circuit, no network).

On reload, `this.conversations` is empty. `getHistory(chatc-A)` finds a receipt, synthesizes a
`projectionPending` record with `EMPTY_TURN_STATE`, and the pending short-circuit at `:1691`
returns that empty mirror **without ever attempting the transcript GET**. Any conversation whose
receipt is still within its 24h TTL — i.e. every chat created in the last day, since
`retireReceiptAfterMaterialization` only fires in a tab that stayed open — cold-loads as an empty
chat with a syncing banner until the reconciler's first observation lands, and stays empty for
the whole backoff ladder if that read fails transiently. Before this PR the same load did one
direct read and rendered.

Fix: only short-circuit on `projectionPending` when the record has local content or a local
terminal fact; a synthesized-from-receipt record should attempt the wire read first and fall back
to the syncing mirror on 404 (which the `:1725` branch already does correctly).

### P2-3 — `deleteConversation` silently drops the wire DELETE when the receipt is missing

`aevatar-transport.ts:1424-1447`.

The new early-return branch triggers on `isPlaceholder && !pendingReceipt?.conversationId`. It
consults **only the receipt store** — never `conversationAliases`. If a placeholder has already
been aliased to `chatc-A` but its receipt is gone (cap-20 eviction, 24h TTL, storage disabled, or
`activeOwnerUserId === null` making every receipt call a no-op), the branch tombstones locally and
returns with no request. `main` would have canonicalized through the alias and issued a real
DELETE. The server row survives, permanently.

Reachable when the URL still carries the placeholder id at delete time (the repair effect in
`assistant.tsx:209-271` has not yet swapped the id, or a live turn is blocking the swap).

Fix: take the local-only branch only when the placeholder has neither a receipt-known canonical id
**nor** an entry in `conversationAliases`; otherwise fall through to the canonical DELETE path.

### P2-4 — The shipped test set is well short of the plan, and the gap is exactly where the P1 lives

Counted per file, base vs HEAD: `aevatar-transport.test.ts` 150→156, `use-assistant.test.tsx`
22→24, `assistant.test.tsx` 20→22, plus 3 backoff + 2 schema + 7 receipt-store = **22 new tests**
against roughly 35 named in the plan's §4.

Not implemented, all named in the plan:

- **W8 keep-max — zero new tests.** The plan named three (two falsifiable + one `[guard]`). This
  is the highest-risk edit in the change and the one that is broken.
- **W2** — the P1.2 resurrection-window test (recovery names `chatc-A` → wire DELETE observed →
  merged list omits it) and the reload-sweep test.
- **W3** — the `stateVersion: 0` context test, the context-free failed create, and
  "serves the mirror without a transcript request while projection is pending".
- **W4** — pause/resume, late-wake final observation, remote-delete `absent` transition,
  recovery-mode adoption.
- **W6** — the dual-key projection test.

The account-switch test that does exist asserts list emptiness and a `getHistory` throw, but not
the plan's "fetch spy shows no further A-id requests" — the abort half of W0 is untested.

This is not padding (what shipped is real), but the plan is part of the diff and claims this
coverage. Either write the missing tests or amend §4 to state what was deferred and why.

---

## P3 / notes

- `materializedStateVersion` (`:616`, written at `:3201`) is write-only — never read by any
  decision. Either use it in the materialization criteria or drop the field.
- `retireReceiptAfterMaterialization` (`assistant-receipt-store.ts:185`) schedules an untracked
  `setTimeout` on **every** materializing history read until the receipt is actually deleted, so a
  chat read repeatedly inside its 60s floor accumulates redundant timers. Idempotent, but it
  should be guarded or the handle stored.
- `resetScope` (`:1309`) clears `reconcileEntries` without settling their promises. A pending
  `useRetryConversationProjection` mutation would then stay `isPending` forever (Retry button
  permanently disabled). Settle abandoned entries as `timed_out` on scope reset.
- `mergeIndexEntry` (`:3007`) calls `listDeletionIntents()` once per index row, and each call runs
  a full `pruneState` sort of receipts and intents. O(n) prune passes per list fetch. Hoist the
  intent set out of the loop.
- `use-assistant.aevatar.test.tsx` changes `serverHistory`'s fixture from `stateVersion: 4` to
  `100`. I checked: this is a harness change only, it moves the fence *up* (making replacement
  more likely, i.e. stricter), and no #1304 card-content or ordering assertion was modified or
  removed (test count unchanged at 6). The "assertions preserved verbatim" claim holds.

---

## Verified-sound (do not re-litigate)

- **Gates.** Lint, test, build, wizard freshness all reproduce the implementer's numbers exactly.
  No `package.json` / lockfile / `cli/src/wizard/**` / backend changes. Frontend-only holds.
- **CLAUDE.md Rule 4.** Zod schema in `schemas/assistant-receipts.ts`, store in
  `stores/assistant-receipt-store.ts`, hooks in `hooks/use-assistant.ts`. No `console.log` and no
  raw `useForm` introduced (both greps clean).
- **W0 account scope, cross-user leakage.** I could not construct a path where B observes or acts
  on A's data. `resetScope` (`:1309`) aborts running turns, scope controllers and reconcile
  timers, and clears every map before flipping `ownerScopeId`. Post-await scope re-checks are
  present at the load-bearing points (`:1381`, `:1741`, `:1791`, `:2005`, `:2027`,
  `fetchRawIndexMembership:1874`). Receipts are namespaced per user in localStorage with an
  `ownerUserId` equality check on read (`assistant-receipt-store.ts:28`), so B's session cannot
  read A's blob; A's unresolved intents survive under A's key and resume on return, as designed.
  `cleanupDeletionIntent` re-checks `scopeId !== this.ownerScopeId` after every await
  (`:1315`, `:1331`, `:1344`, `:1362`) so an intent cannot fire a DELETE under another account.
  Same-user token refresh does not reset (the subscription compares ids, not tokens) — asserted by
  the shipped test.
- **W2 intent durability.** Intents live in a separate keyspace with their own cap (10) and are
  pruned by a separate scan (`pruneState`, `:44-49`), so receipt eviction genuinely cannot touch
  them. The intent is recorded *before* the receipt is deleted (`:1440-1447`). `mergeIndexEntry`
  skips ids named by an intent (`:3007`). The single-flight `deletionCleanup` map prevents
  duplicate sweeps within a tab.
- **W3 provenance / `stateVersion: 0`.** `applyMaterializationObservation` (`:3181`) requires
  `freshStateVersion >= max(1, positiveStateVersion(stored.stateVersion) ?? 1)`, so a `0` fence
  cannot satisfy it and the conversation parks in `projectionPending` until a positive-fence read
  containing the turn arrives — exactly what the existing regression at
  `aevatar-transport.test.ts:6806-6849` locks in, and that test is still green in the full run.
  Provenance is carried as optional success fields, never an `ApiError`.
- **W4 mechanics.** Single-flight keys on the canonicalized id and aliases are installed from the
  receipt before canonicalization (`:1907-1916`), so placeholder and canonical addresses reach the
  same entry. The shared promise is created with `resolve` only and is never rejected; pause aborts
  the controller and returns from inside `try` so `scheduleReconcileEntry` after the `finally` is
  skipped — no settle, attempt/deadline preserved. `fetchRawIndexMembership` rethrows only on
  explicit abort, which lands in `resumeReconcileEntry`'s `.catch` and is filtered by the
  waiters/scope check. The hook attaches a defensive `.catch` (`use-assistant.ts:355`).
  `nextBackoffDelay` (`backoff.ts:25`) is correctly floored: `random() === 0` yields exactly
  `floorMs`, so there is no zero-delay burst.
- **W5 raw-membership evidence.** `fetchRawIndexMembership` (`:1868`) computes `present` from the
  raw `response.conversations` array **before** calling `mergeIndexEntry` (`:1879-1882`). The
  merged local list can never be its own evidence. The cold-404-with-no-evidence path throws
  `AssistantConversationNotFoundError` after exactly one confirmation — no retry storm — and the
  shipped test asserts the fetch count.
- **`timed_out` is a recoverable state, not a spinner.** `settleReconcileEntry` sets
  `projectionStalledAt` (`:2173`); `historyFromStored` (`:1805`) then serves
  `projectionStalled: true` and drops `awaitingProjection`, so the hook effect's
  `awaitingProjection === true` guard cannot re-arm off the refetched snapshot. The page renders
  the stalled strip with a working Retry (`pages/assistant.tsx:504-518`), and
  `reconcileProjection` clears `projectionStalledAt` and restores `projectionPending` on retry
  (`:1919-1922`). No limbo.

---

## Unverified

- **Multi-tab behavior.** I did not exercise two tabs sharing one localStorage namespace. The
  `storage`-event rehydrate (`assistant-receipt-store.ts:260`) only invalidates the memory cache;
  the plan is honest that reconciliation is not coordinated. I found no correctness hazard by
  inspection, but I did not reproduce a two-tab race.
- **Real upstream fence behavior.** P1-1's clause-2 bypass depends on a lagging server transcript
  ever reporting `stateVersion >= ` the locally observed fence. I demonstrated the content loss
  with a fence of 4 against a stored fence of 3, which is what a *materialized-but-behind-the-live-
  turn* read looks like. I did not confirm against a live Aevatar instance how often the server's
  transcript fence sits at or above the last context-frame fence while the transcript is still
  short. The `applyHistoryResponse` active-turn guard (fix 1) closes the finding regardless of that
  frequency.
- **`activeOwnerUserId === null` in production.** P2-3's worst trigger is receipts no-oping because
  the auth store's `user` is null. I confirmed the module reads `useAuthStore.getState().user?.id`
  at import and updates via subscription, but did not trace every app-boot ordering to prove the
  transport can never dispatch a create before `user` is populated.

---

# Re-review (round 2)

Reviewer: Opus 5. Head now `73b7bb94`, 5 new commits on top of `58ab3594`
(the round-1 head). Round-1 findings above are left intact as the record.

VERDICT: REWORK

P1-1 is **not** closed. The guard landed in two of the three places it was needed and the
one place it is missing is exactly the window the implementer claims to have closed. My
round-1 reproduction still deletes the user's message, unchanged. The four other findings
(P2-1, P2-2, P2-3, all P3s) are genuinely fixed and I verified each; the coverage label is
honest. This is one call-site away from APPROVE.

## Gate results (round 2)

All re-run from a clean worktree at `73b7bb94`. Every reported number reproduced.

```
$ npm run lint
✖ 23 problems (0 errors, 23 warnings)

$ npm run test
 Test Files  199 passed (199)
      Tests  2446 passed (2446)
   Duration  46.62s

$ npm run build
dist/credential-accept/assets/credential-accept-Bk5hKdwf.js  154.03 kB │ gzip: 50.35 kB
✓ built in 47ms

$ cargo test -p nyxid-cli wizard_bundle_is_fresh
test wizard_bundle_is_fresh ... ok
test result: ok. 1 passed; 0 failed
```

2446 confirmed (up 10 from 2436), 199 files. Diff hygiene re-checked at the full range
`origin/rollup-chat-2026-08-04...HEAD`: no `package.json`, no lockfile, no `cli/src/wizard/**`,
no `backend/**`. Round-2 source touches only `aevatar-transport.ts` and
`assistant-receipt-store.ts`. Worktree clean after every experiment.

## Coverage accounting — the `[branch-regression]` label is honest

Verified directly, as asked. Checked out `58ab3594`'s `aevatar-transport.ts` +
`assistant-receipt-store.ts` under the round-2 test file and ran the four labelled tests:

```
 × [branch-regression] keeps a new-chat mirror untouched while a follow-up turn is active
 × [branch-regression] keeps a longer new-chat mirror when the current fence omits its required turn
 × [branch-regression] deletes an aliased placeholder through the wire when its receipt is gone
 × [branch-regression] reads a cold canonical receipt from the transcript before serving pending
      Tests  4 failed | 179 skipped (183)
```

All four fail against the pre-review implementation. They are branch regressions, not padding,
and the label is correctly applied. Plan §4's shipped-vs-deferred accounting is likewise
accurate against what I counted.

I also confirmed the claim that the W8 tests exercise the `requiredTurnId` fallback rather than
the seeded shape that hid the round-1 bug — `aevatar-transport.test.ts:7907-7912` asserts every
local assistant message has `turnId === undefined` and that `stored.requiredTurnId` is the
fallback source. That claim is true and it is the right assertion.

---

## P1 findings (block the PR)

### P1-1 (SURVIVES) — the active-turn guard misses the window it was added for

`aevatar-transport.ts:3124` (the guard), `:2079-2080` (the unguarded apply),
`:2032-2045` (the pre-check that does it correctly).

**What landed.** Three interlocks were added. Two are correct and consult the transport's
running-turn map: `getHistory` (`:1693`, `this.running.has(...) || isTurnActive(...)`) and the
reconciler's **pre-flight** check (`:2032`, same disjunction). The third —
`applyHistoryResponse` itself (`:3124`) — checks **only** `isTurnActive(existing.turnState
.activeTurn?.status)`. It does not consult `this.running`.

**Why that is the wrong predicate.** I measured the reducer's state immediately after a real
`sendMessage`:

```
"activeTurnAtSend": "completed"
```

Between `sendMessage()` and the first delivered frame, `activeTurn` still carries the *previous*
turn's terminal status. `isTurnActive` is false for the entire duration of the create/continuation
POST plus SSE header round trip — hundreds of milliseconds on every send. That is precisely the
"between `sendMessage()` and `RUN_STARTED`" window the implementer says it closed; it is closed in
`getHistory` and in the reconciler pre-check, and left open at the point where the mirror is
actually overwritten.

**Reachability.** The reconciler pre-check at `:2032` runs *before* the transcript GET is issued.
After the `await` at `:2075` the only re-check is scope (`:2079`); `applyHistoryResponse` is then
called at `:2080`. So: reconciler passes the pre-check (no turn in flight) → issues the GET → the
user sends a follow-up → the response lands → the guard at `:3124` sees `status: "completed"` →
replacement proceeds. The reconciler polls on a 250 ms–30 s ladder for up to 90 s while the user
reads and types, so there is an in-flight window on every poll.

**Reproduced, unchanged from round 1.** Same scratch test against the real transport at
`73b7bb94` (appended to the suite, run, reverted; `git status` verified clean):

```
"beforeMirror":     ["user#-", "assistant#-", "assistant#-"]
"midMirror":        ["user#-", "assistant#-", "assistant#-", "user#-"]   ← optimistic send
"activeTurnAtSend": "completed"                                          ← guard does not fire
"afterMidTurnRead": ["user#T1", "assistant#T1", "assistant#-"]           ← user message GONE
```

**Why the new test does not catch it.** `aevatar-transport.test.ts:7815`
("[branch-regression] keeps a new-chat mirror untouched while a follow-up turn is active")
hand-assigns `activeTurn: { turnId: null, status: "running", error: null }` at `:7857` before
calling `applyHistoryResponse`. That is the one state in which the guard fires, and it is not the
state a real `sendMessage` produces. The test validates the guard as written rather than the
behaviour it was written for. It does fail at `58ab3594`, so the label is honest — but it is
false assurance.

**Fix — at the call site, not in `applyHistoryResponse`.** Do **not** add a blanket
`this.running` check inside `applyHistoryResponse`: three legitimate in-run callers
(`:3589`, `:3649` create-recovery, `:3726` reservation-retry fence refresh) must still apply while
their own run is in `this.running`, and a blanket check would silently break the continuation
fence refresh. The correct change is at `:2079-2080` — re-evaluate the same `turnInFlight`
disjunction already computed at `:2032` after the await, and on a hit skip the apply, refresh
`entry.deadlineAt`, and reschedule, exactly as `:2038-2044` does. Then rewrite the test at
`:7815` to reach `applyHistoryResponse` through `reconcileProjection` with a real `sendMessage`
in the in-flight window, rather than hand-setting `activeTurn`.

**Severity note, stated honestly.** This is narrower than round 1: it needs a reconcile GET
already in flight when the user sends, and it self-heals at the next materialization (the server
transcript will contain the user message once the turn commits). It is still the silent
disappearance of the user's own message from the transcript while the assistant answers it — the
#1304 class — and the fix is a few lines at one call site.

---

## Verified fixed (I re-derived each; do not re-litigate)

- **P2-1 — healthy first turns are clean.** Confirmed by direct measurement, not by reading the
  summary. During a normal new-chat first turn: `awaitingProjection` is `undefined` and
  create-recovery request count is **0**. After the terminal, `awaitingProjection` becomes `true`
  as intended.

  ```
  "duringFirstTurn": undefined,
  "createRecoveryCallsDuringHealthyTurn": 0,
  "awaitingAfterTerminal": true,
  ```

  Mechanism checks out: `historyFromStored` (`:1823`) suppresses `awaitingProjection` when
  `turnInFlight || isTurnActive`, so the hook effect never arms during the turn and the syncing
  strip does not render. `runReconcileObservation` (`:2032-2045`) reschedules locally with a fresh
  post-terminal deadline and no network when a turn is in flight.
  `adoptRecoveredReceipt:2199-2201` now updates `activeConversationId` when the placeholder is the
  active address, matching `recoverWorkflowCreate:3543`.

- **Convergence is not frozen.** The specific risk I flagged — that a guard could trade content
  loss for a permanently stale mirror. It does not. With no turn in flight, past the grace, a
  fence-current read containing the required turn wins:

  ```
  "pendingBefore": true,  "requiredTurnId": "turn-d619…",
  "beforeMirror":  ["user#-", "assistant#-", "assistant#-"],
  "afterMirror":   ["user#T1", "assistant#T1", "assistant#-"],
  "pendingAfter":  false,  "awaitingAfter": undefined,
  ```

  The `latestAssistantTurnId` fallback to `safeTurnId(stored.requiredTurnId)` (`:3415`) closes the
  round-1 vacuity: clause 4 is now a real condition for new-chat mirrors. The unlabelled sibling
  test at `:7882` asserts the same convergence and is genuine net-new coverage.

- **P2-2 — cold receipt-backed load reads the wire first.** The pending short-circuit at
  `:1707-1715` now additionally requires `messages.length > 0 || requiredTurnId != null ||
  lastLocalTurnCompletedAt !== undefined`. A record synthesized purely from a receipt has none of
  those, so it falls through to the transcript GET and renders on success; on 404 it lands in the
  `existing` branch and serves the syncing mirror. `identityPending` still short-circuits, which is
  correct — a placeholder has no server address to read.

- **P2-3 — receiptless alias deletion.** `:1436` captures `conversationAliases.get(conversationId)`
  and `:1438-1441` requires both `!pendingReceipt?.conversationId` **and** `!aliasedConversationId`
  before taking the local-only branch. An evicted receipt can no longer suppress the canonical
  wire DELETE.

- **All four P3s landed as described.** `materializedStateVersion` removed from the interface and
  both spread sites; retirement timers keyed `owner\0commandId`, deduplicated at
  `retireReceiptAfterMaterialization:208-209`, cancelled in both `deleteReceipt:186-192` and
  `deleteReceiptForOwner:99-102`, and cleared in the test reset; `resetScope:1313-1320` settles
  abandoned entries as `timed_out` (over a copied array, so the later `clear()` is safe);
  deletion-intent ids computed once per index response and passed into `mergeIndexEntry`
  (`:1374-1381`, `:1879-1886`, `:3053-3059`).

- **Nothing regressed in the round-1 verified-sound areas.** Re-checked: the account-scope reset
  and post-await scope guards are intact (and now also settle waiters); intent durability still
  rests on separate keyspace/cap/prune; the `stateVersion: 0` criterion at
  `applyMaterializationObservation` is unchanged and the `:6806-6849` regression is green in the
  full 2446-test run; `nextBackoffDelay`'s floor is untouched; the #1304 assertions in
  `use-assistant.aevatar.test.tsx` are unchanged (file untouched in round 2, still 6 tests).

## The three deferrals — my honest assessment

- **W0 abort-spy test** (assert no further A-id requests after an account switch) —
  **legitimate hardening deferral.** The abort code exists and I traced it (`resetScope` aborts
  run controllers, scope controllers, and reconcile timers before clearing state); the shipped
  test covers the observable outcome (empty list, `getHistory` throws). A spy would be stronger
  but its absence hides no known defect.
- **W4 pause/resume, late-wake, remote-delete-mid-loop, recovery-adoption timing** —
  **legitimate deferral, with residual risk.** I re-derived all four by inspection in round 1 and
  again here and found them correct: pause aborts and returns from inside `try` so the post-`finally`
  reschedule is skipped and the entry keeps its attempt/deadline; `finalObservationDue` guarantees
  one observation after a late wake; the two-observation ≥10 s absent transition is coherent. These
  are the least-exercised paths in the change and the first place I would look at a future bug
  report, but nothing here is load-bearing for a defect I can name.
- **W6 pump-level dual-slot copy test** — **legitimate deferral.** The code
  (`use-assistant.ts:136-159`) is straightforward `setQueryData` mirroring guarded by
  `getQueryData(...) === undefined`, and the pre-existing pre-navigation copy in
  `assistant.tsx:243-254` remains the primary path. Low risk.

None of the three is load-bearing for correctness. The coverage gap that *was* load-bearing —
W8 — has been filled, and filled with the right assertions.

## Unverified (round 2)

- I did not measure the wall-clock probability of the P1-1 window in production. I established
  the window exists (`activeTurnAtSend: "completed"`), that the send-to-first-frame gap is a
  network round trip, and that the reconciler has a GET in flight on a 250 ms–30 s ladder for up
  to 90 s. I did not instrument a live session to estimate how often those overlap.
- Multi-tab behaviour remains unexercised, unchanged from round 1.

---

# Re-review (round 3 — final)

Reviewer: Opus 5. Head `eb77b3bc`, 2 commits on top of `73b7bb94`. Rounds 1 and 2 left
intact as the record.

VERDICT: APPROVE

P1-1 is closed on the reachable path, verified by a reproduction I built specifically to
drive the real reconciler rather than the private method — and confirmed by running that
same reproduction against the pre-fix source, where it fails. Convergence is not swallowed.
The double-schedule and livelock hazards I was asked to check are both absent. The rewritten
test is a genuine behaviour test, not an implementation mirror. All gates reproduce and
nothing in the round-2 verified-fixed list regressed. This is safe to merge against
`rollup-chat-2026-08-04`.

## Gate results (round 3)

```
$ npm run lint
✖ 23 problems (0 errors, 23 warnings)

$ npm run test
 Test Files  199 passed (199)
      Tests  2446 passed (2446)
   Duration  17.22s

$ npm run build
dist/credential-accept/assets/credential-accept-Bk5hKdwf.js  154.03 kB │ gzip: 50.35 kB
✓ built in 47ms

$ cargo test -p nyxid-cli wizard_bundle_is_fresh
test result: ok. 1 passed; 0 failed
```

Diff hygiene re-checked across the **full** range `origin/rollup-chat-2026-08-04...HEAD`:
`*package.json`, `*package-lock.json`, `cli/src/wizard/**`, `backend/**` all empty. Round-3
source delta is 19 lines in `aevatar-transport.ts` and nothing else, as stated. Worktree clean
after every experiment.

## P1-1 — CLOSED

### My round-2 reproduction no longer measures anything

Stated plainly because it matters for the record: my round-2 repro called
`internals.applyHistoryResponse(...)` **directly**. The fix went in at the call site, by my own
recommendation, so that repro still shows the message being dropped — and now proves nothing,
because nothing reaches `applyHistoryResponse` that way except the three in-run callers that
must apply. Running it verbatim would have produced a false REWORK. I rebuilt it.

### Real-path reproduction

New scratch test (appended to the suite, run, reverted; `git status` verified clean) that drives
the actual sequence: create → turn 1 to terminal → clock +20 s → `reconcileProjection` issues its
transcript GET against a **gated** fetch → real `sendMessage` for turn 2 whose stream hangs so the
run stays in `this.running` → the gated transcript response is then released.

HEAD (`eb77b3bc`):

```
"activeTurnAtSend": "completed",          ← still the same window; isTurnActive alone misses it
"runningKeys":      ["chatc-8bd999…"],    ← run keyed canonically → arm 1 fires
"midMirror":        ["user#-","assistant#-","assistant#-","user#-"],
"afterMirror":      ["user#-","assistant#-","assistant#-","user#-"],
"keptOptimistic":   true                  ← user's message SURVIVES
```

Same test, `73b7bb94` source (pre-fix):

```
"activeTurnAtSend": "completed",
"afterMirror":      ["user#T1","assistant#T1","assistant#-"],
"keptOptimistic":   false                 ← user's message wiped
```

The reproduction still enters the identical window — `activeTurnAtSend` is `"completed"`, so the
`isTurnActive` arm alone would not have caught it — and the fix catches it there. The window did
not move; it was closed.

### Convergence is not swallowed

Counter-check through the real reconciler, same harness with turn 2 never sent:

```
"outcome":     { "status": "materialized", "conversationId": "chatc-8bd999…" },
"beforeMirror":["user#-","assistant#-","assistant#-"],
"afterMirror": ["user#T1","assistant#T1","assistant#-"]
```

A legitimate fence-current read containing the required turn still wins and clears pending. The
early return costs nothing when no turn is in flight.

### Double-schedule: absent, by construction

`rescheduleAfterTurn = true` is assigned on the single line immediately preceding `return`
(`:2094-2095`), inside `try`. JS semantics: the `return` runs `finally` — which fires
`scheduleReconcileEntry` once (`:2167`) — and then exits the function, so the unconditional
`scheduleReconcileEntry` after the try/finally (`:2170`) is unreachable on that path. There is no
other assignment to the flag, so the two can never co-execute. No second timer, no doubled request
rate, no leak. The shipped test corroborates it from the outside by asserting `entry.attempt`
stays `0` — the guard returns before `entry.attempt += 1`, so the guard path does not consume
budget either.

### Deadline refresh is not a livelock

Each refresh requires a turn to be in flight, and while a turn is in flight the hook has already
flipped `awaitingProjection` off (`historyFromStored:1823`), so the effect's cleanup calls
`releaseProjectionWaiter` → `waiters === 0` → `scheduleReconcileEntry` returns early at `:2176`
without arming a timer. The entry parks, keeps its refreshed deadline, and resumes when the turn
ends. Extending the budget across a streaming turn is the correct semantic — streaming time should
not be charged against the projection deadline — and every extension is bounded by a turn that
itself sets a fresh `projectionPending`. When sends stop, the deadline expires normally into
`timed_out` and the stalled UI. Not a livelock.

### The placeholder arm is correct and sufficient

`this.running` is keyed by `canonicalConversationId(sendAddress)` evaluated at send time.

- Continuation on an aliased conversation → keyed canonical → covered by
  `this.running.has(entry.conversationId)`. My repro confirms it: `runningKeys: ["chatc-…"]`.
- Create whose run predates aliasing → keyed by the placeholder while `adoptRecoveredReceipt`
  re-keys `entry.conversationId` to canonical → covered by the new
  `this.running.has(entry.placeholderId)` arm. `entry.placeholderId` is reliably populated on
  exactly that path (`reconcileProjection` sets it from `requestedId` when the request came in on
  a placeholder, which is the only way an entry becomes `identityPending`).
- Any run that has already announced its turn is caught by the third arm, `isTurnActive`,
  regardless of keying.

I looked for a run keyed under an address covered by none of the three and could not construct
one: an entry on a canonical id with `placeholderId === undefined` only arises from a cold load
with raw-index evidence, where this tab has no local create run to key under a placeholder. The
arm is a correct addition, not a hole.

### The rewritten test is a real behaviour test

`aevatar-transport.test.ts:7822`. It gates the transcript fetch behind a manually-resolved
promise, starts reconciliation, waits for the GET to be observed, then issues a **real**
`sendMessage` whose second stream hangs, and asserts at `:7926`:

```ts
expect(activeTurnAtSend).toBe("completed");
```

That is precisely the assertion round 2 found missing — it pins that the test is exercising the
window where `isTurnActive` is false, so the test cannot pass by accident on the old predicate.
It then asserts the follow-up survives, the deadline was refreshed, `attempt` stayed `0`, and
finally that an account reset settles the outcome as `timed_out` (which also exercises the
round-2 `resetScope` settle P3). Verified it fails against `73b7bb94`:

```
 × [branch-regression] keeps a new-chat mirror untouched while a follow-up turn is active
      Tests  1 failed | 3 passed | 179 skipped (183)
```

(The other three `[branch-regression]` tests pass at `73b7bb94` — correctly, since they are
regressions of the round-2 fixes, already verified against `58ab3594` in round 2.)

## Round-2 verified-fixed list — no regressions

- **P2-1** re-measured at round-3 head, unchanged: `duringFirstTurn: undefined`,
  `createRecoveryCallsDuringHealthyTurn: 0`, `awaitingAfterTerminal: true`.
- **P2-2 / P2-3 / all four P3s** — untouched by round 3 (source delta is confined to
  `runReconcileObservation`), and their tests are green in the 2446-test run.
- **Account scope, intent durability, `stateVersion: 0` provenance (`:6806-6849`), backoff
  floor** — all green in the full run; none of their code paths were modified.
- **#1304 assertions** — `use-assistant.aevatar.test.tsx` was not touched in round 2 or round 3
  (confirmed from both diffstats); still 6 tests, assertions unchanged since round 1.

## Residual notes (non-blocking, no action required to merge)

- `scheduleReconcileEntry` would overwrite `entry.timer` if it were ever called twice on a live
  entry, leaking the first handle. Unreachable today for the reasons above; a defensive
  `if (entry.timer !== undefined) return;` would make it structurally safe against future edits.
- The W4 timing probes (pause/resume, late wake, remote-delete-mid-loop, recovery-adoption) remain
  the least-exercised paths in the change, as recorded in round 2. Still a legitimate deferral —
  I re-derived them and found no defect — but the first place to look at a future bug report.
- Multi-tab behaviour remains unexercised across all three rounds.

## Sign-off

I am treating this as a merge sign-off, not a "looks fine". What convinced me: the one defect I
could reproduce is now closed on the path that actually reaches it, demonstrated by a
reproduction I wrote myself that fails against the immediately preceding commit and passes here;
the fix does not trade content loss for stalled convergence, which I checked separately through
the same real path; the two hazards I was asked to look for in the new control flow (double
schedule, unbounded deadline extension) are provably absent rather than merely untriggered; and
the accompanying test asserts the discriminating condition rather than restating the
implementation. Ship it.
