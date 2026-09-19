# Brief: per-component service pricing, per-component allowances, and high-precision unit prices

Repository: NyxID (this worktree, branch `flexible-service-billing`, based on main 81c0e5f4 / 0.25.3).
Read `CLAUDE.md` first (Critical Rules 3, 5 billing bullets, 8, 9) and then
`docs/PLATFORM_KEYS_AND_INFERENCE.md` ("Billing lanes and durable accounting"),
`docs/USAGE_BILLING_LAGO_SPEC.md`, `docs/BILLING_UI_GLOSSARY.md`, `docs/ADR-014-usage-billing-lago.md`.

## Problem

Today a billing lane (`ServiceBilling.byok_pricing` / `platform_key_pricing`, model
`backend/src/models/service_billing.rs`) is exactly ONE `metric` (`tokens|requests|bytes`) and
ONE `credits_per_unit`. Usage rows already carry a provider-reported `TokenBreakdown`
(prompt / completion / cache-read / cache-write) but it is observability only: every LLM
request is charged as a single total-token quantity. Image generation (OpenAI
`/v1/images/generations|edits|variations`, gpt-image-*; also other providers) is not a
distinct billable unit at all. Admins therefore cannot price input, output and cached
tokens differently, cannot price image generations, and cannot grant free allowances for
any of those individually.

Second problem: `credits_per_unit` is capped at 6 fractional digits (backend
`pricing::normalize_price`, `lago_client::decimal_credits_to_micros` truncates at 6,
frontend regex `^\d+(?:\.\d{1,6})?$` in `frontend/src/schemas/platform-keys.ts` and
`frontend/src/schemas/services.ts`, `billing_rate_cache.credits_per_unit_micros`,
`UsageFunding.credits_per_unit_micros`). Real token prices (e.g. 0.00000025 credits per
cached token) need more precision.

## Deliverable (all of it, nothing partial)

### A. Component pricing on lanes (additive, backward compatible)

1. Extend `BillingMetric` with new variants, serde snake_case, and matching `as_str()`
   strings (these strings enter the ledger canonical encoding for NEW rows only; existing
   strings must not change): `input_tokens`, `output_tokens`, `cache_read_tokens`,
   `cache_write_tokens`, `images`. Keep `tokens` (= provider total), `requests`, `bytes`
   with unchanged semantics. Design the enum/labels so adding further units later
   (audio seconds, characters, ...) is a small additive change: centralize the list of
   metrics, their human labels, whether they are "token family", and their default
   estimator in one place on the backend and one place on the frontend/CLI.
2. `LanePricing` keeps its existing fields with unchanged meaning (the existing
   `metric`/`credits_per_unit`/`lago_metric_code`/`sync_status`/`sync_error` become the
   lane's *primary component*) and gains an optional, serde-defaulted
   `components: Vec<LanePriceComponent>` where
   `LanePriceComponent { metric, credits_per_unit, lago_metric_code, sync_status, sync_error }`.
   Metrics must be unique within a lane (primary + components). Legacy documents without
   `components` deserialize unchanged. Old admin payloads that omit `components` preserve
   them; explicit `[]`/null clears them (same rule as lanes today).
3. Stable server-owned Lago codes per component:
   `platform_svc_{slug}_{byok|pk}_{metric}` (primary keeps today's
   `platform_svc_{slug}_byok` / `_pk`). Each component gets its own sync status, its own
   `billing_rate_cache` row, its own durable cleanup marker (removing a component or the
   lane must remove its Lago standard charge via reconcile exactly like lane cleanup
   today). Plan updates must still round-trip the full existing charge array with ids.
   Reuse `pricing.rs` sync machinery; do not fork a second implementation. Concurrent
   admin edits must keep the existing "fence sync completion against current price and
   metric" guarantees per component.
4. Legacy-only fields `platform_metric` and `resale_metric` must REJECT the new variants
   with `AppError::ValidationError` (they remain `tokens|requests|bytes`). New variants
   are valid only in lane primary/components and allowances.
5. Usage capture. Extend `PlatformUsage` with per-component quantities derived at
   capture time (keep `requests`, `bytes`, `tokens`, `token_breakdown` as-is):
   - `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens` must be
     NON-OVERLAPPING classes suitable for pricing. Providers differ: OpenAI-style
     `prompt_tokens_details.cached_tokens` / `input_tokens_details.cached_tokens` and
     Gemini `cachedContentTokenCount` are subsets of prompt/input tokens (subtract them
     from `input_tokens`); Anthropic `cache_read_input_tokens` /
     `cache_creation_input_tokens` are reported outside `input_tokens` (do not
     subtract). Record which accounting applied at extraction in
     `llm_usage_service::ReportedLlmUsage` (e.g. a `cached_tokens_included_in_prompt`
     flag) and derive the normalized classes from it. Never produce a negative
     quantity. `TokenBreakdown` keeps provider accounting (document that the normalized
     classes are the priced ones). Cover JSON, SSE-accumulated, Realtime WS and MCP
     capture paths (`handlers/proxy.rs`, `handlers/llm_gateway.rs`,
     `services/mcp_service.rs`, `services/llm_usage_service.rs`).
   - `images`: number of generated images in a successful image-generation response
     (OpenAI `data[]` length for `/images/generations|edits|variations`; extend for any
     other provider whose response shape is already recognized in the repo; otherwise 0).
     Image responses that also report `usage.input_tokens/output_tokens` (gpt-image-*)
     must ALSO populate the token classes. Detection must be robust to path prefixes
     (`/v1/...`, slug-routed and UUID-routed proxy paths, node-routed responses) and
     must never read/parse bodies beyond the existing body-size and capture limits.
   - Streaming image responses (if OpenAI `stream: true` for images is handled) count
     completed images once.
   - Ensure capture runs whenever the selected lane (or any configured lane, including
     pending ones, matching the existing `captures_tokens` rule) prices any token-family
     or image component, on JSON and SSE, for `llm-*` and non-`llm-*` slugs.
6. Charging. A request on a service whose selected lane has N priced components
   produces N platform-layer charges (one usage row per component is the recommended
   shape: each row has its own metric, `lago_metric_code`, allowance/grant/wallet
   funding, ledger entry and Lago event, so the existing settlement, recovery, outbox,
   ledger and dashboard code is reused; `transaction_id` must stay unique per row and
   idempotent across retries and flushes). Requirements:
   - Reservation/gate estimates every component (existing token estimator for token
     classes, request `n` parameter (default 1) for images, existing rules for others),
     reserves allowances -> grants -> wallet for each with the existing precedence, and
     releases everything on failure. Components with zero final quantity settle at zero
     without a Lago event and without leaking reservations.
   - A lane whose primary is e.g. `input_tokens` and has an `output_tokens` component
     is charged only for those two; `tokens` is not charged unless configured.
   - Pending/failed component sync keeps today's rule: a synced component charges,
     an unsynced one falls back like an unsynced lane does today (document exactly
     what happens per component; a lane with a synced primary and a pending component
     charges the primary and treats the pending component as legacy-fallback/free
     consistently with the existing lane rule).
   - Platform-key usage keeps billing the acting person
     (`BillingOwnerResolver::resolve_for_execution`); credential-class lane selection,
     `platform_charge_nyxid_credentials_only`, resale layer, NoAuth-never-charged are
     unchanged.
   - Ledger canonical fields/order/hash/dedupe keys/verification unchanged; new rows
     simply carry the new metric strings.
7. Allowances (`usage_allowances`, `services/billing/allowances.rs`,
   `metric_resolution::allowance_metric`, admin `/api/v1/admin/credits` handlers): an
   allowance's `metric` may be any metric configured on any lane of the service
   (primary or component), plus the existing legacy fallback rule. Allowances fund only
   rows with the identical metric. `configured_lane_metrics` / `effective_platform_metric`
   must include components. Recurrence/period behaviour unchanged.
8. Surfaces:
   - Admin API `POST/PUT /services` (handlers/services.rs, catalog admin DTOs): accept
     `components` per lane with validation; responses expose components with sync
     status; clients cannot author `lago_metric_code`, `sync_status`, `sync_error`,
     cleanup markers.
   - Catalog/user-facing pricing display (`frontend/src/schemas/platform-keys.ts`
     `formatLanePrice`-style helpers, `provider-services.tsx`, key dialogs, MCP/LLM
     listings if they show prices, `GET /catalog` responses): show all components.
   - Frontend admin service editor (`components/services/platform-service-fields.tsx`,
     `pages/service-edit.helpers.ts`, `schemas/services.ts`): per lane, an editable
     list of price components (unit select from the centralized metric list + price),
     add/remove, per-component sync badge, uniqueness validation, Zod schema,
     `useAppForm` conventions, DESIGN.md compliance, tests updated/added.
   - Frontend admin credits allowance dialog (`components/admin-credits/*`): the unit
     selector offers every configured lane metric (primary + components); labels for new
     metrics; `billingMetricLabel` and `schemas/billing.ts` cover new metrics.
   - Frontend billing/usage page (`pages/billing.tsx`, `schemas/billing.ts`) and
     CLI `nyxid billing usage` (`cli/src/commands/billing.rs`): rows with new metrics
     render with proper labels and quantities; nothing crashes on unknown/new metric
     strings (fall back to the raw string).
   - CLI catalog admin (`cli/src/commands/service/catalog_admin.rs`, `cli/src/cli.rs`):
     repeatable `--byok-component <metric>=<price>` / `--platform-key-component
     <metric>=<price>` and `--byok-clear-components` / `--platform-key-clear-components`
     (or equivalent clearly documented flags); `nyxid catalog show` prints components.
   - `handlers/billing.rs` usage listing and any usage aggregation that groups by metric
     must include the new metrics; `GET /billing/usage` gross-cost splits stay correct
     when a request has several rows.
   - MCP (`mcp_service.rs`) token estimation: treat token-family components as
     token-metered.
   - OpenAPI (utoipa) schemas updated.

### B. Unit price precision

1. Raise the supported precision to 12 fractional digits everywhere (constant in one
   place on the backend, mirrored on the frontend and CLI messages): `normalize_price`,
   the frontend regexes and helper texts in `schemas/platform-keys.ts` and
   `schemas/services.ts`, CLI validation if any. Max price 1,000,000 credits unchanged.
2. Internal rate representation: add a higher-precision integer rate
   (e.g. `credits_per_unit_pico: i64`, 1e-12 credits; 1e6 credits * 1e12 fits i64) to
   `BillingRateCache`, `UsageFunding`, `LayerReservation` and every place a rate is
   carried. Keep the existing `credits_per_unit_micros` fields populated (truncated) for
   rolling deploys and old replicas; new code prefers the precise field when present.
   Cost math stays exact integer (i128 intermediates), no floating point anywhere,
   saturating, with the existing rounding direction (ceil to whole credits for wallet
   debits, existing micro rounding for gross cost); state the rounding rule in docs.
   Amount fields stored/ledgered in micros stay in micros (ledger unchanged).
3. `decimal_credits_to_micros` must not silently truncate NyxID-authored prices when
   the Lago plan is mirrored back into the rate cache (`reconcile::refresh_rate_cache`,
   `LagoApi::plan_rates`): parse the full precision into the precise field. Lago
   standard-charge `amount` is sent as the exact normalized decimal string.
4. Tests proving a 12-digit price round-trips admin API -> Mongo -> Lago payload ->
   rate cache -> reservation -> settlement -> ledger with the expected micro amounts,
   and that a 6-digit legacy row still behaves identically.

### C. Docs and rules

- Update `docs/PLATFORM_KEYS_AND_INFERENCE.md` (lanes section), `docs/BILLING_UI_GLOSSARY.md`,
  `docs/USAGE_BILLING_LAGO_SPEC.md` §3.1/§3.4 and the `CLAUDE.md` billing-lane bullet
  (keep it terse, same style) with: component model, Lago code scheme, capture
  normalization rules, per-component charging/fallback rules, allowance matching,
  precision constant + rounding, and the rollout note: upgrade ALL replicas before
  authoring component prices or allowances that use the new metrics (old replicas
  cannot deserialize the new enum variants).
- Do NOT bump versions; the reviewer does the release.

## Hard constraints

- Do not break existing functionality. Every existing test must still pass unmodified
  in intent (you may extend fixtures). Existing lanes, legacy platform pricing, resale,
  grants, schedules, allowances, wallet, ledger verification, topup expiry, Lago outbox,
  dashboard queries, CLI and frontend flows must behave identically when no components
  are configured and prices have <= 6 digits.
- Follow CLAUDE.md layer rules (handlers -> services -> models), Mongo conventions
  (no `skip_serializing` on model fields, datetime helpers), `AppError` mapping, no
  secrets in logs, no `console.log`, `useAppForm`, Zod schemas in `schemas/`.
- No new environment variables.
- Migration-free: all new fields optional/serde-defaulted.
- Deterministic, no floats in money math.

## Verification you must run and report (paste real output summaries)

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- Backend DB tests need the replica-set Mongo already running on this machine:
  `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server -- --test-threads 2`
  (the full binary; at minimum every test matching `billing`, `usage`, `allowance`,
  `grant`, `pricing`, `catalog`, `services`, `proxy`, `llm`, `mcp`).
- `cargo test -p nyxid billing_route_coverage_smoke -- --nocapture`
- `cargo test -p nyxid-cli`
- `cd frontend && npm run lint && npm run test && npm run build`
- If you touched any frontend file that the CLI wizard bundle includes, run
  `npm --prefix frontend run build:wizard` and commit the regenerated
  `cli/src/wizard/bundle-meta/index.hash`; then `cargo test -p nyxid-cli --test wizard_bundle_freshness`.

Commit your work on this branch (`flexible-service-billing`) in conventional-commit style
(`feat(billing): ...`). Do not push. Do not create a PR. When finished, write a summary
of design decisions, files touched, test results, and any known gaps (there should be
none) to `.claude-brief/IMPLEMENTATION_REPORT.md`.
