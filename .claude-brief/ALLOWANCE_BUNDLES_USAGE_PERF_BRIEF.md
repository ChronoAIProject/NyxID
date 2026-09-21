# Brief: multi-metric allowance bundles + admin usage dashboard performance

Repository: NyxID, branch `multi-metric-allowances-usage-perf` (this worktree), based on main
`0c314264` (0.28.0 + fixes). Read `CLAUDE.md` (Rules 1-5, the billing bullets in Rule 5),
`DESIGN.md` (mandatory before any UI decision), `docs/BILLING_UI_GLOSSARY.md` (§ credits and
§ Admin usage), and `docs/USAGE_BILLING_LAGO_SPEC.md` before starting.

## Part A — Multi-metric allowances ("allowance bundles")

### Problem
An admin who prices a service on several units (0.27.0 components: input / output /
cache-read / cache-write tokens, images, …) must today create one allowance per unit through the
dialog (`components/admin-credits/credits-dialogs.tsx::AllowanceDialog`), re-entering the
service, recurrence and targets each time. The screenshot in the issue shows five separate rows for
`chrono-llm-public` × the same organization targets.

### Design (additive; the money path must not change)
Keep one `UsageAllowance` row per metric — funding (`services/billing/funding.rs`), periods,
ledger references, reconcile and the user-facing `/billing/allowances` all key on a single-metric
row and must stay byte-for-byte unchanged in behaviour. Group rows instead:

- Model: add `bundle_id: Option<String>` to `models/usage_allowance.rs` (serde default, UUID v4
  string, indexed non-unique + sparse). Rows created together share it. Legacy rows keep `None`;
  their bundle key is their own `id` (`bundle_id.unwrap_or(id)` everywhere).
- Service (`services/billing/allowances.rs`):
  - `create_allowance_bundle(input)` with `service_ref`, `target_kind` + target lists,
    `units: Vec<{ metric, quantity, recurrence }>` (1..=16 units, metrics unique, each in
    `metric_resolution::allowance_metrics(service)`, quantity validated as today). Inserts all rows
    with one fresh `bundle_id`, atomically: use a MongoDB transaction when the server supports it
    (find how this codebase already handles standalone-vs-replica-set for its existing
    multi-document writes and follow that convention; the test replica set is on 27019). On any
    validation failure nothing is written.
  - `replace_allowance_bundle(bundle_key, input)` reconciles a bundle to the desired state:
    same service and targets for all rows; per unit: existing metric row → update quantity /
    recurrence (via the existing `update_allowance` validation), missing metric → insert with the
    bundle id, metric no longer listed → set `is_active = false` (allowances are never deleted).
    When `bundle_key` is a legacy row id, stamp that row (and new rows) with `bundle_id =
    bundle_key`. Changing the service of a bundle is rejected (400) — create a new bundle.
  - `set_bundle_active(bundle_key, bool)` toggles every row of the bundle.
  - Existing single-row `create_allowance` / `update_allowance` stay for compatibility.
- Handlers/routes (admin router, `require_admin` for writes as today):
  - `POST /api/v1/admin/credits/allowances` gains an optional `units` array; when present the
    legacy `metric/quantity/recurrence` fields must be absent (400 otherwise) and the response is
    `{ bundle_id, allowances: [...] }`. Without `units` the legacy behaviour is untouched.
  - `PUT /api/v1/admin/credits/allowances/bundles/{bundle_key}` → replace; `PATCH
    /api/v1/admin/credits/allowances/bundles/{bundle_key}` with `{ is_active }` → toggle.
  - `UsageAllowanceResponse` gains `bundle_id: Option<String>` (also in the user-facing balance
    response, read-only). OpenAPI registered.
  - Audit: metadata-only entries for bundle create/replace/toggle (ids, counts, metrics), matching
    the existing allowance audit events.
- Frontend (`pages/admin-credits.tsx`, `components/admin-credits/*`, `schemas/billing-credits.ts`,
  `hooks/use-billing-credits.ts`):
  - The Free allowances table groups rows by bundle key: one row per bundle showing service,
    a units column listing every metric with its quantity and recurrence (e.g. "100,000,000 input
    tokens · daily"), targets, an aggregate status (Active / Partially disabled / Disabled), and
    actions Edit / Disable-Enable for the whole bundle. Legacy singleton rows render identically.
  - The dialog becomes a bundle editor: service + targets once, then a units table (metric select
    limited to the service's `allowance_metrics` minus already-chosen ones, quantity, recurrence)
    with Add unit / Remove unit; at least one unit; the preview line lists every unit. Editing a
    bundle loads all its units; removing a unit disables that row on save (explain this inline).
    Form via `useAppForm` + Zod (`useFieldArray` is fine). Keep the existing change-review
    confirmation (`useChangeReview`) and show a per-unit before/after diff.
  - Types/schemas updated; the CLI has no allowance commands so no CLI work.
- Docs: update `docs/BILLING_UI_GLOSSARY.md` credits section and the CLAUDE.md credit-benefits
  bullet (one sentence: bundles are a grouping of single-metric rows; funding is unchanged).

## Part B — `/admin/usage` performance (production is timing out)

### Evidence (measured on production 0.28.0 on 2026-09-21, admin session)
| window / filter | result |
| --- | --- |
| 5 min | 200 in 0.32 s, 1,942 events |
| 30 min | 200 in 0.97 s, 13,126 events |
| 2 h | 200 in 2.78 s, 50,287 events |
| 24 h (default) | **503 timeout at 20 s** |
| 24 h, `user=<admin>` | **503 timeout** (actor/owner filter is not indexed, full window scanned) |
| 24 h, `service=chrono-llm-public` | 200 in 9.5 s for 155 matching events (full window scanned) |
| 7 d | 503 timeout |
| personal `/billing/usage` 24 h | 200 in 0.13 s |

Cost is linear at ≈55 µs per meter row; production writes ≈25k meter rows per hour, so 24 h ≈
600k rows and 31 d ≈ 18M rows. Scanning raw `usage_meter` rows per request can never meet the
target, whatever the index. The fix is pre-aggregation.

### Target
On the seeded benchmark below (production density), p50 server time: 24 h < 500 ms, 7 d < 1 s,
31 d < 2 s, with or without user / service filters; no request may approach the 20 s bound. The
response contract of `GET /api/v1/admin/usage` (fields, dedupe rules, exact/legacy cost
convention, ranking semantics, paging) must stay identical except for one new field
`freshness { rolled_up_through: DateTime, tail_rows: i64 }` explained below.

### Design
1. **Hourly rollup collection** `usage_rollup_hourly` (new model in `models/`, `COLLECTION_NAME`):
   one document per (hour bucket UTC, actor_user_id, billing_owner_id, service_id, service_slug,
   credential_class, metric, lago_metric_code, layer, model, billable, exact) — i.e. exactly the
   dimensions the current `base_pipeline` groups on minus `api_key_id` and `lago_acked` (neither is
   used by the admin response; drop them) — holding the same additive measures the current group
   stage computes (quantity, events, requests, exact/legacy event counts, legacy_quantity,
   legacy_allowance_quantity, legacy_grant, the four funding micros sums as i64 (saturating,
   micros are integers), the four token-class sums) plus `rows_folded`. `_id` = deterministic
   string/hash of the key so folds are idempotent upserts with `$inc`. Indexes: `{ hour: 1 }`
   and `{ actor: 1, hour: 1 }`, `{ owner: 1, hour: 1 }`, `{ service_slug: 1, hour: 1 }`
   (choose after explain; keep them minimal and additive).
2. **Exactly-once fold.** A background worker (spawned next to `spawn_reconcile_worker`, same
   `BILLING_RECONCILE_INTERVAL_SECS` cadence but at most 60 s between ticks — derive, do not add an
   env var; `0` disables like reconcile) folds terminal rows (`status = finalized`, or
   `dead_letter` with `forwarded = true`, `quantity != null` — the exact predicate the dashboard
   uses today) into the rollup and marks each row `rolled_up_at`. A row must be counted exactly
   once even across crashes and concurrent replicas: claim a bounded batch (≤ 2,000 rows) with an
   atomic claim marker, `$inc` the rollup documents, then clear the claim and set `rolled_up_at`;
   on restart, re-fold only rows whose claim is stale AND whose fold is provably not applied
   (e.g. record the batch id inside the rollup documents' `applied_batches` capped list, or use a
   multi-document transaction where the deployment supports it, with a documented non-transactional
   fallback that still guarantees exactly-once via the batch-id check). Write a test that kills the
   fold between the `$inc` and the mark and proves no double count after recovery.
   Backfill: the worker walks history newest-first in bounded batches per tick until every terminal
   row is folded; a fresh deploy therefore converges within hours without operator action. The
   sweep must be index-supported: pick between a partial index on `{ finalized_at: 1 }` for
   `{ rolled_up_at: { $exists: false } }` style markers (note MongoDB partial filters cannot
   express `$exists: false` — use a positive marker such as `rollup_pending: true` set at row
   creation for new rows, plus a one-time backfill pass for pre-existing rows keyed on the existing
   `{ status: 1, created_at: -1 }` index) — prove the plan with `explain()`.
   Rows that later change (a dead-letter row becoming forwarded, a legacy row gaining exact
   settlement) must be handled: define the terminal predicate so a row is folded only once it can
   no longer change its measured fields, and add a test for the dead-letter → forwarded path.
3. **Query path.** `admin_usage_service::get_usage` reads the rollup for whole hours inside the
   window plus a **live tail**: raw rows in the window that are not yet folded (`rollup_pending`
   rows bounded by `created_at`, index-supported) so the dashboard is current to the second; the
   partial first/last hours are handled by folding only complete hour buckets or by keeping
   bucket + `created_at` range filters consistent — document the choice and test window edges.
   Both sources feed the same rate lookup / rollup / ranking stages (rate join stays in MongoDB
   or moves to a one-shot Rust map of `billing_rate_cache`, whichever explain shows is cheaper —
   the cache has few rows). One aggregation for summary + ranking + service options via `$facet`
   (today three pipelines scan the window three times). Response adds
   `freshness { rolled_up_through, tail_rows }`; the page shows "Live · includes N unfolded
   rows" or the watermark in the footer line.
4. **Frontend**: no functional change beyond the freshness line; keep `retry: false`; the
   expanded per-user breakdown must reuse the same fast path.
5. **Retention**: rollup documents are permanent operational data (small); raw rows keep their
   existing TTL. Document that dashboards for windows older than the raw TTL come purely from
   rollups.
6. **Benchmark** (`#[ignore]`, like `admin_query_execution_stats`): seed 3 days at production
   density (≈25k rows/hour → ≈1.8M rows across 15 users, 17 services, 6 credential classes, 5
   metrics, with component and resale rows and both funding conventions), run the fold to
   completion, then measure the endpoint for 24 h / 7 d (windows may exceed the seed; that is fine)
   / 31 d and with user and service filters, reporting docs examined, keys examined and ms, and
   asserting the same totals as the raw-scan implementation on the same data (keep the raw
   pipeline available under `#[cfg(test)]` as the oracle). Also measure fold throughput.

### Guard rails
- No existing index modified or dropped; new indexes additive and justified by explain.
- No new environment variables. Migration-free (backfill is automatic, idempotent, bounded).
- Layer rules, `AppError` conventions, no PII in logs/telemetry; the fold logs counts only.
- `usage_meter` writes on the hot path may only gain the cheap `rollup_pending: true` marker.

## Verification you must run and report (paste real output summaries)
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`
- `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server -- --test-threads 2`
- `cargo test -p nyxid-cli`
- `cd frontend && npm run lint && npm run test && npm run build`
- `npm --prefix frontend run build:wizard` + `cargo test -p nyxid-cli --test wizard_bundle_freshness` if any bundled file changed
- The ignored benchmark with the numbers above; check `df -h /System/Volumes/Data` before long
  cargo runs (this machine has filled its disk before).

Commit on this branch in conventional-commit style (`feat(billing): allowance bundles`,
`perf(admin): hourly usage rollups`). Do not push, do not open a PR, do not bump versions. Finish
by writing `.claude-brief/ALLOWANCE_BUNDLES_USAGE_PERF_REPORT.md` with design decisions, files
touched, exact commands and results, benchmark numbers, and known gaps (aim for none).
