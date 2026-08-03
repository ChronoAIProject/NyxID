# Adversarial Implementation Review: Mock Scenario Interception

Reviewer: OpusRed (Opus 5). Target: `807f52c0..58134c62` on `mock-github-chat-calls`
(8 commits, 16 code files + docs). Contract: `mock-scenario-intercept-spec.md` (v2 +
round-2 amendments), `mock-scenario-intercept-plan.md`, and the two review files
(F1–F14, P1–P12).

**Gates run locally (all green):**
- `npm run build` — full chain incl. credential-accept + `assert-mock-footprint.mjs`.
  Footprint assertion passes; the script greps compiled bytes, not filenames
  (`scripts/assert-mock-footprint.mjs:41-47`), and the prod artifact is genuinely
  free of `mockchat-` / `scenario-engine` / `mockscenarios`. F12 holds.
- `npx vitest run --no-file-parallelism` over the 7 new suites + `transport.test.ts`:
  8 files, 61 tests passed.
- Per-commit hygiene: each WP commit carries its own test file; the diff stays inside
  the §13 authoritative scope; no `console.*` in new source; no import cycle
  (`transport.ts` has zero static references to any mock module; the interceptor never
  imports `transport.ts`).

What follows is what those green gates do **not** cover.

---

## [SEV-high] `ensureUser` resets `engineState` to `idle`, so the feature is dead on first run and after every account switch

**Claim.** The store's account-scoping reset (F14) wipes a runtime field that is not
account state, and nothing ever restores it. On the first session on a browser profile
— and on every login as a different user — interception throws on every matched message
for the rest of the session.

**Evidence (new code).**
- `frontend/src/stores/assistant-mock-scenarios-store.ts:77-85` —
  `ensureUser: (userId) => set((state) => state.userId === userId ? state : { ...DEFAULT_STATE, userId })`.
- `frontend/src/stores/assistant-mock-scenarios-store.ts:37-44` — `DEFAULT_STATE`
  includes `engineState: "idle"`.
- `frontend/src/lib/assistant/scenario-intercept-transport.ts:752-758` —
  `createScenarioInterceptTransport` sets `engineState: "ready"` at install.
- `frontend/src/lib/assistant/scenario-intercept-transport.ts:787-794` —
  `installScenarioInterceptor` runs the install (→ `ready`) and **then**
  `installAuthSubscription()`, which calls `ensureUser` immediately
  (`:768-773`) and on every non-null user transition (`:775-785`).
- `frontend/src/lib/assistant/scenario-intercept-transport.ts:288-291` —
  `sendMessage` throws `MockScenariosLoadingError` whenever `engineState !== "ready"`.
- `engineState` has exactly one setter and one non-test caller (grep: only
  `createScenarioInterceptTransport`), and it is excluded from `partialize`
  (`store:92-97`), so it is `"idle"` on every page load until install sets it.

**Concrete failure.** Fresh profile (persisted `userId` is `null`) or account switch:
app boots → interceptor installs → `engineState = "ready"` → `auth-store.checkAuth()`
resolves user A (`stores/auth-store.ts:44-49`, user starts `null` and resolves async)
→ subscription fires → `ensureUser("A")` → `state.userId (null) !== "A"` → reset →
`engineState = "idle"`. The dev opens the popover (which renders no status for `idle`),
flips the master switch on (not disabled — only `"loading"` disables it,
`mock-scenarios-action.tsx:104`), types "connect to my github", and gets
**"Mock scenarios are still loading. Retry in a moment."** — permanently. A page reload
"fixes" it (persisted `userId` now matches), which makes this read as a flake rather
than a bug.

**Test honesty.** The wipe is *codified as intent*:
`stores/assistant-mock-scenarios-store.test.ts:118-125` asserts
`engineState: "idle"` after `ensureUser("user-b")`. The lifecycle test that would catch
it — `lib/assistant/transport-shell.test.ts:222-245` (P6/F14, null → A → logout → B) —
drives the exact transition that triggers the reset and asserts `userId`/`enabled`/
`world` but never `engineState`. The WP7 integration test asserts
`engineState === "ready"` (`use-assistant.mock-scenarios.test.tsx:89-91`) but runs with
`useAuthStore.user === null`, so the subscription never fires, and it then overwrites
the field with `setState({ engineState: "ready" })` anyway (`:92-98`).

**Minimal fix.** Scope the reset to persisted, account-owned fields:

```ts
ensureUser: (userId) =>
  set((state) =>
    state.userId === userId
      ? state
      : { ...DEFAULT_STATE, userId, engineState: state.engineState }),
```

and flip the two tests: the store test should assert `engineState` is **preserved**
across `ensureUser`, and `transport-shell.test.ts:222-245` should assert it stays
`"ready"` across null → A → logout → B.

---

## [SEV-med] One parked mock card freezes the entire conversation sidebar for the rest of the session

**Claim.** The list snapshot (P7) is gated on a **global** predicate that includes the
open-ended `mock-parked` state, so a single parked card suspends delegated
`listConversations` for *all* conversations until it settles or the page reloads.

**Evidence (new code).**
- `scenario-intercept-transport.ts:742-749` — `hasMockActivity()` scans **all**
  ownership values for `mock-running | mock-parked | verifying`.
- `scenario-intercept-transport.ts:181-187` — `listConversations` calls the delegate
  only `if (!this.hasMockActivity())`; otherwise it serves `this.baseList` forever.
- `mock-parked` is entered by `runHooks.onPark` (`:584-591`) and left only by settle,
  cancel, delete, or a newer send in that same conversation — i.e. human time, not
  stream time.

**Concrete failure.** Dev sends "connect to my github" in conversation X; the turn parks
(`mock-parked`) while they work through the real AddKeyDialog. They switch to a real
conversation Y and chat normally. Every projection now serves the pre-park sidebar
snapshot: Y stays titled "New chat" after its first live turn, recency ordering is
frozen, and any conversation created in another tab is invisible. This persists for the
whole session if the card is never resolved. Spec §6.4 scoped the snapshot to *"while a
scripted turn streams"* (F8) and *"a >5 s script"* (P7); it did not license freezing the
index across a human-length park.

Related, same mechanism: `getHistory` skips the delegate for a `mock-parked`
conversation (`:225-237`) and throws `AssistantConversationNotFoundError` if no base was
ever cached (`:238-239`) — reachable when the first send in a freshly-opened
conversation matches before the initial history read resolves.

**Test honesty.** The P7 test (`scenario-intercept-transport.test.ts:580-629`) is
genuinely honest about what it covers — it drives the **real** `AevatarAssistantTransport`
with mocked HTTP, a 6 s script and a 5 s list TTL, and would fail if the snapshot were
removed — but it only ever exercises `mock-running`. No test covers list behavior while
parked.

**Minimal fix.** Restrict the freeze to states that actually stream:
`hasMockActivity()` → `mock-running | verifying` only, and keep projecting the overlay
over a **refreshed** base while parked (the overlay projection at `:188-207` already
handles a changed base; anchored merge at `:686-694` already falls back to tail append
when an anchor disappears). Add a test: park a card, then assert a delegated list
refresh still happens and the parked overlay still projects.

---

## [SEV-med] A failed dynamic install leaves the toggle ON with no interceptor — F6's exact failure, unmitigated

**Claim.** Under the declared §8.3 deviation (boot-time install), the F6 machinery is
inert: nothing can ever set `engineState` to `"loading"` or `"error"`, and the install
failure path swallows the error, so a persisted-enabled toggle silently sends
intended-mock content to the live assistant.

**Evidence (new code).**
- `frontend/src/lib/assistant/transport.ts:734-737` —
  `void installAssistantTransportInterceptor(shell, () => import(...)).catch(() => undefined)`.
  On rejection the shell keeps the bare `AevatarAssistantTransport`; no state is
  recorded anywhere.
- `engineState` is only ever set to `"ready"`
  (`scenario-intercept-transport.ts:756`); `"loading"` and `"error"` are unreachable in
  production code (grep over `src/`).
- Consequently `MockScenariosLoadingError` (`:67-76`, `:288-291`) and the popover's
  loading/error branches (`mock-scenarios-action.tsx:91-99, 104`) are dead code as
  shipped.

**Concrete failure.** In dev the interceptor chunk fails to load (HMR churn, a transient
dev-server 500, a stale optimize-deps cache). `enabled` is persisted `true`, the popover
renders no error and shows the active dot, and the dev types the destructive rehearsal
they intended to script. It reaches the real assistant — the precise scenario F6 was
accepted to prevent — and nothing in the UI or the console says why.

**Minimal fix.** Wire the two dead states from the dev boot path in `transport.ts`
(still no static mock imports — do it inside the `import.meta.env.DEV` branch with a
dynamic store import):

```ts
if (import.meta.env.DEV) {
  const setState = (s: "loading" | "error") =>
    void import("@/stores/assistant-mock-scenarios-store")
      .then((m) => m.useAssistantMockScenariosStore.getState().setEngineState(s))
      .catch(() => undefined);
  setState("loading");
  void installAssistantTransportInterceptor(shell, () => import("...")).catch(() =>
    setState("error"));
}
```

Then the popover's error branch and the F6 gate become reachable, and
`mock-scenarios-action.test.tsx:35-51` stops testing unreachable UI.

---

## [SEV-med] `deleteConversation` tombstones conversations that have no mock state, so a failed DELETE permanently hides a real conversation

**Claim.** The P10 tombstone-on-failure rule is applied unconditionally, including to
conversations the wrapper has never owned. With the toggle off and zero mock state,
`deleteConversation` is therefore **not** a bit-identical delegate call, and a rejected
DELETE loses a live conversation for the rest of the session.

**Evidence (new code).** `scenario-intercept-transport.ts:243-265` — no ownership check
before `this.tombstones.add(conversationId)`; the delegate call happens after, and the
tombstone is never cleared on rejection (deliberately, per P10). Downstream,
`requireNotDeleted` (`:578-582`) throws `AssistantConversationNotFoundError` from
`getHistory`/`sendMessage`/resumes, and `listConversations` filters the id out
(`:189-193`) even when the delegate still returns it.

**Concrete failure.** Dev deletes an ordinary real conversation; the DELETE fails
(offline, 500, 401 mid-refresh). The real transport's contract is "local state removed
only on success" (`aevatar-transport.ts:1358-1383`), and the hook keeps its caches
(`use-assistant.ts:368-386`) — so the conversation should still be there. Instead the
wrapper has tombstoned it: it is gone from the sidebar, its transcript 404s, and sends
into it throw not-found, until a reload. This is a dev-only but toggle-independent
regression to live behavior.

**Test honesty.** `scenario-intercept-transport.test.ts:838-871` (P10) parks a mock card
first, so it only exercises the owned case and would still pass after the fix.

**Minimal fix.** Tombstone only when the wrapper owns state for that conversation
(`overlays.has(id) || claimedConversations.has(id) || ownership.has(id) ||
verificationRuns.has(id)`); otherwise cancel nothing and delegate straight through. Add
a test: delete an unowned conversation with a rejecting delegate → wrapper stays inert,
subsequent `getHistory` still reaches the delegate.

---

## [SEV-med] A mock resume while a mock script is running throws a bare `Error` and corrupts the ownership state machine

**Claim.** §10 requires `AssistantTurnActiveError` parity for the extended §6.2 states.
The resume paths only guard `delegate-active`; a resume during `mock-running` falls
through to the engine's plain `Error`, and the catch blocks then write a **wrong**
ownership state.

**Evidence (new code).**
- `scenario-engine.ts:915-917` — `startSegment` throws
  `new Error("A mock scenario turn is already active.")` (not `AssistantTurnActiveError`).
- `scenario-intercept-transport.ts:571-576` — `requireMockMutationAllowed` rejects only
  `delegate-active`.
- `scenario-intercept-transport.ts:375-379`, `:449-452`, `:479-483` — every catch does
  `this.ownership.set(conversationId, "mock-parked")` regardless of the state it came
  from.

**Concrete failure.** A scripted turn is streaming in conversation X (`mock-running`).
The dev clicks Approve on an older approval card (or Done in a still-open dialog).
`decideApproval` → engine → `resumeContinuation` → `startSegment` → bare `Error`;
`useDecideApproval.onError` (`use-assistant.ts:745-756`) surfaces the raw string instead
of the "Wait for the current reply to finish" copy. Worse, the catch sets ownership to
`mock-parked` while script A is still streaming; the next `sendMessage` now takes the
settle-then-proceed branch (`:281-284`), **settles a card that was not superseded**, and
then throws the same bare `Error` from `engine.play`.

**Test honesty.** No test sends or resumes while `mock-running`; the only
`AssistantTurnActiveError` assertion is the `delegate-active` case
(`scenario-intercept-transport.test.ts:413-418`). Spec §6.2's first rule
("`sendMessage` in `mock-running` throws `AssistantTurnActiveError`") has no executable
assertion at all.

**Minimal fix.** Reject `mock-running` / `verifying` in `requireMockMutationAllowed`
with `AssistantTurnActiveError`; capture the prior ownership value and restore *it* in
the catch blocks instead of hard-coding `"mock-parked"`; have the engine throw
`AssistantTurnActiveError` so any residual path keeps parity. Add the two missing tests.

---

## [SEV-low] The F10 "one resumable card per await" guard misses cards spliced in from an await branch

**Claim.** The compiler validates branch bodies in isolation, but the runtime replays
`[...branch, ...remaining]` as a single segment — so a branch card plus a
post-await card compiles and parks two resumable cards at one await.

**Evidence (new code).** `scenario-engine.ts:370-380` — at an `await`, `validateSteps`
checks `cardCount > 1`, validates each branch with a fresh count of `0`, then resets
`cardCount = 0` for the remaining steps. `scenario-engine.ts:1436-1451` —
`resumeContinuation` plays `[...branch, ...continuation.remaining]` through one
`buildSegmentPlan`, where `lastCard` (`:1229`, `:1262`) is simply overwritten by the
later card.

**Concrete failure.** This config compiles clean:

```ts
scenario("two-park", /go/, (s) => s
  .action("service.connect", { service: "api-github" })
  .await({ completed: (b) => b.action("service.connect", { service: "api-openai" }) })
  .action("service.connect", { service: "api-lark" })
  .await())
```

On resume, the OpenAI card is emitted and then the segment parks on the Lark card. The
OpenAI card has no continuation: completing it throws "Action continuation was not
found" and it stays pending forever — exactly the F10 failure the compiler restriction
was accepted to make impossible.

**Minimal fix.** Validate the composition, not the branch: for each `await`, run
`validateSteps([...branch, ...stepsAfterThisAwait], flows, 0)` for every branch (and for
the empty/fall-through branch), instead of validating branches with a reset count.
Add the config above as a rejection test.

---

## [SEV-low] Dead production seam in `transport.ts` (+192 vs ~20 planned) and a no-op world seed in the engine

**Claim.** Two of the three flagged size deltas are justified; the transport delta
carries a parallel, production-unused copy of the boot branch, and the engine carries a
seed that can never match.

**Evidence (new code).**
- `transport.ts:669-720` — `AssistantTransportFactories` and
  `createAssistantTransportForEnvironment` are exported but called only from
  `transport-shell.test.ts`; the shipped `createAssistantTransport` (`:721-740`)
  re-implements the same kind-select → shell → dev-install branch inline. The P5 shell
  tests therefore exercise a *copy* of the boot path; the real singleton is proven only
  by the WP7 integration test (`use-assistant.mock-scenarios.test.tsx:80-91`, which
  does correctly consume `transportModule.assistantTransport`).
- `scenario-engine.ts:954-958` — `simulatedConnected` is seeded from
  `Object.keys(this.compiled.flows)` (flow **names**, e.g. `connect-github`) filtered by
  `world.isConnected` (service **slugs**, e.g. `api-github`). The two namespaces never
  intersect, so the seed is always empty; `isConnected` (`:959-960`) already falls back
  to the world port.
- Not a defect, for the record: `pages/assistant.tsx` +46 (`AssistantHeaderActions` with
  an injectable `scenarioAction` prop) is earned — it is what makes the P8 deferred-import
  test real (`mock-scenarios-action.test.tsx:147-184`), and the prod branch still folds
  to `null`. The engine at 1490 lines is proportionate to the verb table + guards +
  cancel/expiry paths; I found no dead verb.

**Minimal fix.** Make `createAssistantTransport` call
`createAssistantTransportForEnvironment` so there is one code path, and delete the
flow-name seed (`simulatedConnected` starts empty).

---

## [SEV-low] Coverage gaps and assertions that cannot fail

Not style nits — each one is a place where a regression in specced behavior would ship
green.

- **`wakeActions` on a mock-owned turn has zero coverage.** `scenario-engine.ts:824-849`
  (branch lookup, default "The assistant resumed the blocked turn." segment,
  continuation removal) and `scenario-intercept-transport.ts:461-484` are never called
  by any test; `StubTransport.wakeCalls` (`test:87, 215-218`) is recorded and never
  asserted. Mitigating: no UI currently calls `wakeActions` (grep: only type + transport
  definitions), so this is latent, not live.
- **The F1 cursor test is tautological.** `scenario-engine.test.ts:432-473` injects
  `TestCursors(20)`, a monotonic source, and then asserts monotonicity. The cursor
  source that actually matters — `overlay.turnState.lastCursor + 1`
  (`scenario-intercept-transport.ts:159-162`) — has no direct assertion; it is covered
  only transitively, by the WP7 integration test rendering "GitHub connected" through a
  real `applyTurnEvent` overlay. That transitive coverage is real (I verified the guard
  at `stream.ts:94` and that every accepted event advances `lastCursor` at `:96`), but a
  direct cursor assertion through the interceptor is one line and would pin it.
- **The F8 wire-log assertion cannot fail.** `scenario-intercept-transport.test.ts:507-535`
  asserts an empty wire log while running against `StubTransport`, which performs no
  HTTP at all. The delegate-read count in the same test *is* meaningful; the wire-log
  half is decorative. (The P7 test is the one that genuinely exercises the real delegate.)
- **Misleading title.** `scenario-engine.test.ts:575` claims expiry is asserted
  "independent of toggle state (F11)"; the test never touches the toggle, and the engine
  has no toggle awareness. Either drive the store through the interceptor or drop the
  claim.
- **Popover loading/error states** (`mock-scenarios-action.test.tsx:35-51`) test states
  production cannot produce — see the SEV-med finding above; they become honest once the
  install path sets them.

---

## Checks that passed (recorded so they are not re-litigated)

- **F1 (cursors).** Conversation-monotonic in the wrapper: every engine emission is
  stamped `overlay.turnState.lastCursor + 1` at delivery
  (`scenario-intercept-transport.ts:159-162`, `scenario-engine.ts:1339-1342`,
  `:1480-1489`), and the overlay reducer only ever ingests mock events, so real server
  cursors cannot shadow them. The hook's per-episode watermark starts at 0
  (`use-assistant.ts:443, 543-544`), so continuations are accepted there too.
- **F2 (toggle-independent ownership).** Routing is by `mockchat-` prefix and ownership
  state, never by `enabled`; `enabled` gates only new-`sendMessage` matching
  (`:286-298`). The toggle-off suite covers Stop, progress, block, failed report,
  completed report, approve and deny with zero delegate mutations
  (`test:329-395`).
- **F3, F4, F5/P1/P2, F7, F9/P12, F13, P10.** Implemented as specced and covered by
  tests that can fail: the verification test asserts the real
  `GET /api/v1/keys/{id}` path and `KeyInfo.catalog_service_slug` through the real
  `apiClient` (`test:633-693`), with mismatch → `failed` branch + world untouched, 404
  and malformed → unverified note, and cancel/delete aborting the in-flight lookup
  (`test:744-801`). The provisional handle returns `turnId: null` synchronously and the
  card patch fires before the lookup resolves, so the hook's pump is not disowned
  (`:499-507`, verified against `use-assistant.ts:801-814` and the 8 s start-deadline at
  `:515-537`).
- **F11 (approval expiry).** Enforced lazily at decision time with both boundary sides
  tested (`scenario-engine.ts:792-804`, `test:575-617`).
- **F12 (prod footprint).** Verified by running the build, not by reading the script.
- **§10 protocol items.** `turn.status waiting` → `turn.completed blocked`
  (`scenario-engine.ts:1297-1308`); resumes are new turn ids
  (`:1436-1451`); every `block.started` closed, incl. cancel via `toTerminalBlock`
  (`:1402-1434`); `message.completed` before the terminal (`:1294-1296`); action params
  through `assistantActionRequestSchema.parse` + `resolveAssistantAction` at both
  compile and play time (`:300-326`, `:1173-1202`).
- **N1 (no full-mock regression).** `MockAssistantTransport`, `mock-data.ts`,
  `createScriptedTurn` untouched apart from a pure formatting change
  (`transport.ts:133-145`); `MODE === "test"` still selects the full mock
  (`transport.test.ts` green, and `transport-shell.test.ts:195-220`).
- **DESIGN.md / a11y.** Trigger matches `AssistantWireLogAction` (ghost icon button +
  Tooltip, `aria-label="Mock scenarios"`); every token used
  (`nyx-secondary-400`, `warning`, `success`, `text-tertiary`, `destructive`) exists in
  `app.css`; the required P9 warning copy is present and asserted
  (`mock-scenarios-action.tsx:112-118`, `test:126-145`); both mount points wrap the lazy
  action in a local `<Suspense fallback={null}>` (`pages/assistant.tsx:57-72`, mounted
  at `:527` and `:538`).

---

## Verdict

**MERGEABLE AFTER LISTED FIXES.**

The architecture is sound and the hard parts (ownership routing, anchored merge,
synchronous verifying handle, cancel/delete ordering, prod footprint) are genuinely
implemented and genuinely tested. But finding #1 means the feature does not work on a
first run, and it slipped through because a test asserts the broken behavior as intent
— that has to be fixed before this ships, not after.

Fix list, in order for the implementer:

1. **`ensureUser` must not reset `engineState`** (store `:77-85`); invert the two tests
   that currently lock in the wipe (`store.test.ts:118-125`,
   `transport-shell.test.ts:222-245`). *Blocker.*
2. **Scope the list-snapshot freeze to streaming states** — drop `mock-parked` from
   `hasMockActivity()` (`:742-749`); add a parked-refresh test.
3. **Set `engineState` `loading`/`error` from the dev boot path** in `transport.ts`
   (`:734-737`) so F6 and the popover's error state are reachable.
4. **Tombstone only wrapper-owned conversations** in `deleteConversation` (`:243-265`);
   add an unowned-failed-delete test.
5. **`AssistantTurnActiveError` parity for mock resumes during `mock-running`/
   `verifying`**, and restore the prior ownership state in the three catch blocks
   (`:375-379`, `:449-452`, `:479-483`); add the two missing §6.2 tests.
6. **Close the F10 branch+remaining hole** (`scenario-engine.ts:370-380`).
7. **Collapse the duplicate boot branch** (`transport.ts:721-740` → call
   `createAssistantTransportForEnvironment`) and drop the flow-name world seed
   (`scenario-engine.ts:954-958`).
8. **Coverage**: `wakeActions` mock path; a direct cursor-monotonicity assertion through
   the interceptor; retitle the F11 test; either make the F8 wire-log assertion
   meaningful or drop it.

Items 1–5 change behavior and should land before the PR. Items 6–8 are cheap and should
land with them; none of them require re-opening the spec.

---

# Re-review (delta `58134c62..3725e294`)

Two commits: `09aa5eee` (items 1–7) and `3725e294` (item 8). 316 insertions / 59
deletions across 8 files — no scope creep beyond the review's fix list, no doc edits, no
spec changes.

**Gates re-run by me, not taken on report:**
- `npx vitest run --no-file-parallelism` — **203 files / 2469 tests passed**, matching
  the implementer's claim.
- `npm run build` — full chain green; **footprint assertion still passes**, and
  `grep -rl "assistant-mock-scenarios\|mockscenarios\|scenario-intercept" dist/` returns
  nothing. This was the delta's riskiest change for F12 (a new dynamic `import()` of the
  store now sits inside `createAssistantTransport`), so it was worth re-verifying rather
  than assuming.
- `npm run lint` — 0 errors, 23 warnings, **none** in any mock-scenario file (all
  pre-existing: `button.tsx`, `form.tsx`, `docs-layout.tsx`, `ai-setup.tsx`).

## Per-fix verdicts

### 1. `ensureUser` no longer resets `engineState` — **FIXED**

`assistant-mock-scenarios-store.ts:84` adds `engineState: state.engineState` to the
account-switch reset. Both tests were inverted honestly, not weakened:
`store.test.ts:112-128` now calls `setEngineState("ready")` **after**
`persist.rehydrate()` and asserts `"ready"` survives `ensureUser("user-b")`, and
`transport-shell.test.ts:247-271` now asserts `"ready"` at three points — including
immediately after the `null → user-a` transition, which is the exact assertion whose
absence let the bug ship. Both fail if the carve-out is removed (they revert to `"idle"`).

Persist/rehydrate interaction (asked): safe. `engineState` is outside `partialize`
(`:92-97`), and zustand's default merge is `{...currentState, ...persistedState}`, so
rehydration never writes the field; the store test now pins that ordering explicitly by
setting `ready` post-rehydrate. `lastActivity` is still cleared on account switch, which
is correct — it is user-scoped.

### 2. Sidebar freeze scoped to streaming states — **FIXED**

`hasMockActivity()` (`:769-773`) is now `mock-running || verifying`, and `getHistory`
(`:227-230`) no longer skips the delegate while parked.

**F8/P7 are not re-opened**, and I checked the specific mechanism asked about — does a
parked conversation's history fall-through reissue delegate GETs per event during a
*later* mock-running turn elsewhere? No, for two independent reasons:
- The per-event projection is single-conversation: the pump calls
  `projectTransportState(queryClient, targetId)` with its own stream's `targetId`
  (`use-assistant.ts:116-142`, `:477-492`). A parked conversation's `getHistory` is
  simply never called by another conversation's event loop.
- `hasMockActivity()` still scans **all** conversations, so any `mock-running` /
  `verifying` turn anywhere still suppresses the delegated list GET globally. P7's
  guarantee is untouched, and its test (real `AevatarAssistantTransport`, stale 5 s TTL,
  6 s script) drives `mock-running`, so it still means what it meant.

The new guard test (`scenario-intercept-transport.test.ts:602-641`) is real: it parks a
card, mutates the delegate's stored history/list, then asserts exactly
`historyCalls === 1` / `listCalls === 1`, that the refreshed base title projects, that
the parked overlay's user message survives the base swap (anchored merge over a changed
base), and that history and list counts still agree (P12). Reverting either half of the
fix fails it.

*Note, not a defect:* interacting with a parked card (`setInProgress` / `blockAction` /
`continueAction`) now triggers ordinary delegated history+list GETs, which appear in the
wire-log panel. That is the intended price of unfreezing the sidebar — F8's guarantee is
scoped to streaming turns and N3 to mock turns crossing the network, and neither changes.

### 3. `loading` / `error` wiring — **FIXED-WITH-NOTE**

`transport.ts:713-716` reports `"loading"` before install and `"error"` in the catch;
`:733-753` threads a store-writing reporter through the dev branch only, and the prod
branch passes no loader and no reporter. Both states are now reachable, so the popover's
inline error and the F6 gate stop being dead code. `transport-shell.test.ts:122-193`
asserts `idle → loading → ready` and `idle → loading → error`, and both assertions fail
if the reporter is dropped.

**Note (the race asked about).** The `"loading"` write is itself a dynamic `import()` of
the store, racing the installer's own `"ready"` write. It is ordered *by dependency*: the
store is a static import of the interceptor module
(`scenario-intercept-transport.ts:18`), so the interceptor's module promise cannot settle
before the store's, and each write is one `.then` hop after its own promise — `"loading"`
lands first in both a browser graph load and vitest (the WP7 integration test asserts
`ready` after both imports and passes). I could not construct an inversion. But the
consequence if it ever inverted is silent and severe: `engineState` pinned at `"loading"`
disables the master switch (`mock-scenarios-action.tsx:104`) *and* throws on every
matched send. One defensive line retires the reasoning burden — have the reporter refuse
to downgrade a terminal state:

```ts
const setState = (state: "loading" | "error") =>
  void import("@/stores/assistant-mock-scenarios-store")
    .then((m) => {
      const store = m.useAssistantMockScenariosStore.getState();
      if (state === "loading" && store.engineState === "ready") return;
      store.setEngineState(state);
    })
    .catch(() => undefined);
```

### 4. Tombstone scoped to wrapper-owned conversations — **FIXED-WITH-NOTE**

`:244-252` gates the whole tombstone path on
`overlays || claimedConversations || ownership || verificationRuns`, and an unowned
conversation now delegates straight through. The new test
(`scenario-intercept-transport.test.ts:913-934`) proves inertness after a *rejected*
delegated DELETE — history and list still reach the delegate and return the real data —
and it fails if the gate is removed. P10's owned-case test is unchanged and still passes,
so the distinction is genuinely tested on both sides.

**Note A (residual, SEV-low).** `this.ownership.has(id)` is also true for
`delegate-active`, i.e. a purely live conversation with a streaming pass-through turn and
no mock state at all. Delete during a live reply + a failed DELETE therefore still
tombstones a real conversation — my original failure mode, narrowed from "any real
conversation" to "a real conversation being deleted mid-stream". Minimal tightening:
`const owned = this.ownership.get(id); ... || (owned !== undefined && owned !== "delegate-active")`.

**Note B (cosmetic).** The unowned path does not drop the row from `baseList` /
`baseHistories` on success; if a mock turn happens to be streaming at that moment the
deleted conversation lingers in the projected sidebar until the next list refresh.
Self-healing.

### 5. `AssistantTurnActiveError` parity + ownership restore — **FIXED**

`requireMockMutationAllowed` (`:582-592`) now rejects `mock-running` / `verifying` /
`delegate-active`; `restoreOwnership` (`:594-603`) restores the captured prior state or
deletes the entry. The engine throws `AssistantTurnActiveError` (`:919`). The two new
tests (`:328-352`) assert the throw type **and** zero delegate calls; the resume-while-
running one genuinely fails without the guard (it would surface a plain
`"Action continuation was not found."`).

Nested-failure check (asked): `priorOwnership` is captured before the guard runs, so a
rejected guard never mutates state; the catch bodies (`discardEmptyActiveGroup` +
`restoreOwnership`) cannot themselves throw. The restore is strictly better than the old
hard-coded `"mock-parked"`: a failure with no prior state now returns the conversation to
idle instead of leaving a phantom park that made the next send settle nothing and
mis-route.

One defensive residual: `resolveVerification` is fired as `void this.resolveVerification(...)`
(`:505`) with no try/catch around `prepared.resume(...)` (`:542`). A throw there becomes
an unhandled rejection *and* pins ownership at `mock-running` — a bricked conversation. I
could not reach it (the new guard makes a concurrent script impossible while `verifying`),
so this is hardening, not a defect: wrap the body and `restoreOwnership` on failure.

### 6. F10 composition validation — **FIXED-WITH-NOTE**

`scenario-engine.ts:377-381` now validates `[...branch, ...remaining]` for every branch
plus the fall-through `remaining`. My exact counter-example is the new rejection test
(`scenario-engine.test.ts:218-238`) and it fails if the composition is reverted.

**It does not reject legal-and-correct configs** — I checked the case that matters most:
two *sequential* awaits each parking one card (`action A → await → action B → await`)
still compiles, because each await's composition sees exactly one card. The shipped
config also still compiles (`scenarios.config.test.ts` green).

Notes: (a) the walker does not treat `.stop()` / `.fail()` as segment terminators, so a
branch that emits a card and then unconditionally stops is now counted together with a
post-await card and rejected even though it can never park both at runtime — the
over-count was already possible within a single list; composition just makes it reachable
through branches. Two lines to fix (break on `stop`/`fail`). (b) The composition is
lexical only: an await spliced in from a `run`/`need` flow is still validated against the
*flow's* remainder, not the caller's, so `.need(x, f).action(C).await()` where `f`'s
branch emits a card can still strand a card at runtime. Narrower than what I reported and
untouched by the shipped config; a v2 item. (c) `match()` re-runs `validateSteps` per send
(`:590`), and composition makes that ~3^(sequential awaits) list walks — irrelevant at
current config size, worth knowing before someone authors a long multi-park rehearsal.

### 7. Single boot path + dead seed removed — **FIXED**

`createAssistantTransport` (`transport.ts:729-756`) now delegates both branches to
`createAssistantTransportForEnvironment`, so the P5 tests exercise the actual production
function. Better than asked: `transport-shell.test.ts:139` now installs the **real**
`installScenarioInterceptor` instead of a fake one, and asserts through it
(`shell.current() !== live`, the real interceptor's pass-through reaching `live`,
`engineState === "ready"`). `simulatedConnected` starts empty (`scenario-engine.ts:957`).

### 8. Coverage — **FIXED; I accept the F8 deletion**

- `wakeActions` is now covered at both layers: engine (`scenario-engine.test.ts:388-431`
  — default branch text, remainder plays, continuation consumed, second wake throws) and
  transport (`scenario-intercept-transport.test.ts:352-401` — routes to the engine with
  `delegate.wakeCalls === 0`, authored `wake` branch selected).
- That same transport test carries the **direct cursor assertion** I asked for: strict
  monotonicity across the entire park → resume sequence, plus
  `events[parkedEventCount].cursor > parkedCursor`. That exercises the real
  `overlay.turnState.lastCursor + 1` source (`:159-162`), not an injected one, and fails
  if it regresses to per-turn numbering. F1 now has a first-class guard.
- F11 title corrected.
- **The F8 wire-log half was deleted** (assertion, `beforeEach` setup, and the store
  import), with the test title updated to match what it now claims. That is the option I
  explicitly permitted, and I accept it: the honest half — zero delegated history reads
  across >20 projections — remains and still fails if the snapshot breaks, and the
  wire-log-adjacent guarantee is carried by the P7 test against the real Aevatar delegate
  with mocked HTTP. Deleting a vacuous assertion is the right call over dressing it up.

## New findings

### [SEV-low] Four changed files no longer satisfy Prettier

`npx prettier --check` fails on `scenario-intercept-transport.ts`, `transport.ts`,
`scenario-intercept-transport.test.ts`, and `scenario-engine.test.ts`. I verified the
same four files were clean at `58134c62`, so the delta introduced it (hand-edited
conditionals that now fit on one line, e.g. `:227-230` and `:769-773`, and the new test
blocks). There is no `format` script and no CI step, so nothing gates it — but the repo
is uniformly Prettier-formatted and this will show up as noise in the PR diff.
Fix: `npx prettier --write` on those four files.

No other new defects. I specifically hunted the five interactions flagged for this pass —
persist/rehydrate vs. the `engineState` carve-out, parked history fall-through
re-opening F8/P7, the reporter's dynamic import racing the installer's `ready` write,
ownership restore under nested failures, and F10 composition over-rejecting legal
configs — and all five are either clean or covered by the notes above.

## Green-list re-confirmation

No coverage regression. F1 (now directly asserted at the interceptor layer, strictly
stronger), F2, F3, F4, F5/P1/P2, F6 (now actually armed rather than dead), F7,
F9/P12, F10 (strengthened), F11, F12 (re-verified by build + `dist` grep), F13, F14
(strengthened), P5 (strengthened — real installer), P6, P7 (semantics unchanged),
P8/P9/P11 (untouched), P10 (retained and correctly narrowed). The only deleted assertion
is the vacuous one, by agreement.

## Final verdict

**MERGEABLE AS-IS.**

All eight items are genuinely fixed — no test was weakened to pass, and three of them
(P5's real installer, the parked-refresh guard, the interceptor-level cursor assertion)
came back stronger than I asked for. Nothing outstanding blocks the PR.

Recommended before pushing, all optional one-liners, none of which I would hold the merge
for:

1. `prettier --write` the four files (the only thing I would actually do first — it is
   free and keeps the PR diff clean).
2. Exclude `delegate-active` from `wrapperOwned` in `deleteConversation` (§4 Note A).
3. Make the state reporter refuse to downgrade `ready → loading` (§3 Note).
4. `try/catch` + `restoreOwnership` around `resolveVerification`'s body (§5 residual).
5. Break the config walker on `.stop()` / `.fail()` (§6 Note a).

Items 2–5 are hardening against paths I could not reach; if they are deferred, they
belong in the PR body as known residuals rather than in a follow-up nobody files.
