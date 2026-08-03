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
