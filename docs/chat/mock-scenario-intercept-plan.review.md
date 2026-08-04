# Adversarial Review: Mock Scenario Interception Implementation Plan

The work-package decomposition is directionally usable, but several package boundaries cannot implement the v2 contract as written. The most serious failures are in the real-journey verification path, the declared file scope, and the proposed build-script replacement.

## Findings

### [SEV-high] WP4 verifies successful connections through a GET route that does not exist

**Evidence:** WP4 mandates `apiClient` GET `user-services/{id}` (`docs/chat/mock-scenario-intercept-plan.md:81`), following the spec's `GET /api/v1/user-services/{id}` requirement (`docs/chat/mock-scenario-intercept-spec.md:342-357`). The actual user-services router has GET only on the collection; `/{service_id}` supports PUT and DELETE, not GET (`backend/src/routes.rs:1002-1015`). The existing detail endpoint is `GET /api/v1/keys/{key_id}` (`backend/src/routes.rs:963-973`, `backend/src/handlers/keys.rs:1024-1044`), and its frontend shape exposes the required `catalog_service_slug` (`frontend/src/types/keys.ts:7-29`; existing usage at `frontend/src/hooks/use-keys.ts:27-33`).

**Concrete failure scenario:** The user completes the real GitHub wizard and reports a valid `userServiceId`. The planned GET receives 405/route failure, so every genuine success takes the spec's "lookup fails" branch: the card completes as unverified but `.connect("api-github")` never mutates world. The next GitHub-issues prompt asks the user to connect again. A stubbed WP4 test can pass because it invents a response for a route the application cannot call.

**Suggested plan fix:** Change WP4 to resolve the report with `GET /keys/{encodeURIComponent(userServiceId)}` and compare `KeyInfo.catalog_service_slug`. Make the verification test use the real endpoint and response shape, including match, mismatch, 404, and malformed-response cases.

### [SEV-high] The asynchronous verification lookup has no valid ownership contract across synchronous `continueActions`

**Evidence:** WP2 expects a verification outcome "passed in by the interceptor" (`docs/chat/mock-scenario-intercept-plan.md:58-60`), while WP4 owns the network GET (`docs/chat/mock-scenario-intercept-plan.md:81`). That GET is asynchronous, but `AssistantTransport.continueActions` must synchronously return `TurnHandle | null` (`frontend/src/types/assistant.ts:314-319`). The hook immediately disowns its event pump when it receives neither a handle nor a synchronous event (`frontend/src/hooks/use-assistant.ts:797-814`), and a disowned pump ignores later events (`frontend/src/hooks/use-assistant.ts:539-545`). The plan defines no verifying state, provisional handle, abort signal, or cancellation behavior for this interval.

**Concrete failure scenario:** `continueActions(completed)` starts the service lookup and returns `null` because no continuation exists until verification resolves. No card event has been emitted yet, so the hook disowns the pump. The GET then confirms GitHub and the engine emits the card patch and resumed turn, but the hook drops every event; alternatively, returning an uncancellable ad hoc handle makes Stop/delete unable to abort the lookup and its later continuation.

**Suggested plan fix:** Specify that WP4 synchronously enters a mock-owned `verifying`/running state and returns a real provisional `TurnHandle`; its cancel path aborts the GET and suppresses all later delivery. Add a hook-level test with a deferred lookup covering resolution, Stop, delete, rejection, and the handle registry.

### [SEV-high] The declared file-scope rules make WP7 and WP8 impossible to execute compliantly

**Evidence:** The ground rules allow only spec section 13 files plus `docs/chat/README.md` and explicitly freeze `package.json` (`docs/chat/mock-scenario-intercept-plan.md:19-23`). WP7 nevertheless adds `frontend/scripts/assert-mock-footprint.mjs`, edits `frontend/package.json`, and needs a use-assistant integration test (`docs/chat/mock-scenario-intercept-plan.md:106-114`); WP8 edits the spec status (`docs/chat/mock-scenario-intercept-plan.md:116-119`). None of the script, package file, spec edit, or an explicitly named integration-test path appears in the section 13 table (`docs/chat/mock-scenario-intercept-spec.md:556-567`). The final diff gate still permits only the section 13 list plus README/plan (`docs/chat/mock-scenario-intercept-plan.md:123-133`).

**Concrete failure scenario:** A scope-compliant implementer reaches WP7 and must report a blocker instead of installing the footprint gate. An implementer who follows WP7 and WP8 instead fails the final diff-scope gate with exactly the files those packages require. There is no interpretation under which all instructions are simultaneously satisfied.

**Suggested plan fix:** Expand the authoritative file list before execution to name the footprint script, `frontend/package.json`, the exact integration-test file, README, plan, and spec status edit. Replace "package.json frozen" with "dependencies and lockfile frozen; the named scripts-only edit is allowed."

### [SEV-high] WP7's replacement build command silently removes the credential-accept production build

**Evidence:** The current build is `tsc -b && vite build && vite build --config vite.credential-accept.config.ts` (`frontend/package.json:11`). WP7 replaces it with `tsc -b && vite build && node scripts/assert-mock-footprint.mjs` (`docs/chat/mock-scenario-intercept-plan.md:109-113`). The omitted stage produces `dist/credential-accept` and its release-integrity output (`frontend/vite.credential-accept.config.ts:14-27`). Frontend CI invokes the generic build script (`.github/workflows/ci.yml:291-309`). The CLI wizard is not the problem: it has a separate `build:wizard` script (`frontend/package.json:13`) and separate Vite output (`frontend/vite.wizard.config.ts:9-35`).

**Concrete failure scenario:** After WP7, `npm run build` exits successfully and the footprint assertion passes, but the build no longer emits the credential-accept application that the existing script contract produced. Local/CI green status therefore masks a missing production artifact.

**Suggested plan fix:** Append the assertion to the complete existing command: `tsc -b && vite build && vite build --config vite.credential-accept.config.ts && node scripts/assert-mock-footprint.mjs`. Add a gate that also asserts the credential-accept output/integrity manifest still exists.

### [SEV-high] WP5 has no test that exercises the transport shell or dev installation boundary

**Evidence:** WP5 changes the exported singleton and dynamic installation path but assigns only the unchanged `transport.test.ts` as proof (`docs/chat/mock-scenario-intercept-plan.md:88-95`). That test runs in `MODE === "test"`, selects the full mock (`frontend/src/lib/assistant/transport.test.ts:10-42`), and explicitly asserts that Aevatar is never instantiated (`frontend/src/lib/assistant/transport.test.ts:45-58`). WP4 tests instantiate the interceptor against a stub; they do not prove the exported plain-dev singleton ever installs it.

**Concrete failure scenario:** The shell delegates correctly, but the dynamic import resolves without calling its installer (or installs into a discarded shell instance). Every WP1-WP4 test and the unchanged transport test pass, `npm run build` passes, and the UI can show an enabled toggle, yet actual assistant sends continue going to Aevatar. If WP7 mounts a directly constructed interceptor, even the final integration test misses the broken singleton.

**Suggested plan fix:** Add a WP5 shell/boot test with an injectable loader or installer seam. Assert bare delegation before install, in-place interception after install, import failure behavior, idempotent install, full-mock non-installation, and production no-import; require WP7 to use the exported singleton.

### [SEV-high] `ensureUser` is planned as a one-time boot call, so F14 is not enforced across an SPA account change

**Evidence:** WP5 says to call `store.ensureUser(...)` "once auth state is known" (`docs/chat/mock-scenario-intercept-plan.md:88-93`), while WP1 tests only the store operation in isolation (`docs/chat/mock-scenario-intercept-plan.md:35-42`). Auth starts with `user: null` and resolves asynchronously (`frontend/src/stores/auth-store.ts:44-49`, `frontend/src/stores/auth-store.ts:101-122`); logout and `setUser` can change identity without unloading the module singleton (`frontend/src/stores/auth-store.ts:84-99`, `frontend/src/stores/auth-store.ts:125-130`). Existing assistant cleanup does not know about the new store (`frontend/src/stores/auth-store.ts:14-18`).

**Concrete failure scenario:** User A enables interception and connects GitHub in mock world, logs out, and User B logs in without a hard reload. The interceptor and store modules remain alive; the one-time boot call is not repeated, so B inherits A's in-memory `world.connected` even if persistence itself was correctly user-scoped. WP1's direct `ensureUser(B)` test passes while the real lifecycle never makes that call.

**Suggested plan fix:** Assign an owner for an auth-store subscription and cleanup, and require `ensureUser` on every resolved non-null user transition (with an explicit logout policy). Add a WP5 integration test for initial loading/null, User A, logout, and User B in one module lifetime.

### [SEV-med] The F8 gate ignores `listConversations`, which can still make mock activity issue and wire-log a real request

**Evidence:** Every event projection reads both history and the conversation list (`frontend/src/hooks/use-assistant.ts:117-145`, `frontend/src/hooks/use-assistant.ts:477-510`). The spec/plan caches delegated history, but `listConversations` is still defined as delegate-then-overlay (`docs/chat/mock-scenario-intercept-spec.md:287-305`; WP4 at `docs/chat/mock-scenario-intercept-plan.md:78-85`). The real delegate performs a list GET whenever its five-second TTL is stale and it has no delegate-owned run (`frontend/src/lib/assistant/aevatar-transport.ts:214`, `frontend/src/lib/assistant/aevatar-transport.ts:1244-1253`); a wrapper-owned mock run does not populate that running map. The GET uses wire-log options (`frontend/src/lib/assistant/aevatar-transport.ts:73-90`, `frontend/src/lib/assistant/aevatar-transport.ts:177-188`).

**Concrete failure scenario:** Start a mock turn after the conversation-list TTL has expired, or use a scripted wait longer than five seconds. The hook's projection calls wrapper `listConversations`, which calls the real delegate and records a Chat History index GET in the wire log solely because of mock activity. WP4's stub delegate and two-second 20-event test can still report zero delegated history calls and an untouched fake wire-log store.

**Suggested plan fix:** Define a list snapshot/metadata projection while a conversation is mock-owned, not only a history snapshot. Test through the real Aevatar delegate with mocked HTTP, a stale TTL, and a greater-than-five-second script; assert zero history and list requests caused by mock events.

### [SEV-med] The nested lazy header action has no local Suspense boundary and can blank the whole assistant page

**Evidence:** WP6 requires a dev-gated `lazy` component in `assistant.tsx` but does not require `Suspense` (`docs/chat/mock-scenario-intercept-plan.md:97-104`). `AssistantPage` is already lazy (`frontend/src/pages/lazy.ts:29-31`). The nearest boundaries wrap the entire route outlet (`frontend/src/router.tsx:96-103`, `frontend/src/components/layout/dashboard-layout.tsx:121-129`), and the two header mount points sit inside that page (`frontend/src/pages/assistant.tsx:482-499`).

**Concrete failure scenario:** The assistant page chunk has rendered, then the nested mock-action chunk is still loading or is re-fetched after HMR. Rendering the header action suspends to the route-level boundary, whose fallback is empty, so the entire assistant workspace disappears until the small dev-only control resolves. A component test with an already-resolved mocked import will not reveal the page-level suspension.

**Suggested plan fix:** Require both mount points to use one locally defined `<Suspense fallback={null}>`-wrapped action node, or use an explicit async-loaded component state. Add a deferred-import render test that proves the existing assistant shell remains visible.

### [SEV-med] WP6 omits the required warning that scripted action cards perform real backend mutations

**Evidence:** The plan's UI package lists session-only/pass-through copy, rows, world chips, and a file pointer (`docs/chat/mock-scenario-intercept-plan.md:97-104`), but not the spec's safety disclosure that Connect launches the actual journey and can create real keys/connections (`docs/chat/mock-scenario-intercept-spec.md:496-503`). Neither WP6 nor the component-test assignment includes an assertion for that warning (`docs/chat/mock-scenario-intercept-spec.md:541-542`).

**Concrete failure scenario:** A developer sees "Mock scenarios" and "Connected (mock)", reasonably treats the popover as an offline simulator, and clicks Connect. The real AddKeyDialog writes credentials/connections to the real account, but the planned UI gave no notice before that side effect.

**Suggested plan fix:** Add explicit visible copy to WP6 stating that action cards open real journeys and may create real connections, and assert it in `mock-scenarios-action.test.tsx`.

### [SEV-med] The delete gate never exercises the spec's distinct tombstone-on-failure rule

**Evidence:** The spec requires the wrapper to remain tombstoned even when delegated DELETE fails (`docs/chat/mock-scenario-intercept-spec.md:287-290`). WP4 tests only a delayed DELETE while timers advance (`docs/chat/mock-scenario-intercept-plan.md:82-85`; detailed assignment at `docs/chat/mock-scenario-intercept-spec.md:535-537`). The real transport deliberately does the opposite after failure: local removal happens only on success and its deletion reservation is cleared in `finally` (`frontend/src/lib/assistant/aevatar-transport.ts:1358-1383`), making it a dangerous implementation pattern to copy incompletely.

**Concrete failure scenario:** A running script is deleted; the delegate DELETE rejects. An implementer mirrors Aevatar and clears the wrapper tombstone in `finally`. A queued timer or retained continuation can then write into or resume the conversation after the failed delete, violating F13, while the delayed-success test still passes.

**Suggested plan fix:** Add a rejected-DELETE test that advances timers after rejection and attempts send, resume, card patch, history, and list operations. State the exact post-failure behavior expected for the surviving real conversation and its discarded mock overlay.

### [SEV-med] The WP6 tests do not cover the error, no-match, or per-scenario control states that make interception diagnosable

**Evidence:** Section 8.2 requires an inline engine-load error, per-scenario switches, matched relative activity, an unmatched-message line, chip removal/reset, and an empty-world state (`docs/chat/mock-scenario-intercept-spec.md:421-439`). The assigned component test covers only gated rendering, toggle/loading, rows, chips, and an accessibility label (`docs/chat/mock-scenario-intercept-spec.md:541-542`), and WP6 adds no stronger test list (`docs/chat/mock-scenario-intercept-plan.md:97-104`).

**Concrete failure scenario:** The config import fails and the master remains enabled, but the component renders neither the error nor a recovery-relevant state; or a row switch changes visually without updating `disabledScenarioIds`, so the supposedly disabled regex still intercepts. WP4 loading/error and match tests pass because they mutate the store directly, never exercising the popover wiring.

**Suggested plan fix:** Assign explicit WP6 tests for load error, matched and unmatched activity, per-row enable/disable behavior, chip removal/reset, empty world, and both header mount points.

### [SEV-low] F9's accepted message-count requirement has no unambiguous implementation or test

**Evidence:** The v2 disposition log says F9 was accepted for "title/recency/count metadata" (`docs/chat/mock-scenario-intercept-spec.md:612`), but section 6.4 defines only `last_message_at` and claimed-chat title (`docs/chat/mock-scenario-intercept-spec.md:312-316`). WP4's metadata test likewise names only title, recency, and ordering (`docs/chat/mock-scenario-intercept-plan.md:78-85`; `docs/chat/mock-scenario-intercept-spec.md:531-534`). The actual contract has optional `Conversation.message_count` (`frontend/src/types/assistant.ts:158-168`), and the real list preserves the server materialized count (`frontend/src/lib/assistant/aevatar-transport.ts:2305-2315`).

**Concrete failure scenario:** A real conversation reports `message_count: 4`, then receives one intercepted user/assistant exchange. History visibly contains six messages while `getHistory().conversation` and `listConversations()` still report four. One implementer may add two, another may count only assistant turns, and a third may preserve the server value; all satisfy the plan's named test.

**Suggested plan fix:** Resolve the spec inconsistency and define count semantics for known and unknown base counts, claimed chats, card-only turns, and anchored overlays. Add identical assertions to both history and list projections.

## Verdict

The plan is not executable by a fresh implementer without further clarification. The three changes that matter most are: (1) replace the nonexistent verification endpoint and define a synchronous, cancellable handle around its asynchronous lookup; (2) reconcile the authoritative file list with WP7/WP8 and preserve the existing credential-accept build while appending the footprint assertion; and (3) make WP5 an independently tested lifecycle package, including exported-singleton installation and auth-user rescoping. After those blockers, the list snapshot, local Suspense boundary, delete-failure gate, and UI safety/state tests are needed for the plan's F1-F14 claims to be credible.
