# Admin usage dashboard and audit listing performance

Branch: `admin-usage-dashboard`; base: main `167025ae` (0.27.0).

## Design decisions

- Added admin/operator `GET /api/v1/admin/usage` and `/admin/usage`, with the existing admin guard, dedicated response DTOs, OpenAPI registration, and `AdminUsageViewed` telemetry containing only applied control names.
- A default rolling 24-hour window, 7/30-day presets, and explicit RFC 3339 `[from, to)` windows are supported. Invalid/inverted windows and windows over 31 days are rejected. Actor-or-wallet-owner filtering supports organization traffic. Service filtering accepts slug or ID.
- All reductions, rate lookup/arithmetic, distinct-user/service counting, ranking sorting and pagination execute in MongoDB. Independent summary, ranking and service-option pipelines run concurrently. Each aggregation has `maxTimeMS=20000`, with a 22-second wall-clock guard covering the complete service read. Timeout error 12200 returns HTTP 503 and actionable retry guidance.
- Primary transaction IDs are uniquely indexed. Only primary platform rows contribute requests and provider token classes. Component and resale rows contribute their legitimate per-metric quantities, event counts and costs. Historical duplicated component breakdowns are ignored. New component materialization persists the breakdown only on the primary row, with a retry regression test.
- Exact settlement funding takes precedence over current model-specific/generic legacy rates. Free rows cost zero. Unknown legacy groups stay null; known groups contribute to totals, while a visible partial-estimate notice exposes missing rates. Decimal128 multiplication/accumulation avoids floating-point money arithmetic; response totals saturate to int64, matching the existing money convention.
- Rankings are actor × billing owner × service rows, retaining organization owner attribution. Quantity sorting uses an explicit metric (default tokens), never a sum of unlike units. Stable identity tie-breakers make pagination deterministic. Expanding a user reads all their services in the exact response window.
- One projected `users` query batches ranking actors, owners and the selected filter. Unknown/deleted users display “Unknown user”; IDs appear only in tooltips. Service metadata is also batched. Service options retain the window/user scope independently of the selected service.
- The frontend validates responses with Zod and preserves future metric names. Controls live in URL state. Tables have corresponding mobile cards, semantic badges, compact primitives and mono numbers. Billing number/credit formatters were extracted without changing their behavior.

## Audit diagnosis and fix

- Existing main-era indexes already cover every `ADMIN_SORT_OPTIONS` key, both directions, and `event_data.response_status`. No audit index was added, removed or changed; the benchmark compares complete before/after index specifications.
- Count and page queries execute concurrently, alongside event-type option retrieval. Following review round 1, empty-filter counts use collection metadata via `estimated_document_count` (exact except briefly after an unclean shutdown); every non-empty filter retains exact `count_documents`. Pagination is never capped.
- The measured global substring search count chose seven full index scans (3.5 million keys on 500,000 rows). For **global-search-only** counts, one natural collection pass is faster. Their page queries explicitly use the existing matching sort index, avoiding expensive competing plans. Any additional narrowing predicate leaves normal index selection available; date-plus-search explains demonstrate the indexed date bound before regex evaluation. The regex document, escaping, case-insensitive contains semantics, response shape and row order are unchanged.
- Event-type filter options are derived display data cached in `AppState` for 30 seconds, with coalesced cold requests and no cached failures. Separate states/databases do not share the cache. Rows, counts, permissions and audit-chain facts are never cached.
- The long benchmark exposed an existing test database heartbeat panic: `tokio::time::timeout` was constructed on a standard thread before entering its runtime. Timer construction now happens inside `block_on`'s async block. This changes only the test harness.

## Initial implementation verification

All required checks passed on the final implementation. Disk space was checked with
`df -h /System/Volumes/Data` before each long Cargo phase; the final chain started
with 79 GiB free and retained 77 GiB before the benchmark.

Commands executed from the repository root unless stated otherwise:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server -- --test-threads 2
cargo test -p nyxid-cli
cd frontend && npm run lint && npm run test && npm run build
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server admin_query_execution_stats -- --ignored --nocapture
git diff --check
```

The benchmark command above runs from the repository root, after returning from
`frontend`. Real final output summaries:

| Check | Result |
| --- | --- |
| Rust formatting | Exit 0; no output |
| Workspace/all-target Clippy with warnings denied | Exit 0; `Finished dev profile ... in 41.58s` |
| Full backend binary, real replica-set MongoDB | `6307 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 333.47s` |
| CLI unit suite | `1245 passed; 0 failed`, 6.21s |
| CLI integration suites | Agent Key login: 10 passed; Codex connection: 5 passed; login resume: 13 passed; platform service display: 2 passed; wizard freshness: 1 passed; zero failures |
| CLI total | 1,276 passed; zero failures; zero ignored |
| Frontend ESLint | Exit 0; `27 problems (0 errors, 27 warnings)`; existing fast-refresh/dependency warnings, no new warnings |
| Frontend Vitest | `Test Files 342 passed (342)`; `Tests 3455 passed (3455)`; 71.87s |
| Frontend production build | Exit 0; TypeScript/Vite, credential-accept build, and `Mock scenario footprint assertion passed; credential-accept output is present.` |
| Explicit explain benchmark | `1 passed; 0 failed; 0 ignored; 0 measured; 6307 filtered out; finished in 36.14s` |
| Diff whitespace check | Exit 0; no output |

The one ignored backend test is the explicitly executed large-dataset benchmark.
New coverage checks primary/component/resale deduplication, all credential classes
and metric units, status/window predicates, actor and organization ownership,
service ID/slug filters, stable ranking/pagination, exact/legacy/model-specific
costs, unknown rates, int64 money saturation, admin/operator authorization,
component settlement retries, and event-option cache expiry/isolation. Frontend
coverage includes receipt validation, unknown metrics, URL controls, user searching,
organization attribution, expansion using the exact response window, pagination,
loading/error/empty states and invalid custom ranges.

**Wizard condition:** compared every changed file with
`cli/src/wizard/bundle-meta/index.manifest` and its freshness-test extras; the
intersection was empty. Therefore `npm --prefix frontend run build:wizard` was
not required. `wizard_bundle_is_fresh` ran and passed as part of the full CLI
command. No CLI source or bundle changed.

**Browser smoke:** Playwright against local Vite with mocked API responses at
1440px desktop and 390px mobile widths; verified rendered content, period changes
in the URL, no page errors and no horizontal overflow. This is browser/layout
validation, not an additional live-server integration claim. Screenshots:
[desktop](admin-usage-desktop.png), [mobile](admin-usage-mobile.png). The temporary
Vite process was stopped afterward.

Local full logs (not needed to reproduce the checks):
`/tmp/admin-usage-fmt.log`, `/tmp/admin-usage-clippy.log`,
`/tmp/admin-usage-backend-full-final.log`, `/tmp/admin-usage-cli.log`,
`/tmp/admin-usage-lint.log`, `/tmp/admin-usage-frontend-test.log`,
`/tmp/admin-usage-frontend-build.log`, `/tmp/admin-query-explain-final.log`.

## Explain evidence (refreshed after review round 1)

The reproducible ignored test is
`services::admin_usage_service::tests::admin_query_execution_stats`. It seeds a
disposable MongoDB database with **200,000 usage rows across 100 actors and 25
services**, and **500,000 audit rows with 120 event types**, distributed over 90
days. It constructs the main 0.27.0 indexes, measures the original queries, calls
production `ensure_indexes`, and measures the final queries. The primary usage
measurement uses the default 24-hour window; audit pages have 50 rows. Audit date
filters cover yesterday through today with the existing inclusive-day semantics.

Raw plans, index bounds and stats are retained in
[admin-query-explain.json](admin-query-explain.json). Numbers below are MongoDB
`executionStats`, not end-to-end HTTP latency: aggregate source scans use the
`$cursor` stage stats, and find commands use root stats. Timings are one local
run, subject to cache/host variance; examined counts and plans are the durable
comparison.

| Query | Before docs | Before keys | Before ms | After docs | After keys | After ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `audit.default.count` | 500,000 | 0 | 108 | 0 | 0 | 0 |
| `audit.default.find` | 50 | 50 | 0 | 50 | 50 | 0 |
| `audit.event_type.count` | 0 | 4,167 | 1 | 0 | 4,167 | 0 |
| `audit.event_type.find` | 50 | 50 | 0 | 50 | 50 | 0 |
| `audit.search.count` | 0 | 3,500,000 | 1,784 | 500,000 | 0 | 547 |
| `audit.search.find` | 10,202 | 10,202 | 421 | 10,202 | 10,202 | 15 |
| `audit.date.count` | 0 | 14,658 | 1 | 0 | 14,658 | 1 |
| `audit.date.find` | 50 | 50 | 0 | 50 | 50 | 0 |
| `audit.search_date.count` | 14,658 | 14,658 | 107 | 14,658 | 14,658 | 103 |
| `audit.search_date.find` | 10,202 | 10,202 | 56 | 10,202 | 10,202 | 58 |
| `audit.status.count` | 0 | 50,000 | 9 | 0 | 50,000 | 7 |
| `audit.status.find` | 498 | 498 | 0 | 498 | 498 | 0 |
| `usage.summary` | 200,000 | 180,000 | 298 | 2,831 | 2,548 | 73 |
| `usage.ranking` | 200,000 | 180,000 | 258 | 2,831 | 2,548 | 84 |

- Unfiltered audit totals now use `RECORD_STORE_FAST_COUNT`: zero documents and
  zero keys examined, 0 ms at millisecond resolution. The page remains a
  50-key/50-document sort-index scan. Non-empty filters still receive exact totals;
  metadata-based unfiltered totals can briefly differ after an unclean shutdown.
- Global search counts change from seven unbounded index scans to one collection
  pass (1,784 → 547 ms). The page keeps its sort-index scan and avoids plan
  competition (421 → 15 ms). No regex or pagination semantics change.
- Event-type, date and status queries retain their existing usable indexes.
  Search-plus-date uses a date-bounded IXSCAN followed by regex FETCH (14,658
  candidates for the count), not a global regex scan.
- Both usage pipelines have bounded IXSCAN plans and no COLLSCAN after indexing.
  The summary examines 2,831 documents rather than 200,000. The after plan uses
  `usage_meter_admin_window`, with index bounds constrained to the window.
- Every one of the 16 `ADMIN_SORT_OPTIONS` directions was explained: each examined
  50 documents and 50 keys, reported 0 ms at millisecond resolution, and used
  IXSCAN. The benchmark also compares production audit-service exact counts and
  returned row order with the original queries for all six measured scenarios.
- The test asserts the **entire audit index specification list is unchanged**,
  including `audit_log_seq_unique`, and every baseline usage index remains
  identical. The full backend suite includes audit/ledger chain and append tests.

Additional usage measurements, before adding the test-only competing candidate:

| Summary filter | Documents | Keys | ms |
| --- | ---: | ---: | ---: |
| `30d` | 67,077 | 60,369 | 739 |
| `actor` | 2,548 | 2,550 | 28 |
| `service` | 2,548 | 2,550 | 29 |

Each filter plan uses IXSCAN. Candidate comparison on the same disposable database:

| Candidate | Documents | Keys | ms |
| --- | ---: | ---: | ---: |
| `benchmark_date_candidate` | 2,830 | 2,830 | 71 |
| `usage_meter_admin_window` | 2,831 | 2,548 | 69 |

Only `{ status: 1, created_at: -1 }` is installed by production code: it excludes
nonterminal statuses at the index and scans fewer keys than date-only, without
extra indexes per optional filter. `benchmark_date_candidate` is created only
inside the disposable test database. No index is dropped or modified. The one
added B-tree adds insert/status-transition write work; it is neither unique nor
partial and cannot reject existing writes.

Event-type cache measurement: cold **2.800 ms**, warm **0.004333 ms**,
120 types, 30-second TTL. Only the derived filter vocabulary is cached.

## Files touched

- Backend API/wiring: `backend/src/handlers/admin_usage.rs`,
  `backend/src/handlers/mod.rs`, `backend/src/routes.rs`,
  `backend/src/api_docs.rs`, `backend/src/errors/mod.rs`,
  `backend/src/telemetry/schema.rs`.
- Backend reporting/indexes: `backend/src/services/admin_usage_service.rs`,
  `backend/src/services/admin_usage_service/tests.rs`,
  `backend/src/services/mod.rs`, `backend/src/db.rs`.
- Audit concurrency/cache: `backend/src/services/admin_audit_service.rs`,
  `backend/src/handlers/admin.rs`, `backend/src/main.rs`.
- Component regression fix: `backend/src/services/billing/meter.rs`.
- Test state setup and heartbeat-runtime repair: `backend/src/test_utils.rs`.
- Frontend feature/contracts: `frontend/src/pages/admin-usage.tsx`,
  `frontend/src/pages/admin-usage.test.tsx`,
  `frontend/src/hooks/use-admin-usage.ts`,
  `frontend/src/hooks/use-admin-usage.test.tsx`,
  `frontend/src/schemas/admin-usage.ts`,
  `frontend/src/schemas/admin-usage.test.ts`,
  `frontend/src/test/admin-usage-fixture.ts`, `frontend/src/types/admin.ts`.
- Shared formatting: `frontend/src/lib/billing-format.ts`,
  `frontend/src/pages/billing.tsx`.
- Frontend navigation: `frontend/src/router.tsx`, `frontend/src/pages/lazy.ts`,
  `frontend/src/components/dashboard/sidebar.tsx`,
  `frontend/src/components/layout/dashboard-layout.tsx`,
  `frontend/src/components/navigation/command-palette.tsx`.
- Documentation/evidence: `CLAUDE.md`, `docs/BILLING_UI_GLOSSARY.md`,
  `.claude-brief/ADMIN_USAGE_REPORT.md`, `.claude-brief/admin-query-explain.json`,
  `.claude-brief/admin-usage-desktop.png`, `.claude-brief/admin-usage-mobile.png`.

## Known limits and delivery

No known unimplemented brief requirements or failing verification checks.
Data and semantic limits are explicit:

- Traffic is available only when metering was enabled at recording time; there
  is no backfill for periods with `BILLING_ENABLED` off.
- Historical provider cache counters can overlap prompt/input, so the displayed
  total is prompt + completion, with cache classes shown separately. Estimated
  token metrics without provider breakdowns remain in per-metric quantities.
- Legacy costs use current cached rates; missing rates yield partial estimates.
  This is the existing billing convention, not historical repricing accuracy.
- Exact global substring search remains O(N); the optimization removes seven
  full index passes without capping counts or altering contains semantics.
- Event-type options may lag by up to 30 seconds. Rows/counts are queried live.
- Unfiltered audit totals are collection-metadata counts; they can briefly differ
  after an unclean shutdown. All non-empty filters retain exact totals.
- ESLint retains 27 pre-existing warnings; it reports no errors.

Delivery is on `admin-usage-dashboard` with conventional commit title
`feat(admin): add platform usage reporting and optimize audit queries`.
No versions changed, no push, and no PR was opened.

## Review round 1

Addressed all eight findings from the review of `16c54069`:

1. Added the non-ignored MongoDB test
   `production_indexes_support_all_list_hints_and_metadata_count`. It runs
   production `db::ensure_indexes`, checks named hints against
   `list_index_names()`, and calls the production listing with empty and global
   substring filters for all 16 sort options. It checks both collection totals
   and exact filtered totals/rows. A pure test pins all sort/index mappings,
   including `status` → `audit_log_sort_event_data_response_status`.
2. Unfiltered audit totals now use `estimated_document_count`; all non-empty
   filters keep exact `count_documents`. The service comment and `CLAUDE.md`
   document metadata-based totals and the possible short-lived discrepancy after
   an unclean shutdown. The benchmark explains the no-query `count` command used
   by the driver and still compares actual service results with original queries.
3. Usage identities fall back from an absent/blank display name to email;
   missing user documents retain “Unknown user.” Database coverage checks null,
   empty and whitespace names. The user-picker fixture and page test cover a
   null display name and assert selection by email.
4. Credential labels now come from `lib/billing-units.ts`, using all six requested
   admin-perspective labels and retaining the raw-string fallback.
5. Extracted the billing page's existing `MetricBlock` into
   `components/shared/metric-block.tsx` with an optional detail slot. Its original
   DOM/classes are unchanged when detail is absent, preserving billing's layout.
   Usage now shares that component, and its tables/mobile cards use
   `rounded-lg border border-border`.
6. Usage in the command palette now uses `ChartNoAxesCombined`, matching the
   sidebar.
7. Usage schema types and `z` now use top-level type-only imports in `types/admin.ts`.
   The neighboring platform-credential type uses the same convention.
8. Usage uses `useNavigate()` with absolute `to: "/admin/usage"` and reads search
   from the production route ID `/dashboard/admin/usage`. The new router test
   mounts the actual application route tree (including guards, search validation,
   lazy page, and real query/router hooks) at `/admin/usage?period=7d&sort=cost`.
   It asserts both controls and the initial HTTP query, then changes the period
   and checks the updated URL and HTTP query. Only HTTP and unrelated dashboard
   chrome are replaced in this test.

The desktop/mobile screenshots above were refreshed at 1440px and 390px using
Playwright and local Vite with mocked API responses. Both widths have no page
errors or horizontal overflow. The temporary Vite server was stopped.

### Review verification

All required verification was rerun on the final review implementation and passed.
Disk space was checked with `df -h /System/Volumes/Data` before every long Cargo
phase; 76 GiB remained free throughout this verification chain.

Exact commands (from repository root, except the explicitly scoped frontend line):

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server -- --test-threads 2
cargo test -p nyxid-cli
(cd frontend && npm run lint && npm run test && npm run build)
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server admin_query_execution_stats -- --ignored --nocapture
git diff --check
```

| Check | Real final result |
| --- | --- |
| Rust formatting | Exit 0, no output |
| Clippy, workspace/all-targets, warnings denied | Exit 0; `Finished dev profile ... in 1m 24s` |
| Full backend against replica-set MongoDB | `6310 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 337.41s` |
| Full CLI | 1,276 passed, zero failures: 1,245 unit + 10 Agent Key login + 5 Codex connection + 13 login resume + 2 platform service display + 1 wizard freshness |
| Frontend lint | Exit 0; 0 errors, 27 unchanged existing warnings |
| Full frontend tests | `Test Files 343 passed (343)`; `Tests 3457 passed (3457)`; 59.21s |
| Frontend build | Exit 0; TypeScript/Vite and credential-accept build passed; `Mock scenario footprint assertion passed; credential-accept output is present.` |
| Explain benchmark | `1 passed; 0 failed; 0 ignored; 0 measured; 6310 filtered out; finished in 36.91s` |
| Diff whitespace check | Exit 0, no output |

The full CLI command ran `wizard_bundle_is_fresh` successfully. The changed-file
intersection with its bundle source manifest and extra inputs is empty, so the
conditional `npm --prefix frontend run build:wizard` rebuild was not needed.
No CLI/bundle files changed. Focused checks before the full runs also passed:
29 audit-service tests, and 11 tests across Usage page, real-router and billing
page suites.

Logs: `/tmp/admin-review1-fmt.log`, `/tmp/admin-review1-clippy.log`,
`/tmp/admin-review1-backend-full.log`, `/tmp/admin-review1-cli.log`,
`/tmp/admin-review1-lint.log`, `/tmp/admin-review1-frontend-test.log`,
`/tmp/admin-review1-build.log`, `/tmp/admin-review1-explain.log`.

### Updated performance evidence

The main before/after table and raw `admin-query-explain.json` above now contain
the fresh review run. The same dataset sizes, baseline indexes and all 16 audit
sort checks were used. Existing audit index specifications remained identical;
all baseline usage indexes were preserved. Production filtered counts and row
ordering still matched the original queries.

| Unfiltered count implementation | Documents examined | Keys examined | Execution ms |
| --- | ---: | ---: | ---: |
| Original main query, fresh review run | 500,000 | 0 | 108 |
| Reviewed commit `16c54069`, its recorded measurement | 0 | 500,000 | 55 |
| Review round 1 metadata count, fresh run | 0 | 0 | 0 |

The new winning plan is `RECORD_STORE_FAST_COUNT`; default page retrieval remains
50 keys/50 documents. Filtered counts stay exact. Fresh global-search count time
was 547 ms (baseline 1,784 ms), and page time was 15 ms (baseline 421 ms).
Individual millisecond timings vary by host/cache state; the metadata count's
zero document/key scans establish the removal of collection-sized count work.

### Review files and delivery

Changed backend files: `backend/src/services/admin_audit_service.rs`,
`backend/src/services/admin_usage_service.rs`, and
`backend/src/services/admin_usage_service/tests.rs`.
Changed frontend files: `frontend/src/components/shared/metric-block.tsx` (new),
`frontend/src/pages/admin-usage.router.test.tsx` (new),
`frontend/src/pages/admin-usage.tsx`, `frontend/src/pages/admin-usage.test.tsx`,
`frontend/src/pages/billing.tsx`, `frontend/src/lib/billing-units.ts`,
`frontend/src/types/admin.ts`, and
`frontend/src/components/navigation/command-palette.tsx`.
Documentation/evidence: `CLAUDE.md`, this report, `admin-query-explain.json`,
and both Usage screenshots.

No review finding is deferred, and no verification failure remains. The existing
data limitations above still apply. The follow-up is a new commit on
`admin-usage-dashboard`, titled
`fix(admin): optimize audit totals and align usage dashboard`.
No versions changed, no push, and no PR was opened.
