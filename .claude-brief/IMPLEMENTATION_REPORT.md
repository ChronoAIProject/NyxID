# Billing components implementation report

Branch: `flexible-service-billing`. No version changes, push, or pull request.

## Design decisions

- Existing lane fields remain the primary price. Defaulted `components` add independently priced, unique metrics. Omitted component arrays preserve stored prices; null/empty arrays clear them. Legacy platform/resale metric fields still accept only tokens, requests, and bytes. Sync identities, statuses, errors, and cleanup markers remain server-owned.
- Primary Lago codes and transaction IDs are unchanged. Additional codes append the metric, and additional platform transaction IDs append `:component:{metric_code}`. All prices reuse the existing standard-charge synchronization path and full-plan charge-ID preservation. Removed components have durable cleanup markers; stale completions are fenced against the live metric/code/price and force reconciliation when necessary.
- Each synced component has its own usage row, rate, reservation, allowance/grant/wallet funding, settlement, and Lago event. Multiple unsynced components add at most one legacy fallback charge when legacy billing is enabled. Missing lanes remain free; credential restrictions, acting-person platform-key billing, and resale retain their existing rules.
- A primary-row `pending_platform_usage` snapshot durably coordinates component finalization and resale intent. Retries reuse existing rows and holds. Zero-quantity components release holds before nonzero components consume grants, avoiding unnecessary wallet charges. Existing ledger canonical encoding and money-movement hooks are unchanged: zero-wallet and allowance-only settlements do not invent wallet ledger entries.
- Token pricing uses non-overlapping input/output/cache-read/cache-write quantities. OpenAI/Gemini cache counts are subtracted from input; Anthropic's separately reported caches are not. Provider `TokenBreakdown` remains observational and unchanged. JSON, SSE, Realtime WS, node responses, and MCP feed the normalized quantities. Successful OpenAI image endpoints count `data` entries across path prefixes; completed image SSE events are deduplicated. Error responses and partial image previews do not count images.
- Usage capture reuses existing response reads and limits. Node SSE observation is bounded per event. Legacy direct SSE and MCP capture retain their existing transport behavior. New token components use the token-byte estimate; images use JSON request `n` (default one). Standalone legacy metrics and resale retain their existing one-unit reservation gate.
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

## Verification

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

## Known gaps

None identified against the brief. All required verification passed. The documented rollout prerequisite, supported provider response shapes, and transport/capture bounds remain part of the contract.
