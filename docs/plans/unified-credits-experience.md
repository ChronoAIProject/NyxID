# Unified Credits experience

## Release blocked: current-main compatibility audit (2026-10-19)

The release check fetched `origin/main` at
`f96cf877834445d4976f62691aa192e534535c1f`, 332 commits after this feature's
original base `baccaa13`. The implementation is preserved in local commit
`5e6c0ec7`. A trial merge exposed 29 conflicting files and was aborted; the
branch has not been pushed, a PR has not been opened, and nothing has been
merged or deployed. Earlier passing tests and the Astra source review cover
the old base only. They do not establish compatibility with current main.

Blocking findings:

1. **Persisted money and precision.** Main uses `Credits(i128)` at 10^-12
   resolution, Decimal128 BSON fields such as `amount` and `remaining`, and
   a balanced v2 ledger. This feature uses required integer `*_micros` fields
   and the old ledger/admission machinery. Its defaulted cap/reclaim fields
   support its original legacy schema, but do not make current persisted
   decimal-credit records compatible. Port to main's money model and keep
   its exact migration/read adapters; do not downgrade the schema or convert
   balances back to microcredits.
2. **Current recipient, allowance and permission contracts.** Main includes
   organization/group recipients, allowance bundles with multiple units,
   sparse edits with change review, URL-persisted tabs, and Credits Manager
   authorization for grant issuance/revocation. Reusing the old page,
   handlers, or schemas would regress these behaviors. Preserve them while
   applying the approved admin-only presentation.
3. **Additional financial admission paths.** Cap and reclaim enforcement
   must cover ordinary usage, durable pool/public/managed admissions, and
   Machine reservations in caller-owned lifecycle transactions. Main binds
   funding to public/managed authority transitions and pool leases; Machine
   settlement requires evidence and uses reserved funding only. The old
   general admission replacement cannot replace those fences. Port shared
   grant predicates/counters into all current paths and verify each path.
4. **Older backend UI availability.** The summary capability guard disables
   every mutation, including otherwise supported allowance controls. Keep
   new policy writes protected from older servers ignoring fields, while
   preserving supported legacy actions and current role authorization.

The unrelated replacement of standard `npm run preview` has been corrected:
it remains `vite preview`; the extra preflight is explicitly available as
`npm run preview:credits`. All nine preview tests pass and `git diff --check`
passes. The replica-set requirement is already present in current main's
exact accounting; it is not a newly introduced deployment requirement
relative to current main.

The required next implementation is a port onto current main, followed by
legacy integer BSON and current decimal BSON tests, recipient/role/bundle
regressions, and cap/reclaim/settlement coverage across all financial paths.
Only after those checks and CI pass is the requested squash merge safe.

## Current scope: admin Credits page only (2026-10-19 revision)

The user rejected the broader billing redesign. The existing user Billing page, wallet card, included-benefits card, top-up flow, navigation, and routes retain their original presentation. The `Credit grants`, `Free allowances`, and `Schedules` tabs remain in their original order on `/admin/credits`.

Only the admin Credits page and its allocation controls are redesigned. Credits remain the unit administrators allocate. Grants and allowances retain their existing internal identities and their existing tabs. The backend allocation and accounting work remains necessary for scoped limits and isolated reclaim.

### Admin Credits page

- Compact remaining-credit balance, with optional plain-language details. No diagnostic tile dashboard.
- Credit grants open by default, with the allocation table immediately visible: recipient, allocated amount, used amount, remainder, service limits, expiry, status, and a direct reclaim action.
- Keep the original tab labels and order. Free allowances remain independently manageable. Schedules retain creation, editing, pause, and resume controls.
- Allocation creation supports selected recipients or all billing owners, one-time/daily/weekly/monthly issuance, expiry, eligible services, and service/group caps. Service limits are visible when the dialog opens.
- Per-row allocation breakdown disclosures are removed. Amounts stay in their columns; service/group caps and their actual services are shown inline. Reclaimed amounts appear next to the remaining balance. Allocation identity is shown in the reclaim confirmation.
- Reclaim applies only to the selected allocation. Allocating 1,000 and spending 900 allows reclaiming the remaining 100; other allocations and purchased credits remain independent. Existing holds retain the reviewed settlement behavior.
- A group limit is a ceiling within the shared allocation total, not an earmarked balance. Show the actual limits on each allocation and retain the precise explanation in the editor.
- Preserve operator read-only access, summary compatibility guards, error/loading states, exact amounts, pagination, and mobile controls.

### User Billing page

Restore the original page, wallet/benefit components, and their query hooks byte-for-byte where possible. The additive combined-summary API remains available internally but does not replace the existing Billing screen. Mockup links use the existing shell and explicitly labeled sample data.

## Implementation sequence

1. Retain reviewed backend allocation, service-cap, recurring issuance, reclaim, ledger, and recovery changes.
2. Restore user Billing UI and the original read hooks/schemas used by it.
3. Revise only admin Credits presentation, preserving its tabs and exposing allocation controls.
4. Verify the existing Billing surface is unchanged in the diff, run relevant frontend checks, and exercise all three Credits tabs and the allocation flows in the local mockup.

## Accounting and security invariants

- No changes to funding precedence: platform allowance units, eligible expiring-first issued credits, then wallet credits. Resale continues to skip usage allowances.
- Only the wallet-funded fraction is sent to Lago. Summary reads never issue credits, append financial movements, or change settlement state.
- Preserve durable ledger activation, expiry/revocation records, reservation admission, settlement locks, deterministic recurring issuance identity, and purchased-credit expiry holds.
- Fixed-point amounts throughout; checked/bounded arithmetic and frontend-safe numeric contracts. Do not add unit quantities or future recurring policy amounts into monetary totals.
- Summary eligibility must agree with current expiry/status/activation semantics, including boundary timestamps and outstanding holds. Admin labels distinguish issued value from account-level spend permission.
- Preserve recipient and organization authorization; an operator may read but cannot mutate. No secret-bearing responses or relaxed delegated-read permissions.
- No production mutations, new paid subscriptions, pushes, PR creation, or deployment are required by this implementation task.

## Validation and review acceptance

- Backend: summary calculations across multiple recipients and more than one page; partial consumption/reservations; pending activation; expiry/revocation; absent and negative wallet balance; organization access; administrator/operator permissions; integer/precision boundaries. Run meaningful existing billing tests for any changed service behavior with a real local test MongoDB where required.
- Frontend: original tabs and default allocation history; compact balance; exact allocated/used/remaining values; visible service caps; create once/recurring; expiry and service restrictions; operator read-only controls; deferred activation; mutation invalidation; loading, error and empty states. Verify the existing user Billing page and components have no presentation diff. Tests assert user-visible outcomes and API contracts.
- Typecheck, lint and build the frontend. Format/check/test changed Rust targets. Run browser verification of admin and user flows using local fixtures or a local deployment; no production financial writes.
- Review all changes personally for correctness, security, UX, accessibility, stale data, pagination, rounding and misleading totals. Resolve actionable findings regardless of severity. Record the actual checks and results; never describe unexecuted or skipped tests as passed.

## Execution ownership

The user requested implementation through Heca with `gpt-6.1-sol`, effort `xhigh`, mode `full-access`. One Heca implementer works in the existing `early-aspen` worktree. The coordinating agent performs independent code review and acceptance testing and sends corrections to that implementer. For the expanded allocation phase, the user additionally requested an Astra reviewer; that reviewer examines the financial changes and recovery paths. The coordinator retains final review and validation responsibility.

## Historical acceptance: superseded presentation (before expanded financial changes)

The presentation described below was implemented and reviewed, then superseded by the admin-only scope revision above. Its combined user Billing UI and removed-tab presentation are no longer part of the change. All findings raised during
four correction rounds have been resolved and rechecked. The coordinating agent
reviewed the code personally; the same Heca Codex implementation thread used
`gpt-6.1-sol`, `xhigh`, and full access throughout. No native subagents or additional
reviewers were used. Changes remain uncommitted in this worktree.

Delivered behavior:

- One administrator Credits page with a full-dataset issued-value headline,
  collapsed breakdown, one Add credits flow, and optional history, recurring
  policy management, and advanced included usage.
- One personal Credits headline combining purchased and eligible issued value,
  with service restrictions, expiry, holds, pending activation, wallet setup,
  suspension, and exact usage units available in its breakdown.
- Read-only summary endpoints at `/api/v1/admin/credits/summary` and
  `/api/v1/billing/credits`. Organization access, rollout rules, operator read
  access, admin mutations, and delegated-read boundaries retain their existing
  authorization rules.
- Checked `i128` aggregate arithmetic and string monetary responses. Negative
  wallet availability remains visible in the breakdown; an unsuspended account
  uses `max(wallet availability, 0) + eligible issued credits`. Suspension makes
  the combined spendable amount zero while retaining recorded balances.
- Summary reads project current allowance windows without creating records.
  Existing grant, schedule, allowance, wallet, settlement, and ledger identities
  and backend compatibility APIs remain intact. There is no financial migration.
- Visible summaries and issued history refresh every 30 seconds, with background
  polling disabled. History updates at expiry without a remount. Failed refreshes
  mark cached totals stale and expose retry controls.
- All touched forms guard unchanged, required-empty, pending, and duplicate
  submissions. Scoped lookup failures have retry controls. One-time/recurring
  switches retain valid targets and restrictions. Mobile values wrap fully, and
  exact formatting preserves six decimal places and locale-specific digits.
- Superseded cards, standalone creation flows, unused detail hooks, and obsolete
  form schemas were removed. Useful validation and organization-access tests now
  exercise the combined Credits API and Add credits flow.

Independent acceptance checks:

| Check | Result |
| --- | --- |
| `npm run build` (includes actual TypeScript build checking, both Vite outputs, and the mock-footprint assertion) | Passed |
| `npm test -- billing credits` | 17 files, 74 tests passed |
| Strict ESLint on all 27 changed frontend files with `--max-warnings=0` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo check -p nyxid --tests` | Passed |
| `cargo test -p nyxid billing -- --test-threads=4` with an explicit local MongoDB 8 replica-set URL | 164 tests passed, zero failures or ignored tests |
| Exact delegated-account-read boundary regression | 1 test passed |
| Independent Chromium acceptance using local API fixtures | 24 scenarios passed |
| Independent exact-formatter checks against the production utility | 9 assertions passed |
| `git diff --check` and whitespace checks for new files | Passed |

The browser scenarios cover collapsed headlines and breakdowns, exact monetary
values and unit exclusion, one-time and recurring issuance, mode round trips,
restricted/expiring issuance, invalid expiry, missing and negative wallets,
suspension, operator access, initial and cached-refresh failures, mobile and
large balances, recurring edits/pause, included-usage edits/disable, cancellation,
service lookup retry, expiry on an open page, and keyboard disclosure controls.
Desktop and mobile screenshots were personally inspected. Source hashes confirmed
that product code stayed unchanged during the final independent build, test, lint,
and browser pass.

The real-MongoDB tests cover summary aggregation beyond one page, activation and
expiry, large totals, wallet rules, edited and missing allowance periods, owner
and organization access, rollout and operator rules, and the existing billing
service behavior. The only later Rust change clarified a response-field comment.
No production payment or financial mutation was used for validation.

## Expanded allocation requirements (2026-10-17, completed)

The user clarified that grants and unit allowances stay internal implementation
concepts, while credits become the main amount administrators allocate and users
see. This phase extends the completed presentation work with enforceable
allocation policies. It is not covered by the acceptance results above.

- An allocation retains its own grant identity, amount, consumed balance, holds,
  expiry, and reclaimed amount. Reclaiming the remainder of a 1,000-credit
  allocation after 900 credits have been spent removes only that allocation's
  remaining 100 credits. It never debits other allocations or purchased credits.
- Reclaim is accepted even while requests hold credits. It durably stops new
  reservations and extra consumption immediately; already admitted holds settle
  or release through the existing journaled path. The remaining amount is
  finalized automatically when those holds and settlement locks finish. Repeated
  requests and reconciliation must not duplicate the terminal ledger movement.
- One-time, daily, weekly, and monthly allocations use the same policy editor.
  Each scheduled disbursement gets fresh counters. Editing or pausing a schedule
  controls future issuance; reclaiming an issued allocation targets that instance.
- Optional service limits are ceilings within this allocation's shared total.
  Example: 10,000 credits total, Twitter up to 500, a named group of selected LLM
  services up to 300. Unlisted services can use the remaining shared total when
  scope is all services. Caps do not reserve money, increase the total, or impose
  account-wide spending limits across other allocations and purchased credits.
  These allocation-local ceilings are the semantics used for this implementation
  and communicated during implementation.
- A service-limit row selects one or more concrete catalog services, optionally
  names that group, and assigns a credit ceiling. Groups cannot overlap. No rules
  means the current unrestricted behavior. Existing selected-service eligibility
  remains supported. All caps apply to actual settled charges as well as holds,
  across platform and resale layers, with atomic allocation-level accounting.
- The interface uses Allocate credits, optional Service limits, recurrence and
  expiry, and an allocation breakdown showing allocated, used, held, available,
  and reclaimed amounts. Reclaim remaining credits explains held work and the
  independent recurring schedule. Unit-based included usage stays under Advanced.
- Issuance retries must not create duplicate allocations or recurring policies.
  Summary reads retain expired or reclaiming sources with outstanding holds while
  excluding their unavailable amount from the spendable headline.

Implementation used the existing Heca GPT-6.1-sol / xhigh / full-access
session. An additional GPT-6 Astra reviewer completed a read-only review at the
user's explicit request. The coordinator also personally reviewed the
implementation. Final acceptance below includes real-Mongo concurrency/recovery
coverage and frontend/browser coverage of policy editing, recurring copies,
isolated reclaim, pending reclaim, and service caps.

### Expanded implementation record

The expanded policy work is implemented in the shared worktree. This record
covers implementation and implementer validation. The coordinator's final
independent source/browser acceptance follows this record; the earlier presentation
acceptance above does not substitute for that review. No commits, pushes, deployments or
production financial writes were performed.

- One `CreditGrant` remains each allocation's financial identity. Disjoint,
  server-resolved concrete service groups share hard caps within its total.
  Total/rule holds and consumption move atomically. Platform/resale share cap
  counters; allowance-covered platform units do not use them. Actual true-up
  also respects caps and can use independent allocations/wallet for overage.
- Durable reclaim accepts outstanding holds, fences new reservations and extra
  consumption immediately, then finalizes only that allocation's exact remainder
  after admitted work/claimed operations finish. Existing terminal ledger keys,
  activation and recovery markers are preserved. Pending requests have accurate
  audit status; completed removal retains the established revoked event.
- Keyed allocation/schedule creation freezes actor-scoped request identity,
  payload and recipients/policy. Deterministic insert-only issuance recovers
  partial/concurrent replay, retains at-most-50 inline activation, and rejects
  changed payloads. Every recurring issuance has fresh counters and frozen rules;
  edits affect future periods.
- Financial admission now commits all allowance/grant/rule/wallet holds and all
  layer usage rows in one MongoDB transaction before forwarding. External checks
  happen first; abort needs no financial compensation. Disabled billing keeps
  its no-database path. Resource lock acquisition excludes stale settlement
  claimants through a conditional usage-row write, and takeover completes
  already durable locks before recalculating remaining funding.
- Summaries retain expired-held and reclaiming sources, account for unapplied
  extra commitments, and distinguish realizable service capacity from the full
  reclaimable accounting remainder. Limits count as restrictions. Unit allowances
  and future issuance never enter monetary totals. New exact fields stay strings;
  legacy numeric detail fields and backend routes stay available.
- Allocate credits and recurring edits include compact optional group limits,
  live per-owner/total previews, actionable lookup/validation recovery and stable
  retry UUIDs. Allocation details identify issue date/ID/recurring period and show
  exact allocated/used/held/remaining/available/reclaimed values and rule usage.
  Reclaim remains available during holds and displays pending state. Mobile lists
  retain warnings/actions; dialog shells cannot scroll the heading/close away.
- `CLAUDE.md`, the billing spec, environment guidance and UI glossary describe
  actual contracts, including replica-set admission and the requirement to
  upgrade **all** billing/backend workers before enabling caps/pending reclaim.
  Older binaries cannot enforce additive policies. No legacy orphan balances are
  inferred, released or migrated.

Reviewer correction dispositions and evidence:

| Findings | Resolution / coverage |
| --- | --- |
| A, F2 | Same-session admission and full multi-layer usage handoff; real-Mongo abort tests before/after handoff and second-layer insert failure, plus concurrent duplicate replay. |
| B, F5, F9–F11, F13 | Reservation excludes active locks; bounded resource-specific helping, exhausted/unrelated sources skip correctly; disabled unreachable-DB call, unreserved duplicate and exact-one-capacity duplicate tests. |
| C | Mobile recurring cards retain skipped-period warning; responsive component coverage. |
| D, F1, F15 | Real-Mongo expired-held/reclaim summaries, selected-only realizable capacity, unapplied/applied commitment accounting, matching rule capacity and read-only assertions. |
| F3, F8 | Canonical ID controls scope and rule selection; real admission/true-up reused-slug regression and shared catalog identity checks. |
| F4, F7 | Once-per-layout date/identity/UTC-period details, per-owner preview, concise conditional reclaim copy and valid description markup; page/history/card component coverage. |
| F6 | Bounded insert-only chunks and stable request snapshot; greater-than-50 recipient partial/concurrent replay and recurring replay/future-only tests. |
| F12, F16 | Compact limit disclosures, metric-free monetary picker, shared-total preview, error-driven outer/inner expansion and focus; create/edit and collapsed-invalid component tests. |
| F14 | Original claim is immutable through refresh; acquisition conditionally writes usage row; takeover drains exhausted-resource locks; deterministic stale worker, write-conflict, token-adoption and pre-applied/unrecorded grant/allowance recovery tests. |
| F17 | Named sparse per-usage indexes on grant/allowance resources; standard index initialization/idempotency and hinted execution-stat smoke. |
| F18 | Pending reclaim response/audit test; API and glossary distinguish accepted request from final removal; failed-journal capped settlement retains lock until one exact terminal movement. |
| F19 | Scoped `overflow-clip` on Allocate/Edit recurring shells. Real Chromium375/320 deep-picker screenshots and geometry: shell scrollTop0, titleY20, closeY16, scrollable body and visible footer; validation autofocus and desktop also checked. |

Final implementer checks are recorded below after execution. Coordinator final
acceptance is recorded separately below.

| Final implementer check | Actual result |
| --- | --- |
| Frontend `npx tsc -b --pretty false` | Passed; no diagnostics. |
| Frontend `npm run build` | Passed, including both Vite outputs and mock-footprint assertion; repeated after coordinator's final copy-spacing polish. |
| Frontend `npm test -- billing credits` | 17 files / 84 tests passed; no skipped tests; repeated after final copy polish. |
| Strict ESLint on every changed frontend file (`--max-warnings=0`) | 29 files passed; the coordinator's final one-file copy change was linted again. |
| Frontend `npm run lint` | Passed with 0 errors and 27 existing warnings; unrelated warnings unchanged. |
| Independent formatter script against production utility | 9 exact-format assertions passed. |
| `cargo fmt --all -- --check` | Passed. |
| `cargo check -p nyxid --tests` | Passed. |
| Real-Mongo `cargo test -p nyxid billing -- --test-threads=4` | 190 passed, 0 failed, 0 ignored; includes pending audit, immutable claim, concurrency, abort/recovery, caps, reclaim and existing billing contracts. |
| Real-Mongo exact `db::tests::ensure_indexes_runs_without_error_on_fresh_db` | 1 passed; index initialization is idempotent and hinted lookups return the expected usage lock. Initial smoke assertion assumed BSON Int64; corrected to accept exact BSON Int32/Int64 and rerun passed. |
| Exact `mw::auth::tests::delegated_account_read_allows_expected_management_families` | 1 passed, including the combined Credits read route. |
| Implementer real Chromium layout checks | Allocate and Edit recurring at375/320px retain heading/close/footer while body scrolls; invalid-limit autofocus and desktop checked. Coordinator-owned full browser suite remains independent. |
| `git diff --check` and new-file whitespace scan | Passed; no generated artifacts or unrelated files added. |

Mongo-backed checks used the explicit task-owned replica set:
`NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27018/?replicaSet=rs0&directConnection=true'`.
No Mongo-required case above was described as passed through a skip. Logs and
implementer browser geometry/screenshots are under
`/tmp/nyxid-unified-credits-implementation/expanded-*` and `implementer-*`.
The Rust toolchain reports an existing dependency future-incompatibility notice
and a macOS test-linker unwind-size warning; no required command failed after the
listed corrections.

All identified actionable implementation findings through F19 are resolved.

### Final independent acceptance (2026-10-17)

The coordinator completed personal code review, source-delta review, and the
checks below. The additional Astra review found no unresolved actionable financial
source findings. All identified implementation findings are addressed, and the
implementation is accepted in this worktree. Changes remain uncommitted.

| Coordinator check | Actual result |
| --- | --- |
| Real-Mongo `cargo test -p nyxid billing -- --test-threads=4` | 190 passed, 0 failed, 0 ignored. |
| Real-Mongo `cargo test -p nyxid credit_schedule_tests -- --test-threads=4` | 10 passed, 0 failed, 0 ignored. |
| Real-Mongo `cargo test -p nyxid grant_visibility_tests -- --test-threads=4` | 3 passed, 0 failed, 0 ignored. |
| Exact delegated-account-read denied-class/protocol regression | 1 passed. |
| `cargo fmt --all -- --check` and `cargo check -p nyxid --tests` | Passed. |
| `npm run build` | Passed, including TypeScript, both Vite outputs and the mock-footprint assertion; repeated after final static copy spacing. |
| `npm test -- billing credits` | 17 files, 84 tests passed. |
| Strict ESLint on all 29 changed frontend files, `--max-warnings=0` | Passed. |
| Independent Chromium acceptance | 37 distinct scenarios passed, including desktop and 320px/375px mobile. |
| Per-usage settlement index explain smoke | Both resource lookups examined one matching document out of 1,001 documents. |
| `git diff --check` and new-source whitespace inspection | Passed. |

The implementer additionally passed the real-Mongo index initialization and
idempotency test. Its evidence is recorded above and was inspected by the
coordinator; it is not counted as a separately rerun coordinator test.

Browser acceptance uses local API fixtures; financial behavior is independently
exercised by the Mongo tests. The browser coverage includes unrestricted and capped
issuance, validation recovery, shared service groups, recurring policy preservation,
lost-response retry identity, isolated remainder reclaim, reclaim with holds,
expired-held allocation details, skipped-period warnings, and the retained
presentation flows. One mobile scenario initially used the desktop action label
in the harness; correcting that selector and rerunning passed. The final aggregate
retains this provenance. Screenshots were personally inspected, including the
polished mobile recurring editor after a static JSX spacing correction.

Source hashes were compared against both the acceptance-start manifests and
Astra's immutable reviewed snapshot. The only post-review handler difference was
a test fixture correction; the index test also corrected BSON integer decoding.
The final frontend difference was static JSX spacing, followed by a successful
production build and targeted mobile rerun. No substantive financial code changed
after Astra's final review. The coordinator separately reviewed the index additions.

Validation receipts, source manifests, browser results and screenshots are in
`/tmp/nyxid-unified-credits-implementation/`; the final review is in
`/tmp/nyxid-unified-credits-astra-review/final-backend-review.md`. The expanded
acceptance record identifies the exact evidence files. Required deployment setup
is a transaction-capable MongoDB replica set and upgrading every backend/billing
worker before enabling service caps or pending reclaim. No production financial
data was converted, inferred, or mutated during this work.

### Preview compatibility correction (2026-10-19)

The first local-frontend/production-backend preview was incorrectly described as
ready after only its login page and public API had loaded. Authenticated launch
acceptance had not been performed. Live read-only probes subsequently established
that production lacks both new Credits summary routes (HTTP 404), and its social
login callbacks return to the production API while the proxy stores the state
cookie on localhost. Production also rejects the localhost return URL. That
preview was stopped; the requested production-backed preview remains **not ready**.

Corrections:

- `npm run preview` now performs read-only backend and login configuration
  checks before building and starting Vite. Missing protected credit routes,
  HTML fallbacks, incompatible social callback hosts and rejected local return
  URLs stop the command. It builds current source, binds to loopback, and uses
  a strict port. Passing configuration checks is explicitly not authenticated
  browser acceptance. `npm run preview:check` checks without launching.
- Administrator credit changes require a successfully validated summary.
  Initial loading, unsupported backends and failed cached refreshes disable
  allocation and hide other credit mutations. Open mutation dialogs and their
  submit handlers also respect the guard. A missing backend route displays a
  specific compatibility error rather than allowing new policies to be sent to
  an old handler that may ignore them.
- The frontend README records the actual preview setup and required authenticated
  checks. No production backend or authentication policy was changed.

Verification completed after the correction:

| Check | Result |
| --- | --- |
| Preview preflight regression suite | 9 passed, including actual process refusal before build/startup against an incompatible server. |
| Frontend billing/credits suite | 17 files, 86 tests passed. |
| Fresh production frontend build and strict lint of modified TSX | Passed. |
| Real local browser login through the frontend proxy | Passed against the matching backend and isolated MongoDB. |
| Real admin allocation flows | One-time shared service caps and unrestricted allocation persisted; the worker issued a monthly allocation with fresh counters; recurring edit/pause and isolated reclaim passed. |
| Real user summary API | Authenticated HTTP 200, exact 1,200 remaining issued credits after the flows. |
| Billing ledger verification | `status=ok`, 4 entries, valid anchor at sequence 4. |
| Unsupported-backend browser regression | Summary 404 disables allocation and removes reclaim actions with an explanatory error. |
| Production launch check | Correctly refused with missing summary APIs and social-login origin mismatches. |

The local user Billing page was not accepted as a complete preview: the isolated
backend has no Lago configuration, so its existing billing capability guard
redirects that page. The payment integration was neither simulated nor connected
to production. This local verification does not establish production readiness.
The browser test initially waited for PUT on a recurring edit whose real method
is PATCH; the successful edit was verified in a corrected continuation. Evidence
is retained under `/tmp/nyxid-unified-credits-implementation/live-preview/`, including
`acceptance.json`, `production-preflight.log`, and the browser screenshots.

## Admin-only revision acceptance (2026-10-19)

The surrounding Billing presentation was restored. Seven original Billing,
wallet, benefit, hook, test, and router files were compared byte-for-byte against
this checkout's base and match. The superseded combined-balance UI components
were removed; restored user benefit reads use their original endpoints.

The admin page retains the original three tabs and shows allocation history by
default, a compact total, allocated/used/remaining columns, inline service caps,
and a direct reclaim button. The service-limit editor is initially expanded.
Creating a schedule presets monthly recurrence and returns to the Schedules tab.

Validation: 70 relevant frontend tests passed (one stale copy assertion was
updated and its six-test suite rerun); production build and strict changed-file
lint passed. Chromium verified all tabs, grant allocation, grouped caps,
1,000/900/100 reclaim isolation, recurring creation/edit/pause, restored Billing,
and mobile fit with no browser errors or external requests. Local fixture-only
mockup: http://127.0.0.1:3000/mockup/admin. Evidence lives under
`/tmp/nyxid-unified-credits-implementation/mockup/`. Production was not modified.
