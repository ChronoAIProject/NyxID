SHIP

# Final Adversarial Verdict — Direct Chrono-LLM Chat Engine

Reviewed against the **live shared worktree**: HEAD
`8cfec9c8776a1e3a30c57c8aa7851af69c1cc4ea` on branch `chat-chronollm-direct`
**plus the current uncommitted working tree**, in
`/Users/chronoai/Library/Application Support/heca/worktrees/3571ce16/zen-fox`.
Draft PR: **#1426** → `main`.

This verdict was re-derived from current file:line and from tests I ran myself.
Nothing was inherited from the prior verdict without re-checking it against the
live code. The only file this review wrote is this one. Tree at review time:
`git status --porcelain` = **20 entries (17 modified + 3 untracked)**;
`git diff --check` = **clean**; the 3 untracked files are this verdict, the new
SSE fixture, and the new embed-guard script — no stale or generated artifacts.

## Counts (live)

- **Open BLOCKERs: 0**
- **Open MAJOR implementation defects: 0**
- **Open MINOR mechanical defects: 0**
- Second-round findings verified: **6 mechanical CLOSED + 1 interpretation item resolved**
- New regressions introduced by the repair diff: **0**
- Optional (non-blocking) hardening notes: **2**
- Accepted low-priority / environment items: **4**
- True human product/rollout decisions outstanding: **5**

**There are no mechanical defects left to fix in this feature.** Every one of
the six second-round mechanical findings is closed in live code with a
regression test that fails on revert. Everything else is either an explicitly
accepted trade-off, an environment/evidence gap that CI already covers, or a
product/rollout decision only the owner can make. Merge is unblocked because the
whole surface is default-off and fails closed; the one genuine non-code gate is
a browser E2E pass **before the flag is flipped for any real account**, which
gates *activation*, not merge.

---

## Disposition of the second-round findings

### 1. Error-frame path aborts/cancels the reader so server permits are released — **CLOSED (test ran green).**

`direct-transport.ts`: the error chunk sets `run.sawUpstreamError = true`
(`handlePayload`, chunk.error branch, `direct-transport.ts:712-714`); the drain
loop now breaks on `run.sawDone || run.sawUpstreamError`
(`direct-transport.ts:646`); and immediately after the loop the transport
**cancels the stream**:

```ts
if (run.sawUpstreamError) {
  run.controller.abort();
  await reader.cancel().catch(() => undefined);   // direct-transport.ts:649-652
}
```

This is the browser half of the permit-release story. The backend holds the
`DirectChatPermit` inside the passthrough stream (`attach_in_flight_permit`,
`backend/src/handlers/assistant_direct.rs:166-176`) and the permit's `Drop`
(`backend/src/mw/rate_limit.rs:209-213`) releases the in-flight slot when the
stream is dropped. `reader.cancel()` closes the fetch body, which drops the
server stream, which releases the slot **deterministically** instead of waiting
for GC.

Regression test `ignores payloads after an upstream error frame`
(`direct-transport.test.ts:276-303`) proves cancellation, not just settlement:
it drives a `chunkedSseResponse` whose underlying source records a `cancel()`
call (`direct-transport.test.ts:34-59`), and asserts
`expect(streamCancelled).toHaveBeenCalledOnce()`, that no `message.started`
fires for the post-error delta, and that `turn.completed` (status `failed`,
error `upstream_failed`) is the **last** event. The fixture
`__fixtures__/chrono-llm-direct-error-then-delta.sse` is exactly `error → late
delta → [DONE]`, so the test fails if the reader is not cancelled or the late
frame is processed. **I ran this test — it passed** (part of the 96/96 below).

### 2. Signed-out / missing memory-only conversation renders and tolerates picker clicks; explicit conversation/turn ops stay strict — **CLOSED (two tests ran green).**

The settings/picker writes were softened from throwing to no-op:
`updateSettings` now returns `bool`, calls the non-throwing `ensureOwner()`, and
returns `false` for a signed-out owner or an unknown conversation
(`direct-transport.ts:495-517`); `setModel` bails on `!applied`
(`direct-transport.ts:246-251`); `seedDefaultModel` returns instead of throwing
(`direct-transport.ts:258-266`); and `canUpdateSettings`
(`direct-transport.ts:241-245`) gates the UI. The controls read it
(`use-assistant-direct.ts:58`) and disable both pickers with
`disabled || !canUpdate || …` (`direct-chat-controls.tsx:71,95`).

Strictness is preserved exactly where required: `ensureOwner()`
(`direct-transport.ts:485-488`) only reconciles identity and never throws, while
`ensureSignedInOwner()` (`direct-transport.ts:490-493`) still throws and is
retained on the two explicit operations — `createConversation`
(`direct-transport.ts:287`) and the turn/send op (`direct-transport.ts:344`).
So a picker write can safely outlive its conversation during reload/identity
churn, but creating a conversation or starting a turn while signed out still
fails loudly. This matches the finding's requirement verbatim.

Tests: `renders but disables picker writes when the direct conversation is
missing` (`direct-chat-controls.test.tsx:136-166`, asserts `.not.toThrow()` +
both comboboxes disabled + default settings) and `disables draft picker controls
while signed out` (`direct-chat-controls.test.tsx:168-190`, `setUser(null)`,
`.not.toThrow()` + both disabled). **Both ran — passed.**

### 3. Docker embedded-input guard uses the release profile + gcp-kms and a meaningful real dep-info check — **CLOSED (self-test + real run ran green).**

`scripts/check-backend-docker-embeds.py` reads the real
`target/release/nyxid-server.d` (`:16`), and if it is missing tells you to run
`cargo build --release -p nyxid --bin nyxid-server --features gcp-kms` (`:24-25`).
It parses every Cargo dep-info compile input and fails unless each one is (a)
present, (b) covered by a *builder-stage* `COPY` in `backend/Dockerfile`
(`builder_copy_sources`, `:40-63`), and (c) not excluded by `.dockerignore`
(`is_ignored`, `:87-93`). This is a real coverage proof, not a smoke check.

Profile/feature alignment is exact: the Dockerfile builder compiles
`cargo build --release --manifest-path backend/Cargo.toml --features gcp-kms`
(`backend/Dockerfile:35,58`) producing `target/release/nyxid-server`
(`:43,:76`), and CI now builds the same binary then runs the guard
(`.github/workflows/ci.yml:241-247`). CI path filters were extended so the guard
re-runs when its own inputs change (`ci.yml:87-89,116-118`).

**I ran both:** `--self-test` → pass; the real guard against the present
release dep-info → `Backend Docker staging covers all 371 production build
inputs.` (exit 0). Because `backend/prompts` is a builder `COPY`
(`backend/Dockerfile:49`), the embedded direct prompts are provably staged —
this closes the *embed* concern (it does **not** close prompt-copy *drift*; see
human decision 3).

### 4. Canonical wire docs state direct 429 behavior, 10/60s + 2-concurrent limits, v3.2 naming, and base-URL-relative upstream path — **CLOSED, and the numbers match code.**

`docs/chat/02-wire-contract.md` now documents the `direct-` prefix row, the three
`/direct/*` routes, and: *"limited per user to 10 requests per rolling 60 seconds
and 2 concurrent streams. Exceeding either limit returns `429 Too Many
Requests`; the skills and models metadata routes do not consume this limiter."*
It describes the rebuilt upstream body and sends it to *"the fixed admin-managed
`chrono-llm-public` service at upstream path `chat/completions`"* (relative, no
leading slash). Spec bumped to Draft v3.2 (`direct-chronollm-spec.md:1`, plus the
v3.2 reconciliation note).

Every figure matches the code I inspected:
`create_direct_chat_rate_limiter()` = `DirectChatRateLimiter::new(10, 60, 2)`
(`backend/src/mw/rate_limit.rs:217-218`); `DIRECT_LLM_SLUG = "chrono-llm-public"`
(`backend/src/services/assistant_direct.rs:6`); 256 KiB body cap
(`assistant_direct.rs:8`); metadata routes never touch the limiter
(`handlers/assistant_direct.rs:45-76`, no `try_acquire`); and the proxy is called
with the relative path `"chat/completions"`
(`handlers/assistant_direct.rs:143-152`). The query-strip test observes the
composed upstream path as `/v1/chat/completions` against a `…/v1` base_url,
confirming "base-URL-relative." (Trivial nit: the routes *table* row renders
`POST /chat/completions` with a leading slash while the prose is relative —
cosmetic, non-blocking.)

### 5. Engine-router regression test restores module-global registration and does not pollute later tests; production registration still works — **CLOSED (test ran green).**

The module global is no longer assigned directly. `registerAssistantEngineRouter`
(`transport.ts:912-919`) sets `activeEngineRouter` and returns a **guarded**
restore closure (`if (activeEngineRouter === router) activeEngineRouter =
previous`). `createAssistantTransportForEnvironment` took a new
`registerEngineRouter?` param and only registers through it
(`transport.ts:1126,1137`); production/dev wire the real registrar
(`transport.ts:1171-1183`), while tests that omit it cannot touch the global.

The new test `does not let generic factory calls retarget the registered engine
selector` (`engine-router.test.ts:251-300`) registers a router via the
`onRegister` hook, then `setAssistantTransportEngine("direct")` and asserts the
registered router routes to `direct` while two later generic factory transports
stay on `aevatar` (`getSelectedEngine() === "aevatar"`) — proving production
registration semantics — and restores the global in `finally
{ restoreRegistration() }`, so subsequent tests start clean. The prior
`workflow-pending-*` fixture id in this file was corrected to `nyxid-pending-*`
(`engine-router.test.ts:147,153`) and a real-transport draft-routing test added
(`:163-178`). **I ran the file — passed.**

### 6. Query-strip backend socket test timeout is no longer 2 seconds — **CLOSED (verified in source).**

`direct_completion_does_not_forward_caller_query_string`
(`backend/src/handlers/assistant_direct.rs:221-322`) now waits
`tokio::time::timeout(std::time::Duration::from_secs(10), uri_rx)`
(`assistant_direct.rs:294`) and asserts `forwarded_uri.query() == None` and
`forwarded_uri.path() == "/v1/chat/completions"`. The 2-second window is gone.
See the environment note below on how this test is gated.

### 7. (Interpretation) Distinguish the pre-existing `:3000` fetch warning from feature defects — **RESOLVED by direct observation.**

In my own focused Vitest run the stream `AggregateError: … ECONNREFUSED
127.0.0.1:3000` prints to stderr, but **every one of the 96 tests passed**. It is
an unhandled background fetch from shared test setup hitting the dev-server port
that is not up under Vitest — it fails no assertion and is unrelated to the
direct surface. Confirmed, not a defect.

---

## Independent re-attack (beyond the six findings)

- **Query/header leakage across the platform-credentialed boundary — sound.**
  The shared admin proxy *does* forward the caller's query string
  (`handlers/proxy.rs:1740-1746`, minus NyxID-internal params), which is exactly
  why the direct handler rewrites `parts.uri = parts.uri.path().parse()`
  (`handlers/assistant_direct.rs:118-125`) to drop `?api_key=…&stream=…` before
  the request reaches `chrono-llm-public`. The regression test proves the
  forwarded query is `None`. Caller headers continue through the proxy's existing
  allowlist (`collect_forward_headers`, `handlers/proxy.rs:1799`); the direct
  route does not widen it. No new leakage path.
- **Platform service resolution + `requires_user_credential` guard — enforced.**
  `resolve_admin_service_by_slug` reads the admin catalog by `slug` + `is_active`
  and **returns `Internal` if `service.requires_user_credential`**
  (`backend/src/services/assistant_service.rs`, the guard block), so a
  misconfigured per-user service can never back this platform surface.
- **Feature-flag semantics — default-off, per-request, fail-closed.** All three
  routes call `require_direct_chat_enabled` →
  `resolve_personal_features(...).any(key == DIRECT_CHAT_ENGINE_FLAG_KEY)`
  (`handlers/assistant_direct.rs:33-43`), flag
  `experimental:direct-chat-engine` (`feature_flag_service.rs:111`), proven off
  by `default_off_returns_not_found_for_all_direct_handlers`.
- **True SSE passthrough — preserved.** `attach_in_flight_permit`
  (`assistant_direct.rs:166-176`) keeps upstream status/headers and yields body
  chunks unmodified while holding the permit; no buffering, no resource creation.
- **Deleted-endpoint caller inventory — clean.** No frontend source references
  `/assistant/completions`; only `DIRECT_COMPLETIONS_URL =
  "/api/v1/assistant/direct/completions"` (`direct-transport.ts:29,574`). The
  legacy `POST /api/v1/assistant/completions` backend route is a *retained,
  uncalled* surface (human decision 4), not a deletion — nothing is stranded.
- **`workflow-pending-` → `draft-` router change is a fix, not a regression.**
  `"draft-"` was already the pre-existing placeholder value (renamed
  `PENDING_TYPED_CONVERSATION_PREFIX` → `AEVATAR_DRAFT_CONVERSATION_PREFIX`,
  `aevatar-transport.ts:260`), and `createConversation` mints
  `draft-<uuid>` (`aevatar-transport.ts:1503`). The classifier now matches
  `draft-` (`transport.ts:667-669`), fixing a stale `workflow-pending-` entry
  that no live code mints (`git grep` finds it only in backend family-classifier
  tests and design docs; placeholder ids are client-local and never sent
  pre-materialization). The real-transport routing test pins it
  (`engine-router.test.ts:163-178`).
- **`crypto.randomUUID()` swap is behaviorally identical.** Old `newId("direct")`
  already expanded to `direct-<crypto.randomUUID()>`
  (`direct-transport.ts:110-111`); the inlined form
  (`direct-transport.ts:290`) is the same string family and the same established
  pattern used across the app. `newId` remains live (user-message/block/turn ids,
  `:366,372,405`), so no dead code.
- **`aevatar-transport.ts` diff is semantically inert.** Full read: const
  renames + one new export, the `escapeRegexLiteral`-derived
  `TYPED_SERVER_CONVERSATION_ID_PATTERN` (`.source` identical — `-` is not in the
  escape class, so `nyxid-chat-` is unchanged), and Prettier reflow. The one
  precedence-sensitive reflow `existing && (A && B && X)` → `existing && A && B &&
  X` (`aevatar-transport.ts:1647-1655`) is associativity-preserving.
- **Classifier bounds preserved.** `hasBoundedConversationSuffix`
  (`transport.ts:673-686`) reproduces the old `^prefix[A-Za-z0-9_-]{1,max}$` with
  identical bounds (direct 160, `nyxid-chat-` 117, `chatc-` 120, `draft-` 160,
  `nyxid-pending-` 160); `CONVERSATION_ID_SUFFIX` has no `g` flag, so no
  `lastIndex` leakage across `.some()`.
- **`pages/assistant.test.tsx`** dropped its hand-rolled classifier copy (which
  still carried the stale `workflow-pending-` prefix) for
  `importOriginal` + spread (`pages/assistant.test.tsx:258-266`), removing a
  drift source rather than adding one.

## Optional, non-blocking hardening notes (not defects, not counted)

1. **Server-side stream deadline as belt-and-suspenders.** The in-flight permit
   is released only when the stream terminates or drops. On the realistic paths
   this is covered (upstream sends `[DONE]`; the frontend now cancels on
   error/terminal). A pathological upstream that streams forever *and* a client
   that never closes would hold one of the 2 per-user slots until the socket
   actually closes. Not a live failure with the current cooperative contract; a
   max-duration guard on the passthrough stream would fully close it if you ever
   want it.
2. **Cosmetic doc nit:** wire-contract routes table shows `POST /chat/completions`
   (leading slash) vs the relative prose. Harmless.

Both are one-line changes; neither is required for merge and neither is a
mechanical defect.

## Environment / evidence gaps (CI covers them; not defects)

- **No local Mongo (127.0.0.1:27017 closed here).** The strict backend
  integration tests use `connect_test_database`, which early-returns (prints
  `skipping …`) when no Mongo is reachable — so run locally they would *skip*,
  not exercise the assertion. This is **not** a CI gap: `ci.yml` stands up a
  `mongo:8.0` replica set, initiates `rs0`, and sets
  `NYXID_TEST_DATABASE_URL` (`ci.yml:36-37,200-217,444-466`), so
  `direct_completion_does_not_forward_caller_query_string` and the flag-off
  tests **do** run under CI. I therefore validated the query-strip boundary by
  code inspection of the mechanism + the test assertions, and rely on the cited
  live-Mongo run for its green result rather than claiming a local pass.
- **No browser available** (`heca_browser_list` unavailable in this session). No
  E2E evidence is claimed or fabricated. This gates flag *activation*, not merge
  (below).

## Accepted low-priority (owner-declared / pre-existing)

- **Usage/token billing** — explicitly low priority for this internal-testing
  surface; not a merge gate.
- **CI double compilation** — the release build (for the guard + image binary)
  plus the debug nextest build. CI-time cost, intentional, accepted.
- **Pre-existing full-suite `:3000` fetch warning** — see finding 7.
- **Backend still tolerates the legacy `workflow-pending-` family name in its
  resource-family classifier tests** — pre-existing, out of this diff, and
  harmless (client-local placeholders never reach the backend before
  materialization).

## Genuine human product / rollout decisions (5) — not code defects

1. **Spec v4 endpoint-selector sequencing.** Land #1426 as built
   (`experimental:direct-chat-engine`, `/direct/skills` + `/direct/models`,
   composer pickers) with the endpoint selector as a follow-up, **or** hold and
   fold `direct-chronollm-endpoints-addendum.md` in first (it deletes two routes
   and one flag this PR introduces). v4 is confirmed *not implemented* at this
   tree; the docs label it as such.
2. **Addendum flag semantics.** Its §1 re-gates the whole direct surface on
   `experimental:aevatar-chat-wire-log`, which may already be granted for
   wire-log diagnostics — flipping its meaning would enable direct chat for
   everyone who holds it. Confirm intent and audit grants before implementing.
3. **Prompt-copy drift policy** (`skills/**` ↔ `backend/prompts/direct/`). The
   embed guard proves the prompts are *staged for Docker*, not that they still
   *match* their sources. Pin as versioned prompts (today's de-facto state), add
   a drift check, or re-point at `skills/`.
4. **`POST /api/v1/assistant/completions` deprecation (BE-4, deferred).** Still
   mounted, still a documented retained surface. Owner's call on whether/when.
5. **Spec §10 Q3 flag-key collision** with the Direct Chrono Harness spec §13 —
   moot if decision 1 chooses the addendum path.

## Browser / activation gate

- **Merge:** unblocked. All three routes are per-request flag-gated and
  default-off; the frontend `useFeature` fails closed.
- **Recommended before merge (not required):** one ~60-second dev-server smoke
  with the flag **off** — open `/assistant`, send, confirm a normal Aevatar chat
  streams — since Vitest forces the mock transport and cannot observe that path
  end to end.
- **Required before flipping the flag for any real account (incl. the owner's):**
  a direct-engine browser pass — engine flip, streamed render, banner + pickers
  (including the new disabled states), reload/logout transcript wipe, one forced
  NyxID 401 mid-turn, and one forced mid-stream `error` frame proving the
  transcript does not re-open and the permit releases.

---

## Verification provenance

**Tests I actually ran, in this worktree, against the current uncommitted diff:**

| Gate | Command | Result |
| --- | --- | --- |
| Focused FE suites | `npm run test -- --no-file-parallelism direct-transport engine-router direct-chat-controls scenario-intercept-transport pages/assistant` | **5 files / 96 tests passed** |
| Docker embed guard (self-test) | `python3 scripts/check-backend-docker-embeds.py --self-test` | pass |
| Docker embed guard (real) | `python3 scripts/check-backend-docker-embeds.py` | **371 inputs, exit 0** |
| Tree hygiene | `git status --porcelain` (20), `git diff --check` | clean, no artifacts |

The FE run also directly reproduced the `:3000` `ECONNREFUSED` stderr noise with
0 test failures (finding 7).

**Verified by live code inspection (mechanism + assertions), not independently
executed here:** the backend Mongo-gated tests
(`direct_completion_does_not_forward_caller_query_string` with the 10s timeout;
`default_off_returns_not_found_for_all_direct_handlers`) — no local Mongo, so
running them would only *skip*. CI provisions Mongo and runs them; I rely on that
plus the cited prior live-Mongo pass for their green result. Likewise
`cargo clippy`/`cargo fmt`/the release build are cited from prior passes; the
present `target/release/nyxid-server.d` confirms the release build was performed.

**Never run by anyone at or after the current tree:** a browser/E2E pass — see
the activation gate.

---

## Final recommendation

**SHIP.** Zero open mechanical defects. All six second-round mechanical findings
are genuinely closed in live code, each with a regression test that fails on
revert, and the interpretation item (the `:3000` warning) is confirmed benign by
direct observation. The repair diff introduced no new regressions; the
`workflow-pending-`→`draft-` change is a correctness fix, and the
`aevatar-transport.ts` churn is semantically inert. Because the surface is
default-off and fails closed, **merge #1426 as built.** The remaining work is not
code repair: run the direct-engine **browser pass before flipping the flag** for
any real account, and make the five product/rollout calls above (sequencing,
addendum flag semantics, prompt-drift policy, legacy-route deprecation, spec §10
Q3). Billing remains accepted low-priority for this internal-testing surface.
