# Brief: platform-wide admin usage dashboard + audit-log query optimization

Repository: NyxID, branch `admin-usage-dashboard` (this worktree), based on main 167025ae (0.27.0).
Read `CLAUDE.md` (Critical Rules 2, 3, 4, 5 billing bullets), `DESIGN.md` (mandatory before any
UI decision), `docs/BILLING_UI_GLOSSARY.md`, `docs/PLATFORM_KEYS_AND_INFERENCE.md` (billing lanes
section) and `docs/USAGE_BILLING_LAGO_SPEC.md` §3.2 before starting.

## Part A — Admin "Usage" section

### Goal
Platform admins (and read-only operators) need one place to see usage across ALL users and ALL
services, regardless of credential lane (NyxID platform key, BYOK, agent override, node-managed,
no-auth), for a chosen window (default last 24 hours, maximum 1 month), with a user ranking by
service and metric, filterable by user (shown by display name and email, never bare ids) and by
service.

### Data source
`usage_meter` rows (`models/usage_meter.rs`). Facts you must respect:
- One request produces one PRIMARY platform row (`transaction_id = {billing_request_id}:platform`)
  and, since 0.27.0, zero or more component rows (`...:platform:component:{code}`) plus an
  optional resale row. Component rows duplicate the request; NEVER count requests, users or
  `token_breakdown` more than once per `billing_request_id`. Count requests from primary
  platform rows only. Sum `quantity` per `metric` (this is already per-row-correct).
- `token_breakdown` (prompt / completion / cached / cache_creation, provider accounting) is the
  only persisted per-class token signal available for every LLM row. It currently is ALSO copied
  onto component rows by `meter::materialize_component_rows`; fix that at the source so only the
  primary row carries it (component rows keep `token_breakdown: None`), add a regression test,
  and still defend the aggregation against historical duplicates by reading token classes from
  primary rows only.
- Only rows with `quantity != null` and `status in [finalized, dead_letter(forwarded)]` count
  (mirror the exact status predicate of `handlers/billing.rs::get_usage`).
- `actor_user_id` is the person who made the request; `billing_owner_id` is the wallet owner
  (same person, or an org). Attribute usage to the actor; expose the owner separately when it
  differs (org-owned usage) so org traffic is visible without hiding who used it.
- `credential_class` distinguishes platform key vs BYOK vs override vs node vs no-auth; expose it
  as a breakdown so admins can see platform-key vs own-key traffic.
- `funding.total_charge_micros` / `wallet_funded_micros` / `grant_funded_micros` /
  `allowance_funded_micros` give gross and funded cost (present only on exactly-settled rows);
  legacy rows have none. Reuse the exact/legacy convention from `get_usage`.
- Rows exist only when `BILLING_ENABLED` is on (metering is part of the billing open path).
  State this in the docs and show an empty-state hint in the UI when the response is empty.

### Backend API (admin router, `require_admin_or_operator`)
`GET /api/v1/admin/usage` with query:
- `period`: `24h` (default) | `7d` | `30d`; or `from` + `to` (RFC 3339) — the effective window
  is validated and clamped to at most 31 days; anything longer or inverted is a
  `ValidationError`.
- `user`: user id (UUID) — filter by actor OR billing owner equal to that user (org filter
  therefore shows all its members' org-owned usage).
- `service`: service slug or service id.
- `sort` for the ranking (`quantity`, `requests`, `cost`, one of the token classes) and
  `page` / `per_page` (max 100) for the ranking table. Sorting and paging of rankings happen in
  MongoDB (`$sort` + `$skip` + `$limit`), never by loading all groups into memory.
Response (dedicated response structs, never models):
- `window { from, to, period }`
- `totals`: requests (distinct primary rows), events, quantities per metric (map metric ->
  quantity for every `BillingMetric`, including the new component metrics), token classes
  (prompt, completion, cached, cache_creation, and derived totals), gross/wallet/grant/allowance
  cost micros with the exact-vs-legacy convention, unique users, unique services.
- `by_service[]`: service slug + id + name, requests, per-metric quantities, token classes,
  cost, unique users, credential-class split.
- `by_credential_class[]`: class -> requests, quantities, cost.
- `ranking[]` (paged): user id, display name, email, user_type, optional billing owner
  (id/name/email when org), service slug/name, per-metric quantities, requests, token classes,
  cost; plus `ranking_total` for paging.
- `users` display data comes from ONE batched `users` lookup by `_id` `$in` (as the audit-log
  enrichment does); no per-row queries; unknown/deleted users render as "Unknown user" with the
  id available in a tooltip only.
Implementation constraints:
- All reductions in MongoDB aggregation pipelines; run the independent pipelines concurrently
  (`tokio::try_join!`). Add `maxTimeMS` (e.g. 20 s) to these aggregations so a pathological
  window cannot hold a connection forever; map timeouts to a clear 503-style `AppError`
  (reuse an existing variant or add one following `errors/mod.rs` conventions and the CLAUDE.md
  code-block table).
- Indexing: the platform-wide predicate is `created_at` + `status` (+ optional `actor_user_id`
  / `billing_owner_id` / `service_slug`). Existing indexes are per owner only. Add the minimum
  additive index(es) in `db.rs::ensure_indexes` (for example `{ status: 1, created_at: -1 }` or
  `{ created_at: -1 }`; choose by `explain()`), document the write-cost trade-off in a comment,
  and prove with `explain("executionStats")` on a seeded dataset (≥ 200k rows across ≥ 50 users
  and ≥ 20 services; script it in a test or a `#[ignore]` benchmark) that the new endpoint uses
  an index scan bounded by the window and does not do a collection scan. Do not modify or drop
  any existing index; do not add a partial/unique index that could reject writes.
- Telemetry: emit an `AdminUsageViewed { filter }` event with an opaque applied-filter marker,
  mirroring `AdminAuditLogViewed` (no PII in telemetry).
- Audit: viewing is read-only; no audit entry needed beyond telemetry (match the audit-log page).
- Utoipa schemas for the new response types; register in the OpenAPI doc like the other admin
  endpoints.

### Frontend (`/admin/usage`, sidebar label "Usage", after "Audit Log" in `ADMIN_NAV`)
- Route in `router.tsx` under the admin layout (reachable by admin and operator like the audit
  log), page `pages/admin-usage.tsx`, hook in `hooks/use-admin.ts` (or a new `use-admin-usage.ts`
  if it keeps `use-admin.ts` readable), types in `types/admin.ts`, Zod schema in
  `schemas/admin-usage.ts` for the response (validate on receipt, lenient on unknown metrics —
  fall back to the raw metric string via `schemas/billing-metrics.ts` labels).
- Controls (URL-state driven like the audit log): period select with `24h` (default), `7d`,
  `30d`, and a custom range limited to 31 days; user filter = a searchable picker backed by the
  existing `useAdminUsers` search (by name or email), showing display name + email; service
  filter select from the services in the current window; ranking sort control; pagination.
- Sections: summary stat blocks (requests, unique users, total tokens with input / output /
  cache-read / cache-write beneath, images, other metrics present, gross cost); "By service"
  table; "Platform key vs own key" split; "Top users" ranking table (user name + email, service,
  metrics, requests, cost) with an expandable per-service breakdown per user when no service
  filter is applied.
- Follow DESIGN.md exactly (compact density, table head/cell sizes, JetBrains Mono for numbers,
  semantic colors only, no purple decoration). Reuse `PageHeader`, data-table components, `Badge`,
  `Select`, `Skeleton`, and the existing number/credit formatters from `pages/billing.tsx` (extract
  to `lib/` if needed rather than duplicating).
- Tests: vitest page test (renders totals, ranking, filters, empty state, error state), schema
  test, hook test if you add a new hook file. `npm run lint && npm run test && npm run build`
  must pass; rebuild the wizard bundle if any file in its graph changes.
- No CLI work is required for this feature.

## Part B — Audit-log listing performance (`/api/v1/admin/audit-log`)
The page is reported slow. Diagnose with `explain("executionStats")` against a seeded
`audit_log` of ≥ 500k rows (script it under `#[ignore]` or a test helper) and fix the real costs
WITHOUT changing query semantics or response shape. Known suspects (verify, don't assume):
1. `admin_audit_service::list_entries` runs `count_documents(filter)` and then `find` strictly
   sequentially, and the handler afterwards runs `distinct_event_types` sequentially; run the
   independent operations concurrently.
2. The free-text search is a 7-branch case-insensitive `$regex` `$or` (including `_id`) that
   cannot use any index; every page re-counts it. Keep "contains" semantics, but: bound the
   count (e.g. `$limit`-capped count that the UI already renders as an upper bound only if the
   frontend copes — if you change count semantics you MUST update the frontend pagination to
   handle a capped total and test it), and make sure the regex only runs after the indexed
   predicates narrow the candidates (check the winning plan).
3. `distinct_event_types` is computed on every request; it is filter-option data, so serve it
   from a short-lived in-process cache (≤ 60 s TTL) — document it as derived display data.
4. Unfiltered listing can use `estimated_document_count` for `total` only when the filter is
   empty; otherwise keep the exact count.
5. Check that every `ADMIN_SORT_OPTIONS` sort key and the `status` bucket filter
   (`event_data.response_status`) have a usable index; add only additive indexes and never touch
   `audit_log_seq_unique` or the hash-chain fields.
Report the before/after `executionStats` (docs examined, keys examined, execution time) for:
default listing, listing filtered by event type, listing with a free-text search, listing filtered
by date range. Regressing any other operation (append path, verify chain, seq uniqueness) is not
acceptable.

## Hard constraints
- Do not break existing functionality; existing tests must pass unchanged in intent.
- Layer rules (handlers -> services -> models), `AppError` mapping, no secrets/PII in logs or
  telemetry, `useAppForm` for forms, Zod schemas in `schemas/`, no `console.log`.
- No new environment variables. Migration-free.
- Docs: add a short "Admin usage" subsection to `docs/BILLING_UI_GLOSSARY.md` (what is counted,
  dedupe rule, window limits, BILLING_ENABLED prerequisite), update `CLAUDE.md` Key API Routes
  (`/admin/usage`) and the audit-log note if behaviour (caching) changed.
- Before any long cargo run check `df -h /System/Volumes/Data`; this machine has filled up
  before.

## Verification you must run and report (paste real output summaries)
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server -- --test-threads 2` (full binary)
- `cargo test -p nyxid-cli`
- `cd frontend && npm run lint && npm run test && npm run build`
- `npm --prefix frontend run build:wizard` + `cargo test -p nyxid-cli --test wizard_bundle_freshness` if any bundled file changed
- The explain-based measurements described above.

Commit on this branch in conventional-commit style (`feat(admin): ...`, `perf(audit): ...`). Do
not push, do not open a PR, do not bump versions. Finish by writing
`.claude-brief/ADMIN_USAGE_REPORT.md` with design decisions, files touched, exact commands and
results, explain() before/after numbers, and known gaps (aim for none).
