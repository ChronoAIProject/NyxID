BLOCKED

# Adversarial Review: Direct Chrono-LLM Implementation

Reviewed `git diff origin/main...HEAD` (6 implementation commits: `5a1e6532`,
`6f64136c`, `58f4d5d9`, `7b558507`, `473a258f`, `fc5fa9a9`) against spec
v3.2, impl plan v2, the plan-stage review, and CLAUDE.md Critical Rules
2/3/4/5. Every plan-stage BLOCKER/MAJOR was checked at the code, not at the
summary. The implementation is substantially correct and the three plan-stage
blockers are genuinely fixed — but the backend no longer builds in its
release container, which is a merge stopper that no local gate can catch.

Gates re-run by this reviewer on the final tree (not trusted from the
hand-off): `cargo fmt --check` clean; `cargo clippy --all-targets -D warnings`
clean; the 13 targeted backend tests (`assistant_direct`, `direct_chat`,
`admin_service_by_slug`, `billing_route_coverage_smoke`) pass against a live
mongod; `npm run test` 2657/2657 in 225 files; `npm run build` (tsc -b)
succeeds; `npm run lint` 0 errors (23 pre-existing warnings, none from new
files).

## BLOCKER

### BLOCKER 1: the embedded skills break the backend release image build

- **Claim attacked:** BE-2 embeds the skill bodies with `include_str!` from
  the repo's `skills/` tree and treats "cargo build is green" as sufficient
  (`docs/chat/direct-chronollm-impl-plan.md:47-49`, spec §3.3).
- **Evidence:** `backend/src/services/assistant_direct.rs:64`, `:69`, `:74`
  (and the const asserts at `:78-85`) read
  `../../../skills/{nyxid,github-via-nyxid,firecrawl-via-nyxid}/SKILL.md`,
  i.e. `<workspace-root>/skills/**` — outside the backend crate.
  `backend/Dockerfile:47-51` stages **only** `backend/src`,
  `backend/build.rs`, `backend/specs`, `cloud-auth/src`, and
  `docs/AI_AGENT_PLAYBOOK.md` into the builder before
  `cargo build --release` (`backend/Dockerfile:57`); `skills/` is never
  copied, so the second build fails with
  `couldn't read backend/src/services/../../../skills/nyxid/SKILL.md`.
  That the two other out-of-crate embeds are each explicitly staged is the
  proof this is an omission, not an accident of my reading:
  `backend/src/handlers/llms_txt.rs:7` embeds `docs/AI_AGENT_PLAYBOOK.md`
  (copied at `backend/Dockerfile:51`) and
  `backend/src/services/catalog_spec_registry.rs:28+` embeds
  `backend/specs/**` (copied at `:49`). Every other `include_str!` reaching
  outside `backend/` is inside a `#[cfg(test)]` module
  (`backend/src/handlers/assistant_readiness.rs:116` under the `cfg(test)`
  at `:100`; `backend/src/handlers/assistant.rs:1798` under `:1111`) and so
  never compiles in the image.
  Nothing catches this before main: `.github/workflows/ci.yml` builds no
  container, and `.github/workflows/publish-images.yml:20-23,71-72` runs the
  backend image build only on push to `main` and on `v*.*.*` tags. The
  failure therefore lands as a red Publish Images run on main (both amd64
  and arm64), after the PR is merged.
- **Concrete fix:** add `COPY skills skills` alongside the other
  embedded-asset copies in `backend/Dockerfile` (before line 57), with a
  comment naming `services/assistant_direct.rs` as the consumer so the next
  Dockerfile edit does not drop it again. If the backend should not depend on
  a top-level product surface that churns independently (my preference),
  copy the three curated SKILL.md files under `backend/prompts/` and embed
  from there — `backend/**` is already staged, and the direct-mode prompts
  then version with the code that ships them. Either way, add the path to the
  build so a future `skills/` rename fails loudly at CI rather than at
  publish time.

## MAJOR

### MAJOR 1: the client never honors the server's 64-message / 256 KiB caps, so a long direct conversation dead-ends with a generic error

- **Claim attacked:** spec §3.2 caps a direct request at 1..=64 messages and
  256 KiB of aggregate content; §5.2 says the transport "resends the full
  local transcript" every turn and §1 promises conversations that simply live
  in browser memory.
- **Evidence:** the transport builds the request from the entire stored
  transcript with no clamp — `frontend/src/lib/assistant/direct-transport.ts:324-330`
  (`messages: toDirectMessages(stored.turnState.messages)`); the only client
  cap is per-message (`:289-291`, `MAX_MESSAGE_CHARS` at `:31`). The server
  rejects the 65th message with `AppError::BadRequest`
  (`backend/src/services/assistant_direct.rs:143-147`). The client then maps
  that 400 through `unavailableMessage`
  (`frontend/src/lib/assistant/direct-transport.ts:144-155`), which has no
  400 branch and returns "The direct model stream could not be started.",
  discarding the server's precise envelope message. Because `sendMessage`
  appends the user's message to the local transcript *before* the fetch
  (`:295-314`), the next attempt sends 66 messages: once a conversation
  crosses the boundary at turn 33 it is permanently unusable, with no copy
  telling the user why or that a new chat is the remedy. The aggregate
  256 KiB cap (`assistant_direct.rs:160-164`) fails the same way, earlier,
  for transcript-heavy chats.
- **Concrete fix:** clamp the outgoing transcript in `sendMessage` — send at
  most the last 63 messages (and stop at the aggregate byte budget), which is
  the normal stateless-window behavior and keeps the turn working — and
  surface the NyxID envelope `message` for 4xx responses that carry one, so
  any residual cap breach reads as "this conversation is too long, start a
  new chat" rather than a generic stream failure. Add a test that a 70-message
  stored transcript still produces a request the server would accept.

### MAJOR 2: the BE-6 gate silently removes reported-usage capture from `llm-*` services that carry an admin metric override

- **Claim attacked:** BE-6 requires the fix to be surgical and to not alter
  behavior for existing `llm-*` services
  (`docs/chat/direct-chronollm-impl-plan.md:88-91`); spec §7 [v3.1] asks to
  gate on the effective token metric **or** allowlist the new slug.
- **Evidence:** `backend/src/handlers/proxy.rs:2635-2646` replaces
  `target.service.slug.starts_with("llm-")` with
  `platform_metric == BillingMetric::Tokens`. That is not a superset:
  `platform_metric_for_target` (`backend/src/handlers/proxy.rs:3131-3153`)
  returns the **admin-set** `billing.platform_metric` when present and only
  falls back to the slug heuristic otherwise — a supported, deliberately
  tested configuration (`backend/src/handlers/proxy.rs:5611-5632`,
  `admin_platform_metric_override_beats_the_slug_heuristic`). An `llm-*` row
  whose admin set `platform_metric: requests|bytes` therefore loses its
  `UsageAuditContext`: no more `llm_usage_reported` audit rows
  (`proxy.rs:2694-2699` and the four sibling settle branches) and no model
  attribution on the settled meter (`settle_meter_async(..., model)`), for
  both the SSE and chunked-JSON branches. Billed quantity is unaffected
  (requests → 1, bytes → byte count), so this is a silent data-quality
  regression rather than a money bug — but it is exactly the "existing
  services unchanged" property BE-6 asked to preserve, and no test pins
  either direction of the delta.
- **Concrete fix:** make the gate a union rather than a replacement —
  `(platform_metric == BillingMetric::Tokens || target.service.slug.starts_with("llm-"))`
  — which restores the previous behavior exactly while adding the
  tokens-metered case the direct route needs. Reported usage is harmless for
  non-token metrics (`resale_usage_from_optional_reported`,
  `proxy.rs:3155-3173`, ignores it for `Requests`/`Bytes`), so the union has
  no billing side effect. Add a unit test over the gate predicate covering
  the three cases: `llm-` + override, non-`llm-` + tokens, non-`llm-` +
  default.

## MINOR

### MINOR 1: the router's per-turn delegate registry is inert, and it changes cancel semantics on the flag-off path

`AssistantEngineRouter` routes every conversation-scoped call by validated id
prefix (`frontend/src/lib/assistant/transport.ts:574-580`, `:768-778`), so the
delegate for a given conversation id is already deterministic and immutable.
The `running` registry (`:590`, `:787-823`) can therefore never resolve a
different delegate than the prefix would — including in the mid-flip cancel
test (`frontend/src/lib/assistant/engine-router.test.ts:135-149`), which
passes identically with the registry removed. Two costs come with the dead
weight: (a) `wrapHandle.cancel` (`transport.ts:800-816`) substitutes
`delegate.cancelActiveTurn(conversationId)` for the delegate's own
`handle.cancel()` whenever the token still matches, which on the Aevatar path
swaps "cancel this run" for "cancel whatever run is live under this id or its
aliases" (`frontend/src/lib/assistant/aevatar-transport.ts:2503-2531`) — a
semantic change to today's shipping chat with no test pinning it; (b) entries
are released only on `turn.completed` (`transport.ts:787-798`), so any turn
that ends without that event leaks a Map entry for the session. Fix: delete
the registry and keep prefix routing as the stated invariant, or keep it and
make `wrapHandle.cancel` call `handle.cancel()` unconditionally.

### MINOR 2: two backend assertions cannot fail

`backend/src/handlers/assistant_direct.rs:178-187`
(`raw_body_overflow_maps_to_bad_request`) never calls `completions`; it
re-implements the handler's `to_bytes(...).map_err(...)` expression inline and
asserts on that local copy. Deleting the cap at `:89-91` leaves the test
green — it tests axum, not this handler. Fix: enable the flag (as
`enabled_flag_serves_curated_skill_and_model_tables` at `:220-237` already
does), post a 300 KiB body through `completions`, and assert `BadRequest`;
the body read happens before service resolution, so no downstream row is
needed. Related, smaller: the size half of
`registry_has_one_default_and_bounded_skills`
(`backend/src/services/assistant_direct.rs:396-400`) restates the
compile-time `const _: () = assert!(...)` at `:78-85` over the same three
literals and cannot fail independently — keep the one-default assertion, drop
the duplicate.

### MINOR 3: the model picker ignores the `default` flag the endpoint exists to publish

`GET /assistant/direct/models` returns `default: true` for the server's
choice (`backend/src/handlers/assistant_direct.rs:61-76`) and the schema
parses it (`frontend/src/schemas/assistant-direct.ts:8-11`), but the client's
initial selection is the hardcoded
`DEFAULT_DIRECT_MODEL = "gpt-5.5"` (`frontend/src/lib/assistant/direct-transport.ts:30`,
used at `:165-168`, `:186`, `:199-201`). The moment §8's "swap the const
table" seam is used to change the default or retire `gpt-5.5`, the picker
shows and sends a model the server rejects with a 400 on every turn. Fix:
seed the draft model from the models query's `default: true` row (first row
as fallback), keeping the const only as a pre-fetch placeholder.

### MINOR 4: engine selection mutates module state during render

`useAssistantEngine` calls `setAssistantTransportEngine(engine)` in the render
body (`frontend/src/hooks/use-assistant.ts:122-128`). The comment justifies it
(query functions can start during the same render), and the only caller is the
assistant page, so this is low-risk today — but a render that React discards
still leaves the global router switched. If it stays, say so in the comment
("a discarded render leaves the selection set; the next committed render
re-asserts it") so the tradeoff is deliberate rather than assumed.

### MINOR 5: a flag flip clears every draft, not just the other engine's

`frontend/src/pages/assistant.tsx:234-253` calls
`useAssistantDraftStore.getState().clear()` on any engine change, discarding
the user's unsent text in *all* conversations rather than only the ones
pointing at the retired engine (the non-flip branch immediately below already
does the targeted `clearDraft`). Spec §5.1 only asks for draft/URL state
"pointing at the other engine". Fix: clear per-engine draft keys, or accept it
and note it in the PR body as an admin-flip side effect.

### MINOR 6: dev `?mock` with the flag on renders direct-mode chrome over the mock transport

`createAssistantTransportForEnvironment` returns the mock before the router is
constructed (`frontend/src/lib/assistant/transport.ts:993-999`), so
`setAssistantTransportEngine` is a no-op in that context — correct for keeping
vitest/mock semantics untouched. But the page's chrome is driven by the flag
alone (`frontend/src/pages/assistant.tsx:145-146`, `:625`, `:671-678`), so a
dev session with `?mock` and the flag on shows the "conversations are not
saved" banner and pickers that write to a `directAssistantTransport` instance
nothing is chatting through. Spec §5.1 asked for defined behavior in all four
contexts; define this one (suppress the direct chrome when
`selectAssistantTransportKind` resolved to `mock`) or record it as accepted
dev-only cosmetics.

### MINOR 7: the NyxID-401 branch swallows its own failed-turn event

On a structured NyxID 401 the transport calls
`useAuthStore.getState().setUser(null)` *before* emitting the terminal event
(`frontend/src/lib/assistant/direct-transport.ts:484-491`). `setUser(null)`
runs `transitionAssistantIdentity(null)`
(`frontend/src/stores/auth-store.ts:138-145`), which marks every running turn
`discarded` (`direct-transport.ts:179-188`), and `emit` then drops the
`turn.completed` frame (`:422`). The composer is left mid-turn until the
auth-state change bounces the tab to `/login`, which is why the existing test
(`direct-transport.test.ts:256-274`) asserts only the auth-store effect. Fix:
emit `finishUi(...)`/`finishDrain(...)` first, then clear auth — and extend
the test to assert the failed turn is delivered.

## Verified fixed (plan-stage findings)

- **B1 / BE-6 — reported-usage billing.** The gate is keyed on the effective
  metric (`proxy.rs:2635-2646`), the change is 24 lines and does not touch the
  generic streaming branch, and `billing_route_coverage_smoke` now drives the
  **real** mounted `POST /api/v1/assistant/direct/completions` boundary with a
  route access token (`billing_integration_tests.rs:308-329`), replays the
  saved fixture as the upstream (`:1157-1188`, asserting the rebuilt body
  carries `stream`, `stream_options.include_usage`, and a leading `system`
  message), and asserts a finalized `tokens` meter row of exactly 149 plus the
  `llm_usage_reported` audit with 30/119/149 on path `chat/completions`
  (`:1218-1253`) — reported provenance, not a byte estimate. Verified passing.
  Residual: MAJOR 2 above (the narrowing half of the same gate).
- **B2 / FE-2 — engine router.** `install()` is not used for engine selection;
  the router is constructed once and returned directly in production, with the
  dev shell (and therefore the scenario interceptor's one-shot slot) wrapping
  it (`transport.ts:993-1008`, `scenario-intercept-transport.ts:837-844`).
  Mock/vitest returns the mock before the router exists. Conversation-scoped
  calls route by prefix regexes copied verbatim from the Aevatar transport's
  own id patterns (`transport.ts:574-580` vs
  `aevatar-transport.ts:275-278`) and fail closed on anything else. Flag-off
  query keys are the untouched `assistantKeys` object
  (`use-assistant.ts:122-151` returns it unchanged for `"aevatar"`), the flip
  effect no-ops when the engine did not change, and
  `DelegatingAssistantTransport` is a pure pass-through
  (`transport.ts:831-946`), so dropping it from the production chain changes
  no behavior. `pendingCreate` is engine-stamped
  (`use-assistant.ts:602-614`). Residual: MINOR 1.
- **B3 / FE-5 — identity-transition reset.** All four transitions reach
  `transitionAssistantIdentity`: explicit `logout()`
  (`auth-store.ts:102`), session-invalidating 401 in `checkAuth`
  (`:130`), `setUser(null)` (`:142`), and A→B without reload via both
  `checkAuth` (`:116`) and `setUser(user)` (`:144`) — the latter is the path
  `useUser`'s 60 s `/users/me` poll takes (`use-auth.ts:15-31`). The transport
  aborts in-flight runs and wipes conversations, picker state, and draft
  settings (`direct-transport.ts:176-188`), with a defensive `ensureOwner`
  re-check on every read (`:384-392`). Direct query keys are owner-stamped and
  removed on transition from two independent subscriptions
  (`main.tsx:38-40`, `use-assistant.ts:143-150`). The module-lifetime test is
  honest: it proves the abort actually fired, the transcript, settings, and
  drafts are gone, and the old id now 404s
  (`direct-transport.test.ts:322-371`). Residual: MINOR 7.
- **M4 / BE-7 — per-user limiter.** User-id-keyed fixed window plus in-flight
  cap (10/60s, 2 concurrent) with a `Drop` permit
  (`mw/rate_limit.rs:129-220`), acquired per user id in the handler
  (`handlers/assistant_direct.rs:84-86`) and tied to the response body's
  lifetime (`:158-168`) so a client disconnect or an early error return
  releases it via RAII. No permit leak found on any error path — the permit is
  a local binding until it is moved into the body stream. Buckets are swept
  every 60 s (`main.rs:777-784`, retaining only in-flight or recent entries).
  Skills/models are not limited by it. 429 maps to `AppError::RateLimited`
  (HTTP 429, code 1005) and the client has copy for it
  (`direct-transport.ts:151-153`). Isolation and in-flight behavior are both
  tested (`rate_limit.rs:861-871`, `assistant_direct.rs:265-286`).
- **M5 / BE-8 — billing inventory + layer ordering.** The route is registered
  through a billing macro (`routes.rs:124-139`), collected into
  `mounted_billing_route_inventory()` (`:348-357`), declared in the const
  inventory as `Metered(Proxy)` (`billing/route_inventory.rs:141-145`), and
  the metered `route_layer` is applied to the router *after* the macro adds it
  (`routes.rs:1333-1369`). The coverage gate is a real equality —
  every mounted metered route must cross its boundary in the smoke test
  (`billing_integration_tests.rs:2027-2039`) — and the new route is in the
  exercised set (`:327`). Skills/models are mounted outside the forwarding
  layer (`routes.rs:1370-1377`).
- **M7 — 401 handling.** Skills/models use plain `api.get`
  (`hooks/use-assistant-direct.ts:22-24`, `:33-35`), asserted by test
  (`direct-chat-controls.test.tsx:60-62`). The completions POST parses the
  pre-SSE envelope and clears auth only for a structured NyxID 401
  (`direct-transport.ts:123-134`, `:482-499`), with distinct tests for the
  NyxID 401, downstream 401/403, and flag-off 404
  (`direct-transport.test.ts:255-320`).
- **M2 — drain-through.** `finish_reason` closes the visible message but the
  reader keeps going; the run leaves `running` only at `[DONE]`/EOF
  (`direct-transport.ts:534-551`, `:610-616`), bare EOF settles `failed`
  (`:544-549`), abort settles exactly once (`:552-557`, `:684-690`), and every
  current `AssistantTransport` method is implemented explicitly, including the
  action/approval methods that must refuse (`:252-382`). The usage-frame test
  proves the client pulled the usage frame after finish
  (`direct-transport.test.ts:99-130`).
- **M3 — flag-resolution cost**, **M6 — BE-4 deletion**, **MINOR 1 — limit
  units**, **MINOR 2 — prompt override**: dispositioned as specified.
  `POST /api/v1/assistant/completions` is untouched (`routes.rs:1350`). The
  three limits are separate and separately tested
  (`assistant_direct.rs:139-186`, `:264-314`). `BASE_SYSTEM_PROMPT` and
  `DIRECT_MODE_OVERRIDE` are byte-identical to spec §3.3 (diffed
  programmatically), the override is always last including the no-skill case,
  and the prompt-shape test pins the exact composition for every skill
  (`:363-388`).
- **Layering / security spot-checks.** Handlers keep validation and prompt
  composition in `services/assistant_direct.rs` and serve dedicated response
  structs, never model structs (`handlers/assistant_direct.rs:20-31`).
  Flag-off returns `AppError::NotFound` on all three routes with no existence
  leak (`:33-43`, tested at `:189-218`). Tracing is metadata-only — model,
  skill slug, message count, byte count, status; no content (`:125-154`).
  The client's `Cookie`/`Authorization` cannot reach the upstream: the proxy
  forwards only allowlisted headers
  (`services/proxy_service.rs:2663-2677`, `:3247`). No new secrets, no
  identity/delegation minting on this path. No `console.log` and no raw
  `useForm` in any new frontend file; the pickers are Radix Selects outside
  form machinery. The wizard commit `fc5fa9a9` touches only the three
  generated bundle files with no unrelated churn.

**Counts: 1 BLOCKER, 2 MAJOR, 7 MINOR.**
