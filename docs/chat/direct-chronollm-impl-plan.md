# Direct Chrono-LLM Chat — Implementation Plan (v2)

Companion to `docs/chat/direct-chronollm-spec.md` (Draft v3.1 — the SSOT
for WHAT to build; this file is HOW and WHO). Branch:
`chat-chronollm-direct`.

**v2 reconciliation:** all findings in
`docs/chat/direct-chronollm-plan.review.md` are dispositioned. Where this
plan and the review conflict with spec v3 text, spec v3.1's `[v3.1]`
sections win. Summary of dispositions: B1 → new task BE-6 (usage-capture
gate fix); B2+M1 → FE-2 rewritten (engine-router transport); B3 → new task
FE-5 (identity-transition reset); M2 → FE-1 drain-through semantics; M3 →
accepted loudly (spec §3.4); M4 → new task BE-7 (per-user limiter); M5 →
new task BE-8 (billing inventory); M6 → BE-4 DROPPED (deletion deferred);
M7 → FE-1 401 handling corrected; MINOR 1/2 → folded into BE-2 (limit
units, prompt override suffix).

## Pipeline (owner-directed)

| Role | Agent | Output |
| --- | --- | --- |
| Plan + necessity check | Fable (PM session) | spec v3 + this plan |
| Adversarial plan review | GPT Sol (codex/gpt-5.6-sol) | `docs/chat/direct-chronollm-plan.review.md` |
| Plan reconciliation | Fable (PM) | spec/plan revisions |
| Implementation | GPT Sol | code on this branch, task-by-task commits |
| Adversarial impl review | Opus (claude/claude-opus-5) | `docs/chat/direct-chronollm-impl.review.md` |
| Final check + PR | Fable (PM) | PR to `main` detailing work done |

Quota fallback: any Fable role degrades to Opus if Fable is unavailable.
Single worktree (`zen-fox`); ONE writer at a time (implementer), reviewers
are read-only except their own review file — avoids the shared-index
commit races. PM lands nothing while the implementer is active.

## Task breakdown

### BE-1 — flag registry
`experimental:direct-chat-engine` in
`backend/src/services/feature_flag_service.rs::FEATURE_FLAGS` (default
off, description says it selects the assistant chat engine) + FE constant
`FEATURE_FLAG.DIRECT_CHAT_ENGINE` in `frontend/src/lib/feature-flags.ts`.
Keys must match exactly (comment in feature-flags.ts mandates it).

### BE-2 — `assistant_direct` service module
New `backend/src/services/assistant_direct.rs` (or a module under
`assistant_service.rs` if layering fits better — implementer's call, keep
handlers/ -> services/ -> models/ discipline):
- consts: `DIRECT_LLM_SLUG`, `BASE_SYSTEM_PROMPT`, `DIRECT_SKILLS`
  (include_str! SKILL.md only; const-assert ≤ 64 KiB each), `DIRECT_MODELS`
  (gpt-5.5 default, gpt-5.4, gpt-5.4-mini, gpt-5.2).
- pure fns: `validate_direct_request(...) -> AppResult<DirectChatRequest>`,
  `build_upstream_body(req) -> serde_json::Value` per spec §3.2 (single
  system message; forced `stream:true` + `stream_options.include_usage`).
- `resolve_admin_service_by_slug(db, slug)` — generalize the existing
  `resolve_admin_service` (aevatar call site keeps behavior identical).

### BE-3 — handlers + routes
`backend/src/handlers/assistant_direct.rs`:
- `POST /api/v1/assistant/direct/completions` — flag check → validate →
  rebuild → `execute_admin_proxy(state, auth_user, &service.id,
  "chat/completions", ...)` → SSE passthrough response. NO delegation
  bridge, NO identity token, NO upstream echo collector.
- `GET /api/v1/assistant/direct/skills`, `GET .../models` — flag check →
  serve the const tables.
- Flag check helper: resolve caller's effective features (reuse the same
  service the `/users/me` `enabled_features` path uses; grant-union incl.
  org grants) → off = `AppError::NotFound` (no existence leak).
Mount all three inside `assistant_proxy_routes` (`routes.rs` ~1313) so the
human-only rejection layers apply.

### BE-4 — DROPPED (Sol MAJOR 6)
`POST /api/v1/assistant/completions` stays: it is a documented retained
surface (docs/chat/02-wire-contract.md). Deletion deferred to a follow-up
with a deprecation decision. Do not touch it.

### BE-5 — backend tests
Spec §9 backend list + the v3.1 additions (prompt-shape override-suffix
test, limit-unit boundary tests, flag-off 404 on all routes).
Anchor style: existing `assistant_service.rs` tests.

### BE-6 — SSE usage-capture gate fix (Sol BLOCKER 1) — REQUIRED
`handlers/proxy.rs` enables SSE usage observation only for slugs starting
`llm-`; chrono-llm-public never enters the reported-usage path and would
bill byte-estimates. Fix per spec §7 [v3.1]: gate on effective token
billing metric (row `platform_metric: tokens`) + SSE response shape, or
explicitly allowlist `chrono-llm-public` as an OpenAI-SSE usage source —
implementer picks the narrower, better-tested option. MUST include a
route-level test through the real `POST /assistant/direct/completions`
boundary: emit the saved fixture stream, assert the settled billing
quantity equals the fixture's reported `total_tokens` with reported (not
estimated) provenance. Keep the change surgical — do not restructure the
generic streaming branch.

### BE-7 — per-user rate limit (Sol MAJOR 4)
Dedicated authenticated-user limiter for the direct completions POST:
user-id-keyed bucket (model on the existing limiter machinery in
`mw/rate_limit.rs`) + small per-user in-flight stream cap (2). Defined 429
behavior; tests prove per-user isolation and that concurrent streams from
one account hit the cap. Skills/models routes stay on default policy.

### BE-8 — billing route inventory (Sol MAJOR 5)
Register the direct POST in
`services/billing/route_inventory.rs` + the route macro so
`mounted_billing_route_inventory()` and the billing smoke suite cover it
at the real authenticated assistant boundary; metered `route_layer`
applied only after all forwarding routes are registered. Skills/models →
control-plane exemptions (or mounted outside the forwarding layer).

### FE-1 — direct transport
Resurrect `git show e3c2f36e^:frontend/src/lib/assistant/completions-transport.ts`
as `frontend/src/lib/assistant/direct-transport.ts`; adapt per spec §5.2
[v3.1]: typed body, new URL, `direct-` id prefix, skill/model fields,
user-scoped local store (FE-5), **drain-through semantics** (read past
finish_reason through usage + [DONE]; bare EOF = failed; abort settles
once), full CURRENT `AssistantTransport` contract (the historical class
predates it — implement every projection/delete/cancel/approval/action
method explicitly), **corrected 401 handling** (skills/models = plain
`api.get`, session-clearing allowed; completions pre-SSE envelope
distinguishes NyxID auth error vs downstream 401/403 vs flag-off 404).

### FE-2 — engine-router transport (REWRITTEN per Sol BLOCKER 2 + MAJOR 1)
Do NOT use `DelegatingAssistantTransport.install()` (one-shot; slot owned
by the dev scenario interceptor; singleton exported as the narrow
interface). Build the engine-router per spec §5.1 [v3.1]: permanent router
owning both delegates; explicit `selectedEngine` state set from the flag
via the hook layer; conversation-scoped ops routed by validated id prefix
(fail closed on unknown); per-turn delegate registry so cancel /
stream-start-timeout / Stop always hit the ORIGINATING delegate; scenario
interceptor wraps OUTSIDE; mock/vitest paths untouched; engine-stamped
query keys + flip invalidation + engine-stamped `pendingCreate`
(`hooks/use-assistant.ts`). Specify and test init in all four contexts
(prod, dev, dev+`?mock`, vitest).

### FE-3 — pickers + copy
Skill picker + model picker in the composer (direct mode only), fed by the
two GET routes (TanStack Query hooks, one per domain convention —
`hooks/use-assistant-direct.ts`). Banner + empty-state copy per spec §5.3.
Read DESIGN.md notes: live app is the visual source of truth.

### FE-4 — FE tests
Spec §9 frontend list + v3.1 additions (drain-through/usage-frame
consumption, EOF matrix, engine-router routing/cancel matrix, cache
separation, 401 disambiguation). Fixture already saved:
`frontend/src/lib/assistant/__fixtures__/chrono-llm-direct-stream.sse`.
Gotcha: vitest flakes under machine load / live dev servers on :3000/:3001
answering test fetches — keep fetch hermetic (stub), re-run
`--no-file-parallelism` if flaky.

### FE-5 — identity-transition reset (Sol BLOCKER 3)
User-scope the direct store; abort + wipe on every identity transition
(logout, session-invalidating 401, `setUser(null)`, user A → user B
without reload), wired at the same boundary as the other auth-scoped store
cleanups (`stores/auth-store.ts` cleanup list + `use-auth` logout);
invalidate direct queries there too. Module-lifetime test:
`null → A → transcript → logout → B` leaks no list/history/running
request/picker/draft state.

### T-1 — gates (implementer runs ALL before handing to review)
- `cargo fmt --check`; `cargo clippy -- -D warnings`; `cargo test`
- `cd frontend && npm run build && npm run test && npm run lint`
  (`npm run build` = tsc -b with noUncheckedIndexedAccess — the actual CI
  gate; `tsc --noEmit` passing means nothing)
- Wizard bundle: if the freshness index trips on FE source changes,
  `npm --prefix frontend run build:wizard` AFTER final rebase, commit
  `cli/src/wizard/`.

## Known gotchas (from prior sessions — do not rediscover)

1. Upstream path is `chat/completions`, NOT `v1/chat/completions` — the
   row's base_url already ends in /v1 (live-verified; v1/... 404s).
2. `include_usage` must be in the rebuilt body or streamed billing falls
   back to byte estimates (proxy force-list covers only llm-openai /
   llm-deepseek).
3. Session-cookie callers work on this path (master credential is the
   row's own bearer) — the aevatar-style TD-3 cookie-401 cannot occur; do
   not port the delegation bridge.
4. `useAppForm` not raw `useForm` (lint-enforced) if any form machinery is
   touched; pickers wired via watch+setValue behave per CLAUDE.md Rule 4.
5. FE api-client: any non-allowlisted 401 clears global auth state — every
   direct-mode JSON call must pass `preserveSessionOn401`.
6. No `console.log` in production FE code; Debug impls redact; never log
   message content server-side (metadata only).
7. Commit style: conventional commits; never commit to main; PR targets
   `main`. Push needs the `ctkm-aelf` gh account (calvintkm is pull-only).
8. Worktrees are cut from possibly-stale local main — this branch already
   exists; implementer works ON `chat-chronollm-direct`, no new branch.

## Review protocol

- Sol (plan review): attack spec v3 + this plan against the actual code.
  Verify every file/line anchor, hunt for: flag-resolution mismatches with
  the real feature service API, execute_admin_proxy signature drift, SSE
  passthrough pitfalls (content-length, buffering), FE transport
  assumptions vs current `transport.ts`, deletion risks (BE-4), missing
  test surface. Write findings ONLY to
  `docs/chat/direct-chronollm-plan.review.md` (severity: BLOCKER / MAJOR /
  MINOR, each with evidence file:line). No other file edits.
- Opus (impl review): adversarial pass over the diff vs origin/main —
  correctness, CLAUDE.md Critical Rules compliance, spec conformance,
  test honesty (no padding — Calvin rejects no-op tests), security
  regressions. Findings to `docs/chat/direct-chronollm-impl.review.md`,
  same severity scheme. Read-only otherwise.
- PM (Fable): reconciles each review, directs fixes, final gate re-run,
  raises PR with a work-done narrative (spec links, review dispositions,
  gates evidence).

## PM independent verification (owner directive 2026-08-11)

After Opus review + fixes, the PM (Fable) runs its OWN end-to-end test —
not a re-run of the agents' suites:

1. Boot the local stack (Mongo via docker compose :27018; backend
   `cargo run --bin nyxid-server`; seed a chrono-llm-public-shaped row +
   mock OpenAI-SSE upstream, or point the row at the real llm.aelf.dev if
   a master credential is available locally).
2. Flag OFF: all three direct routes 404; aevatar chat path untouched.
3. Flag ON (per-user override): drive
   `POST /api/v1/assistant/direct/completions` with a real session —
   streamed turn end-to-end; skills/models GETs; skill-injected turn.
4. Behavioral spot-check of the SS-derived prompt: ask an actionable
   question ("connect github for me") → expect a copyable `nyxid` command
   + explicit cannot-execute note, NO execution claim; ask a live-state
   question → expect "cannot check from here" + the checking command.
5. Billing: assert the settled row for the streamed turn carries
   reported-token provenance (not byte estimate).
6. Browser pass (gstack browse): flag-driven engine switch, direct chat
   renders streamed turn, banner copy, reload wipes, logout wipes.
7. Full gate re-run on the final tree.

Only after this passes does the PR go up.

## Definition of done

- All BE/FE tasks landed on `chat-chronollm-direct`; both review files'
  BLOCKER/MAJOR items fixed or explicitly dispositioned in the PR body.
- All T-1 gates green locally (CI will also run — main-target PR).
- Spec v3 + plan + reviews committed with the code (docs travel with the
  PR).
- PR open against `main`, DRAFT, body: what/why, endpoint table, flag
  rollout note (default off; Calvin flips per-user first), test evidence,
  residual risks.

## PM verification results (2026-08-12, executed)

Ran the full independent check against a local stack (backend on :3021,
Mongo :27018, a mock OpenAI-SSE upstream on :3099 replaying the prod
fixture, seeded chrono-llm-public admin row, FE dev on :3020).

- Flag OFF → all three direct routes return 404 (code 1003); Aevatar path
  untouched. PASS
- Flag ON (user-target override) → `GET /direct/skills` and
  `/direct/models` serve the curated tables (gpt-5.5 default). PASS
- Streamed turn end-to-end via CLI/curl: OpenAI SSE deltas → finish → usage
  frame → [DONE]. PASS
- Upstream body capture: single `system` message = base prompt + nyxid
  SKILL.md + override suffix (18,323 chars), `stream:true`,
  `stream_options.include_usage:true`, path `chat/completions`, master
  credential injected server-side. PASS
- Validation matrix: unknown model / unknown skill / `system` role / 65
  messages all → 400 with precise messages. PASS
- Rate limit: 7th rapid turn → 429; skills/models unaffected. PASS
- Billing (BILLING_ENABLED=true): `usage_meter` row metric=tokens
  quantity=149 slug=chrono-llm-public; `llm_usage_reported` audit with
  30/119 prompt/completion — reported provenance, not byte estimate. PASS
- Prompt behavior against the REAL prod gpt-5.5: "connect github for me" →
  copyable `nyxid service add github` + "chat can't run it", no execution
  claim; "is my github connected?" → "I can't check from here" +
  `nyxid service list`. Matches the SS Class-L + cannot-check contract.
  PASS
- Browser (gstack browse): login → /assistant renders direct chrome
  (banner, model/skill pickers), streamed reply rendered in-thread, reload
  wipes the conversation (stateless). PASS after fixing one bug:

**Bug found & fixed (commit 43e2bed9):** the transport defaulted `fetchFn`
to the bare global `fetch` and called it as `this.fetchFn(...)`; real Chrome
throws "Illegal invocation", jsdom does not — so all fetch-injected unit
tests were green while the live browser turn failed. Wrapped the global and
added a regression test that reproduces the window-binding guard (red
without the wrap). This is exactly the gap an agent test-suite structurally
could not catch, which is why the independent browser pass was required.
