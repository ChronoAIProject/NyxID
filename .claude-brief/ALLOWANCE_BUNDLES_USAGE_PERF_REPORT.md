# Allowance bundles and admin usage performance report

Branch: `multi-metric-allowances-usage-perf`. No version changes, pushes, or PRs.

## Design

### Allowance bundles

Bundles group independent single-metric `usage_allowances` rows with an optional, sparse-indexed UUID `bundle_id`. Legacy rows use their own IDs. Create validates every unit and target before inserting anything; replacement retains existing row/period identities, disables removed metrics, and adds new metrics. All three bundle writes are transactional, following the repository's fail-closed convention for multi-document mutations on standalone MongoDB. Legacy single-row APIs remain available. Funding, reconciliation, ledger references, and consumption periods retain their existing behavior.

The POST endpoint distinguishes omitted legacy fields from explicit nulls and rejects mixed `units`/legacy payloads. PUT replaces a bundle; PATCH toggles every row. Writes require admin permission and record metadata-only audits. Admin and user response DTOs expose the optional bundle ID; OpenAPI includes the new contracts.

The admin table and mobile cards group units, display aggregate status, and provide whole-bundle actions. The editor uses `useAppForm`, Zod and `useFieldArray`, service-specific available metrics, shared targets, per-unit quantity/recurrence, previews and per-unit change review (including the disabled status of units that saving will enable). Removing a unit disables it; saving enables listed units. Toggle applies to all historical rows, including removed units, as required by the brief.

### Usage rollups

Permanent hourly summaries use deterministic SHA-256 IDs over the requested UTC hour and reporting dimensions. The hot meter insertion only gains `rollup_pending: true`. Missing legacy markers participate in the same indexed pending scan, so no migration or manual backfill is needed. Newest-first batches contain at most 2,000 rows; ticks process at most 100 batches with a 20-second inter-batch time budget. Cadence is `min(BILLING_RECONCILE_INTERVAL_SECS, 60)`; zero disables it. No environment variables were added.

A singleton durable journal atomically claims immutable batch inputs. Monotonically increasing batch sequences fence every summary update. Replica sets atomically commit summary increments, source markers and journal advancement with majority/journal durability. The standalone fallback applies the same fenced increments before source marking and advances the journal last. Replay cannot count a batch twice, even after a crash between increments and marking. Readers validate the journal around idle reads and use snapshot reads during active replica-set folds. Transient snapshot expiry retries within the existing request timeout. Standalone reads always validate the journal.

Only completed UTC hours fold. Free finalized rows are stable immediately; charged rows require released reservations, completed funding settlement (or absent legacy funding), and stable Lago acknowledgement/dead-letter status. Mutable rows remain live, including late forwarding and settlement. Integer microcredit sums saturate without floating-point conversion.

The fast query combines full-hour summaries with disjoint raw ranges for unfolded interior rows and partial first/last hours, using additive indexes. One facet produces summary, ranking, service options and freshness. A single rate-cache fetch supplies a literal MongoDB lookup map. Single-partition summaries retain a canonical display key and an exact Decimal128 mirror of their four integer cost sums. These derived query fields commit with the integer measures and let MongoDB group before partition expansion, without per-document decimal conversion. Multi-partition summaries retain the full compatibility path and use an additive partial hour index. Raw terminal filtering uses an expression alongside the indexed status/date range, avoiding unbounded status-only OR plans. Service options remain independent of the selected service, so a service-only index would not improve this combined scan and was omitted. Actor/owner filters use their indexes.

**Compatibility finding:** although API key and acknowledgement are absent from the public dashboard, the old grouping uses them for legacy truncation and missing-rate masking. Dropping those boundaries changes totals. Internal `cost_partitions` preserve them without adding them to the hourly primary key. Canonical BSON field order also lets raw-tail groups merge with folded groups before truncation. A raw-oracle regression covers both behaviors, including a fractional legacy cost split across the two sources.

The existing timeout bounds and `retry: false` remain. Expanded user details use the same fast endpoint. Freshness reports the gap-free watermark and unfolded-row count; the UI shows the live-tail count.

### Retention boundary

Hourly summaries have no TTL; existing raw TTL indexes and retention behavior are unchanged. Whole-hour history survives raw expiration. The existing acknowledged-row retention is 30 days after cleanup scheduling (`ACKED_RETENTION_DAYS`). Exact arbitrary partial-hour boundaries require retained raw events: hourly aggregates cannot reconstruct timestamps within a bucket after those events expire. This limitation is documented in the billing glossary. No additional event-history store or fabricated prorating was introduced.

## Verification

All required checks pass. Commands ran from the repository root; `npm --prefix frontend` is equivalent to running each script inside `frontend/`.

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed. |
| Backend command below | **6,404 passed**, 0 failed, 2 intentionally ignored; **405.62 s** on the final production implementation. |
| `cargo test -p nyxid-cli` | **1,277 passed**: 1,246 unit tests plus integration suites of 10, 5, 13, 2 and 1. |
| `npm --prefix frontend run lint` | Exit 0: 0 errors, **27 pre-existing warnings**. |
| `npm --prefix frontend run test` | **346 files / 3,491 tests passed**; 76.22 s on the final frontend. |
| `npm --prefix frontend run build` | Passed, including the mock-scenario footprint assertion and credential-accept build. |
| `npm --prefix frontend run build:wizard` | Passed; generated HTML/CSS refreshed. Source manifest/hash unchanged. |
| `cargo test -p nyxid-cli --test wizard_bundle_freshness` | **1 passed**. |
| Ignored production-density benchmark below | **1 passed**, 12/12 latency budgets and raw-oracle comparisons passed; **782.76 s** for the final refold and query run. |
| `git diff --check` | Passed. |

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server -- --test-threads 2
```

Focused real-database tests cover bundle validation/atomicity, legacy singleton upgrades, retained allowance/period IDs, bundle enable/disable, admin-only writes, mixed legacy/unit payloads including explicit nulls, concurrent fold helpers, crash-after-increment replay, transaction abort, reads during an uncommitted fold, late forwarding/settlement, exact time-window edges, raw TTL deletion, and cross-source legacy rounding/unknown-rate masking. The fold tests passed 3/3; reporting tests passed 8/8 with 2 ignored benchmarks.

`df -h /System/Volumes/Data` ran before long Cargo commands. The last pre-benchmark check showed 50 GiB free; the final cleanup check showed 52 GiB free. No existing production index was modified or dropped, no new configuration environment variable was added, and the billing money path was unchanged.

## Production-density benchmark

Environment: **Apple M2, 16 GiB RAM; MongoDB 8.0.23**, replica set `nyxid1530` on `127.0.0.1:27019`. WiredTiger cache: **256 MiB**. Snapshot history: **5 s**. These server settings were not changed.

Seed: **1,800,000 historical rows** (72 hours × 25,000), plus **125 live rows**, across 15 users, 17 services, 6 credential classes and 5 metrics. Includes components, resale, free usage, exact settlements and legacy pricing. The 7-day and 31-day windows contain the same 3-day seed, as explicitly permitted by the brief. Each case measures five complete `get_usage` service calls, including its database reads, validation fences and identity enrichment; HTTP transport/middleware is excluded. All costs, funding values, quantities, request/event counts, token classes and distinct counts in `totals` equal the original raw-scan oracle on every measured call.

| Window | Filter | p50 ms | Max ms | Documents examined | Keys examined |
|---|---|---:|---:|---:|---:|
| 1 d | none | 360.5 | 463.6 | 78,301 | 78,306 |
| 1 d | user | 63.7 | 97.4 | 7,016 | 10,458 |
| 1 d | service | 354.1 | 378.7 | 78,301 | 78,306 |
| 1 d | both | 62.6 | 64.4 | 7,016 | 10,458 |
| 7 d | none | 633.9 | 781.4 | 188,765 | 188,769 |
| 7 d | user | 191.3 | 203.3 | 17,865 | 25,220 |
| 7 d | service | 600.8 | 663.8 | 188,765 | 188,769 |
| 7 d | both | 145.0 | 184.5 | 17,865 | 25,220 |
| 31 d | none | 655.0 | 687.6 | 188,765 | 188,769 |
| 31 d | user | 174.0 | 205.1 | 17,865 | 25,220 |
| 31 d | service | 611.8 | 664.5 | 188,765 | 188,769 |
| 31 d | both | 158.1 | 188.1 | 17,865 | 25,220 |

All p50s are below **500 / 1,000 / 2,000 ms** respectively. The slowest individual call was **781.4 ms**.

Folding all 1.8 million historical rows took **715.83 s**, or **2,514.6 rows/s**. With the worker's 20-second budget on a 60-second tick, that sustained rate implies roughly 838 rows/s before ordinary scheduling overhead, or about six hours for 18 million rows. This is an extrapolation, not a separate 18-million-row benchmark. The raw fixture remained unchanged when the final worker rebuilt its derived rollups, so this throughput includes the final query-accelerator writes.

The initial newest-first fold explain examined **2,000 documents / 2,000 keys in 9 ms**, using the existing `usage_meter_admin_window` index. After backfill, live interior reads use `usage_rollup_pending_window`; partial-hour reads use the existing status/date index; actor filters use the new actor/date and existing owner/date indexes. Hourly queries use `hour_1` or actor/hour and owner/hour indexes. `usage_rollup_partitioned_hour` lets the compatibility branch examine zero documents when all summaries have a single display partition. These plans justify the additive indexes; no service index is needed because the service picker requires all services within the selected window/user scope.

Full plans, execution statistics and numeric measurements are in [`allowance-bundles-usage-benchmark.json`](allowance-bundles-usage-benchmark.json). The final evidence counter excludes nested SBE copies of cursor totals; the original timing measurements and oracle comparisons are unchanged. That counter-only correction was checked against the saved complete explains and followed by formatting and Clippy verification; it changes no production code.

Fresh, self-contained reproduction:

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server hourly_rollup_production_density_benchmark -- --ignored --nocapture --test-threads 1
```

The benchmark retains its disposable synthetic database on failure for diagnosis. An explicit `nyxid_benchmark_*` database in the existing test URI resumes it; other names are rejected. Before the final passing run, only derived rollups/state and source fold markers were reset in the retained fixture, and the stored fold timer was cleared. The final command therefore refolded **all 1.8 million rows** with the final worker:

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/nyxid_benchmark_0c66699dc787459a91380e0b25311e8b?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server hourly_rollup_production_density_benchmark -- --ignored --nocapture --test-threads 1
```

The successful run dropped that disposable database. Earlier unsuccessful/interrupted synthetic databases were also removed. Profiling scratch scripts were outside the repository.

## Resolved verification findings

- Bounded bulk commands and a shared update program avoid MongoDB's 16 MiB command limit and repeated expression compilation during folding.
- Canonical BSON key ordering combines raw and folded display groups before fractional legacy truncation.
- Idle journal validation avoids an unnecessarily old snapshot on the test server's short history window; active-fold snapshot errors retry within the existing request bound.
- An expression-based terminal check prevents an OR branch from selecting an unbounded status-only raw scan (1.8 million rows examined for a 125-row tail).
- Derived exact decimal measures and early grouping removed the reporting bottleneck that initially produced a 714 ms 24-hour p50.

## Known limitations

There are no outstanding test failures or benchmark target misses. The intentional retention limitation is the partial-hour boundary described above: once raw events expire, whole-hour summaries remain exact, but arbitrary timestamps inside an expired hour cannot be reconstructed. Bundle writes require a replica set or mongos, following the repository's transaction convention; standalone rollup folding still has an exactly-once fallback.

## Files touched

- `.claude-brief/ALLOWANCE_BUNDLES_USAGE_PERF_REPORT.md`
- `.claude-brief/allowance-bundles-usage-benchmark.json`
- `CLAUDE.md`
- `backend/src/api_docs.rs`
- `backend/src/billing_integration_tests/usage.rs`
- `backend/src/db.rs`
- `backend/src/grant_visibility_tests.rs`
- `backend/src/handlers/billing_credits.rs`
- `backend/src/handlers/billing_credits/target_tests.rs`
- `backend/src/main.rs`
- `backend/src/models/mod.rs`
- `backend/src/models/usage_allowance.rs`
- `backend/src/models/usage_meter.rs`
- `backend/src/models/usage_rollup_hourly.rs`
- `backend/src/models/usage_rollup_state.rs`
- `backend/src/routes.rs`
- `backend/src/services/admin_usage_service.rs`
- `backend/src/services/admin_usage_service/tests.rs`
- `backend/src/services/billing/allowances.rs`
- `backend/src/services/billing/funding.rs`
- `backend/src/services/billing/funding/target_tests.rs`
- `backend/src/services/billing/lago_client.rs`
- `backend/src/services/billing/meter.rs`
- `backend/src/services/billing/mod.rs`
- `backend/src/services/billing/reconcile.rs`
- `backend/src/services/billing/targets/tests.rs`
- `backend/src/services/billing/usage_rollup.rs`
- `backend/src/services/billing/usage_rollup/tests.rs`
- `backend/src/services/billing/webhook.rs`
- `cli/src/wizard/assets/index.html`
- `docs/BILLING_UI_GLOSSARY.md`
- `frontend/src/components/admin-credits/allowance-bundles.test.ts`
- `frontend/src/components/admin-credits/allowance-bundles.ts`
- `frontend/src/components/admin-credits/allowances-table.tsx`
- `frontend/src/components/admin-credits/credits-dialogs.test.tsx`
- `frontend/src/components/admin-credits/credits-dialogs.tsx`
- `frontend/src/components/admin-credits/recipient-targets.test.tsx`
- `frontend/src/hooks/use-billing-credits.ts`
- `frontend/src/pages/admin-credits-safety.test.tsx`
- `frontend/src/pages/admin-credits.tsx`
- `frontend/src/pages/admin-usage.tsx`
- `frontend/src/schemas/admin-usage.ts`
- `frontend/src/schemas/billing-credits.ts`

## Review round 1

This section supersedes the initial cutoff, retry, acknowledgement, editor, index, and performance statements above. All ten findings on `5a210635` are addressed; no version change, push, or PR is included.

### Changes and reasoning

1. **Production scan budget:** presets now start at `hour(now - duration)` and end at `now`, with the exact bounds still visible. Stable rows fold below `now - 60 seconds`, including the current hour. A defaulted journal `folded_before` bound is published atomically with each claim, before any summary increment; reads use the partially filled end bucket only while that bound proves it contains no source outside the request. Concurrent advancement revalidates the bound. Historical custom edges keep disjoint indexed raw ranges. The pending scan is the top-level aggregation with an explicit pending-index hint: the first density run exposed MongoDB selecting the old status/date index inside `unionWith` and fetching 607,208 already-folded sources. Rollups and custom edges now join that root scan. The old benchmark also fetched approximately 60,000 hourly summary documents for 24 hours; three additive covering indexes (window, actor, owner) eliminate those document fetches on the common single-partition path. The compatibility path and all prior indexes remain intact. Covering reductions sum stored nonnegative integer costs, clamp overflow to `i64::MAX`, then convert only reduced groups to Decimal128 for the shared pricing stages; tests prove exactness at `2^53 + 1`, near `i64::MAX`, and above saturation against the raw oracle.
2. **Bounded availability:** all three contention/snapshot-expiry paths share an eight-attempt budget. An unvalidated completed result is returned after exhaustion; if every snapshot expired, one ordinary aggregation using conservative raw edges supplies the result. `freshness.validated: false` and the UI's “Updating totals” identify this temporary result. Persisted folding remains exactly once. A test holds the journal batch active throughout and requires standalone-mode HTTP 200 within two seconds.
3. **Lago outages:** released, settled exact rows fold without ack. Legacy billable rows still need ack/dead-letter to freeze their display partition. Unacked charged rows also require forwarding: `reconcile::mark_dead_letter` changes status without setting `forwarded`, so an anomalous finalized/unforwarded row could leave the dashboard predicate. Such rows stay raw; a regression exercises their disappearance without a stale summary. Forwarding is monotonic: meter creation initializes false, `meter::mark_forwarded` sets true before dispatch, and finalization/reconciliation never reset it. The test covers current-hour folding, exact/unacked versus legacy/unacked rows, the 60-second edge, subsequent ack, watermark advancement, and custom boundaries.
4. **Worker survival:** every `expect`, `unwrap`, and `unreachable!` path in the fold module has been removed. Missing/malformed state, invalid sequence fences, and invalid group documents return `AppError`. The worker warns without identifiers and retries on a later tick. Tests cover missing, malformed, and exhausted-sequence journal state.
5. **Backfill budget:** the worker gives a tick 45 seconds while the gap-free watermark is more than two hours old, retaining 20 seconds once caught up and the existing 100-batch cap. Cadence and configuration are unchanged. A unit test covers both budgets.
6. **Watermark UX:** before the request start the watermark renders “Backfilling history · rollups complete through <time>”; otherwise the footer shows the live unfolded count. Both branches and the unvalidated indicator are tested. The watermark is now an exclusive source-time bound, not an hour boundary; the glossary documents its initialization and unstable-row behavior.
7. **Bundle transaction consistency:** create now uses `transaction_result` and `map_transaction_error`, matching replacement and toggles. A focused test against a temporary real standalone MongoDB confirms all three fail closed, preserve a legacy row, and create no partial bundle.
8. **Inactive units:** editing loads active rows only. Saving a quantity change does not propose re-enabling removed metrics; explicitly adding one does. Change review now derives the before-values from active form defaults. Regression tests cover both save paths, an entirely disabled bundle, and whole-bundle toggles. The inline explanation and glossary describe restoration.
9. **Table consistency:** allowances use the sibling's `overflow-x-auto rounded-lg border border-border` container and inline Pencil plus Disable/Enable controls, per the review's explicit design direction. Recurrences are capitalized. Mobile actions remain accessible without overlapping the service name.
10. **Hook boundary:** both bundle hooks normalize target lists before schema validation and request submission. A hook regression switches to `all_users` while stale user/org/group selections remain and asserts empty arrays for both POST and PUT.

### Verification and benchmark

| Command | Review round 1 result |
|---|---|
| `cargo fmt --all -- --check` | Passed. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed after simplifying two boolean expressions; final run 1m 47s. |
| `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server -- --test-threads 2` | **6,410 passed**, 0 failed, 2 intentionally ignored; **417.85 s**. Includes all six added backend regressions. |
| `cargo test -p nyxid-cli` | **1,277 passed** (1,246 unit; integration suites 10 + 5 + 13 + 2 + 1). |
| `npm --prefix frontend run lint` | Passed: 0 errors, 27 existing warnings. |
| `npm --prefix frontend run test` | **346 files / 3,498 tests passed**; **56.38 s**. |
| `npm --prefix frontend run build` | Passed, including credential-accept and mock-footprint checks. |
| `npm --prefix frontend run build:wizard` | Passed; generated assets and source hash remain unchanged. |
| `cargo test -p nyxid-cli --test wizard_bundle_freshness` | **1 passed**. |
| Actual standalone checks below | **2 passed**, including HTTP 200 with `validated: false`; **0.58 s** for both tests. |
| `git diff --check` | Passed. |

Standalone verification used a disposable local server, subsequently stopped and removed:

```sh
mongod --dbpath /tmp/nyxid-review1-standalone-final --port 27020 --bind_ip 127.0.0.1 --wiredTigerCacheSizeGB 0.25 --logpath /tmp/nyxid-review1-standalone-final/mongod.log --pidfilepath /tmp/nyxid-review1-standalone-final/mongod.pid --fork
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27020/?directConnection=true' target/debug/deps/nyxid_server-8697d3d03a01e14a --exact services::billing::allowances::bundle_tests::bundle_writes_fail_closed_on_standalone services::admin_usage_service::tests::persistently_active_standalone_batch_returns_unvalidated_success_within_bound --nocapture
```

The replica-set tests exercise the forced standalone aggregation mode too; the additional run above verifies the real topology decision and full authorized HTTP handler. No existing test was removed. Initial focused failures from the intentionally changed cutoff and inline action selectors were updated to the new behavior. The first density run exposed the broad raw index choice; its successor met the document budget but narrowly missed 24-hour latency and exposed the slower Decimal128 reduction. Both were fixed before the final density run.

`df -h /System/Volumes/Data` ran before long Cargo commands and throughout verification. There were **46 GiB free** before the final benchmark and **47 GiB free** after fixture cleanup. No application environment variables, billing funding paths, versions, or pre-existing index definitions changed.

### Final production-density benchmark

Environment remains Apple M2 / 16 GiB, MongoDB 8.0.23 replica set `nyxid1530`, 256 MiB WiredTiger cache. The retained disposable fixture contains **1,800,000 rows uniformly across the 72 hours ending at the benchmark time**, including the partial current hour, plus **125 extra live rows**. Every exact-settled charged row is deliberately **unacknowledged**. All legacy billable rows are acknowledged. This exercises a prolonged Lago outage without making its exact traffic accumulate in the live tail.

For the final run, the disposable derived rollup/state collections and source fold markers were reset and the previous fold timer removed. The raw event contents and raw indexes were retained. This also removed experimental derived indexes, so the final worker measures only the final index set. The final test refolds every eligible event and preserves the one-minute cutoff tail. No other heavy verification runs overlap its measurements.

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server hourly_rollup_production_density_benchmark -- --ignored --nocapture --test-threads 1
# Final full refold of that retained disposable raw fixture:
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/nyxid_benchmark_39277ed65dea4712aca15bb79290c1a5?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server hourly_rollup_production_density_benchmark -- --ignored --nocapture --test-threads 1
```

| Window | Filter | p50 ms | Max ms | Documents examined | Keys examined |
|---|---|---:|---:|---:|---:|
| 24 h | none | 382.0 | 448.9 | 541 | 65,443 |
| 24 h | user | 58.5 | 95.0 | 541 | 9,214 |
| 24 h | service | 353.6 | 368.0 | 541 | 65,443 |
| 24 h | both | 60.2 | 67.2 | 541 | 9,214 |
| 7 d | none | 893.8 | 1064.7 | 541 | 191,082 |
| 7 d | user | 131.8 | 170.6 | 541 | 25,998 |
| 7 d | service | 866.9 | 948.5 | 541 | 191,082 |
| 7 d | both | 124.9 | 130.8 | 541 | 25,998 |
| 31 d | none | 896.1 | 942.8 | 541 | 191,083 |
| 31 d | user | 131.9 | 146.4 | 541 | 25,998 |
| 31 d | service | 865.5 | 932.5 | 541 | 191,083 |
| 31 d | both | 123.5 | 134.8 | 541 | 25,998 |

**12/12 cases passed**, including all five raw-oracle comparisons per case (60 total). Worst p50s are **382.0 / 893.8 / 896.1 ms** for 24 h / 7 d / 31 d, below **500 / 1,000 / 2,000 ms**. The slowest individual call was **1,064.7 ms**, far below the request guard. The final ignored test passed in **832.85 s**, excluding compilation. Full evidence is in [`allowance-bundles-usage-benchmark.json`](allowance-bundles-usage-benchmark.json).

All cases examine **541 raw documents and zero hourly documents**. This is the production-density one-minute tail (416 rows) plus 125 extra live rows. The 24-hour total falls from the reviewed **78,301** to **541 documents**, a **99.31% reduction**. The hinted pending index reads 542 keys; the single-partition hourly path uses the new covering indexes, and the compatibility branch uses its existing partial index. Keys examined remain explicit in the table: the cost of traversing compact hourly index entries is not hidden. Counts refer to the combined usage aggregation's execution stats, as before; latency includes the full service read and enrichment. The 7-day and 31-day cases contain the same three-day seed, as allowed by the brief; 31 days uses a custom range within the existing maximum.

The final worker folded **1,799,584 rows** in **755.34 s**, or **2,382.5 rows/s**. With the 45-second budget in each 60-second backfill tick, this corresponds to **1,786.9 rows/s** after duty-cycle adjustment, before scheduling overhead (an extrapolation from continuous folding, not a separately timed scheduled-worker run). The prior report's 20/60-second estimate was 838 rows/s. The final fold uses only the final indexes and includes all accelerator writes. The fixture is dropped on benchmark success; its deletion was verified afterward.

### Remaining behavior and scope

No review finding or verification failure remains open. Custom partial edge hours intentionally use indexed raw scans and may cost more than presets. Exact historical partial-hour boundaries still require retained raw events; complete-hour history remains exact after raw TTL expiry. An exhausted read-validation budget returns explicitly unvalidated, temporarily approximate totals. Legacy unacknowledged priced rows intentionally remain live, while exact traffic no longer accumulates during Lago outages. Bundle mutations retain the repository's standalone fail-closed transaction requirement.

### Files changed in this review

- `.claude-brief/ALLOWANCE_BUNDLES_USAGE_PERF_REPORT.md`
- `.claude-brief/allowance-bundles-usage-benchmark.json`
- `backend/src/models/usage_rollup_state.rs`
- `backend/src/services/admin_usage_service.rs`
- `backend/src/services/admin_usage_service/tests.rs`
- `backend/src/services/billing/allowances.rs`
- `backend/src/services/billing/usage_rollup.rs`
- `backend/src/services/billing/usage_rollup/tests.rs`
- `docs/BILLING_UI_GLOSSARY.md`
- `frontend/src/components/admin-credits/allowance-bundles.test.ts`
- `frontend/src/components/admin-credits/allowance-bundles.ts`
- `frontend/src/components/admin-credits/allowances-table.tsx`
- `frontend/src/components/admin-credits/credits-dialogs.tsx`
- `frontend/src/hooks/use-billing-credits.test.tsx`
- `frontend/src/hooks/use-billing-credits.ts`
- `frontend/src/pages/admin-credits-safety.test.tsx`
- `frontend/src/pages/admin-credits.tsx`
- `frontend/src/pages/admin-usage.test.tsx`
- `frontend/src/pages/admin-usage.tsx`
- `frontend/src/schemas/admin-usage.ts`
