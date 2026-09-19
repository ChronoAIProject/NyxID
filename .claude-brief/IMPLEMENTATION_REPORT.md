# Billing components implementation report

Branch: `flexible-service-billing`. No version changes, push, or pull request.

## Design decisions

- Existing lane fields remain the primary price. Defaulted `components` add independently priced, unique metrics. Omitted component arrays preserve stored prices; null/empty arrays clear them. Legacy platform/resale metric fields still accept only tokens, requests, and bytes. Sync identities, statuses, errors, and cleanup markers remain server-owned.
- Primary Lago codes and transaction IDs are unchanged. Additional codes append the metric, and additional platform transaction IDs append `:component:{metric_code}`. All prices reuse the existing standard-charge synchronization path and full-plan charge-ID preservation. Removed components have durable cleanup markers; stale completions are fenced against the live metric/code/price and force reconciliation when necessary.
- An unsynced primary substitutes legacy billing (or free) for its entire lane, ignoring extras. After the primary syncs, each synced component has its own usage row, rate, reservation, allowance/grant/wallet funding, settlement, and Lago event; unsynced extras are free and never add legacy charges. Missing lanes remain free; credential restrictions, acting-person platform-key billing, and resale retain their existing rules.
- A primary-row `pending_platform_usage` snapshot durably coordinates component finalization and resale intent. Retries reuse existing rows and holds. Zero-quantity components release holds before nonzero components consume grants, avoiding unnecessary wallet charges. Existing ledger canonical encoding and money-movement hooks are unchanged: zero-wallet and allowance-only settlements do not invent wallet ledger entries.
- Token pricing uses non-overlapping input/output/cache-read/cache-write quantities. OpenAI/Gemini cache counts are subtracted from input; Anthropic's separately reported caches are not. Provider `TokenBreakdown` remains observational and unchanged. JSON, SSE, Realtime WS, node responses, and MCP feed the normalized quantities. Successful OpenAI image endpoints count `data` entries across path prefixes; completed image SSE events are deduplicated. Error responses and partial image previews do not count images.
- Usage capture reuses existing response reads and limits. Node SSE observation is bounded per event. Legacy direct SSE and MCP capture retain their existing transport behavior. Input/output/total token estimates use request bytes; cache-read/cache-write estimates use a one-unit gate because input already covers cache quantities. Images use JSON request `n` (default one), parsed only when an active platform spec prices images. Standalone legacy metrics and resale retain their existing one-unit reservation gate.
- `PRICE_FRACTIONAL_DIGITS = 12` and a 1,000,000-credit maximum are enforced with exact integer parsing. Optional picocredit rates coexist with populated, truncated micro rates; absent precise fields use legacy rates. Integer i128 intermediates saturate, gross/funding amounts truncate only after multiplication to microcredits, and exact wallet remainders ceil to whole credits. Grant and ledger encodings remain in their existing units. Lago sends/mirrors exact decimals, while external legacy Lago values outside precise syntax/range retain the previous micro-rate fallback.
- Frontend schemas, component editor, price labels, allowance selector, usage quantities, and CLI component/clear flags share centralized metric names/labels and exact decimal validation. Unknown display metrics fall back to their raw names. The CLI wizard was regenerated, including its manifest and hash.
- Migration-free additive fields preserve old documents and payloads. Upgrade all replicas before configuring components, new-metric allowances, or prices beyond six decimals. Old replicas cannot deserialize the new units or charge the additional precision/components.

## Regression coverage

New/extended tests cover exact decimal bounds and rounding, API omission/null semantics, duplicate/legacy unit validation, provider cache normalization, prefixed image paths, SSE completion deduplication and bounded node observation, Realtime and MCP component usage, partial-sync fallback, precise Lago HTTP payloads and rate-cache mirroring, CLI flag/update/clear behavior, frontend editor round trips and allowance options.

The database component scenario covers admin DTO -> normalization -> Mongo -> Lago sync -> precise rate cache -> reservation -> settlement -> ledger verification, failure cleanup, repeated opens/settlements, crash recovery, zero image output, matching token allowances, grant/wallet splits, and removal/stale-sync cleanup.

During verification, the new regression exposed grant credits held by a zero-quantity image reservation; settlement now releases that hold first. The new test's initial expectation of wallet ledger entries for zero-wallet rows was corrected to preserve the existing money-movement contract. The node SSE boundary test also exposed CRLF frames being passed to an LF-only shared parser; the bounded observer now normalizes line endings after applying its byte cap. A test-only missing function qualifier was also fixed. Final results below supersede those intermediate failures.

The precise settlement scenario asserts these gross amounts before funding:

| Component | Price (credits/unit) | Actual units | Gross microcredits |
| --- | --- | ---: | ---: |
| Input tokens | `0.000000250001` | 4,000,000 | 1,000,004 |
| Output tokens | `0.000001500001` | 1,000,000 | 1,500,001 |
| Cache-read tokens | `0.000000250001` | 4,000,000 | 1,000,004 |
| Images | `0.25` | 0 | 0 |

The matching input allowance funds 1,000,004 microcredits, grants fund exactly 1,000,000, and the remaining exact costs are funded by the wallet. Whole-credit rounding is independent per component (3 wallet credits in this scenario). All holds are released; ledger verification succeeds.

## Initial implementation verification (47b3c950)

Database-backed commands use `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true'` (including the route smoke test). An initial standalone smoke invocation without that variable failed because the test harness requires an explicit MongoDB URI; the configured rerun passed.

| Exact command | Final result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS, no diff/output. |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS, finished dev profile in 49.62s; no warnings. |
| `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server -- --test-threads 2` | PASS: 6,262 passed, 0 failed, 0 ignored; 339.61s. |
| `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid billing_route_coverage_smoke -- --nocapture` | PASS: 1 passed, 0 failed, 6,261 filtered out; 8.64s. |
| `cargo test -p nyxid-cli` | PASS: 1,245 unit tests plus 31 integration tests; 0 failed/ignored; doc tests passed (0 tests). |
| `cd frontend && npm run lint && npm run test && npm run build` | PASS: lint 0 errors / 27 existing warnings; 338 test files and 3,402 tests passed; TypeScript, production builds and footprint assertion passed. |
| `npm --prefix frontend run build:wizard` | PASS; generated assets, 114-file manifest, and `index.hash` (`46c2b6eb00b7…`). |
| `cargo test -p nyxid-cli --test wizard_bundle_freshness` | PASS: 1 passed, 0 failed; 0.06s. |

Final local logs: `/tmp/nyx-billing-fmt-verified.log`, `/tmp/nyx-billing-clippy-last.log`, `/tmp/nyx-billing-backend-final3.log`, `/tmp/nyx-billing-route-verified.log`, `/tmp/nyx-billing-cli-final.log`, `/tmp/nyx-billing-frontend-{lint,test,build}-verified.log`, `/tmp/nyx-billing-wizard-build.log`, `/tmp/nyx-billing-wizard-test-final.log`. These are local verification artifacts, not repository files.

Focused final regression: `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server bounded_sse_capture_handles_chunk_boundaries_limits_and_failed_images -- --nocapture` — PASS: 1 passed, 0 failed (0.03s).

## Files touched

- `CLAUDE.md`
- `backend/src/billing_integration_tests.rs`
- `backend/src/billing_integration_tests/usage.rs`
- `backend/src/handlers/billing.rs`
- `backend/src/handlers/llm_gateway.rs`
- `backend/src/handlers/proxy.rs`
- `backend/src/handlers/public_mcp.rs`
- `backend/src/handlers/public_proxy.rs`
- `backend/src/handlers/services.rs`
- `backend/src/models/billing_rate_cache.rs`
- `backend/src/models/service_billing.rs`
- `backend/src/models/usage_meter.rs`
- `backend/src/services/anonymous_endpoint_service.rs`
- `backend/src/services/billing/amounts.rs`
- `backend/src/services/billing/funding.rs`
- `backend/src/services/billing/lago_client.rs`
- `backend/src/services/billing/meter.rs`
- `backend/src/services/billing/metric_resolution.rs`
- `backend/src/services/billing/mod.rs`
- `backend/src/services/billing/pricing.rs`
- `backend/src/services/billing/reconcile.rs`
- `backend/src/services/billing/reservation.rs`
- `backend/src/services/billing/route_context.rs`
- `backend/src/services/billing/webhook.rs`
- `backend/src/services/inference_service.rs`
- `backend/src/services/llm_usage_service.rs`
- `backend/src/services/mcp_service.rs`
- `backend/src/services/platform_key_service/tests.rs`
- `cli/src/cli.rs`
- `cli/src/commands/billing.rs`
- `cli/src/commands/billing_units.rs`
- `cli/src/commands/mod.rs`
- `cli/src/commands/service/catalog_admin.rs`
- `cli/src/wizard/assets/index.html`
- `cli/src/wizard/bundle-meta/index.hash`
- `cli/src/wizard/bundle-meta/index.manifest`
- `docs/BILLING_UI_GLOSSARY.md`
- `docs/PLATFORM_KEYS_AND_INFERENCE.md`
- `docs/USAGE_BILLING_LAGO_SPEC.md`
- `frontend/src/components/admin-credits/credits-dialogs.test.tsx`
- `frontend/src/components/admin-credits/credits-dialogs.tsx`
- `frontend/src/components/providers/provider-services.tsx`
- `frontend/src/components/services/platform-service-fields.test.tsx`
- `frontend/src/components/services/platform-service-fields.tsx`
- `frontend/src/lib/billing-units.test.ts`
- `frontend/src/lib/billing-units.ts`
- `frontend/src/pages/billing.tsx`
- `frontend/src/pages/service-edit.helpers.ts`
- `frontend/src/pages/service-edit.test.tsx`
- `frontend/src/schemas/billing-metrics.ts`
- `frontend/src/schemas/billing.ts`
- `frontend/src/schemas/platform-keys.test.ts`
- `frontend/src/schemas/platform-keys.ts`
- `frontend/src/schemas/services.test.ts`
- `frontend/src/schemas/services.ts`
- `.claude-brief/IMPLEMENTATION_REPORT.md`

## Review round 1

All seven findings and both cleanups are addressed:

1. Lane fallback is substitution, governed solely by primary sync status. Synced-primary lanes charge only synced components; pending/failed extras never add a legacy charge. Pending/failed primaries ignore all extras and retain legacy/free behavior. Regression coverage includes both legacy-enabled/free cases, pending/failed primaries, and partial component sync.
2. Request-body length remains cheap; JSON parsing for image `n` runs only when `platform_specs()` includes images. Regression coverage includes skipped parsing for non-image and pending-image prices, plus valid/missing/invalid image counts.
3. Cache-read and cache-write reservations use one unit. Total/input/output retain the byte estimate; images use `n`; requests/bytes use one. The standalone legacy/resale reservation gate is unchanged. All units have explicit estimate assertions.
4. A lost conditional component-finalization claim now rereads by `_id` and uses the fresh quantity-bearing row. A deterministic Mongo regression finalizes and settles output out-of-band after reading forwarded rows, then materializes the stale snapshot and replays the coordinator. It asserts one debit per component and no remaining holds or intent.
5. Service-layer `allowance_metrics` is shared by allowance validation and the computed admin response field; it is never stored. Frontend type/schema and dialog consume this list. Older responses fall back to the existing backend-computed display unit. The dialog retains an existing allowance's unit on unrelated edits. Tests cover primary/extra sync combinations, response serialization/non-storage, and server-specified units that deliberately differ from client-visible legacy metadata.
6. Provider token counts again accept floats, clamped at zero and truncated after preferring exact unsigned integers; strings keep integer parsing. Tests cover `25.0`, fractional/negative counts, strings, and `u64::MAX`. Price arithmetic remains entirely integer-based.
7. The three operator docs explicitly state that unreported input/output/cache classes are zero while legacy total tokens still use byte estimation. They also document each reservation estimate, non-additive fallback, and the authoritative allowance list. CLAUDE.md reflects the corrected lane rule.

The helper now imports `LanePricingView` at the top of its file. All syntactically valid prices exceeding the maximum, including integer/picocredit overflow, return the same “must not exceed 1,000,000 credits” validation message; the exact cap remains accepted.

### Review verification

Database-backed commands use the running `nyxid1530` replica set. The first frontend build caught an incomplete test fixture after adding the optional response field; the fixture now supplies the required service fields. All frontend checks were rerun after that correction.

| Exact command | Final result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS; no output. |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS; dev profile finished in 1m 28s, no warnings. |
| `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid --bin nyxid-server -- --test-threads 2` | PASS: 6,269 passed, 0 failed, 0 ignored; 333.74s. |
| `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=nyxid1530&directConnection=true' cargo test -p nyxid billing_route_coverage_smoke -- --nocapture` | PASS: 1 passed, 0 failed, 6,268 filtered out; 2.97s. |
| `cargo test -p nyxid-cli` | PASS: 1,245 unit tests + 31 integration tests, no failures/ignored tests; doc tests passed (0 tests). |
| `cd frontend && npm run lint && npm run test && npm run build` | PASS: lint 0 errors / 27 existing warnings; all 338 files / 3,405 tests passed (59.34s); TypeScript, production builds, and footprint assertion passed. |
| `npm --prefix frontend run build:wizard` | PASS; rebuilt 114-file manifest and assets; hash `46c2b6eb00b7…` unchanged. No generated-file diff. |
| `cargo test -p nyxid-cli --test wizard_bundle_freshness` | PASS: 1 passed, 0 failed; 0.05s. |

Review logs are local files at `/tmp/nyx-billing-review1-{fmt,clippy,backend,route,cli,frontend-lint,frontend-test,frontend-build,wizard-build,wizard-test}.log`.

### Review files touched

- `.claude-brief/IMPLEMENTATION_REPORT.md`
- `CLAUDE.md`
- `backend/src/handlers/services.rs`
- `backend/src/handlers/services_helpers.rs`
- `backend/src/services/billing/meter.rs`
- `backend/src/services/billing/metric_resolution.rs`
- `backend/src/services/billing/pricing.rs`
- `backend/src/services/billing/route_context.rs`
- `backend/src/services/llm_usage_service.rs`
- `docs/BILLING_UI_GLOSSARY.md`
- `docs/PLATFORM_KEYS_AND_INFERENCE.md`
- `docs/USAGE_BILLING_LAGO_SPEC.md`
- `frontend/src/components/admin-credits/credits-dialogs.test.tsx`
- `frontend/src/components/admin-credits/credits-dialogs.tsx`
- `frontend/src/pages/service-edit.helpers.ts`
- `frontend/src/schemas/services.test.ts`
- `frontend/src/schemas/services.ts`
- `frontend/src/types/api.ts`

## Known gaps

None known. All review findings are addressed and the full required verification set passed. The documented rollout prerequisite, supported provider response shapes, and transport/capture bounds remain part of the contract.
