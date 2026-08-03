# Plan: New-chat projection race — client-side projection lifecycle (rev 2)

Branch `fix-new-chat-timing`, base `origin/rollup-chat-2026-08-04` (== `main` @ `fde6041f`).
Frontend-only. No backend changes, no new dependencies, no lockfile changes.
Rev 2 incorporates the GPT-Sol adversarial review
(`docs/plans/new-chat-projection-race.review-gpt.md`); §7 maps every finding to
its resolution.

Architecture (decided, not re-litigated): the SSE stream + the transport's local
mirror are authoritative through a conversation's first turn; the transcript GET
is background reconciliation, never a prerequisite. Aevatar's CQRS read model
404s every brand-new `chatc-…` conversation until first-terminal + async
projection, and "pending", "never existed", and "deleted" are indistinguishable
at the wire — so the client must carry provenance, evidence, and a reconciler of
its own. Backend `get_history` (`backend/src/handlers/assistant.rs:828`) stays
an opaque proxy.

---

## 1. Verification of the seven claimed defects

All file:line references are against `fde6041f` (current worktree HEAD).

### D1 — placeholder id on the wire: PARTIALLY WRONG as described (reviewer-confirmed)

- The described leak — `projectTransportState()` (`frontend/src/hooks/use-assistant.ts:121`,
  called at `:630` pre-send and `:647` post-send) driving `getHistory` with a
  `workflow-pending-…` id onto the wire — **no longer exists at HEAD**.
  `getHistory` has TWO placeholder guards: no-local-record → throw
  (`frontend/src/lib/assistant/aevatar-transport.ts:1394-1400`) and
  locally-held → serve the local mirror with no round trip
  (`aevatar-transport.ts:1414-1425`). The second guard was added in
  `d418c74a` (PR #1321, "no doomed placeholder reads") and has a regression
  test at `aevatar-transport.test.ts:6945`.
- The reviewer independently re-audited every outbound family (history GET,
  workflow create/continuation `:2557-2575`, approval `:1626-1638`, stop
  `:4846-4874`, create-recovery `:2685-2705`, no browser `/state`) and
  confirmed: **`deleteConversation` (`:1302-1354`) is the only live placeholder
  leak.** Deleting a draft whose alias never arrived sends
  `DELETE /api/v1/assistant/conversations/workflow-pending-…`, which
  `conversation_resource_family` (`backend/src/services/assistant_service.rs:98-109`)
  rejects with 404 — the delete fails, the row stays, the user gets an error
  toast for a conversation that exists only in their tab.

**P1 therefore narrows to: fix the placeholder delete (with the pre-alias
deletion-intent design in W2 — see review P1.2) + an invariant test.**

### D2 — transcript GET forced on `turn.completed`: CONFIRMED (with nuance)

`use-assistant.ts:577` — `scheduleProjection(event.event === "turn.completed")`
projects immediately; `project()` (`:481-496`) calls `projectTransportState` →
`assistantTransport.getHistory`. At the terminal the turn is no longer active
(`getHistory`'s live-mirror branch `:1403` no longer applies) and, once the
context frame aliased the id, the id is canonical `chatc-…` — so `loadHistory`
(`:2346-2353`) fires a wire GET that is guaranteed to 404 on every new chat's
first turn. Nuance: the 404 lands in the `noServerTranscriptYet` fallback
(`:1458-1467`) and the local mirror is served, so the UI usually survives — the
defect is a *guaranteed doomed read at the single worst moment*, which pollutes
the wire log, burns the reconciliation opportunity, and (via the same fallback)
is the mechanism behind D3/D7 ambiguity. Agreed.

### D3 — stuck state on cold load during the projection window: CONFIRMED

`frontend/src/pages/assistant.tsx:189-199` — `explicitConversationIsConfirmedStale`
requires (id absent from index) AND (history 404). During the projection window
the index row exists but the transcript 404s, so the redirect never fires.
What the user gets depends on a race between the two queries:

- history query wins (transport has no record yet): `getHistory` throws
  `AssistantConversationNotFoundError` (`aevatar-transport.ts:1443-1445`);
  `useConversation` disables retry on 404/not-found
  (`use-assistant.ts:305-310`); nothing ever invalidates the query →
  **permanently stuck error banner** until manual reload.
- index query wins (`mergeIndexEntry` `aevatar-transport.ts:2304-2344` created a
  record with `EMPTY_TURN_STATE`): the 404 falls into `:1458-1467` and returns
  the empty mirror as **success** → silent empty chat that never fills in.

Both shapes confirmed. One wording correction from review (accepted): "the
index row exists" via the *public* `listConversations()` result is not server
evidence — the merged list includes the transport's own local records
(`:1259-1269`). Any membership check used as evidence must read the **raw
upstream response before `mergeIndexEntry`** (see W5).

### D4 — create-recovery not reload-safe: CONFIRMED (precedent claim corrected)

`conversationAliases` (`aevatar-transport.ts:1229`), `StoredConversation.createRequest`
(`:600-601`, holds `commandId`), and `stateVersion` (`:589`) are all in-memory
only. `sendMessage` mints/retains the `commandId` at `:1557-1567`;
`applyWorkflowChatContext` adopts the alias at `:3864-3872`;
`recoverWorkflowCreate` (`:2708-2773`) needs `run.clientRequestId`. Nothing is
persisted. A reload before context-frame adoption loses the only key that can
recover the server allocation, and a reload at any point loses the continuation
fence.

Correction accepted from review: `assistant-context-store` is **not** a
read-time self-heal precedent — it scopes only `recordScreen()` writes
(`assistant-context-store.ts:41-51`) and is cleared explicitly by the auth store
on logout (`auth-store.ts:14-17`), and its `clear()` assumes localStorage
exists. The receipt store defines its own policy in §2.4 (auth-transition
subscription owned by the receipt module, best-effort storage) rather than
citing parity.

### D5 — 3s unjittered reconciliation ladder: CONFIRMED

`HISTORY_RECONCILIATION_DELAYS_MS = [0, 300, 900, 1_800]`
(`aevatar-transport.ts:289`) — 4 attempts, 3.0s total, no jitter, shared by
`reconcileWorkflowHistory` (`:2633`), `pollCreateRecovery` (`:2691`), and the
continuation preflight (`:2810`). Real projection lag exceeds this envelope,
and identical schedules across tabs poll in lockstep. Scope note (per review):
this PR gives the **new** background reconciler and **new** cold create
recovery their own deadline-aware schedules (W4); the existing foreground
preflight/in-run-recovery ladder is deliberately NOT migrated here (see W9).

### D6 — misleading 15s grace comment vs unbounded keep-max: CONFIRMED (comment AND code both need work)

The constant's comment (`aevatar-transport.ts:216-219`) describes only the
"equal-length stale read" case — and indeed the grace bound
(`withinMaterializationGrace`, `:2393-2404`) applies only to the
equal-length + structured case. The longer-local keep-max at `:2390-2392`
(`comparableLocalMessageCount > messages.length → return existing`) has **no
time bound and no fence check**. The *code* is the defect for fence-current
reads (W8, with the exact predicate including turn presence per review P2.2);
the comment additionally needs to say what each branch is bounded by.

### D7 — index-backed empty mirror resurrects deletions: CONFIRMED, IN SCOPE

Mechanism verified: a stale index row (index TTL 5s,
`CONVERSATION_LIST_TTL_MS` `:214`) creates an `EMPTY_TURN_STATE` record via
`mergeIndexEntry`; the transcript 404 then returns success-empty via
`:1458-1467`. A genuine delete renders as an empty chat instead of not-found.
Review correction accepted: a one-time index confirmation alone does NOT close
this — evidence is historical, and the reconciler needs a remote-delete
transition. D7's closure is now W5 + W4's raw-membership rechecks and the
absent/timed-out deadline transitions (§2.5), not a single confirmation read.

---

## 2. Design

### 2.1 Conversation readability lifecycle

One lifecycle per conversation, owned by the transport
(`AevatarAssistantTransport`); the hook layer renders it and triggers the
reconciler. The pending states are **two independent stored facts** (review
P1.1) — a positive fence is a reconciliation *criterion*, never the state
discriminator, because a context frame with `stateVersion: 0` is a valid,
regression-locked create sequence (`aevatar-transport.ts:3874-3885`,
`aevatar-transport.test.ts:6806-6849`):

- **`identityPending`** — a workflow create command was dispatched (it may have
  been admitted upstream) and no canonical `chatc-…` id is known yet.
  Set: `sendMessage` create dispatch. Cleared: context-frame adoption
  (`applyWorkflowChatContext`), create-recovery adoption
  (`recoverWorkflowCreate`), or a **provable pre-admission rejection** (the
  create POST answered with a definitive non-retryable HTTP error — 400, 401,
  403, 422 — before any stream: the create demonstrably never happened).
- **`projectionPending`** — a workflow turn on this conversation reached a
  local terminal, and no wire transcript read satisfying the reconciliation
  criteria has been observed since. Set: `finishTurn` for workflow turns
  (also set at adoption time if the terminal preceded adoption). Cleared:
  materialization (below).

```
        createConversation()                    context frame adopts chatc-…
 (none) ───────────────► DRAFT ────────────────────────────────────────────┐
                          │ id workflow-pending-…, local only              │
                          │ sendMessage() → identityPending=true           │
                          ▼                                                ▼
                      STREAMING ──── terminal, id still pending ──► IDENTITY_PENDING
                          │            (context-free failure,        (same tab OR
                          │             cancel-after-dispatch,        cold reload w/
                          │             truncated stream)             command receipt)
                          │ terminal, id known                             │ recovery
                          ▼                                                │ adopts
                  PROJECTION_PENDING ◄─────────────────────────────────────┘
                          │  projectionPending=true; mirror authoritative;
                          │  transcript GET suppressed; reconciler running
                          │  criteria met: wire read at positive fence
                          │  ≥ max(1, storedFence) containing requiredTurnId
                          ▼
                     MATERIALIZED ── next turn terminal ──► PROJECTION_PENDING
                          │
   DELETE accepted / raw-index-absent transition / definitive rejection
                          ▼
                        ABSENT  (deletedConversationIds → NotFound)

   reconciler deadline with membership still present/unavailable
                          ▼
                       STALLED  (projectionStalled provenance; explicit retry)
```

`awaitingProjection` provenance served to callers =
`identityPending || projectionPending`.

Cold entry points (reload, second tab, direct URL) join at:

- `?c=chatc-…` + receipt or raw-index evidence → PROJECTION_PENDING;
- `?c=chatc-…` + no evidence after one raw-index confirmation → ABSENT;
- `?c=workflow-pending-…` + receipt with adopted id → alias repair → canonical path;
- `?c=workflow-pending-…` + command-only receipt → IDENTITY_PENDING (recovery mode);
- `?c=workflow-pending-…` + no receipt → ABSENT (today's behavior, unchanged).

### 2.2 Transition ownership

| Transition | Owner |
| --- | --- |
| none → DRAFT | `createConversation` (`aevatar-transport.ts:1278`) |
| DRAFT → STREAMING (+ identityPending) | `sendMessage` (`:1528`, create branch `:1557-1567`) |
| identityPending cleared by adoption | `applyWorkflowChatContext` (`:3805`) / `recoverWorkflowCreate` (`:2708`) |
| identityPending cleared by rejection | `streamWorkflowTurn` http_error branch (`:2874`) for definitive 4xx on a create |
| STREAMING → PROJECTION_PENDING | `finishTurn` (workflow terminals) |
| PROJECTION_PENDING → MATERIALIZED | `reconcileProjection` via `applyHistoryResponse` (wire-flagged) |
| → ABSENT | `deleteConversation` tombstone; reconciler raw-index-absent transition; `getHistory` no-evidence confirmation |
| → STALLED | reconciler deadline with membership present/unavailable |
| cold → PROJECTION/IDENTITY_PENDING | `getHistory` evidence check (receipts + raw index) |
| any → scope reset | transport owner-scope boundary (§2.7) |

### 2.3 Provenance representation

`ConversationHistory` (`frontend/src/types/assistant.ts:170`) gains two
optional fields (a separate status, never encoded as an `ApiError` — review P3):

```ts
export interface ConversationHistory {
  readonly conversation: Conversation;
  readonly messages: AssistantMessage[];
  readonly has_more: boolean;
  /**
   * The answer is the local mirror: identity or projection is still pending
   * server-side. Absent/false = authoritative server read.
   */
  readonly awaitingProjection?: boolean;
  /**
   * Reconciliation exhausted its deadline while the conversation still
   * appears to exist. Rendered as a quiet notice with an explicit retry.
   */
  readonly projectionStalled?: boolean;
}
```

Optional, so `MockAssistantTransport` and existing fixtures compile unchanged.

New `StoredConversation` fields (`aevatar-transport.ts:571`):

- `identityPending?: boolean`, `projectionPending?: boolean` (§2.1);
- `requiredTurnId?: string | null` — the chat-history turn id of the last local
  workflow terminal (from the context frame / recovery), the turn-presence
  criterion for materialization; null when unknown;
- `projectionStalledAt?: number` — set on reconciler timeout, cleared on
  explicit retry or any later materialization.

Materialization criteria (one private method used by the reconciler):
a wire read whose `historyStateVersion` is a positive integer
`≥ max(1, positiveStateVersion(stored.stateVersion) ?? 1)` AND — when
`requiredTurnId` is set — `historyIncludesAssistantTurn(entries, requiredTurnId)`.
This is the same double condition create recovery already uses
(`:2736-2741`). A legacy-array read (no `stateVersion`) that contains
`requiredTurnId` also materializes. A `stateVersion: 0`
context therefore parks in PROJECTION_PENDING until a *positive* fence read
containing the turn arrives — exactly the sequence the existing regression
locks in.

### 2.4 Persisted create receipts and deletion intents (P5 + review P1.2)

New module `frontend/src/stores/assistant-receipt-store.ts` with a Zod schema
in `frontend/src/schemas/assistant-receipts.ts` (Critical Rule 4). Persisted
shape:

```ts
{
  version: 1,
  ownerUserId: string | null,
  receipts: {
    [commandId: string]: {
      placeholderId: string,          // workflow-pending-…
      conversationId?: string,        // chatc-… once adopted
      stateVersion?: number,          // last observed positive fence (never decreased)
      createdAt: number,              // epoch ms
      updatedAt: number,
    }
  },
  deletionIntents: {
    [commandId: string]: {
      placeholderId: string,
      conversationId?: string,        // filled in once recovery names it
      createdAt: number,
    }
  }
}
```

- **Never persisted:** prompt text, tokens, message content.
- **Receipts:** TTL 24h, cap 20, oldest-`updatedAt` evicted first. Deleted on:
  conversation delete (canonical known), definitive pre-admission rejection
  (§2.1 — a 400/401/403/422 create must not leave 24h of false existence
  evidence), and on materialization once older than a 60s floor. Retained for:
  ambiguous delivery (network error/truncation), cancel-after-dispatch,
  context-free terminal, adopted identity.
- **Deletion intents:** separate keyspace, cap 10, expiry 24h. **Receipt
  eviction never touches intents** (separate cap/scan). An intent means "the
  user deleted this draft; the canonical resource may exist upstream and must
  be deleted when identified." Removed only after the canonical DELETE
  succeeds (or on expiry). See W2 for the flow.
- **Storage is best-effort** (review P2.3): all storage access
  (feature-detect, get, set, remove, rehydrate) is wrapped in try/catch with an
  in-memory fallback map; a receipt write can never throw out of
  `sendMessage()` after the request identity was chosen. Timestamps are
  validated on read: non-finite/negative entries dropped; timestamps more than
  5 minutes in the future are clamped to now (clock-rollback cannot evade TTL
  indefinitely).
- **Scope:** the receipt module owns a **one-way subscription** to
  `useAuthStore` (receipt-store imports auth-store; auth-store never imports
  receipt-store): on user-id transition (login, logout, account switch) the
  store clears (logout) or re-keys (switch) immediately — not on next access.
  Same-user token refresh (user id unchanged) preserves everything. Reads and
  writes additionally verify `ownerUserId` as defense in depth.
  **CI note (review P2.3):** `auth-store.ts` and the three existing assistant
  stores are in the wizard bundle manifest
  (`cli/src/wizard/bundle-meta/index.manifest:83-86`). The one-way import
  direction keeps the new receipt store OUT of the wizard's module closure —
  the closure grows only via imports *from* graph members. W10 verifies the
  manifest is unchanged; if it is not, stop and re-examine the import graph
  rather than rebuilding the bundle.
- **Cross-tab (honest statement, review P2.4):** persistence gives other tabs
  *evidence* (aliases, fences, intents) via a `storage`-event rehydrate; it
  does NOT coordinate reconciliation. Reconciliation is per-tab single-flight
  with independent jittered loops; a storage event carries no materialization
  outcome. When storage or storage events are unavailable, each tab reconciles
  independently — bounded per tab by the deadline (≤ ~12 requests over 90s).
  No lease, no BroadcastChannel.
- **Fence rules honored** (docs/chat/02 "State-version fences"): only positive
  safe integers stored; updates take `Math.max`; a persisted fence is a
  previously observed value for this user+conversation, so seeding
  `stored.stateVersion` from it on reload is exactly as strict as the tab that
  observed it. The fence field is **fence-only data**: W5 does not accept a
  bare fence as existence evidence (review P1.3) — evidence is the receipt's
  existence within TTL, and even that is only permission to reconcile, never
  proof of current existence.

Transport touchpoints (the transport already imports `useAuthStore`,
`aevatar-transport.ts:2713`, so importing plain functions from the receipt
module follows precedent): write on create dispatch (`:1557-1567`); update on
adoption (`:3864-3886`, `:2749-2760`); update `stateVersion` where the stored
fence advances; delete per the lifecycle above.

### 2.5 Single-flight background reconciler (P3 → W4)

```ts
reconcileProjection(conversationId: string): Promise<{
  readonly status: "materialized" | "absent" | "timed_out";
  readonly conversationId: string;   // canonical id the outcome applies to
}>
releaseProjectionWaiter(conversationId: string): void
```

Both methods **canonicalize their argument through `conversationAliases`
first** (review P3): placeholder and canonical addresses resolve to the same
entry, and the resolved canonical id is returned in the outcome so callers can
invalidate the right cache keys.

- **Single-flight per canonical id within the current owner scope.** The entry
  map lives on the transport and is cleared wholesale on scope reset (§2.7),
  which is what keys entries by `(scope, canonicalId)` without a composite key.
  Concurrent callers share one entry promise.
- **Entry structure & pause semantics (review P3):**
  `ReconcileEntry = { promise, settle, attempt, startedAt, deadlineAt, timer?,
  controller?, waiters }`. The shared `promise` is settled ONLY by a terminal
  outcome (`materialized` / `absent` / `timed_out`) — never rejected. Pausing
  (waiters hit 0): clear the timer, abort the in-flight fetch controller; the
  loop function catches its own abort and returns WITHOUT settling; the entry
  (attempt index, deadlineAt, promise) stays in the map. Resuming (a waiter
  registers again): spawn a new loop continuation from the stored attempt/
  deadline that settles the same promise. The hook attaches `.catch` anyway as
  a defensive guard against unhandled rejections.
- **Schedule (review P2.1):** deadline-driven, not array-driven. Policy
  objects, one per mode:
  - background projection: `{ floorMs: 250, baseMs: 500, capMs: 30_000, deadlineMs: 90_000 }`
  - ambiguous create recovery (cold, receipt-only): `{ floorMs: 250, baseMs: 500, capMs: 8_000, deadlineMs: 60_000 }`

  Delay for attempt *n*: `floor + random() * (min(cap, base * 2^n) - floor)` —
  full jitter with a **nonzero floor**, so `random() = 0` yields 250ms spacing
  (bounded request rate, no burst) and `random() → 1` yields capped
  exponential. The loop generates delays until `now() >= deadlineAt`; the
  deadline is checked before *scheduling*, but a timer that fires after the
  deadline (background-tab throttling, resume from sleep) still performs **one
  final observation** before the deadline transition. Randomness is injectable:
  `AevatarAssistantTransport` constructor gains a second optional parameter
  `random: () => number = Math.random` (mirrors injectable `now`, `:1235`).
- **Loop body (projection mode):** transcript GET; on a wire body, apply via
  `applyHistoryResponse` (wire-flagged) and test the materialization criteria
  (§2.3) → `materialized`. On 404, continue. **Every second 404 attempt (and
  always at the deadline) recheck raw index membership** (§ W5's
  `fetchRawIndexMembership`, which inspects the raw upstream rows BEFORE
  `mergeIndexEntry` — review P1.3). Two membership-absent observations spaced
  ≥ 10s apart with transcript still 404 → tombstone (`deletedConversationIds`)
  → `absent`. Before every attempt: check `deletedConversationIds` /
  `deletingConversations` (local delete → `absent`) and the owner scope
  (§2.7 — scope change aborts the request and settles the abandoned entry as
  `timed_out` before clearing it, so a mounted retry mutation cannot remain
  pending forever).
- **Loop body (recovery mode, identityPending):** poll
  `create-recovery/{commandId}` (existing `pollCreateRecovery` request logic,
  driven by the recovery policy schedule) with the same adoption guards
  `recoverWorkflowCreate` uses; on adoption, clear `identityPending`, set
  `projectionPending` + `requiredTurnId`, and continue in projection mode under
  the remaining deadline. An active turn never enters this wire loop: public
  history omits `awaitingProjection` while the turn is live, and the transport
  defensively reschedules any already-created entry without a network request
  or consuming its post-terminal deadline.
- **Deadline transition (review P1.3 — no `gave_up` limbo):** at the deadline,
  run one final raw-index membership check.
  - Membership **absent** → tombstone → `absent` (the hook invalidates; the
    refetch throws `AssistantConversationNotFoundError`; the existing
    confirmed-stale repair in `assistant.tsx` navigates away).
  - Membership **present or unavailable** → set `projectionStalledAt`, resolve
    `timed_out`. `getHistory` then serves the mirror with
    `awaitingProjection: false, projectionStalled: true`; the UI drops the
    syncing pill and renders the stalled notice with an explicit Retry (W7).
    Retry clears `projectionStalledAt` and calls `reconcileProjection` fresh.
- **Hook-side contract:** the `useConversation` effect invalidates the mounted
  key AND `assistantKeys.history(outcome.conversationId)` (when different)
  plus `assistantKeys.conversations` on **every** terminal outcome — including
  `timed_out`, so the cached `awaitingProjection: true` snapshot is always
  replaced by the stalled (or absent, or materialized) truth. Cleanup calls
  `releaseProjectionWaiter(conversationId)`.

Sketch (final signature; interface members are REQUIRED — see §2.6):

```ts
useEffect(() => {
  if (!conversationId || query.data?.awaitingProjection !== true) return;
  let released = false;
  assistantTransport
    .reconcileProjection(conversationId)
    .then((outcome) => {
      if (released) return;
      void queryClient.invalidateQueries({ queryKey: assistantKeys.history(conversationId) });
      if (outcome.conversationId !== conversationId) {
        void queryClient.invalidateQueries({ queryKey: assistantKeys.history(outcome.conversationId) });
      }
      void queryClient.invalidateQueries({ queryKey: assistantKeys.conversations });
    })
    .catch(() => undefined);
  return () => {
    released = true;
    assistantTransport.releaseProjectionWaiter(conversationId);
  };
}, [conversationId, query.data?.awaitingProjection, queryClient]);
```

### 2.6 Transport interface changes

`reconcileProjection` and `releaseProjectionWaiter` are added to
`AssistantTransport` (`frontend/src/types/assistant.ts:273`) as **required**
members (review P2.5 — optional members would force optional-call gymnastics
under strict TS and make hook tests brittle). `MockAssistantTransport`
implements them as no-ops: `reconcileProjection` resolves
`{ status: "materialized", conversationId }` immediately;
`releaseProjectionWaiter` is a no-op. Mock e2e behavior is unchanged (it never
serves `awaitingProjection`, so the effect never fires there).

### 2.7 Transport owner-scope boundary (review P1.4)

The production transport is a module singleton
(`frontend/src/lib/assistant/transport.ts:555-567`); logout clears TanStack
Query (`frontend/src/hooks/use-auth.ts:66-77`) but has never reset the
transport, so a logout→login without a reload leaks user A's mirror, list rows,
and transcripts to user B via the merged list (`:1259-1269`) and the
existing-record 404 fallback (`:1448-1467`).

- `AevatarAssistantTransport` gains `private ownerScopeId: string | null`
  (initialized lazily from `useAuthStore.getState().user?.id ?? null`) and a
  constructor-installed `useAuthStore.subscribe` listener. On any user-id
  transition (A→null, null→B, A→B) it runs `resetScope(next)`:
  abort every in-flight controller and timer (running turns, pending stops,
  reconcile entries, recovery loops), then clear `conversations`,
  `conversationAliases`, `deletedConversationIds`, `deletingConversations`,
  `pendingActionBatches`, `actionDrainBlocked`, the reconcile map, and reset
  `listFetchedAt = 0` and `activeConversationId = null`. **Same-user token
  refresh (id unchanged) does not reset.** The subscription (not
  check-on-entry alone) is required so a background reconciler timer cannot
  fire a request for A's conversation id under B's freshly-established session.
- Defense in depth: every public entry point calls `ensureScope()`
  (cheap id compare, resets if the subscription somehow missed a transition),
  and every await inside reconciliation/recovery/history application re-checks
  the scope before applying a response (extends the existing pattern at
  `:2713-2721`).
- The receipt store clears/re-keys on the same transition via its own
  subscription (§2.4). Deletion intents for the outgoing user stay persisted
  under that user's scope and resume when that user returns (they are keyed by
  `ownerUserId`; an intent must not fire DELETEs under another account's
  session).
- The wizard-manifest constraint holds: the subscription lives in
  `aevatar-transport.ts` / the receipt module, which import `auth-store` —
  never the reverse.

### 2.8 Decisions the prompt asked for explicitly (revised)

1. **Continuation fence reads:** split per the review's scope call.
   *(a) In this PR:* receipts persist the fence and reload-adopted
   conversations seed `stored.stateVersion` from them, so the common
   reload-then-send case skips the preflight entirely.
   *(b) Deferred:* migrating the foreground preflight
   (`streamWorkflowTurn` `:2809-2826`) and in-run create recovery off
   `HISTORY_RECONCILIATION_DELAYS_MS` is a separate foreground admission
   contract with its own latency/UX budget; it moves only with its own
   falsifiable case. The `workflowTurnBody` throw (`:2559-2563`) stays as the
   last-resort invariant (never post an unfenced continuation).
2. **`HISTORY_RECONCILIATION_DELAYS_MS`:** NOT deleted. It remains the
   documented schedule for the foreground preflight and in-run recovery. The
   new reconciler/cold-recovery policies (§2.5) are new code paths with their
   own separate policy objects — no shared helper config across the three
   modes (review scope call).
3. **D6:** code fix + comment rewrite, with the complete predicate (W8).
4. **D7:** in scope, closed by W5 + W4's raw-membership transitions (§2.5).

---

## 3. Ordered work items

Order matters: W1 (receipts/intents) and W0 (scope boundary) are the
substrate; W3 (provenance) precedes W4–W7.

### W0 — Transport owner-scope boundary (NEW, review P1.4)

- **Files:** `aevatar-transport.ts` (fields, `resetScope`, `ensureScope`,
  subscription, post-await guards), `frontend/src/lib/assistant/transport.ts`
  (no change to reset helper needed — the subscription is constructor-owned).
- **Change:** §2.7.
- **Failure mode closed:** logout→login without reload serves user A's local
  transcript/list rows to user B; a cross-account reconcile timer fires A's
  conversation id under B's session.

### W1 — Persisted create receipts + deletion intents (P5)

- **Files:** new `frontend/src/schemas/assistant-receipts.ts` (+ test), new
  `frontend/src/stores/assistant-receipt-store.ts` (+ test);
  `aevatar-transport.ts` (touchpoints in §2.4).
- **Change:** store + schema + best-effort storage + auth subscription per
  §2.4. Exports plain functions (`recordCreateReceipt`, `adoptReceiptIdentity`,
  `advanceReceiptFence`, `deleteReceipt`, `findReceiptByPlaceholder`,
  `findReceiptByConversation`, `recordDeletionIntent`, `resolveDeletionIntent`,
  `listDeletionIntents`) so the transport never calls hooks.
- **Failure mode closed:** D4 — reload/second tab destroys the recovery key
  and the continuation fence; plus (new) definitive create rejections leaving
  24h of false evidence.

### W2 — Placeholder delete without resurrection (P1 residual, review P1.2)

- **Files:** `aevatar-transport.ts` (`deleteConversation` `:1302`,
  `mergeIndexEntry` `:2304`, a startup/first-list intent sweep).
- **Change:** when the canonical id still has a pending prefix:
  1. Cancel the run (existing logic) — note the existing
     cancel-after-dispatch path already starts background recovery
     (`:4748-4755`); the deletion path must NOT let that recovery *adopt* into
     a tombstoned record (guard already exists at `:2717-2721`) — instead the
     intent below owns identification.
  2. Tombstone the placeholder, resolve the foreground delete (sidebar row
     gone immediately; user intent honored without blocking on recovery).
  3. Move the receipt to a **deletion intent** keyed by `commandId` (never
     erase the recovery key — review P1.2).
  4. Background cleanup task (also run for persisted intents at transport
     startup / first `listConversations` per scope): poll
     `create-recovery/{commandId}` under the recovery policy; when it names
     `chatc-A`, record it on the intent, issue the real
     `DELETE /conversations/chatc-A`, tombstone BOTH addresses, remove the
     intent. On DELETE failure or recovery exhaustion the intent persists for
     retry at the next sweep until its 24h expiry (best-effort, stated).
  5. `mergeIndexEntry` skips ids matching any intent's known `conversationId`
     (no resurrection flash after identification; before identification a
     brief flash is possible and bounded — stated limitation).
  - When a receipt already maps the placeholder to a canonical id, skip
    recovery: DELETE the canonical id directly, tombstone both, clean up.
- **Failure mode closed:** the doomed 404 DELETE; and the review's
  resurrection window — create accepted, delete-before-context, upstream
  finishes, next list re-materializes `chatc-A` with no tombstone and no
  DELETE ever sent.

### W3 — Two-fact provenance + no doomed read at the terminal (P2, review P1.1)

- **Files:** `frontend/src/types/assistant.ts` (`ConversationHistory`,
  required interface members §2.6), `aevatar-transport.ts`
  (`StoredConversation` fields, `finishTurn`, `applyWorkflowChatContext`,
  `streamWorkflowTurn` rejection branch, `applyHistoryResponse` wire-flag,
  `getHistory`), `frontend/src/lib/assistant/transport.ts`
  (`MockAssistantTransport` no-op impls).
- **Change:** fields and transitions per §2.1–2.3. In `getHistory`:
  - the wire-read branch (`:1426`) serves the mirror with
    `awaitingProjection: true` and NO network call while a local pending mirror
    has content or terminal provenance. A cold canonical record synthesized
    only from a receipt attempts one transcript read first, then falls back to
    the syncing mirror on 404;
  - the **locally-held placeholder branch (`:1414-1425`) stamps
    `awaitingProjection: true` when `identityPending`** — the same-tab
    context-free-terminal case the review flagged; the mounted hook then
    starts recovery-mode reconciliation without a reload;
  - the 404 fallback (`:1458-1467`) stamps provenance per the same facts;
  - a `projectionStalledAt` mark serves `projectionStalled: true` instead of
    `awaitingProjection`.
  - A definitive create rejection (400/401/403/422 pre-stream) clears
    `identityPending` and deletes the receipt (the turn already fails with the
    stream-start error today; no new UI needed).
  `useSendMessage`/pump code is untouched — the pump's terminal projection
  resolves against the mirror instantly and 404-free.
- **Failure modes closed:** D2 (the guaranteed 404 GET at `turn.completed` —
  including the `stateVersion: 0` create the fence-keyed predicate would have
  missed); the same-tab identity-pending gap; pending provenance being
  unrepresentable.

### W4 — Deadline-driven jittered single-flight reconciler (P3)

- **Files:** `aevatar-transport.ts` (`reconcileProjection`,
  `releaseProjectionWaiter`, `fetchRawIndexMembership`, constructor `random`
  param), new `frontend/src/lib/assistant/backoff.ts` (pure
  `nextBackoffDelay(policy, attempt, random)` helper + test),
  `frontend/src/hooks/use-assistant.ts` (`useConversation` effect §2.5).
- **Change:** §2.5 in full — alias canonicalization, scope-keyed single-flight,
  wakeable pause without rejecting the shared promise, floored full-jitter
  deadline loop with a guaranteed final observation on late wake, raw-index
  membership rechecks, the absent and timed_out deadline transitions, and
  invalidation on every terminal outcome.
- **Failure modes closed:** nothing converges the mirror to server truth after
  the terminal; D5's synchronized ladder for the post-terminal/cold paths;
  remote deletes invisible to the loop; `gave_up` limbo; multi-tab lockstep
  (probabilistically, via independent jitter — stated honestly in §2.4).

### W5 — Evidence-gated cold-load 404 handling (P4 + D3 + D7, review P1.3)

- **Files:** `aevatar-transport.ts` (`getHistory` branches `:1394-1400`,
  `:1442-1467`; new private `fetchRawIndexMembership(conversationId, signal)`).
- **Change:**
  - `fetchRawIndexMembership` performs the index GET and answers membership
    from the **raw upstream rows before `mergeIndexEntry`** (it may still merge
    afterwards); it can also return `"unavailable"` on failure. The public
    merged `listConversations()` is never used as evidence (review P1.3 — the
    merged list contains the very local record under test).
  1. `chatc-…` 404 with **no local record** (`:1443`): evidence check —
     (a) a current-user receipt whose `conversationId` matches, or (b) raw
     index membership (one forced call, TTL-exempt). Evidence → stored record
     synthesized, `projectionPending` set, return
     `awaitingProjection: true` empty mirror; the mounted hook starts the
     reconciler, whose raw-membership rechecks own the remote-delete
     transition. No evidence → `AssistantConversationNotFoundError` after that
     single confirmation — zero retries on dead ids.
  2. `chatc-…` 404 with an **index-born empty record** (`:1458-1467`,
     `messages.length === 0`, no local terminal facts): same raw membership
     confirmation. Present → `projectionPending` + syncing mirror (D3's
     silent-empty becomes visibly syncing and self-healing). Absent →
     tombstone + `AssistantConversationNotFoundError` (D7's resurrection
     closed at first read; the reconciler covers later remote deletes).
  3. `workflow-pending-…` with no record (`:1394-1400`): consult receipts by
     `placeholderId`. Adopted id → install alias, seed fence, take the
     canonical path. Command-only → synthesize an `identityPending` record and
     return a syncing mirror bound to recovery mode. No receipt → throw as
     today.
- **Failure modes closed:** D3 (both faces), D7 (with W4), the
  reload/second-tab/direct-URL cold paths, and evidence-as-permanent-existence
  (evidence only ever grants reconciliation permission; existence is decided
  by raw membership + transcript observations).

### W6 — TanStack cache key migration (P6)

- **Files:** `frontend/src/hooks/use-assistant.ts` (`projectTransportState`
  `:121-149`).
- **Change:** after the history read resolves, when
  `history.value.conversation.id !== conversationId` (canonical known), also
  `setQueryData` the history under `assistantKeys.history(canonicalId)`, and
  copy `episode`/`turn` slots to the canonical keys when empty. The existing
  pre-navigation copy in `assistant.tsx:243-254` stays; this covers the gap it
  cannot: a sidebar click on the canonical id **during** the stream (the
  sidebar lists the canonical id as soon as the context frame adopts it,
  `:1263-1269`) currently opens a cold query with no episode/turn state.
- **Failure mode closed:** navigating to the canonical key opens a separate
  cold query while the placeholder key holds the live episode.

### W7 — Syncing + stalled affordances, never a stuck error (P7)

- **Files:** `frontend/src/pages/assistant.tsx` (and
  `chat-thread.tsx` only if the affordance must live inside the thread —
  prefer the page-level strip; check DESIGN.md + live-app styling first).
- **Change:** two quiet, non-destructive `role="status"` states above the
  thread, visually distinct from the destructive error strip (`:507-515`):
  - `history.data?.awaitingProjection` → "Syncing conversation history…";
  - `history.data?.projectionStalled` → "History is taking longer than
    expected." with a Retry button that calls
    `assistantTransport.reconcileProjection(selectedId)` (through a small
    hook-layer mutation so the page stays transport-agnostic) and invalidates
    on settle.
  No change to `explicitConversationIsConfirmedStale` semantics: after W5 the
  projection-window combination arrives as success-with-provenance, so the
  stuck error is unreachable; the genuine-404 path still redirects (existing
  coverage at `assistant.test.tsx:508-551` remains the guard — no duplicate
  test). `describeHistoryError`'s 404 copy (`use-assistant.ts:271-281`) stays
  for residual transient reads.
- **Failure mode closed:** projection-window cold load rendered as a permanent
  error or silent empty chat; timeout leaving a forever-"syncing" UI.

### W8 — Bound the longer-local keep-max; fix the comment (D6, review P2.2)

- **Files:** `aevatar-transport.ts` (`applyHistoryResponse` `:2355-2445`,
  constant comment `:216-219`);
  `frontend/src/hooks/use-assistant.aevatar.test.tsx` (harness adaptation —
  see test plan).
- **Change:** capture the pre-merge fence before `:2373` maxes it. The exact
  replacement predicate — a shorter server transcript replaces a longer local
  mirror ONLY when ALL hold:
  1. `freshStateVersion !== undefined` (wrapped shape — legacy arrays always
     keep local: mixed deployment);
  2. `freshStateVersion >= preMergeFence`;
  3. NOT `withinMaterializationGrace`;
  4. the latest known local assistant turn
     (`latestAssistantTurnId(stored)`, falling back to `requiredTurnId` for
     streamed new-chat messages that carry no per-message turn id), is present
     in the server entries
     (`historyIncludesAssistantTurn`) — a fence-current read missing the just
     streamed turn must NOT wipe its text (review P2.2's completed predicate).
  An active turn is an unconditional keep: `applyHistoryResponse` returns its
  existing mirror before applying any fence or transcript observation.
  Otherwise keep `existing`. Structured-message preservation
  (`preserveLocalStructuredMessages`, `:537-568`) applies after that decision,
  unchanged — the review found no evidence this recreates PR #1304's card
  wipeout, and the #1304 assertions are preserved verbatim (test plan).
  Rewrite the `:216-219` comment to state all bounds: grace for equal-length
  structured keeps; fence-currency + turn-presence for longer-local
  replacement; legacy arrays exempt.
- **Failure mode closed:** a permanently-longer local mirror pins the view
  forever and blocks convergence after materialization; and the comment stops
  promising a bound the code doesn't have.

### W9 — Docs update (scope-reduced; NO ladder migration)

- **Files:** `docs/chat/02-wire-contract.md`, `docs/chat/07-testing-and-gaps.md`.
- **Change:** `HISTORY_RECONCILIATION_DELAYS_MS` and
  `RESERVATION_RETRY_DELAYS_MS` are **untouched** (review scope call — the
  foreground preflight is a separate admission contract; migrating it needs
  its own falsifiable case, filed as follow-up). Docs additions: the receipts/
  intents store, the two-fact provenance and reconciler policies, the raw-
  membership evidence rule, the delete-intent flow, and the new suites in 07's
  inventory. Bump "Last verified against".
- **Failure mode closed:** doc drift for the new behavior. (D5's fix for the
  *new* paths ships in W4's policies; the old ladder consciously remains on
  the foreground paths.)

### W10 — Gates

`npm run lint`, `npm run test` (vitest), `npm run build` (tsc -b with
`noUncheckedIndexedAccess` — the CI gate; `tsc --noEmit` is NOT sufficient)
from `frontend/`. Then verify `git diff --name-only` ∩
`cli/src/wizard/bundle-meta/index.manifest` = ∅ and that the manifest itself is
unmodified — the import-direction rule in §2.4 is what keeps it so; if the
manifest would change, stop and re-examine the import graph (do not rebuild the
wizard bundle for this PR). No `package.json`/lockfile changes anywhere.

---

## 4. Test plan

Conventions observed and matched: `use-assistant.test.tsx` uses
`vi.useFakeTimers()` + `resetAssistantTransport(() => TEST_NOW)`
(`use-assistant.test.tsx:46-54`) — note `resetAssistantTransport` resets only
the **mock** transport (`transport.ts:569-573`), so hook tests exercise the new
reconciler contract through a transport double/spy, while reconciler/backoff
behavior tests instantiate `new AevatarAssistantTransport(now, random)`
directly (the pattern `aevatar-transport.test.ts` already uses).
`assistant.test.tsx` mocks the hooks module with `vi.hoisted` state.

Honesty note (review P2.5): tests marked **[guard]** below pass on `main` and
are kept deliberately as labeled regression guards paired with a falsifiable
sibling; every unmarked original-plan test fails on `main`.

### New files

- `frontend/src/schemas/assistant-receipts.test.ts`
  - `parses a persisted blob and drops malformed entries`
  - `rejects nonpositive or unsafe stateVersion values`
  - `drops non-finite timestamps and clamps future timestamps to now`
    (clock-rollback TTL evasion, review P2.3)
- `frontend/src/stores/assistant-receipt-store.test.ts`
  - `records, adopts, and advances a receipt without decreasing the fence`
  - `deletes a receipt on definitive create rejection and keeps it on ambiguous failure`
  - `clears on logout and re-keys on account switch via the auth subscription; same-user token refresh preserves`
  - `evicts receipts beyond the cap and past the TTL without touching deletion intents`
  - `keeps functioning in-memory when localStorage getters or setItem throw`
    (quota/disabled storage, review P2.3 — no throw escapes the write API)
  - `rehydrates from a storage event from another tab`
- `frontend/src/lib/assistant/backoff.test.ts`
  - `delays are floored above zero even when random() returns 0` (review P2.1)
  - `delays are capped and the sequence spans the policy deadline when random() returns ~1`
  - `deadline check happens before scheduling, not before observing`

### Additions to `frontend/src/lib/assistant/aevatar-transport.test.ts`

W0 (scope):

- `an account switch clears every local record, alias, and tombstone and aborts live reconciliation`
  (user A record + running reconcile loop; swap auth user → fetch spy shows no
  further A-id requests; user B's `listConversations` contains no A rows;
  `getHistory(A-id)` under B throws rather than serving A's mirror)
- `a same-user token refresh preserves transport state`

W1/W3 (receipts + provenance):

- `sending a create turn records a receipt and context adoption completes it`
- `a definitive 4xx create rejection clears identityPending and deletes the receipt`
- `a context frame with stateVersion 0 still yields projectionPending and no terminal transcript GET`
  (the review's P1.1 killer case; pairs with the existing version-0 regression
  at `:6806-6849`, which must stay green)
- `a context-free failed create leaves identityPending provenance on the same-tab mirror`
  (stream dies pre-context; `getHistory(placeholder)` returns
  `awaitingProjection: true` via the locally-held branch)
- `a pre-context cancel keeps the receipt for background recovery`
- `serves the mirror without a transcript request while projection is pending`
  (fetch spy: no GET on post-terminal `getHistory`)
- `clears projectionPending only on a positive-fence wire read containing the required turn`

W2 (delete):

- `deletes an unaliased pending draft locally, persists a deletion intent, and sends no wire request in the foreground`
- `background cleanup recovers the canonical id, DELETEs it, tombstones both addresses, and the next list does not resurrect it`
  (the review's exact P1.2 window: accepted create → delete before context →
  recovery names `chatc-A` → wire DELETE observed → merged list omits it)
- `a persisted deletion intent completes after a simulated reload`
  (fresh transport instance + seeded intent → startup sweep DELETEs)
- `deletes the receipt's canonical conversation directly when the alias was already adopted`

W4 (reconciler):

- `reconcileProjection single-flights concurrent callers and canonicalizes placeholder addresses to the same entry`
  (placeholder + canonical args → one loop, same settled outcome; review P3)
- `retries on floored jittered delays until a fence read containing the turn lands`
  (deterministic `random`; fake timers; 404, 404, wrapped-at-fence →
  `materialized`)
- `random() = 0 does not burst: requests are spaced by the floor and the loop still spans the deadline`
- `a timer that fires after the deadline still performs one final observation`
  (advance clock past `deadlineMs` before the timer runs — tab-resume case)
- `rechecks raw index membership and returns absent when the id disappears remotely`
  (two spaced membership-absent observations mid-loop → tombstone →
  `absent`; the review's device-B-deletes case)
- `at the deadline with membership present it resolves timed_out and marks the record stalled`
  (subsequent `getHistory` serves `projectionStalled: true`,
  `awaitingProjection: false` — no limbo)
- `pausing on last-waiter release aborts the in-flight attempt without settling and resumes from the stored attempt`
- `aborts on local delete and on account change without settling into the new scope`
- `recovery mode adopts an identity via create-recovery under the recovery policy and continues into projection mode`

W5 (evidence):

- `a cold chatc- 404 with a matching receipt returns a syncing mirror instead of not-found`
- `raw index membership is answered from the upstream response, not the merged local list`
  (seed a local pending record; upstream index response WITHOUT the id →
  membership false even though `listConversations()` would list it — review
  P1.3's self-evidence hole)
- `a cold chatc- 404 with no evidence throws not-found after exactly one raw index confirmation`
  (fetch count: 1 transcript GET + 1 index GET, nothing else — no retry storm)
- `an index-born empty record is tombstoned when the raw confirmation no longer lists the id` (D7)
- `a reloaded placeholder with a receipt alias resolves to its canonical conversation`
- `a reloaded placeholder with only a commandId gets identityPending provenance and recovers`
- `a reload-adopted conversation seeds its continuation fence from the receipt`
  (continuation body carries `minimumStateVersion` with no preflight read;
  fetch spy)

W8 (keep-max):

- `a fence-current shorter server transcript containing the latest local turn replaces a longer local mirror after the grace window`
- **[branch-regression]** `a fence-current shorter read MISSING the latest local turn keeps the local mirror`
  (review P2.2's completed predicate)
- **[guard]** `a below-fence or legacy-array shorter read never replaces a longer local mirror`
  (passes on `main` where keep-max is unconditional; kept as the explicit
  regression pair for the two falsifiable cases above)

### Changes to `frontend/src/hooks/use-assistant.aevatar.test.tsx` (review P2.2/P2.5)

After W3, public `getHistory()` intentionally cannot materialize a pending
conversation, so the suite's `switchRead()` helper (`:358-367`) and the S2
immediate-server-id expectations (`:452-498`) must be adapted: the harness
drives materialization explicitly (call `reconcileProjection` on the real
transport with the scripted responses, or mount `useConversation` and flush the
effect) at each point the old flow relied on a direct read. **Every PR #1304
card-content/order assertion is preserved verbatim** — this is a harness
adaptation, not an assertion change; any assertion that would need weakening is
a signal to re-examine W3/W8, not the test.

### Additions to `frontend/src/hooks/use-assistant.test.tsx`

(Transport double implementing the required `reconcileProjection`/
`releaseProjectionWaiter` members.)

- `useConversation triggers reconciliation for a syncing transcript and invalidates on materialization`
- `a timed_out outcome invalidates and the refetched stalled state replaces the syncing snapshot`
  (closes the review's `gave_up`-limbo contradiction at the hook layer)
- `an absent outcome refetches into not-found`
- `releases the projection waiter on conversation switch and ignores late outcomes`
- `invalidates the canonical key when the outcome names a different id than the mounted key`
  (review P3 alias/invalidation note)
- `a rejected reconciliation promise is swallowed, not an unhandled rejection`
  (defensive `.catch` — assert via `process.on("unhandledRejection")` guard or
  vitest's unhandled-rejection failure mode)
- W6: `projects the transcript under both the placeholder and canonical keys`

### Additions to `frontend/src/pages/assistant.test.tsx`

- `renders a syncing notice, not an error, for an awaiting-projection transcript`
  (hoisted state gains `historyAwaitingProjection`; assert the status strip,
  absence of the destructive strip, no redirect)
- `renders the stalled notice with a retry action after reconciliation times out`
  (retry invokes the retry mutation; syncing pill absent)
- The confirmed-stale redirect for a genuine not-found is already covered at
  `assistant.test.tsx:508-551` and its inputs do not change — **no duplicate
  test is added** (review P2.5).

### Docs (no tests, but part of done)

`docs/chat/02-wire-contract.md` and `docs/chat/07-testing-and-gaps.md` per W9.

### Implementation coverage reconciliation (post-Opus review)

The lists above describe the design-time target, not an assertion that every
bullet became a new test. The implementation and follow-up contain the
load-bearing regressions for this change:

- all three W8 predicates against a real new-chat mirror whose streamed
  assistant messages have no turn ids, plus a live-turn transcript barrier;
- the W2 delete-before-context resurrection window, persisted-intent reload
  sweep, and receiptless placeholder-alias DELETE;
- cold receipt-first transcript loading, active-first-turn recovery
  suppression, and account-reset settlement of an outstanding retry;
- W4 single-flight, nonzero-floor timing, deadline-to-stalled behavior, raw
  membership evidence, and cold absence; W5 and W7 rendered outcomes; and the
  receipt-store timer deduplication.

Several design bullets are covered by pre-existing tests rather than counted
as new coverage: `stateVersion: 0`, context-free terminal recovery, and local
pending-mirror/no-request behavior. PR #1304's card/order suite remains the
structured-message guard and its assertions are unchanged.

Follow-up tests marked `[branch-regression]` pass on `main` because `main` has
the older unconditional keep-max/direct-read/direct-alias behavior, but fail
against the pre-review implementation (`58ab3594`). They are required guards
for regressions introduced by this branch's new relaxation and receipt paths,
not claimed as new `main` coverage.

Deferred from this PR's test diff:

- a request-count-specific W0 abort assertion. Account isolation, controller
  aborts, and post-await scope guards are covered by the existing account
  switch test plus the new outstanding-promise settlement case; the Opus
  reproduction independently inspected the abort path. A deterministic fetch
  abort spy would duplicate those mechanics rather than close a remaining
  behavior defect;
- dedicated W4 pause/resume, late-wake, remote-delete-mid-loop, and
  recovery-adoption timing cases. The single-flight/deadline/evidence tests
  exercise their shared state machine and Opus independently verified these
  mechanics. These remain useful hardening work, but are not represented as
  shipped coverage here;
- the W6 `projectTransportState` placeholder/canonical dual-slot copy case.
  The hook suite covers canonical invalidation, while a pump-level cache-copy
  test needs a separate transport-event harness. It is deferred and is not
  claimed as current coverage.

---

## 5. Risks and rejected alternatives

- **Backend 200-empty synthesis — rejected (restated).** The NyxID proxy has no
  row of its own for `chatc-` ids; synthesizing 200-empty for upstream 404s
  would render dead links and deleted conversations as legitimate empty chats
  for *every* client, and would violate the deliberate opacity of
  `get_history` (`backend/src/handlers/assistant.rs:819-827`). The client is
  the only place holding evidence (receipts, mirrors, fences) to disambiguate.
- **This PR does NOT fix the upstream contract.** Aevatar still cannot
  distinguish pending/absent/deleted at the wire; that fix is filed
  separately. Everything here is client-side compensation that remains correct
  after it lands (§6).
- **Risk: keep-max relaxation (W8) regresses card preservation.** Mitigated:
  replacement still routes through `preserveLocalStructuredMessages`; the
  reviewer independently found no wipeout path; the #1304 assertions are
  preserved verbatim under an adapted harness (any needed weakening is a stop
  signal).
- **Risk: receipts/intents leak across accounts or persist sensitive data.**
  Mitigated: no prompt/content persisted; auth-subscription clearing at the
  transition (not next-access); `ownerUserId` checks as defense in depth;
  TTL + separate caps; intents never fire requests under another account's
  session.
- **Risk: pre-identification resurrection flash (W2).** Between the delete and
  recovery naming `chatc-A`, a list refresh can briefly show the row. Bounded
  by the recovery policy deadline; the intent guarantees eventual DELETE +
  tombstone. Accepted and stated.
- **Risk: raw-membership checks add index GETs.** Bounded: one per cold-404
  evidence decision; at most every-second-attempt inside a single-flight loop
  that exists only while a conversation is pending. No steady-state cost.
- **Risk: reconciler + TanStack interplay causes refetch loops.** Mitigated:
  invalidation only on terminal outcomes; `timed_out` flips the served
  provenance to `projectionStalled`, so the effect's
  `awaitingProjection === true` guard cannot re-arm off the refetched stalled
  snapshot; single-flight + waiter refcount prevent stacking.
- **Risk: scope-reset aborts a live turn on account switch.** Intended: no
  request may continue across an identity boundary; the turn fails locally in
  the old scope and nothing is applied to the new one.
- **Rejected: navigating the URL to the canonical id from the transport or
  pump.** Navigation stays owned by `assistant.tsx`'s repair effect
  (`:209-271`); lower layers only make caches coherent (W6).
- **Rejected: a persisted full local transcript.** Storage/privacy surface far
  larger than the race requires.
- **Rejected: storage-backed cross-tab reconciliation lease.** Honest per-tab
  single-flight + independent jitter is bounded and simpler; a lease adds
  ownership/expiry failure modes for marginal savings (§2.4 states the real
  guarantee and the storage-unavailable fallback).
- **Rejected (deferred): migrating the foreground preflight off the 3s
  ladder.** Separate admission contract; needs its own falsifiable case
  (review scope call). Receipts' fence seeding already removes its most common
  trigger.

## 6. Rollout note — safe under today's 404, correct under tomorrow's 200-empty

Per work item, behavior under (A) current Aevatar (404 until projection) and
(B) future Aevatar (200 `{messages: [], stateVersion, projectionStatus}`):

- **W0 scope boundary / W1 receipts+intents:** status-code independent.
  Under (B) receipts still serve fence seeding, create recovery after reload,
  and deletion intents; nothing keys off 404s.
- **W2 delete intents:** identical under both — recovery is keyed by
  `commandId`, DELETE by canonical id.
- **W3 provenance:** facts are set by local events (dispatch, terminal,
  adoption), not by status codes. Under (B) a 200-empty below-criteria read is
  simply a confirmed-but-behind read: `projectionPending` stays true, the
  mirror still serves. If `projectionStatus` ships, reading it is a one-line
  additive signal (extra JSON fields are already ignored by
  `readHistoryEntries`).
- **W4 reconciler:** loops over 404s under (A), over 200-below-criteria bodies
  under (B) — the materialization criteria (positive fence ≥ required,
  turn present) are status-code independent, so it terminates identically and
  stops producing 404s at all. The absent transition still works under (B):
  a genuine 404 there *means* absent, which the raw-membership check simply
  confirms sooner.
- **W5 evidence gating:** load-bearing under (A). Under (B) the
  404-with-evidence branches stop being reached (pending returns 200);
  genuine 404 = genuinely absent, so not-found semantics become *more* correct
  automatically. Retire the branch when the compatibility window closes (same
  policy as the legacy-array form, `aevatar-transport.ts:499-500`).
- **W6 cache migration / W7 affordances:** UI-layer; independent of status
  codes. The syncing strip shows strictly less often under (B).
- **W8 keep-max bound:** more exercised under (B) (200s arrive earlier) —
  which is exactly why the fence-current + turn-present gate, not a bare
  timer, decides replacement.
- **W9:** doc-only.

Mixed deployment is handled per-response, not per-build: every decision point
keys off the response *shape* (status, `stateVersion` presence, turn
presence), matching the repo's deploy-independence posture for the legacy
transcript array.

---

## 7. Review response (GPT-Sol REWORK → this revision)

| Finding | Resolution in this revision |
| --- | --- |
| **P1.1** fence-only state machine | Accepted in full. Two independent stored facts `identityPending`/`projectionPending` (§2.1–2.3); fence + `requiredTurnId` demoted to materialization *criteria*; `stateVersion: 0` context handled explicitly (parks in PROJECTION_PENDING until a positive-fence read with the turn); the locally-held placeholder branch (`:1414-1425`) now stamps identity-pending provenance so the same-tab context-free case starts recovery (W3); receipt terminal policy added — definitive 400/401/403/422 pre-admission rejection deletes the receipt (§2.4, W1/W3 tests). All four demanded tests named in §4. |
| **P1.2** pre-alias delete resurrection | Accepted in full. W2 rewritten: the receipt is *moved* to a persisted deletion intent keyed by `commandId` (separate keyspace, cap 10, 24h expiry, immune to receipt eviction); bounded recovery under the intent → canonical DELETE → tombstone both → remove intent; startup/first-list sweep completes intents across reloads/tabs; `mergeIndexEntry` skips identified intent ids. The exact resurrection-window test is named. Brief pre-identification flash acknowledged as a bounded, stated limitation (§5). |
| **P1.3** evidence permanence / no remote-delete transition / `gave_up` limbo | Accepted in full. New `fetchRawIndexMembership` answers from raw upstream rows before `mergeIndexEntry` (the merged public list is never evidence — §W5, D3 correction adopted in §1); evidence is defined as permission to reconcile only (§2.4); the reconciler rechecks raw membership every second 404 attempt and at the deadline, with a two-observation ≥10s absent transition → tombstone → `absent` (§2.5); `gave_up` is replaced by a real deadline transition: final raw-membership check → `absent`, else `timed_out` + `projectionStalledAt` + `projectionStalled` provenance + explicit Retry (W7); the hook invalidates on **every** terminal outcome. All three demanded tests named. |
| **P1.4** no account scope boundary | Accepted in full. New W0: transport owner scope with a constructor-owned auth-store subscription (prompt abort of timers/controllers, full state clear on user-id transition, same-user refresh preserved), `ensureScope()` on public entry points, post-await scope re-checks, single-flight map cleared per scope (§2.7); receipt store clears on the same transition via its own subscription; account-switch and same-user-refresh tests named. |
| **P2.1** jitter collapse / wrong window | Accepted. Deadline-driven loop with a 250ms floor and per-attempt exponential cap replaces the finite array; `random()=0` yields floor-spaced requests over the full window; deadline checked before scheduling with one guaranteed final observation after a late wake (§2.5); `random=0`, `random≈1`, and late-wake tests named. |
| **P2.2** incomplete W8 predicate / false "untouched suite" claim | Accepted. The four-clause predicate now includes latest-local-turn presence (W8); the shorter-current-but-turn-missing keep-local test is named; the `use-assistant.aevatar.test.tsx` claim is retracted — §4 specifies the harness adaptation (drive `reconcileProjection` / mounted hook) with every #1304 assertion preserved verbatim. |
| **P2.3** storage semantics / wrong precedent / wizard-graph hazard | Accepted. Best-effort storage wrapper with in-memory fallback (no throw out of `sendMessage`), timestamp validation + future-skew clamping, clearing on the auth transition via a one-way receipt-module→auth-store subscription; the `assistant-context-store` parity citation is removed from §1 D4 and §2.4; the import-direction rule that keeps the receipt store out of the wizard closure is stated explicitly and gated in W10. |
| **P2.4** cross-tab overstated | Accepted. §2.4 now states the real guarantee: per-tab single-flight + independent jitter; storage events share evidence only, carry no materialization outcome; storage-unavailable fallback is independent bounded reconciliation. The "skip straight to a single confirming read" claim is removed; the lease alternative is explicitly rejected in §5. |
| **P2.5** padding / harness contract / optional-member TS | Accepted. The two passing-on-main tests: the keep-max guard is retained but labeled **[guard]** and paired with its falsifiable siblings; the confirmed-stale-redirect duplicate is dropped (existing coverage at `assistant.test.tsx:508-551` cited instead). The "every test fails on main" claim is replaced with an explicit guard-labeling rule. `resetAssistantTransport` limitation acknowledged: hook tests use a transport double; backoff/reconciler tests instantiate `AevatarAssistantTransport(now, random)` directly. Interface members are **required** with mock no-op implementations (§2.6) — no optional-call strict-TS hazard. |
| **P3** alias resolution / wakeable pause / status-not-ApiError | Accepted. `reconcileProjection`/`releaseProjectionWaiter` canonicalize through `conversationAliases` and return the canonical id for invalidation (§2.5, alias test named); pause is specified as abort-without-settling with attempt/deadline carried into a resumed loop on the same never-rejecting entry promise, plus a defensive hook `.catch` (§2.5, tests named); syncing/stalled are optional success-data fields, never `ApiError`s (§2.3, W7). |
| **Scope call** (split the ladder migration; separate policies) | Adopted. `HISTORY_RECONCILIATION_DELAYS_MS` stays for the foreground preflight/in-run recovery (§2.8, W9); the new reconciler and cold create recovery get separate named policy objects (§2.5); the preflight migration is explicitly deferred behind its own falsifiable case (§5). §2.8's fence-seeding half of the continuation decision remains in this PR. |
| **D-section corrections** | Adopted: D3's merged-list-is-not-evidence wording, D4's precedent retraction, D7 reclassified as closed only by W5+W4 jointly. D1's narrowed conclusion stands (reviewer-confirmed). |

No findings rejected.
