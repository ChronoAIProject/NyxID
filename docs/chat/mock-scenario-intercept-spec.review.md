# Adversarial Review: Mock Scenario Interception

The ordinary ownership identifiers do flow through the current UI: approval decisions pass the card `block_id`, while action reports copy the card's `origin_turn_id`. The failures below concern cursor scope, ownership after the master toggle changes, mixed live/mock chronology, and real-journey outcomes.

## Findings

### [SEV-high] Turn-scoped cursor resets can make every continuation event disappear from the overlay reducer

**Evidence:** `applyTurnEvent` rejects any event whose cursor is not greater than the reducer's existing `lastCursor` (`frontend/src/lib/assistant/stream.ts:89-96`). The full-mock transport does not actually deliver each continuation from cursor 1: it rewrites every source event to `store.lastCursor + 1` (`frontend/src/lib/assistant/transport.ts:443-449`). The real approval and action continuations likewise seed their run cursor from the stored conversation cursor (`frontend/src/lib/assistant/aevatar-transport.ts:1657-1660`, `frontend/src/lib/assistant/aevatar-transport.ts:2214-2215`). Only the React-query event pump is episode-scoped and starts its dedupe watermark at zero (`frontend/src/hooks/use-assistant.ts:433-444`, `frontend/src/hooks/use-assistant.ts:539-545`).

**Concrete failure scenario:** Segment 1 ends blocked at cursor 14. The engine stores its projected messages in one conversation-level `TurnReducerState`, as the existing transports do. `continueActions` starts the new scripted turn at cursor 1. The hook's new pump accepts the events and eventually closes the episode, but the overlay reducer ignores cursors 1 through N; the card never patches and the continuation message never appears in `getHistory`.

**Suggested spec fix:** Require conversation-monotonic delivered cursors, seeded from the overlay's `lastCursor`, for sends, local card patches, and continuations. If turn-scoped cursors are intentional, specify separate per-turn reducers plus the cross-turn lookup needed to patch the parked card. Add an integration test whose first turn ends above the continuation's terminal cursor.

### [SEV-high] Toggle-off routing can POST mock action reports to the real conversation actor and makes Stop miss a running mock

**Evidence:** Stop uses transport-level lookup, not the retained handle (`frontend/src/hooks/use-assistant.ts:691-701`). Approval resolution passes `blockId` (`frontend/src/hooks/use-assistant.ts:706-727`), and action resolution passes `report.originTurnId` (`frontend/src/hooks/use-assistant.ts:797-808`). On the real transport, an unknown approval block fails locally (`frontend/src/lib/assistant/aevatar-transport.ts:1624-1633`), but an unknown action request is not rejected: validation proceeds (`frontend/src/lib/assistant/aevatar-transport.ts:1881-1923`), the real conversation is selected as actor (`frontend/src/lib/assistant/aevatar-transport.ts:1942-1946`), and the batch is sent upstream (`frontend/src/lib/assistant/aevatar-transport.ts:2186-2223`).

**Concrete failure scenario:** A mock GitHub card is parked and the real AddKeyDialog is open. The user flips interception off, completes the connection, and clicks Done. Because the spec says every method delegates while off, `continueActions(realConversationId, "mockchat-turn-1", ...)` reaches `AevatarAssistantTransport` and can POST the mock origin/action IDs to the real actor. If instead the toggle is flipped during a running script, pressing Stop calls the delegate, which has no matching run, so the mock timers continue.

**Suggested spec fix:** Make ownership routing independent of the master send toggle. Existing mock-owned runs, blocks, origin turns, continuations, cancel calls, and deletes must continue routing to the engine until settled; `enabled` should govern only whether a new `sendMessage` may match. Add toggle-off tests for Stop, approve/deny, dialog close/progress, completed action reports, and failed action reports, with zero delegate mutations.

### [SEV-high] A blocked mock continuation is not defined as a conversation reservation, allowing a live and mock continuation to interleave

**Evidence:** The shared active predicate treats only `running` and `waiting` as active, not `blocked` (`frontend/src/types/assistant.ts:328-330`). The real transport therefore admits a send whenever its own mirror has no running/waiting turn (`frontend/src/lib/assistant/aevatar-transport.ts:1517-1532`). Each new send or continuation creates a new pump owner for the same conversation (`frontend/src/hooks/use-assistant.ts:433-456`, `frontend/src/hooks/use-assistant.ts:512-544`), so a later pump causes earlier stream events to be ignored by the hook.

**Concrete failure scenario:** A scripted action turn emits `turn.completed: blocked` and retains a continuation. Before finishing the connect wizard, the user sends an unmatched message; it passes through and starts a real stream. The wizard then reports completion. Unless the continuation itself reserves the conversation and the wrapper also tracks delegate activity, the engine starts its continuation concurrently. Its pump steals episode ownership, real stream events stop projecting through the hook, active handles overwrite one another, and the two histories race.

**Suggested spec fix:** Define a per-conversation ownership state machine covering `mock-running`, `mock-parked`, and `delegate-running`. Either reject every send while `mock-parked`, or explicitly invalidate/settle the parked cards before allowing a live send; mock resumes must also reject while the delegate is active. Test both event orders.

### [SEV-high] A first intercepted turn remains under a local placeholder, and later canonicalization orphans its overlay and continuations

**Evidence:** `createConversation` does not create a real actor; it returns a `workflow-pending-*` placeholder that is materialized and aliased only by the first delegated workflow turn (`frontend/src/lib/assistant/aevatar-transport.ts:1278-1299`). The delegate maintains a placeholder-to-server alias map and canonicalizes public calls (`frontend/src/lib/assistant/aevatar-transport.ts:1223-1242`, `frontend/src/lib/assistant/aevatar-transport.ts:1388-1389`). The page then transfers caches and navigates from the placeholder to the canonical history ID (`frontend/src/pages/assistant.tsx:214-259`). The full mock explicitly resolves all conversation addresses for this reason (`frontend/src/lib/assistant/mock-data.ts:475-495`), but the interceptor spec keys overlay and continuations by the caller's conversation ID and specifies no alias migration.

**Concrete failure scenario:** The first message in a new chat is a mock GitHub flow, so its overlay and parked continuation are stored under `workflow-pending-X`. The user then sends an unmatched message; the real transport materializes `chatc-Y`, and the page navigates to it. Subsequent `getHistory(chatc-Y)` no longer finds the overlay under `workflow-pending-X`, and completing the still-visible card calls the engine with `chatc-Y`, which cannot find the continuation keyed under the placeholder. A reload before any live turn also loses the entire placeholder conversation, not just its mock transcript.

**Suggested spec fix:** Add explicit conversation alias handling: when delegated history/events reveal a different canonical ID, atomically migrate or dual-address overlay, running state, and continuations. Test `new chat -> mock blocked -> live pass-through aliases -> mock resume/delete by canonical ID`, plus reload semantics for a mock-only placeholder.

### [SEV-high] A completed real journey does not prove the user connected the service the script marks as connected

**Evidence:** The action card gives AddKeyDialog a prefilled slug (`frontend/src/components/assistant/blocks/action-card.tsx:363-390`), but the dialog explicitly allows the user to navigate back and choose another catalog entry (`frontend/src/components/dashboard/add-key-dialog.tsx:2567-2571`, `frontend/src/components/dashboard/add-key-dialog.tsx:2987-2995`). Its completion contract returns only `userServiceId` (`frontend/src/components/dashboard/add-key-dialog.tsx:2438-2440`, `frontend/src/components/dashboard/add-key-dialog.tsx:3129-3136`). The resulting report likewise contains only that ID, not the actual slug (`frontend/src/components/assistant/blocks/action-card.tsx:219-248`).

**Concrete failure scenario:** The scripted card asks for GitHub. The user presses Back in the real wizard, selects OpenAI, and completes it. The report is valid and has disposition `completed`, so the sample flow executes `.connect("api-github")`. Future GitHub scenarios skip `.need(...)` and claim GitHub is usable even though the real backend resource belongs to OpenAI.

**Suggested spec fix:** Either lock action-launched AddKeyDialog to the requested catalog/custom target, or enrich/resolve the completion resource and validate it against the card before mutating world state. Add an end-to-end test that navigates Back, completes a different service, and verifies GitHub is not marked connected.

### [SEV-med] Engine-not-ready pass-through turns a persisted mock intent into an irreversible real send

**Evidence:** `AssistantTransport.sendMessage` is synchronous and must return a handle immediately (`frontend/src/types/assistant.ts:273-282`). The hook calls it as soon as the user submits (`frontend/src/hooks/use-assistant.ts:614-644`); there is no async readiness gate in between.

**Concrete failure scenario:** Interception was persisted as enabled. After reload, the store has rehydrated but the dynamic engine/config chunk is still loading. The user submits a message matching a destructive or credential-oriented rehearsal. The specified fallback delegates it to real Aevatar, and only afterward records a miss in the popover. The user selected mock behavior but their content and intent reached the live assistant.

**Suggested spec fix:** Enabled-but-not-ready must queue behind module readiness with a cancellable placeholder handle, or fail synchronously with a dedicated retryable error; it must not delegate. Keep unmatched-ready pass-through as the only live fallback, and change the test plan to assert zero delegate calls during loading/error.

### [SEV-med] Appending the entire overlay after the real tail corrupts chronology after any later pass-through turn

**Evidence:** The hook replaces cached history wholesale with each transport response (`frontend/src/hooks/use-assistant.ts:117-131`), and the page renders `history.messages` in returned order (`frontend/src/pages/assistant.tsx:421-435`). A real send appends its optimistic user message to the delegate's own transcript (`frontend/src/lib/assistant/aevatar-transport.ts:1473-1503`).

**Concrete failure scenario:** A mock turn M is followed by an unmatched live turn R in the same conversation. The delegate now returns `[older real messages, R-user, R-assistant]`; the wrapper appends its overlay `[M-user, M-assistant]`. M jumps after R on every refetch, despite having happened first. Repeating mock/live turns groups all mock exchanges at the bottom rather than preserving conversational order.

**Suggested spec fix:** Store an ordering anchor and timestamp/sequence for each overlay message and perform a stable merge, rather than tail append. Define tie-breaking and server-materialization behavior explicitly. Add `mock -> live`, `live -> mock -> live`, and repeated identical-text tests.

### [SEV-med] Every mock event can trigger a real history request and a wire-log entry

**Evidence:** Each event schedules `projectTransportState`, whose projection calls `getHistory` (`frontend/src/hooks/use-assistant.ts:477-510`, `frontend/src/hooks/use-assistant.ts:539-573`). Aevatar serves its mirror without network only when its own active turn is running/waiting; otherwise it calls `loadHistory` (`frontend/src/lib/assistant/aevatar-transport.ts:1388-1413`). Its history GET uses the wire-log options (`frontend/src/lib/assistant/aevatar-transport.ts:177-188`, `frontend/src/lib/assistant/aevatar-transport.ts:2330-2336`). A wrapper-owned mock run never marks the delegate's mirror active.

**Concrete failure scenario:** A 20-event scripted reply at the specified 100 ms cadence causes repeated real transcript GETs while it streams. With wire capture enabled, those GETs can appear in the panel even though the spec says mock turns record nothing; under latency, many projections overlap. This is observable network traffic caused solely by the scripted turn.

**Suggested spec fix:** Snapshot delegated history at interception start and serve base-plus-overlay from the wrapper while a mock run or projection is active, then reconcile once at a defined terminal boundary. Add call-count and wire-log tests for long text/tool scripts; a mock stream should not issue per-event delegate reads.

### [SEV-med] Message-only overlay leaves conversation title, recency, and count stale

**Evidence:** The real transport changes title and `last_message_at` when it appends an optimistic user message (`frontend/src/lib/assistant/aevatar-transport.ts:1473-1508`). `listConversations` returns and sorts those stored metadata values (`frontend/src/lib/assistant/aevatar-transport.ts:1259-1275`). An intercepted send never calls the delegate's append path, and `ConversationHistory` carries conversation metadata separately from `messages` (`frontend/src/types/assistant.ts:158-174`).

**Concrete failure scenario:** The first message in a new chat is intercepted. Its transcript shows the overlay, but the sidebar entry stays titled `New chat`, retains the creation timestamp, and may sort below conversations used earlier. In an existing chat, scripted activity does not bump recency or message count, so navigation disagrees with the visible thread.

**Suggested spec fix:** Define a conversation-metadata overlay and apply it consistently in both `getHistory` and `listConversations` (title-on-first-user-message, last activity, and count semantics). Add first-turn and existing-conversation sidebar ordering tests.

### [SEV-med] Multiple cards before one `.await()` have no coherent continuation semantics

**Evidence:** One ActionCard resolution emits one `ActionReport` (`frontend/src/components/assistant/blocks/action-card.tsx:219-249`), and the hook calls `continueActions` with a single-element array (`frontend/src/hooks/use-assistant.ts:797-808`). The real transport at least groups queued reports by `originTurnId` and request ID (`frontend/src/lib/assistant/aevatar-transport.ts:1948-1982`); the proposed engine describes one continuation key and selects one disposition branch.

**Concrete failure scenario:** A scenario emits two `.action(...)` cards and then one `.await(...)`. The user completes the first card. If that report consumes the continuation, the remaining script resumes while card two is pending; clicking card two later finds no continuation. If the engine waits for both, the spec does not define aggregate branch selection for `completed + declined`, nor when patches and world mutations run.

**Suggested spec fix:** For v1, make the compiler reject any segment with anything other than exactly one resumable card at an `.await()`. Otherwise specify an aggregate barrier, per-request state, mixed-disposition precedence, wake behavior, and idempotent partial reports. Test both completion orders.

### [SEV-med] Mock approval expiry is display-only, so an expired card can still resume a script

**Evidence:** The approval card computes that `expires_at` has passed (`frontend/src/components/assistant/blocks/approval-card.tsx:194-200`), but both decision buttons are disabled only by `busy`, not by `expired` (`frontend/src/components/assistant/blocks/approval-card.tsx:247-265`). The type system has explicit `expired` and `cancelled` decisions (`frontend/src/types/assistant.ts:29`, `frontend/src/types/assistant.ts:82-94`), so transport enforcement is required.

**Concrete failure scenario:** A scripted approval parks for more than 15 minutes. The UI says `expired`, but the user can click Approve. With no expiry rule in the engine spec, the continuation is still found, gets patched approved, and runs the approved branch, unlike an actual expired server request.

**Suggested spec fix:** Define expiry as an engine lifecycle event or lazy decision-time check: patch the card to `decision: "expired"`, remove the continuation, and reject/ignore later decisions. Add fake-timer tests at just before and just after expiry, including toggle-off/on.

### [SEV-med] The production tree-shaking claim is not supported by the specified static import graph

**Evidence:** The existing page statically imports its header action (`frontend/src/pages/assistant.tsx:1-12`) and renders it at both mount points (`frontend/src/pages/assistant.tsx:482-499`). The pattern store is initialized by a top-level `create(persist(...))` call (`frontend/src/stores/assistant-wire-log-store.ts:239-361`) with localStorage operations (`frontend/src/stores/assistant-wire-log-store.ts:192-215`), which is not an obviously removable pure module. The assistant transport is also constructed as a synchronous module singleton (`frontend/src/lib/assistant/transport.ts:555-567`).

**Concrete failure scenario:** `assistant.tsx` statically imports `MockScenariosAction`, which statically imports and initializes the persisted store. A production `import.meta.env.DEV && ...` render branch folds away, but the side-effectful module remains reachable. Separately, the unconditional production wrapper must read enabled state somehow; statically importing the store contradicts the claim that only the component imports it, while not importing it leaves no specified synchronous bridge.

**Suggested spec fix:** Specify a genuine dev-only dynamic module boundary/registration path for both UI and transport, plus a production no-op path that cannot import the store. Add a production-build artifact assertion for mock component/store/engine/config symbols and chunks; `npm run build` succeeding is not a footprint test.

### [SEV-med] Delete ordering is underspecified, so mock timers may write through an in-flight deletion

**Evidence:** The real transport cancels its owned run before awaiting the delete fence/request (`frontend/src/lib/assistant/aevatar-transport.ts:1302-1337`). The hook removes caches only after `deleteConversation` succeeds (`frontend/src/hooks/use-assistant.ts:368-386`). Because the delegate does not own engine timers, its cancellation path cannot stop the scripted stream.

**Concrete failure scenario:** The user deletes a conversation halfway through a paced mock script. An implementer follows "always delegate" literally and awaits the real DELETE before dropping engine state. During that request, timers keep firing, event pumps keep projecting, and continuations can be registered against a conversation being tombstoned. A slow or failed DELETE leaves a partially advanced script in an undefined state.

**Suggested spec fix:** Require the wrapper to reserve/tombstone the conversation and synchronously cancel/suppress engine delivery before invoking the delegate, matching the real transport's cancel-first order. Define the state after delete failure. Add a delayed-delete test that advances timers while the request is in flight.

### [SEV-low] Persisted world state is neither account-scoped nor cross-tab coherent

**Evidence:** The assistant page has an authenticated user identity available (`frontend/src/pages/assistant.tsx:76-83`), but the proposed persistence key/state has no owner identity. The store pattern uses one origin-global localStorage key (`frontend/src/stores/assistant-wire-log-store.ts:18`, `frontend/src/stores/assistant-wire-log-store.ts:334-359`) and contains no `storage` event reconciliation; the proposed store copies that pattern.

**Concrete failure scenario:** User A marks GitHub connected, signs out, and User B signs in on the same dev origin; B inherits A's mock world and skips the connect flow. With two open tabs, both hydrate separate in-memory copies; tab A connects GitHub, tab B remains cold and later persistence can overwrite A's newer world. The claimed global-across-conversations world is therefore neither safely global nor predictably local.

**Suggested spec fix:** Namespace/reset persisted state by authenticated user ID and define tab semantics. Either synchronize via `storage`/BroadcastChannel with a revision, or make world explicitly tab/session scoped. Add account-switch and two-tab storage-event tests.

## Verdict

The feature is implementable in principle, but not safely implementable as written. The three changes that matter most are: (1) make delivered cursors conversation-monotonic, or explicitly isolate per-turn reducers; (2) replace toggle-based method routing with an ownership/reservation state machine that survives toggle-off, blocked turns, Stop, and delete; and (3) bind a completed real connect journey to the requested service before mutating world state. After those, the append-only history strategy should be replaced with a chronological, metadata-aware projection that does not refetch the real transcript on every mock event.
