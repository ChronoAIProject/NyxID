# WP-0B — Frontend journey substrate (dispatch refactor + verified completion)

**Master plan:** `docs/chat/waves234-plan.md` (§3.2–§3.5). Self-contained brief; verify
cited files against `origin/main` (and the Wave-1 PR branch
`feat/2026-08-17_wave1-reauthorize-impl` where noted) before editing.

**Mission.** (1) Refactor assistant journeys out of `action-card.tsx` into per-journey
modules so N teams can add verbs without touching shared files. (2) Land the
postcondition-verified completion helper (kills Wave-1 MAJOR A2 for all future verbs).
(3) Land the attempt-correlation helpers (kills MAJOR A3). (4) Land the journey test
harness with the mandatory predicate-false case (kills minor A5).

**Depends on:** WP-0A merged (the evidence endpoint exists) — the helpers can be built
against a stubbed fetch first, so development can overlap; merging waits for 0A.
**Blocks:** all wave team packages.

## Current state (verify, then refactor)

- `frontend/src/components/assistant/blocks/action-card.tsx` implements the
  `service.connect` journey inline and (on the Wave-1 PR branch) the
  `service_reauthorize` journey: preflight fetch + `assertSecretFreeReadBack`, dialog
  launch (`AddKeyDialog` with `reconnectKey`/`prefillScopes`), out-of-band completion
  via `pending-connect-store` + `useKeyAuthorizationWatch`, and direct
  `report("completed", …)` calls at (PR branch) `:513-541` and `:965-973`.
- `frontend/src/lib/assistant/action-registry.ts` exposes
  `ActionJourney` ids and `resolveAssistantAction`.
- Known defects you are generalizing away — read
  `docs/chat/wave1-service-reauthorize-actions.md` §A2/A3/A5/A7/A8 on the PR branch.

## Deliverables

### 1. Journey module system

- New `frontend/src/components/assistant/journeys/` directory. Each journey is one
  module exporting a fixed interface — derive the exact shape from what the two
  existing journeys need; expected minimum:

  ```ts
  export interface JourneyModule<P extends ActionCardParams> {
    preflight?: (params: P, ctx: JourneyContext) => Promise<PreflightResult>;
    Component: React.ComponentType<JourneyProps<P>>;   // renders dialog/flow
  }
  ```

  with `JourneyProps` carrying `report`/`onBlock`/`onCancel` wrappers **already bound
  to the substrate** (see §2 — raw `report` is not handed to journeys).
- `journeys/index.ts` — a `Record<ActionJourney, () => Promise<JourneyModule>>` lazy
  registry. Unregistered journey id → the existing unsupported-card rendering.
  After this package, `index.ts` and `action-card.tsx` are PM-owned; teams only add
  `journeys/<name>.tsx` files.
- Port the existing `service.connect` (catalog + custom) and `service_reauthorize`
  journeys into modules with **zero behavior change** (their A2/A3 fixes are Wave-1 PR
  scope, not yours — port faithfully, including current flaws, unless the Wave-1 PR has
  merged the fixes by the time you start; check the branch state first).
- Keep the action-card chrome (title/body/CTA/risk badge/decline) where it is; only
  journey execution moves.

### 2. `journey-postcondition.ts` — the only way to report `completed`

`frontend/src/lib/assistant/journey-postcondition.ts`:

```ts
export async function reportCompletedAfterVerify(opts: {
  kind: EvidenceKind; id: string;
  predicate: (evidence: unknown) => true | { blocked: string };
  resource: SafeResourceReport;      // exactly one typed variant
  report: ReportFn; onBlock: BlockFn;
}): Promise<void>
```

- Fetches `GET /api/v1/assistant/evidence/{kind}/{id}` (WP-0A), parses with a
  **required-fields-required** Zod schema per kind (`.nullable()` where the wire says
  nullable; never `.optional()` for present-by-contract properties — Wave-1 minor A7:
  optional guards fail open on shape drift, required ones fail loud).
- Runs the per-verb predicate; on `true` → `report("completed", resource)`; on
  `{blocked}` → `onBlock` with the typed reason. No other module may call
  `report("completed", …)`.
- Enforcement: add a `no-restricted-syntax` ESLint rule scoped to
  `components/assistant/journeys/**` forbidding direct `report("completed"` calls
  (same mechanism as the existing `useAppForm` `no-restricted-imports` rule — see
  `eslint` config; verify the config file location on origin/main).

For **delete-verbs**, add `reportCompletedAfterDeleteVerify` — same shape but success
evidence is the evidence endpoint returning 404 under a live session (WP-0A contract).
Treat network errors / 401 as NOT-verified (blocked note), only a clean 404 verifies.

### 3. Attempt correlation helpers

- `runDirectMutation({ mutate, kind, id | idFrom(response), predicate, resource, … })`
  — for dialog-driven CRUD verbs: executes the mutation, uses the mutation response
  itself as the attempt anchor (2xx + identity fields), then chains
  `reportCompletedAfterVerify`. **No timestamp-advancement logic exists in this path at
  all.** Rotation verbs must correlate on returned successor identity
  (`rotated_to_id` / new prefix / secret version), never on `rotated_at` advancing.
- `awaitOutOfBandCompletion({ attemptRef, kind, id, predicate, resource, deadlineMs })`
  — for popup/deep-link/QR flows. Captures a **fresh** evidence baseline inside the
  user-gesture handler (not at card render — Wave-1 A3's stale-snapshot window), polls
  the evidence read, requires predicate + (where available) server-side attempt
  correlation, reports the timeout note otherwise.
- **Spike (timeboxed ~half day, results recorded in this file's PR):** determine which
  out-of-band server flows already persist an attempt identifier onto the written row
  (`initiateOAuthAsync` returns a `connection_id`/attempt nonce — see
  `frontend/src/hooks/use-providers.ts` and the OAuth callback write path
  `user_api_key_service.rs`). Document per-flow: `correlated` (nonce verifiable in
  evidence) vs `baseline-only` (fresh-baseline + predicate floor). Waves consume this
  table; do not silently upgrade `baseline-only` flows to "verified".

### 4. Test harness

`frontend/src/components/assistant/journeys/test-harness.ts(x)`:

- Renders a journey module inside a mock action-card context with a scripted evidence
  fetch (MSW or the existing fetch-stub pattern — match whatever
  `action-card.test.tsx` uses today; note the vitest hermetic-fetch caveat: a live dev
  server on :3000/:3001 can answer test fetches with real 401s).
- Exports `expectNeverCompletes(scenario)` — drives the journey to nominal completion
  while the evidence read returns a predicate-failing shape; asserts `report` is never
  called with `completed` and the block/timeout note renders. **Every journey suite in
  every wave must include at least one such case per verb** — this is the A5 rule
  ("the freshness test does not test freshness"); the harness makes it one line.

### 5. Preflight conventions (A8)

Journey preflights fetch single entries (`GET /catalog/{slug}`, `GET /keys/{id}`,
evidence reads) — never list endpoints filtered client-side. Put the rule in the
journeys `README.md` you create in the directory, alongside the module interface docs.

## Acceptance criteria

- `npm --prefix frontend run test` green including ported journey suites unchanged in
  behavior; `run lint` (with the new restricted-syntax rule active and passing);
  `run build` (CI parity is `tsc -b` with `noUncheckedIndexedAccess` — `tsc --noEmit`
  passing is NOT sufficient).
- Mutation check: delete the predicate call inside `reportCompletedAfterVerify` and at
  least one harness test fails (prove the harness bites, then revert).
- `action-card.tsx` after refactor contains no journey-specific business logic and no
  direct `report("completed"` call sites outside the substrate.
- No wizard-bundle rebuild unless you touched files in
  `cli/src/wizard/bundle-meta/index.manifest` (check; if deps/lockfile changed, rebuild
  `npm --prefix frontend run build:wizard` + commit `cli/src/wizard/`, and re-run after
  any rebase).

## Test commands

```bash
npm --prefix frontend run test
npm --prefix frontend run lint
npm --prefix frontend run build
```
