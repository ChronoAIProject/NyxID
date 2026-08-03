# Mock Scenario Interception — Implementation Plan

Status: IMPLEMENTED — WP1 through WP8 completed in order; code and test commits
`4fe63f71..1a7f6f6b`. CodexSol round-2 findings were folded before execution
(all 11 accepted; see `mock-scenario-intercept-plan.review.md`).
Spec: `docs/chat/mock-scenario-intercept-spec.md` (v2, post-review). The spec
is the contract; this plan is the build order. On any conflict, the spec wins
and the conflict gets reported, not silently resolved.

Pipeline (per Calvin, 2026-08-04): Fable plans → CodexSol reviews plan →
GPT-Sol implements → Opus adversarial review → Fable final check + PR.
Open-question defaults locked: dev-only gating (§14.1), pass-through kept /
no strict toggle (§14.2), no world seeding (§14.3), scenario set per WP3
(§14.4), post-hoc verification only — no AddKeyDialog change (§14.5).

## Ground rules for the implementer

- Branch: `mock-github-chat-calls` in this worktree. Commit after each work
  package with the exact message given. **Do not push. Do not open a PR.**
  Fable owns push + PR.
- Zero changes outside the **authoritative diff scope** = spec §13's table
  (now includes the footprint script, the scripts-only `frontend/package.json`
  edit, `transport-shell.test.ts`, the integration test, and the `docs/chat/`
  docs). Anything beyond it is a blocker to report, not a judgment call.
- `frontend/` only. **Dependencies and the lockfile are frozen** (Wizard
  Bundle Freshness CI trips on dep changes). The named scripts-only
  `package.json` edit is the single allowed touch of that file.
- Existing tests are a hard invariant: `MockAssistantTransport`,
  `mock-data.ts`, `createScriptedTurn`, and every existing spec stay green
  untouched.
- Follow spec §10 checklist literally; each item lands as an executable
  assertion where feasible.
- Repo conventions: `useAppForm` (not applicable — no forms), no
  `console.log`, DESIGN.md before any styling decision (WP6), vitest
  colocated `*.test.ts(x)`.

## Work packages (build order)

### WP1 — Store
`stores/assistant-mock-scenarios-store.ts` + test. Spec §7.
- zustand + persist, key `nyxid.assistant.mockscenarios.v1`, version 1,
  partialize `{enabled, disabledScenarioIds, world, userId}`.
- `ensureUser(userId)`: persisted userId mismatch → reset to defaults (F14).
- `engineState: idle|loading|ready|error`, `lastActivity`, world actions.
- No imports from engine/transport (store is the bottom layer).
- Commit: `feat: add assistant mock-scenarios store`

### WP2 — Engine
`lib/assistant/scenario-engine.ts` + test. Spec §4 (verb table), §5 (all).
- Builder: `scenario(id, regex, build)`, `flow(build)`, fluent steps:
  `say/tool/toolFail/action/approval/artifact/await/need/run/whenConnected/
  whenNotConnected/connect/disconnect/wait/stop/fail`.
- Compile-time validation: unknown flow refs, flow recursion, >1 resumable
  card per await (F10), action params through
  `assistantActionRequestSchema.parse` + `resolveAssistantAction` (§10).
- Runtime: segments split at await; conversation-monotonic cursors via an
  injected `CursorSource` (F1); `EVENT_CADENCE_MS = 100` pacing with tracked
  timers; cancel path mirroring `cancelScript` ordering; continuation
  registry keyed `(conversationId, requestId)`; branch selection by
  disposition / approved / denied / wake; §5.4 card-state guards replicated;
  §5.5 lazy approval expiry (F11).
- World access through a `WorldPort` interface (store-agnostic, injectable in
  tests). `.connect()` gated on §6.6 verification outcome passed in by the
  interceptor.
- Engine is pure TS: no React, no direct store import, no network.
- Commit: `feat: add mock chat scenario engine`

### WP3 — Config
`lib/assistant/scenarios.config.ts` + test. Spec §4 verbatim flows plus two
QA scenarios: `approval-demo` (`/post .*digest/i` → tool + approval card +
await approved/denied branches) and `error-demo` (`/simulate (an? )?error/i`
→ say + `fail("mock_error", ...)`).
- Test: config loads, ids unique, regexes valid, flow refs resolve.
- Commit: `feat: add scripted mock chat scenarios config`

### WP4 — Interceptor
`lib/assistant/scenario-intercept-transport.ts` + test. Spec §6 — the whole
of it; this is the hardest package. Implementation order inside the WP:
1. Ownership state machine (§6.2) + prefix routing (§6.1, §6.3) against a
   stub delegate. Toggle-independence of mock-owned routing (F2), parked
   settle-before-send (F3).
2. History projection: base snapshot, anchored merge, metadata overlay
   (§6.4, F7/F8/F9).
3. Claimed placeholder conversations (§6.5, F4) incl. fallback turn.
4. Journey verification per §6.6 **as amended**: `GET /keys/{id}` compared
   on `KeyInfo.catalog_service_slug` (there is NO GET on
   `/user-services/{id}` — P1); synchronous mock-owned **verifying** state
   with a real provisional, cancellable `TurnHandle` around the async lookup
   (P2). Tests hit the real endpoint path + `KeyInfo` shape: match,
   mismatch, 404, malformed, cancel-during-lookup, delete-during-lookup.
5. `MockScenariosLoadingError` (§6.7, F6); delete cancel-first + tombstone
   (F13), tombstone **persists on rejected DELETE** — do not copy the real
   transport's clear-in-`finally` shape (P10).
6. List snapshot alongside history snapshot (§6.4 as amended, P7): zero
   delegated list GETs during mock activity; test through the real Aevatar
   delegate with mocked HTTP + stale 5 s list TTL + a >5 s script.
- Test file mirrors spec §12's interceptor list one-for-one; every F/P
  number named in a test description. `message_count` projection asserted in
  both history and list (P12).
- Commit: `feat: add scenario intercept transport with ownership routing`

### WP5 — Transport shell + boot install
`lib/assistant/transport.ts` edit (~20 lines). Spec §8.3, §9.
- `DelegatingAssistantTransport` shell; `"aevatar"` branch wraps it; dev-only
  `if (import.meta.env.DEV)` dynamic import installs the interceptor.
- `ensureUser` wiring is an **auth-store subscription** owned by the dev boot
  module — fires on every resolved non-null user transition, not once at
  boot (P6): logout keeps state; the next *different* user resets it.
- New `transport-shell.test.ts` with an injectable installer/loader seam
  (P5): bare delegation before install, in-place interception after, import
  failure, idempotent install, full-mock non-installation; auth lifecycle
  null → A → logout → B rescopes world in one module lifetime. WP7's
  integration test must consume the **exported singleton**, never a
  hand-built interceptor.
- Prove-no-regression: existing `transport.test.ts` untouched and green;
  `MODE === "test"` still yields the full mock.
- Commit: `feat: install mock scenario interceptor behind dev-only boundary`

### WP6 — UI
`components/assistant/mock-scenarios-action.tsx` + test; `pages/assistant.tsx`
edit (dev-gated `lazy`, both headerActions call sites). Spec §8.1–8.2.
- Read DESIGN.md first; match `AssistantWireLogAction`'s trigger styling.
- FlaskConical + active dot; popover: master switch (loading/error states),
  scenario rows + lastActivity marker, world chips + reset, footer file
  pointer + session-only note, **and the real-journey warning line**:
  action cards open real connection journeys and can create real
  keys/connections (P9).
- Both mount points wrap the lazy action in a local
  `<Suspense fallback={null}>` — the route-level boundary would blank the
  whole workspace while the dev chunk loads (P8); a deferred-import render
  test proves the shell stays visible.
- Component tests (P11): engine load error state, matched + unmatched
  `lastActivity`, per-row switch driving `disabledScenarioIds` with a store
  round-trip proving a disabled scenario stops intercepting, chip
  removal/reset, empty world, warning copy, both mount points.
- Commit: `feat: add mock scenarios toggle popover to assistant header`

### WP7 — Integration + footprint
- `use-assistant` integration case (spec §12): send → blocked → continue →
  continuation renders; #1304 preservation across refetch with overlay.
- `frontend/scripts/assert-mock-footprint.mjs`: greps `dist/` for
  `mockchat-`, `scenario-engine`, `mockscenarios`, and additionally asserts
  the `dist/credential-accept` output still exists. Appended to the **full
  existing** build chain — `tsc -b && vite build && vite build --config
  vite.credential-accept.config.ts && node scripts/assert-mock-footprint.mjs`
  — the credential-accept stage is a production artifact and must never be
  dropped (P4). package.json `scripts` edit only — no deps, no lockfile.
- Commit: `test: cover mock interception integration and prod footprint`

### WP8 — Docs
- `docs/chat/README.md`: index the spec + this plan.
- Spec status header → "IMPLEMENTED (v2)" with commit range.
- Commit: `docs: index mock scenario interception spec and plan`

## Verification gates

Per-WP: targeted vitest for the package + `npm run build` (tsc -b catches
`noUncheckedIndexedAccess` issues plain `--noEmit` misses).
Final (implementer, then repeated independently by Fable):
1. `npm run lint` — zero new warnings.
2. `npm run test` — full suite; on unrelated-file timeout flakes re-run with
   `--no-file-parallelism` before concluding regression.
3. `npm run build` — full chain incl. the credential-accept stage and the
   footprint assertion.
4. `git status` clean; diff touches only the §13 file list + README/plan.
5. Wizard freshness pre-check: none of the touched files appear in
   `cli/src/wizard/bundle-meta/index.manifest` (they must not; if any does,
   STOP and report — do not rebuild the wizard bundle).

## Review + landing (Fable-owned)

1. Opus adversarial review of the full diff (spec conformance, §10 checklist,
   F1–F14 regressions, test honesty); findings fixed by GPT-Sol; re-review
   until clean.
2. Fable: independent gate run, diff read, memory-informed checks (wizard
   manifest, lint RHF bailout n/a, no console.log).
3. Sync: merge `origin/main` into `mock-github-chat-calls`; resolve; re-run
   gates. Verify `rollup-chat-2026-08-04` contains main (else merge it too).
4. Push via `ctkm-aelf`; PR base `rollup-chat-2026-08-04`, title
   `feat: mock scenario interception for assistant chat flows`, body:
   summary bullets, spec/review/plan links, F1–F14 disposition note, test
   plan with actual command output, footer per repo Claude Code convention.
   Note: rollup PRs get no per-PR CI (ci.yml gates main/dev), so the PR body
   carries the local gate evidence; CI proper runs when the rollup lands.

## Risks

- R1: `use-assistant` reconciliation subtleties beyond what the spec models —
  mitigated by the WP7 integration case landing before UI polish; if the
  #1304 preservation path still fights the overlay, stop and surface, don't
  hack the hook.
- R2: package.json `scripts` edit sits near the frozen-dep rule — it changes
  no dependency and no lockfile; wizard freshness hashes source closure, not
  scripts. Verified against the manifest in gate 5.
- R3: codex implementer drift from spec — contained by per-WP commits, §10
  as executable assertions, and two independent review passes.
