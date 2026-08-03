VERDICT: REWORK

# P1 findings

## P1.1 - The fence-only state machine cannot represent two live first-turn states

**Plan section:** §2.1-2.3, W1, W3, W5.

**Evidence:** The proposed `awaitingProjection()` returns false unless `positiveStateVersion(stored.stateVersion)` exists (`docs/plans/new-chat-projection-race.md:219-224`). That is not true for every valid workflow create. The transport deliberately accepts a context-frame `stateVersion` of zero at `frontend/src/lib/assistant/aevatar-transport.ts:3874-3885`, and the existing regression at `frontend/src/lib/assistant/aevatar-transport.test.ts:6806-6849` proves the supported sequence: first context has version 0, then the next continuation obtains a positive fence through history. A context-free terminal is the other hole: an existing `workflow-pending-*` record is returned by the unconditional local-placeholder branch at `frontend/src/lib/assistant/aevatar-transport.ts:1414-1424`; W5 changes only the `!existing` placeholder branch, so a command-only receipt in the same tab never produces `awaitingProjection` and never starts W4.

**Concrete failure:**

1. First turn receives `aevatar.chat.context { conversationId: "chatc-*", stateVersion: "0" }`, then completes.
2. W3 evaluates the proposed predicate as false.
3. The terminal projection still executes the transcript GET, receives the exact 404 this work is meant to remove, and no background reconciler is started.

A second failure is `POST accepted -> stream closes before context -> create recovery misses its short budget -> failed terminal`. The local placeholder and receipt still exist, but `getHistory()` returns an ordinary mirror with no pending provenance. Reloading happens to take a different W5 branch and starts recovery; staying in the original tab does not.

There is also no receipt terminal policy. W1 records before dispatch, but a known 400/401/403 rejection is not an ambiguous create and must not remain 24-hour existence evidence. As written, a reload turns a definitively never-created conversation into a syncing one.

**Specific correction:** Model at least two independent facts rather than deriving all pending state from a positive fence:

- `identityPending`: a create command may have been admitted but no canonical id is known yet.
- `projectionPending`: a canonical first turn reached a local terminal but its transcript has not been observed.
- Keep `requiredTurnId` and optional positive `fence` as reconciliation criteria, not as the sole state discriminator.
- A valid context with version 0 must set canonical identity plus `projectionPending`; reconciliation must wait for a positive history fence containing the required turn.
- The existing-placeholder branch must surface identity-pending provenance when an ambiguous command receipt exists.
- Delete a receipt on a provable pre-admission rejection; retain it only for ambiguous delivery, cancellation after dispatch, context-free terminal, or an adopted canonical identity.

Add falsifiable tests for context version 0, same-tab context-free failure, pre-context cancel, and a definitive HTTP rejection that leaves no evidence.

## P1.2 - W2 loses deletion intent before the canonical id exists and can resurrect the chat

**Plan section:** §2.2, §2.4, W2.

**Evidence:** W2 says to cancel, tombstone the placeholder, and delete its receipt immediately when no alias is known (`docs/plans/new-chat-projection-race.md:396-405`). A workflow cancel after dispatch starts create recovery at `frontend/src/lib/assistant/aevatar-transport.ts:4748-4755`, but recovery refuses adoption once the placeholder is tombstoned at `frontend/src/lib/assistant/aevatar-transport.ts:2717-2721`. The list later merges every previously unknown canonical id returned upstream (`frontend/src/lib/assistant/aevatar-transport.ts:1251-1257`) and rejects resurrection only when that same canonical id is in `deletedConversationIds` (`frontend/src/lib/assistant/aevatar-transport.ts:2304-2306`). The client cannot tombstone an id it discarded the only recovery key for.

**Concrete failure:** The create POST is accepted and allocates `chatc-A`; before the context frame reaches the browser, the user deletes the draft. W2 cancels locally, tombstones only `workflow-pending-P`, and erases the `commandId` receipt. Aevatar finishes and lists `chatc-A`. The next list refresh has no tombstone for `chatc-A`, so the supposedly deleted chat reappears in the sidebar. No DELETE was ever sent for the real resource.

**Specific correction:** A pre-alias delete must persist a deletion intent keyed by `commandId`, not erase the receipt. Run bounded create recovery under that intent; when it returns the canonical id, issue the real DELETE, tombstone both addresses, then remove the receipt. If the foreground delete cannot wait for recovery, retain a background cleanup intent across reload/tabs until recovery plus DELETE succeeds or the receipt reaches an explicit cleanup expiry. Scope and cap deletion intents separately so normal receipt eviction cannot discard them. Add the exact `accepted create -> delete before context -> later recovery -> canonical DELETE -> no list resurrection` test; the plan's two W2 tests do not cover this window.

## P1.3 - Evidence is treated as permanent existence, while the reconciler has no remote-delete transition

**Plan section:** §2.4-2.5, W4, W5, W7.

**Evidence:** The plan permits `receipt OR index membership` to turn a 404 into a successful pending mirror (`docs/plans/new-chat-projection-race.md:445-459`). A receipt proves that a create was once attempted or adopted; it does not prove that the conversation still exists. The W4 loop checks only local `deletedConversationIds` / `deletingConversations` (`docs/plans/new-chat-projection-race.md:313-323`); it specifies no authoritative index recheck that can discover deletion in another tab/device. The claimed correction at `docs/plans/new-chat-projection-race.md:676-680` therefore has no implementing transition.

The proposed hook also contradicts the prose. On `"gave_up"` it returns without invalidating or changing cached data (`docs/plans/new-chat-projection-race.md:335-345`), so the cached `awaitingProjection: true` remains visible indefinitely in that mount. It does not "drop the syncing affordance" as claimed at `docs/plans/new-chat-projection-race.md:324-331`.

Finally, a forced call cannot test membership through the public list result. `listConversations()` merges the response into the transport and then returns every local record at `frontend/src/lib/assistant/aevatar-transport.ts:1259-1269`. If W5 calls a force-refresh variant and searches that merged result, the local pending/empty record supplies its own evidence even when the authoritative response omitted the id.

**Concrete failure:** Device B deletes `chatc-A`. Device A has a still-valid receipt and a stale index row, opens `?c=chatc-A`, and receives transcript 404. W5 serves an empty pending mirror. The index later drops the row, but W4 never rechecks it; after 90 seconds W4 returns `gave_up`, the hook leaves `awaitingProjection` untouched, and the deleted chat continues to look alive and syncing. A remount repeats the receipt-backed path until TTL, and clock skew can extend it further.

**Specific correction:**

- Make the forced index helper return membership from the raw response before `mergeIndexEntry`, never from the public merged list.
- Treat evidence as permission to reconcile, not as timeless proof of current existence.
- Recheck raw index membership during the loop and at the deadline. When transcript 404 persists and authoritative membership disappears, tombstone and return `absent` even if a historical receipt exists.
- Retire projection/create evidence immediately on materialization. If a persisted continuation fence must remain for legacy-array deployments, store it as fence-only data that W5 does not accept as existence evidence.
- Define a real deadline transition. Either expire the evidence and invalidate into not-found after a final raw-index confirmation, or return a separate non-error `projectionTimedOut` state with an explicit retry. Do not leave `awaitingProjection: true` in cache after `gave_up`.
- Test `initially stale index -> remote deletion becomes absent`, `gave_up changes rendered state`, and `local merged record cannot satisfy raw membership`.

## P1.4 - The singleton transport and single-flight key are not account-scoped

**Plan section:** §2.2-2.5, W1, W4.

**Evidence:** The production transport is a module singleton at `frontend/src/lib/assistant/transport.ts:555-567`. Its conversations, aliases, tombstones, list TTL, and proposed reconcile map live for the singleton's lifetime; `listConversations()` returns all local records at `frontend/src/lib/assistant/aevatar-transport.ts:1259-1269`. Logout clears TanStack Query at `frontend/src/hooks/use-auth.ts:66-77`, but it does not reset the Aevatar transport. The plan captures a `scopeId` only inside a reconciliation task. That prevents one eventual adoption in some paths; it neither clears the mirror nor namespaces `Map<string, ReconcileEntry>`.

**Concrete failure:** User A completes a local first turn, logs out without reloading the SPA, and User B logs in. The query cache is empty, but the transport still contains A's canonical record and prompt. B's list call fetches B's index, then also returns A's local record. Selecting it causes B's wire GET to 404; the existing-record fallback at `frontend/src/lib/assistant/aevatar-transport.ts:1448-1467` can return A's local transcript. A proposed reconciliation promise keyed only by canonical id can likewise be found or completed in the wrong account after a switch.

**Specific correction:** Give the transport an owner scope and enforce a hard scope boundary on every public entry point: on user-id change, abort active reconciliation/recovery controllers and timers, clear conversations/aliases/tombstones/deletion reservations/list TTL, and start a new scope. Key every single-flight entry by `(scopeId, canonicalId)` and recheck scope after every awaited fetch before applying a response. The receipt store must clear or switch scope at the same boundary. Add an account-switch test that asserts no A mirror/list row/result/timer is observable by B. A token refresh with the same user id should preserve the scope and continue normally.

# P2 findings

## P2.1 - The full-jitter schedule does not actually provide the stated 90-second or 15-second window

**Plan section:** §2.5, §2.6, W4, W9.

**Evidence:** The proposed finite bases are multiplied by `random()` (`docs/plans/new-chat-projection-race.md:294-300`). With the explicitly supported injected `random = () => 0`, all eight delays are zero: every request fires back-to-back at t=0 and the loop gives up immediately. That directly contradicts "No attempt at t=0" and the 90-second deadline. Even under uniform randomness the expected sum of the finite schedule is about 45 seconds, not 90. The same defect can collapse the claimed 15-second send-path recovery to an immediate burst.

**Concrete failure:** A deterministic test or a legitimate very-low random sample performs eight 404 GETs immediately after terminal and gives up before projection has any chance to land. A background-throttled tab can suffer the inverse: it wakes after the wall-clock deadline and gives up without one final observation.

**Specific correction:** Use a nonzero floor and continue generating capped delays until the deadline rather than exhausting one finite array. Check the deadline before scheduling but always allow one final observation on wake/resume. Tests must cover `random=0`, `random=1-epsilon`, background clock advancement, request spacing, and maximum deadline.

## P2.2 - The W8 protection is directionally sound, but its stated predicate is incomplete

**Plan section:** W8, §5 risk note.

**Evidence:** The current unconditional longer-local return is at `frontend/src/lib/assistant/aevatar-transport.ts:2387-2392`. Structured activity/card messages are separate synthetic messages (`frontend/src/lib/assistant/aevatar-transport.ts:3996-4029`), and `preserveLocalStructuredMessages()` reinserts them after their turn anchor at `frontend/src/lib/assistant/aevatar-transport.ts:531-568`. Therefore I do not find evidence that the intended current-fence-and-turn-present replacement itself recreates PR #1304's card wipeout.

However, W8 first defines the keep condition as only `legacy OR below-fence OR grace` (`docs/plans/new-chat-projection-race.md:509-516`), then mentions turn presence in prose. The actual boolean must also keep local when the latest local assistant turn is absent. Otherwise a response with `stateVersion >= preMergeFence`, outside grace, but containing only a prior/user row can replace a longer local transcript. `preserveLocalStructuredMessages` saves the card, but the latest streamed text/user message is lost.

**Specific correction:** State and test the exact predicate: a shorter server transcript may win only when it has a usable current fence, is outside any required grace, **and** contains the required latest local turn; otherwise retain local. Preserve structured messages after that decision. Include the exact shorter-current-but-turn-missing case.

The claim that `use-assistant.aevatar.test.tsx` stays green "untouched" is false. Its `switchRead()` calls public `getHistory()` directly at `frontend/src/hooks/use-assistant.aevatar.test.tsx:358-367`; after W3 that call intentionally serves the awaiting mirror and cannot materialize it. S2 expects immediate server ids at `frontend/src/hooks/use-assistant.aevatar.test.tsx:452-498`. Adapt the harness to run the new reconciler (or mount `useConversation`) while preserving every card/order assertion. That is a necessary behavioral update, not permission to weaken the PR #1304 assertions.

## P2.3 - Persistence failure handling and logout semantics are underspecified

**Plan section:** §2.4, W1.

**Evidence:** The cited `assistant-context-store` does not perform a read-time owner self-heal. It scopes `recordScreen()` writes at `frontend/src/stores/assistant-context-store.ts:41-51`, while logout explicitly clears it through `frontend/src/stores/auth-store.ts:14-17`. Its `clear()` directly calls `localStorage.removeItem` at `frontend/src/stores/assistant-context-store.ts:53-57`, so it is not a robustness model for storage-disabled environments.

**Concrete failure:** If localStorage is disabled or quota-exhausted, recording a receipt must not throw out of `sendMessage()` after the request identity was chosen. It should continue with an in-memory receipt and report no false send failure. A receipt with a far-future `updatedAt` can evade `now - updatedAt > TTL` for an arbitrarily long time after clock rollback. Logout currently leaves the new persisted blob on disk until some later accessor happens to self-heal it.

**Specific correction:** Wrap storage acquisition/read/write/remove/rehydrate as best-effort, retain an in-memory fallback, validate finite safe timestamps, and expire or clamp timestamps beyond a small future-skew allowance. Clear on the auth scope transition, not merely the next receipt access. Test storage getter failure, `setItem` quota failure, malformed/future timestamps, logout, and same-user token refresh.

Be aware of CI scope: `frontend/src/stores/auth-store.ts` and all three existing assistant stores are already in `cli/src/wizard/bundle-meta/index.manifest:83-86`. Importing the new receipt store from `auth-store.ts` changes the wizard module closure and requires `npm --prefix frontend run build:wizard` plus committed bundle metadata/assets. A one-way auth subscription owned by the receipt module can avoid adding it to the wizard graph, but it still needs an explicit test.

## P2.4 - Cross-tab coordination is overstated

**Plan section:** §2.4-2.5, §5.

**Evidence:** The receipt shape has no materialized/lease field, and the normal materialization path does not write a state another tab can interpret. A `storage` rehydrate event can share alias/fence evidence, but it cannot make a neighboring tab "skip straight to a single confirming read" as claimed at `docs/plans/new-chat-projection-race.md:320-323`. Each tab still owns an independent map and retry loop; jitter reduces synchronization probabilistically but is not cross-tab single-flight.

**Specific correction:** Either describe the guarantee honestly as per-tab single-flight plus independent jitter, with storage-event-unavailable fallback to independent reconciliation, or add a storage-backed lease/materialized marker with expiry and ownership rules. Do not require `BroadcastChannel`; the plan correctly avoids assuming it, but it must state what happens when neither storage events nor persistent storage work.

## P2.5 - Several named tests are padding or use the wrong harness contract

**Plan section:** §4, W10.

**Evidence and correction:**

- `a below-fence or legacy-array shorter read never replaces a longer local mirror` (`docs/plans/new-chat-projection-race.md:618-619`) passes on main because the existing branch unconditionally keeps every longer local mirror at `frontend/src/lib/assistant/aevatar-transport.ts:2390-2392`. Keep it only as a clearly labeled regression guard paired with the falsifiable current-fence case; do not claim every listed test fails on main.
- `keeps the confirmed-stale redirect for a genuine not-found` (`docs/plans/new-chat-projection-race.md:643-646`) is already covered by the not-found repair block in `frontend/src/pages/assistant.test.tsx:508-551` and passes on main. Extend the existing test only if the implementation changes its inputs; otherwise it is padding under the owner's rule.
- The untouched PR #1304 suite cannot converge through public `getHistory()` after W3, as described in P2.2. It needs explicit reconciliation in the harness.
- `use-assistant.test.tsx` globally uses fake timers and resets the mock transport at `frontend/src/hooks/use-assistant.test.tsx:46-54`. `resetAssistantTransport()` currently resets only `MockAssistantTransport` at `frontend/src/lib/assistant/transport.ts:569-573`; it cannot "forward" Aevatar randomness. Hook tests should spy the optional capability or install a transport double, while Aevatar backoff tests instantiate `AevatarAssistantTransport(now, random)` directly.
- The §2.5 snippet invokes optional interface methods unconditionally. If the methods remain optional so the mock omits them, narrow both functions or use optional calls before invoking them; otherwise `npm run build` under strict TypeScript will reject the code. Prefer implementing no-op methods on the mock and keeping the interface required, which also makes the hook tests less brittle.

# P3 / notes

- W6's cache migration is coupled to this race and should remain, but test aliasing for `reconcileProjection()` and `releaseProjectionWaiter()` explicitly: both placeholder and canonical arguments must resolve to the same scoped entry, and invalidation must update the key the mounted observer is actually using.
- A refcounted promise can be paused and resumed, but aborting an `abortableDelay()` normally rejects the shared promise. §2.5 must specify a wakeable paused state or settle/remove the old entry and carry only attempt/deadline metadata into a new promise. Add rejection handling in the hook so an abort or unexpected protocol error cannot become an unhandled promise rejection.
- W7's quiet status treatment is appropriate. The page already keeps history failures non-blocking at `frontend/src/pages/assistant.tsx:500-535`; the new state should be a separate status, not encoded as an `ApiError`.
- W10 is correct that no dependency or lockfile edit is needed. Any `frontend/**` edit runs the wizard freshness job (`.github/workflows/ci.yml:129-133`), but the source hash changes only for files in the manifest or its extras (`cli/tests/wizard_bundle_freshness.rs:19-25,62-76`). Recheck the generated module graph if the auth-store import graph changes as described above.

# Corrections to the plan's verification section

## D1

The planner's narrowed conclusion is correct. Commit `d418c74a` exists and added the locally-held placeholder mirror guard now at `frontend/src/lib/assistant/aevatar-transport.ts:1410-1425`, with the regression at `frontend/src/lib/assistant/aevatar-transport.test.ts:6945-7001`. I re-audited the outbound families:

- History GET: both absent and locally-held placeholder branches stop the request at `aevatar-transport.ts:1394-1400,1414-1425`.
- Workflow create: `workflowTurnBody()` sends `commandId` and no `conversationId` at `aevatar-transport.ts:2571-2575`; continuations send only an adopted `chatc-*` id at `:2557-2569`.
- Approval: workflow ids, including pending workflow ids, are rejected before `startChatStream()` at `aevatar-transport.ts:1626-1638`.
- Stop: `requestServerStop()` returns before the request for every workflow run at `aevatar-transport.ts:4846-4854`; the typed body uses the canonical actor id at `:4865-4874`.
- Create recovery is keyed by encoded `commandId`, not conversation id, at `aevatar-transport.ts:2685-2705`.
- There is no browser `/state` request in this transport.
- DELETE remains the live leak: it interpolates the unguarded canonicalized argument at `aevatar-transport.ts:1302-1354`.

So D1 is not a P1 verification error. W2's proposed execution is still unsafe for the pre-alias delete window described in P1.2.

## D2-D3

D2's terminal GET and D3's stuck-error/success-empty split are correctly located. One wording correction: "the index row exists" is not necessarily server evidence. The transport's public list includes its own newly aliased local record even if the fetched upstream index did not contain it (`aevatar-transport.ts:1259-1269`). W5 must keep raw response membership distinct from the merged sidebar list.

## D4

The in-memory-only diagnosis is correct, but the claimed precedent is not. `assistant-context-store` does not self-heal on every read/write; only `recordScreen` scopes writes, and auth explicitly clears the store. The receipt design needs its own owner-check and auth-transition behavior rather than citing parity that is not present.

## D5-D6

D5 is correct: the four delays are exactly `[0, 300, 900, 1800]` at `aevatar-transport.ts:289`, used by the history and create-recovery paths at `:2633` and `:2691`. D6 is also correct that the longer-local return is currently unbounded. The W8 correction remains necessary for eventual convergence, subject to the explicit turn-presence predicate in P2.2.

## D7

The deletion-resurrection bug is real, but D7's proposed closure is not. A one-time index confirmation can itself be stale, a receipt is historical rather than current evidence, and W4 has no later raw-index absence transition. D7 must remain open until P1.3 is incorporated.

# What the plan misses entirely

- Valid `stateVersion: 0` first-turn contexts, already locked in by an existing regression test.
- Same-tab identity-pending recovery after a context-free failed/truncated/cancelled stream; W5 only repairs the fresh-transport `!existing` path.
- A lifecycle for receipts after definitive pre-admission failure versus ambiguous delivery.
- A durable deletion intent when DELETE occurs before alias adoption.
- Raw index membership versus the transport's merged local sidebar list.
- A cache/state transition for reconciliation deadline exhaustion.
- Full transport cleanup and post-await response guards on logout/account switch.
- Storage-disabled/quota-exhausted operation and future-clock skew.
- Browser background timer throttling and a required final attempt when a tab resumes after the nominal deadline.
- The fact that per-tab jitter is not cross-tab single-flight and that a storage event carries no materialization outcome in the proposed schema.

# Scope call

**Keep:** W1-W8's core architecture, including the visible non-error syncing state, cache-key migration, and the fence/turn-gated keep-max relaxation. W8 is coupled to reconciliation because declaring materialization while unconditional keep-max pins a stale local transcript would not actually converge.

**Add before implementation:** explicit identity-pending/projection-pending provenance; receipt outcome/expiry semantics; persisted pre-alias deletion intent; raw-index revalidation plus a real deadline transition; and an account-scope reset/abort boundary. These are not optional hardening. They close concrete resurrection, never-created, and cross-account failure paths created or exposed by the plan.

**Cut or split:** Do not automatically replace all three existing `HISTORY_RECONCILIATION_DELAYS_MS` consumers in the same change. The post-terminal reconciler and cold create recovery need the new deadline-aware schedule. The continuation preflight/reservation behavior is a separate user-facing admission contract and should move only with a dedicated falsifiable case showing the old 3-second budget causes this bug. At minimum, use separate policies rather than one helper configuration for background projection, ambiguous create recovery, and a foreground continuation send.

**CI scope:** Preserve the no-dependency/no-lockfile constraint. If account/logout correction changes the wizard's import closure, rebuild the committed wizard bundle rather than relying on the current manifest assumption.
