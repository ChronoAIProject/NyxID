# Tools Phase 1 verification

Branch: `enrich-ai-search-tools`. Contract: revised plan including user commits `68341e43` and `c428561d`. No version changes or main-branch commits.

The earlier implementation followed the original full plan. `a89e93ca` removes the deferred overlays, editor roles/scopes, Imports/spec commands and all tool seeds. The final implementation contains only Phase 1.

## Implementation commits and files

### `68277188` — feat(catalog): additive tool offering fields and topic vocabulary

- `backend/src/db.rs`
- `backend/src/handlers/admin.rs`
- `backend/src/handlers/admin_anonymous_endpoints.rs`
- `backend/src/handlers/catalog.rs`
- `backend/src/handlers/catalog_tests.rs`
- `backend/src/handlers/delegation.rs`
- `backend/src/handlers/endpoints.rs`
- `backend/src/handlers/mcp_chat_authority_tests.rs`
- `backend/src/handlers/mcp_transport.rs`
- `backend/src/handlers/public_mcp.rs`
- `backend/src/handlers/public_proxy.rs`
- `backend/src/handlers/services.rs`
- `backend/src/handlers/services_helpers.rs`
- `backend/src/handlers/ssh_tunnel.rs`
- `backend/src/models/downstream_service.rs`
- `backend/src/models/service_endpoint.rs`
- `backend/src/routes.rs`
- `backend/src/services/anonymous_endpoint_service.rs`
- `backend/src/services/catalog_service.rs`
- `backend/src/services/catalog_spec_sync.rs`
- `backend/src/services/destination_routing_tests.rs`
- `backend/src/services/durable_operation_grant_service.rs`
- `backend/src/services/mcp_service.rs`
- `backend/src/services/mod.rs`
- `backend/src/services/provider_service.rs`
- `backend/src/services/proxy_service.rs`
- `backend/src/services/service_endpoint_service.rs`
- `backend/src/services/tool_topics.rs`
- `backend/src/services/unified_key_service.rs`
- `backend/src/services/user_service_service.rs`
- `backend/src/test_utils.rs`

### `9fddbee5` — feat(catalog): endpoint publication state with proxy and MCP gating

- `CLAUDE.md`
- `backend/src/errors/mod.rs`
- `backend/src/handlers/endpoints.rs`
- `backend/src/handlers/mcp_transport.rs`
- `backend/src/handlers/proxy.rs`
- `backend/src/routes.rs`
- `backend/src/services/mcp_service.rs`
- `backend/src/services/mod.rs`
- `backend/src/services/proxy_service.rs`
- `backend/src/services/service_endpoint_service.rs`
- `backend/src/services/tool_publication_service.rs`

### `5b35b522` — feat(catalog): runtime spec overlays per service

- `backend/src/db.rs`
- `backend/src/handlers/catalog_spec_overlays.rs`
- `backend/src/handlers/endpoints.rs`
- `backend/src/handlers/mod.rs`
- `backend/src/models/catalog_spec_overlay.rs`
- `backend/src/models/mod.rs`
- `backend/src/routes.rs`
- `backend/src/services/catalog_spec_overlay_service.rs`
- `backend/src/services/catalog_spec_sync.rs`
- `backend/src/services/destination_routing_tests.rs`
- `backend/src/services/mod.rs`

### `99c37709` — feat(auth): catalog services editor role and token scopes

- `backend/src/handlers/catalog_services_editor.rs`
- `backend/src/handlers/catalog_spec_overlays.rs`
- `backend/src/handlers/endpoints.rs`
- `backend/src/handlers/mod.rs`
- `backend/src/handlers/services.rs`
- `backend/src/handlers/services_helpers.rs`
- `backend/src/mw/auth.rs`
- `backend/src/routes.rs`
- `backend/src/services/catalog_editor_service.rs`
- `backend/src/services/catalog_services_access.rs`
- `backend/src/services/catalog_services_editor_service.rs`
- `backend/src/services/mod.rs`
- `backend/src/services/service_account_scope_service.rs`

### `515c3925` — feat(tools): user tools listing and keys/catalog filtering

- `backend/src/handlers/catalog.rs`
- `backend/src/handlers/catalog_routes_tests.rs`
- `backend/src/handlers/keys.rs`
- `backend/src/handlers/mod.rs`
- `backend/src/handlers/tools.rs`
- `backend/src/routes.rs`
- `backend/src/services/mod.rs`
- `backend/src/services/platform_key_service.rs`
- `backend/src/services/tools_service.rs`
- `backend/src/services/unified_key_service.rs`

### `2e5d7ff8` — feat(catalog): seed tools-x and TinyFish tool rows

- `backend/specs/catalog/tinyfish-fetch.openapi.json`
- `backend/specs/catalog/tinyfish-search.openapi.json`
- `backend/src/handlers/mcp_transport.rs`
- `backend/src/services/catalog_spec_registry.rs`
- `backend/src/services/mcp_service.rs`
- `backend/src/services/mod.rs`
- `backend/src/services/provider_service.rs`
- `backend/src/services/tool_publication_service.rs`
- `backend/src/services/tool_seed_service.rs`
- `scripts/check-catalog-spec-drift.py`

### `8ee8729f` — feat(cli): catalog endpoint/spec/publish commands, tools commands, tool flags

- `backend/src/handlers/services.rs`
- `backend/src/services/tool_seed_service.rs`
- `cli/src/cli.rs`
- `cli/src/commands/catalog.rs`
- `cli/src/commands/catalog_tools.rs`
- `cli/src/commands/mod.rs`
- `cli/src/commands/service.rs`
- `cli/src/commands/service/catalog_admin.rs`
- `cli/src/main.rs`
- `cli/src/output.rs`

### `adfabaef` — feat(frontend): Tools page, admin Tools workspace, editor fields

- `backend/src/handlers/catalog.rs`
- `backend/src/handlers/catalog_spec_overlays.rs`
- `backend/src/handlers/endpoints.rs`
- `backend/src/handlers/mod.rs`
- `backend/src/handlers/services.rs`
- `backend/src/handlers/services_helpers.rs`
- `backend/src/handlers/tools.rs`
- `backend/src/handlers/tools_tests.rs`
- `backend/src/routes.rs`
- `backend/src/services/catalog_service.rs`
- `backend/src/services/catalog_services_access.rs`
- `backend/src/services/catalog_services_editor_service.rs`
- `backend/src/services/catalog_spec_overlay_service.rs`
- `backend/src/services/mcp_service.rs`
- `backend/src/services/service_endpoint_service.rs`
- `backend/src/services/tool_publication_service.rs`
- `backend/src/services/tool_seed_service.rs`
- `backend/src/services/tool_topics.rs`
- `backend/src/services/tools_service.rs`
- `cli/src/commands/service.rs`
- `frontend/src/components/assistant/assistant-platform-service-fields.tsx`
- `frontend/src/components/assistant/nyxbot-agent-forms.tsx`
- `frontend/src/components/cli-wizard/access-scope-card.tsx`
- `frontend/src/components/dashboard/api-key-create-dialog.tsx`
- `frontend/src/components/dashboard/api-key-detail/service-scope-card.tsx`
- `frontend/src/components/dashboard/endpoint-form-dialog.test.tsx`
- `frontend/src/components/dashboard/endpoint-form-dialog.tsx`
- `frontend/src/components/dashboard/endpoint-list.tsx`
- `frontend/src/components/dashboard/sidebar.tsx`
- `frontend/src/components/layout/dashboard-layout.tsx`
- `frontend/src/components/services/catalog-tool-metadata.tsx`
- `frontend/src/components/services/tool-publication.tsx`
- `frontend/src/hooks/use-catalog-admin.ts`
- `frontend/src/hooks/use-endpoints.ts`
- `frontend/src/hooks/use-keys.ts`
- `frontend/src/hooks/use-tools.ts`
- `frontend/src/lib/endpoint-changes.ts`
- `frontend/src/lib/tools.ts`
- `frontend/src/pages/admin-edit-forms.test.tsx`
- `frontend/src/pages/admin-tools.tsx`
- `frontend/src/pages/api-key-detail.test.tsx`
- `frontend/src/pages/api-key-detail.tsx`
- `frontend/src/pages/keys.tsx`
- `frontend/src/pages/service-detail.tsx`
- `frontend/src/pages/service-edit.test.tsx`
- `frontend/src/pages/service-edit.tsx`
- `frontend/src/pages/tools.test.tsx`
- `frontend/src/pages/tools.tsx`
- `frontend/src/router.tsx`
- `frontend/src/schemas/endpoints.ts`
- `frontend/src/schemas/tools.test.ts`
- `frontend/src/schemas/tools.ts`
- `frontend/src/types/api.ts`
- `frontend/src/types/keys.ts`

### `72752924` — docs(tools): TOOLS.md, API.md, CLAUDE.md

- `CLAUDE.md`
- `docs/API.md`
- `docs/TOOLS.md`

### `fee6426a` — fix(tools): tighten editor authority and preserve key metadata

- `backend/src/handlers/catalog_services_editor.rs`
- `backend/src/handlers/keys.rs`
- `backend/src/handlers/services.rs`
- `backend/src/handlers/tools_tests.rs`
- `backend/src/services/catalog_editor_service.rs`
- `backend/src/services/catalog_services_access.rs`
- `backend/src/services/tools_service.rs`
- `backend/src/services/unified_key_service.rs`
- `docs/TOOLS.md`
- `frontend/src/components/services/catalog-tool-metadata.tsx`
- `frontend/src/lib/tools.ts`
- `frontend/src/pages/admin-tools.tsx`
- `frontend/src/pages/service-edit.tsx`
- `frontend/src/pages/tools.test.tsx`

### `220f5ba2` — fix(tools): report unpublished MCP operations and validate editor workflows

- `backend/src/handlers/curation_tests.rs`
- `backend/src/handlers/mcp_transport.rs`
- `backend/src/handlers/tools_tests.rs`
- `backend/src/routes.rs`
- `backend/src/services/mcp_service.rs`
- `backend/src/services/tools_service.rs`
- `backend/src/services/unified_key_service.rs`
- `cli/src/wizard/assets/index.html`
- `cli/src/wizard/bundle-meta/index.hash`
- `docs/TOOLS.md`

### `68341e43` — docs(plans): scope Tools plan to Phase 1 with tool twins, no seeds

- `docs/plans/2026-10-08-tools-implementation-plan.md`

### `c428561d` — docs(plans): add programmatic tool twin POC script to Phase 1

- `docs/plans/2026-10-08-tools-implementation-plan.md`

### `a89e93ca` — refactor(tools): remove deferred features and restore admin-only management

- `CLAUDE.md`
- `backend/specs/catalog/tinyfish-fetch.openapi.json`
- `backend/specs/catalog/tinyfish-search.openapi.json`
- `backend/src/db.rs`
- `backend/src/handlers/catalog_services_editor.rs`
- `backend/src/handlers/catalog_spec_overlays.rs`
- `backend/src/handlers/endpoints.rs`
- `backend/src/handlers/mod.rs`
- `backend/src/handlers/services.rs`
- `backend/src/handlers/tools.rs`
- `backend/src/handlers/tools_tests.rs`
- `backend/src/models/catalog_spec_overlay.rs`
- `backend/src/models/mod.rs`
- `backend/src/mw/auth.rs`
- `backend/src/routes.rs`
- `backend/src/services/catalog_editor_service.rs`
- `backend/src/services/catalog_services_access.rs`
- `backend/src/services/catalog_services_editor_service.rs`
- `backend/src/services/catalog_spec_overlay_service.rs`
- `backend/src/services/catalog_spec_registry.rs`
- `backend/src/services/catalog_spec_sync.rs`
- `backend/src/services/mod.rs`
- `backend/src/services/provider_service.rs`
- `backend/src/services/service_account_scope_service.rs`
- `backend/src/services/tool_seed_service.rs`
- `cli/src/cli.rs`
- `cli/src/commands/catalog.rs`
- `cli/src/commands/catalog_tools.rs`
- `docs/API.md`
- `docs/TOOLS.md`
- `frontend/src/components/dashboard/sidebar.tsx`
- `frontend/src/hooks/use-catalog-admin.ts`
- `frontend/src/hooks/use-tools.ts`
- `frontend/src/pages/admin-edit-forms.test.tsx`
- `frontend/src/pages/admin-tools.tsx`
- `frontend/src/pages/service-edit.test.tsx`
- `frontend/src/pages/service-edit.tsx`
- `frontend/src/pages/tools.test.tsx`
- `frontend/src/router.tsx`
- `frontend/src/schemas/tools.test.ts`
- `frontend/src/schemas/tools.ts`
- `scripts/check-catalog-spec-drift.py`

### `5d013f66` — feat(catalog): tool twins of existing catalog services

- `backend/src/handlers/services.rs`
- `backend/src/handlers/tools_tests.rs`
- `backend/src/models/downstream_service.rs`
- `backend/src/routes.rs`
- `backend/src/services/catalog_skill_service.rs`
- `backend/src/services/mod.rs`
- `backend/src/services/tool_twin_service.rs`

### `704a8e1b` — feat(cli): catalog endpoint/publish commands, tools commands, tool flags

- `cli/src/cli.rs`
- `cli/src/commands/catalog_tools.rs`
- `cli/src/commands/service.rs`

### `1f756574` — feat(frontend): Tools page, admin Tools workspace, editor fields

- `frontend/src/components/dashboard/sidebar.tsx`
- `frontend/src/hooks/use-catalog-admin.ts`
- `frontend/src/pages/admin-tools.tsx`
- `frontend/src/pages/tools.test.tsx`
- `frontend/src/router.tsx`
- `frontend/src/schemas/tools.test.ts`
- `frontend/src/schemas/tools.ts`

### `d2745306` — feat(tools): programmatic tool twin script

- `scripts/tools/add-tool-twin.sh`

### `9f5f6c41` — fix(tools): gate unconfigured public twins before credential resolution

- `backend/src/handlers/endpoints.rs`
- `backend/src/handlers/proxy.rs`
- `backend/src/handlers/tools_tests.rs`
- `backend/src/services/tool_publication_service.rs`

### `a5ee0216` — docs(tools): TOOLS.md, API.md, CLAUDE.md

- `docs/API.md`
- `docs/TOOLS.md`
- `scripts/tools/add-tool-twin.sh`

### `b6e0b136` — fix(catalog): retain canonical twin provenance

- `backend/src/services/tool_twin_service.rs`

### `67b0969d` — fix(cli): apply Tools filters to JSON listings

- `cli/src/commands/catalog_tools.rs`

### `77102917` — test(tools): cover provider-linked twin source isolation

- `backend/src/handlers/tools_tests.rs`

### `42727835` — fix(mcp): preserve diagnostics for empty legacy operation sets

- `backend/src/services/mcp_service.rs`

### `ef51563a` — fix(tools): preserve legacy catalog creator checks

- `backend/src/handlers/endpoints.rs`
- `backend/src/handlers/services.rs`
- `backend/src/handlers/services_helpers.rs`

### `2d079e43` — test(tools): default metadata in legacy database fixture

- `backend/src/db.rs`

### `eb0d0053` — fix(tools): sync spec additions into existing tool twins

- `backend/src/services/catalog_spec_sync.rs`

### `774761ab` — test(tools): assert twin generation on the persisted endpoint

- `backend/src/handlers/tools_tests.rs`

### `ebb46e3b` — fix(tools): enforce publication on anonymous HTTP proxy

- `backend/src/handlers/public_proxy.rs`

### `61098c6e` — fix(tools): enforce publication before LLM gateway execution

- `backend/src/handlers/llm_gateway.rs`

### `2325a1af` — fix(tools): separate API import dates from BSON storage

- `backend/src/services/catalog_import_source.rs`
- `backend/src/services/mod.rs`
- `backend/src/services/tools_service.rs`
- `backend/src/handlers/services.rs`
- `backend/src/handlers/services_helpers.rs`
- `backend/src/handlers/catalog.rs`
- `docs/TOOLS.md`

### `0552b86c` — test(tools): grant proxy scope to execution regression tokens

- `backend/src/handlers/tools_tests.rs`

### `6037e566` — test(tools): include inactive drafts in spec sync assertions

- `backend/src/services/catalog_spec_sync.rs`

### Final verification-record commit — docs(tools): record Phase 1 verification and manual proof

- `docs/plans/2026-10-08-tools-verification.md`
- `docs/plans/2026-10-08-tools-manual-proof.txt`

## Deviations and reasons

- The repository's executable proxy lives in `backend/src/handlers/proxy.rs`, rather than the plan's `proxy_service::execute_proxy`. It calls the shared publication service for HTTP and WebSocket execution, including an early check for unconfigured public twins. The separate anonymous HTTP and LLM gateway forwarding paths also enforce publication.
- Existing managed API seeds remain unchanged; no **tool** seeds are present. Chrono LLM catalog rows are admin-created. The isolated fresh proof database therefore prepares an eligible Chrono baseline through the admin CLI, with dummy credentials and published operations, before exercising the in-place flip. This does not add startup seeds.
- The CLI accepts the revised proof's `--slug` flag for catalog creation as well as its existing positional catalog slug, while preserving `--slug` as a custom connection name outside catalog administration.
- The script uses only `nyxid` for server reads/writes; local Python 3 compares JSON. It pauses previously published operations outside the chosen set and skips unchanged writes, preserving generations on the second run.
- The completed original schema/publication/listing commits predate the user's revised commit order. Deferred work is removed in an explicit scope-cleanup commit, then the remaining twin, CLI, frontend, script and documentation steps follow the revised order.
- Twins require an active HTTP source with supported credential injection. Other transports and unsupported authentication methods cannot be safely cloned using the Phase 1 transport-field allowlist; all four managed API targets use supported methods.
- Existing creator authorization on legacy non-tool catalog rows is preserved. Tool management and conversion to Tools require platform admin; Phase 1 introduces no editor roles or token scopes.

## Verification outputs

The final Rust backend run uses `NYXID_TEST_DATABASE_URL="mongodb://127.0.0.1:27049/?replicaSet=toolsfinalrs"` and `RUST_TEST_THREADS=32` against an isolated local MongoDB 7.0.37 replica set. Earlier checks and the manual proof use the shared local replica set on 27019. Frontend commands use `export PATH=/opt/homebrew/bin:$PATH`.

- `cargo fmt --all -- --check`: exit 0.
- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0; no warnings.
- `cargo test -p nyxid`: exit 101; 7,349 passed, 132 failed, 6 pre-existing ignored, 0 filtered out; 4,801.17 seconds. All Tools-specific tests passed. Exact failure diagnostics and serial reruns are recorded below.
- `cargo test -p nyxid-cli`: exit 0; 1,499 passed, 0 failed, 6 pre-existing ignored across 12 test-result summaries. No new ignored tests.
- `cd frontend && npm run lint`: exit 0; 0 errors, 29 pre-existing warnings.
- `cd frontend && npm test`: exit 0; 437 test files, 4,300 tests passed, 0 failed. Existing background requests to unavailable localhost:3000 produced connection-refused stderr without failed tests.
- `cd frontend && npm run build`: exit 0; TypeScript and Vite production build passed, including the credential-accept output and mock-footprint assertion.
- `cd frontend && npm run build:wizard`: exit 0; wizard bundle regenerated and unchanged.

## Manual proof

The complete command/output transcript is [2026-10-08-tools-manual-proof.txt](2026-10-08-tools-manual-proof.txt). It uses an isolated local database, local backend on port 4311, and dummy credentials. Authentication secrets are omitted.


The fresh proof ran against the rebuilt server and a new isolated database. Draft Firecrawl proxy and MCP calls returned 12600; publication exposed two operations. The admin saw the unconfigured offering while the normal user did not; storing dummy credentials made it visible to the normal user. `/keys` hid its platform binding while `/user-services` retained the connection. The published Firecrawl path reached the vendor and returned 401 for the dummy key. The Chrono flip preserved both live operations and the same BYOK UUID. The `tools-x` script created and published three selected operations, then reported `unchanged (no-op)` on its second run with identical endpoint responses. An additional RFC3339 import-date round trip passed; its publish request initially hit the unchanged rate limiter (429), then succeeded after the window reset.

## Verification environment incidents

Earlier backend runs were interrupted and excluded from final counts. The shared replica set first failed opening an FTDC interim file. After restart with diagnostic data collection disabled, it later hit `Too many open files` and a WiredTiger panic. The shared set was restored without dropping databases. An isolated four-thread run on port 27039 was superseded after correcting a test assertion to include inactive drafts. A subsequent sixteen-thread run lost its MongoDB process without a shutdown or fatal message in the server log; the cause is unknown. Neither partial run contributes to the final counts.

The final unfiltered run uses a fresh replica set on port 27049 with a 1 GB cache, a 10-second minimum snapshot-history window, bounded idle file-handle retention, and a 10 ms journal commit interval. Journaling and transactions remain enabled. No production configuration or application behavior was changed to accommodate these incidents. The final cargo command runs the whole suite without filters.

## Backend failure diagnostics

The full suite has 7,487 tests; the observed 132 failures exceed the plan's approximate 29. There are 13 diagnostics explicitly rejecting dynamic `$getField` and 14 explicitly rejecting client bulk writes that require MongoDB 8+. Another 12 report pending ledger appends, and 15 report failed durable rollup batch writes. Ledger account checkpoint writes use `db.client().bulk_write(...)` in `services/billing/ledger.rs`, so unsupported bulk writes also prevent grant activation, settlement and exact-accounting cutover. The billing, analytics and channel billing source trees are unchanged relative to the pre-feature base (`git diff --stat 83ed6457 -- backend/src/services/billing backend/src/services/admin_usage_service backend/src/services/channel_x_billing_tests.rs backend/src/billing_integration_tests.rs` produces no output).

The table records each actual full-suite failure rather than assigning every timeout or assertion to MongoDB incompatibility. Serial rerun results are listed separately. No production workaround, skipped test or new ignored test was introduced.

| Failed test | Observed diagnostic |
| --- | --- |
| `billing_integration_tests::billing_gate_rejects_missing_and_stale_rate_cache_entries` | assertion failed: matches!(service.open(&missing_ctx).await,;     Err(AppError::BillingNotConfigured(message)) if;     message.contains("missing")) |
| `billing_integration_tests::billing_service_lifecycle_regression` | MongoDB 8+ client bulk-write operation unsupported. |
| `billing_integration_tests::settle_after_midstream_suspension_remains_durable` | MongoDB 8+ client bulk-write operation unsupported. |
| `billing_integration_tests::card_backed_wallet_cannot_reserve_past_the_overdraft_cap` | MongoDB 8+ client bulk-write operation unsupported. |
| `billing_integration_tests::usage::exact_funding_costs_survive_retries_repricing_and_missing_rates` | Ledger append remains pending. |
| `credit_schedule_tests::scheduled_grants_use_existing_active_listing_and_expiry_sweep` | assertion left == right failed;   left: 0;  right: 1 |
| `billing_integration_tests::usage::org_platform_key_request_charges_personal_wallet_and_personal_rollout` | mounted route returned 500 Internal Server Error: {"error":"database_error","error_code":1007,"message":"An internal error occurred"} |
| `billing_integration_tests::usage::org_byok_request_keeps_org_wallet_and_org_rollout` | mounted route returned 500 Internal Server Error: {"error":"database_error","error_code":1007,"message":"An internal error occurred"} |
| `billing_integration_tests::billing_route_coverage_smoke` | ; thread 'billing_integration_tests::billing_route_coverage_smoke' (1577372) panicked at backend/src/billing_integration_tests.rs:285:10:; billing route coverage result: Any { .. } |
| `billing_integration_tests::buffered_route_preserves_success_when_settlement_failure_is_replayed` | mounted route reached controlled downstream: Elapsed(()) |
| `grant_visibility_tests::reconcile_worker_activates_grants_without_lago_or_billing` | reconcile worker should activate the grant without Lago: Elapsed(()) |
| `handlers::assistant_nyxagent::tests::assistant_titles_use_toolless_provider_and_no_route_keeps_provisional` | durable usage settlement: Elapsed(()) |
| `handlers::assistant_team::tests::server_started_turn_preserves_picocredits_and_obeys_cutover` | durable usage settlement: Elapsed(()) |
| `credit_schedule_tests::disburse_due_is_idempotent_across_replicas_and_crashes` | assertion failed: grants.iter().all(\|grant\| grant.issued_ledgered_at.is_some()) |
| `handlers::auth_agent_key::tests::traces_record_hashed_ip_identifiers_and_outcomes_without_secrets` | isolated tracing test timed out: Elapsed(()) |
| `handlers::channel_relay::tests::channel_media_http_auth_and_route_body_limits` | assertion left == right failed;   left: 502;  right: 429 |
| `handlers::channel_relay::x_public_tests::x_public_webhook_to_agent_to_bound_reply_is_deduplicated_and_metered` | durable usage settlement: Elapsed(()) |
| `handlers::channel_relay::activity_tests::typed_x_notifications_are_durable_gated_signed_deduplicated_and_never_replyable` | durable usage settlement: Elapsed(()) |
| `handlers::nyxbot::tests::org_group_bots_moved_to_a_specialist_keep_answering` | assertion left == right failed;   left: 0;  right: 1 |
| `handlers::devices::tests::onboard_handler_creates_payload_and_audits_without_secrets` | device_onboard_created audit entry was not written |
| `handlers::devices::tests::approve_handler_returns_before_slow_notification_dispatch_finishes` | approve should return without waiting for notification task: Elapsed(()) |
| `handlers::service_pool_proxy_tests::billing::pool_proxy_billing_preserves_reported_consumption_from_both_attempts` | pool request response: ServicePoolInfrastructureUnavailable |
| `handlers::service_pool_proxy_tests::billing::pool_proxy_billing_releases_proven_unsent_hold_before_byok_admission` | all attempt reservations are durably finalized or released: Elapsed(()) |
| `handlers::service_pool_proxy_tests::billing::pool_proxy_billing_unknown_timeout_releases_hold_without_inventing_usage` | assertion left == right failed;   left: 0;  right: 1 |
| `handlers::service_pool_proxy_tests::billing::pool_proxy_revoked_platform_grant_excludes_member_before_byok_dispatch` | pool request response: OrgQueryTimeout |
| `handlers::service_pool_proxy_tests::billing::pool_proxy_billing_releases_rejected_platform_hold_before_byok_admission` | all attempt reservations are durably finalized or released: Elapsed(()) |
| `handlers::service_pool_proxy_tests::pool_proxy_attempt_deadline_bounds_headers_first_body_and_rejection_drain` | pool request response: ServicePoolDeadlineExceeded { attempts: [] } |
| `handlers::service_pool_proxy_tests::pool_proxy_overall_deadline_stops_before_backup_dispatch` | assertion left == right failed;   left: Null;  right: 1 |
| `handlers::service_pool_proxy_tests::runtime::pool_all_cooled_uses_standard_error_and_retry_after` |     119 |
| `handlers::service_pool_proxy_tests::runtime::pool_hung_403_body_keeps_configured_health_and_retry_policy` | assertion left == right failed;   left: 0;  right: 1 |
| `handlers::trigger_scheduler::tests::schedule_fake_agent_latency_authority_and_event_streak` | ; thread 'handlers::trigger_scheduler::tests::schedule_fake_agent_latency_authority_and_event_streak' (1643899) panicked at backend/src/handlers/trigger_scheduler_tests.rs:220:5:; assertion failed: latency < 5000 |
| `handlers::trigger_webhooks::tests::webhook_target_acks_before_delivery_finishes` | ingress should acknowledge before delivery: Elapsed(()) |
| `services::admin_usage_service::tests::analytics_calendar_intervals_and_token_measures_conserve_folded_usage` | MongoDB 7 rejects dynamic `$getField` (5654601). |
| `services::admin_usage_service::tests::analytics_daily_and_hourly_series_match_across_rollup_sources` | MongoDB 7 rejects dynamic `$getField` (5654601). |
| `services::admin_usage_service::tests::analytics_preserves_unknown_cost_gaps_and_aggregate_population` | MongoDB 7 rejects dynamic `$getField` (5654601). |
| `services::admin_usage_service::tests::analytics_conserves_top_other_and_buckets_before_and_after_folding` | MongoDB 7 rejects dynamic `$getField` (5654601). |
| `services::admin_usage_service::tests::daily_hourly_edges_and_tail_match_raw_summary_and_ranking` | Durable rollup batch write failed. |
| `services::admin_usage_service::tests::deduplicates_components_and_resale_and_attributes_org_usage` | MongoDB 7 rejects dynamic `$getField` (5654601). |
| `services::admin_usage_service::tests::exact_cost_ranking_orders_decimal_values_before_and_after_folding` | MongoDB 7 rejects dynamic `$getField` (5654601). |
| `services::admin_usage_service::tests::handler_allows_admin_and_operator_and_rejects_regular_users` | MongoDB 7 rejects dynamic `$getField` (5654601). |
| `services::admin_usage_service::tests::exact_and_legacy_costs_use_model_rates_and_preserve_unknowns` | MongoDB 7 rejects dynamic `$getField` (5654601). |
| `services::admin_usage_service::tests::hourly_and_daily_reductions_have_covering_indexes` | Durable rollup batch write failed. |
| `services::admin_usage_service::tests::identities_without_display_names_use_email` | MongoDB 7 rejects dynamic `$getField` (5654601). |
| `services::admin_usage_service::tests::hourly_cost_partitions_preserve_legacy_rounding_and_unknown_masking` | MongoDB 7 rejects dynamic `$getField` (5654601). |
| `services::admin_usage_service::tests::hourly_integer_cost_reduction_is_exact_below_saturation_and_clamps_overflow` | Durable rollup batch write failed. |
| `services::admin_usage_service::tests::pico_costs_retain_472_micros_across_raw_rollup_api_and_analytics` | MongoDB 7 rejects dynamic `$getField` (5654601). |
| `services::admin_usage_service::tests::platform_cost_totals_saturate_without_float_rounding` | MongoDB 7 rejects dynamic `$getField` (5654601). |
| `services::admin_usage_service::tests::ranking_is_paged_in_mongo_and_all_metrics_and_lanes_remain_visible` | MongoDB 7 rejects dynamic `$getField` (5654601). |
| `services::admin_usage_service::tests::persistently_active_standalone_batch_returns_unvalidated_success_within_bound` | Durable rollup batch write failed. |
| `handlers::service_pool_proxy_tests::runtime::pool_proxy_known_429_with_hung_first_body_keeps_rejection_evidence` | pool request response: ServicePoolDeadlineExceeded { attempts: [PoolAttemptSummary { attempt: 1, priority: 0, reason: "preparation_timeout", upstream_status: None }] } |
| `handlers::service_pool_proxy_tests::runtime::pool_health_403_requires_explicit_trigger_on_repeated_requests` | assertion left == right failed;   left: 0;  right: 1 |
| `services::assistant_oneshot_inference::tests::assistant_oneshot_platform_usage_bills_acting_person` | durable usage settlement: Elapsed(()) |
| `services::assistant_oneshot_inference::tests::assistant_oneshot_model_cache_skips_discovery_but_resolves_credentials_and_acl` | durable usage settlement: Elapsed(()) |
| `services::assistant_oneshot_inference::tests::assistant_oneshot_org_byok_requires_live_actor_access_and_bills_person` | durable usage settlement: Elapsed(()) |
| `services::assistant_oneshot_inference::tests::assistant_oneshot_timeout_cancels_without_retry_and_settles_meter` | durable usage settlement: Elapsed(()) |
| `services::assistant_oneshot_inference::tests::assistant_oneshot_utility_400_falls_back_without_charging_refused_generation` | durable usage settlement: Elapsed(()) |
| `services::assistant_oneshot_inference::tests::assistant_oneshot_utility_and_learning_use_platform_model_and_acting_person_billing` | durable usage settlement: Elapsed(()) |
| `services::auth_device_service::tests::approve_pending_row_encrypts_tokens_shortens_expiry_and_audits` | audit write timed out: Elapsed(()) |
| `services::billing::exact_tests::concurrent_journal_retries_do_not_add_entries_with_nonunique_dedupe_index` | ; thread 'services::billing::exact_tests::concurrent_journal_retries_do_not_add_entries_with_nonunique_dedupe_index' (1662680) panicked at backend/src/services/billing/exact_tests.rs:989:20:; called Result::unwrap() on an Err value: JoinError::Panic(Id(356588), "called Result::unwrap() on an Err val |
| `services::billing::exact_tests::account_check_uses_constant_work_after_many_postings` | called Result::unwrap() on an Err value: Error { kind: Custom(Any { .. }), labels: {}, wire_version: None, source: None, server_response: None } |
| `services::billing::exact_tests::billing_readiness_does_not_wait_for_rollup_normalization` | called Result::unwrap() on an Err value: BillingProviderUnavailable("Exact accounting cutover is pending; billing is temporarily unavailable") |
| `services::billing::exact_tests::migration_applies_legacy_grant_lock_once_and_recovers_allocation` | assertion left == right failed;   left: Credits(10000000);  right: Credits(7000000) |
| `services::billing::exact_tests::aborted_carry_and_split_retry_with_new_grant_conserve_rational_residue` | MongoDB 8+ client bulk-write operation unsupported. |
| `services::billing::exact_tests::mixed_account_lifecycle_balances_after_issue_hold_release_expire_revoke_and_topup` | called Result::unwrap() on an Err value: BillingProviderUnavailable("Exact accounting cutover is pending; billing is temporarily unavailable") |
| `services::billing::exact_tests::mixed_grant_wallet_fraction_and_legacy_unfunded_row_recovery` | called Result::unwrap() on an Err value: BillingProviderUnavailable("Exact accounting cutover is pending; billing is temporarily unavailable") |
| `services::billing::exact_tests::v2_postings_balance_commit_every_field_and_reconciliation_detects_corruption` | called Result::unwrap() on an Err value: BillingProviderUnavailable("Exact accounting cutover is pending; billing is temporarily unavailable") |
| `services::billing::exact_tests::cutover_isolates_malformed_deltas_and_retries_without_reposting` | assertion left == right failed;   left: 2;  right: 1 |
| `services::billing::exact_tests::pending_cutover_gates_money_but_allows_metering_and_legacy_reads` | assertion failed: exact_migration::ready(&db).await.unwrap() |
| `services::billing::exact_tests::migration_concurrent_idempotent_and_absorbs_late_micro_deltas` | assertion left == right failed;   left: 0;  right: 2 |
| `services::billing::exact_tests::metering_only_and_legacy_wallet_rows_never_consume_benefits` | called Result::unwrap() on an Err value: BillingProviderUnavailable("Exact accounting cutover is pending; billing is temporarily unavailable") |
| `services::billing::grants::tests::all_users_grant_snapshots_active_people_and_organizations` | assertion left == right failed;   left: 0;  right: 2 |
| `services::billing::exact_tests::migration_isolates_bad_documents_and_skips_zero_openings` | assertion failed: exact_migration::ready(&db).await.unwrap() |
| `services::billing::exact_tests::voice_and_reported_token_components_settle_independently_without_replay_debits` | called Result::unwrap() on an Err value: BillingProviderUnavailable("Exact accounting cutover is pending; billing is temporarily unavailable") |
| `services::billing::exact_tests::randomized_real_settlements_conserve_funding_and_account_balances` | called Result::unwrap() on an Err value: BillingProviderUnavailable("Exact accounting cutover is pending; billing is temporarily unavailable") |
| `services::billing::exact_tests::old_actual_append_verify_and_sweep_fail_closed_on_v2` | called Option::unwrap() on a None value |
| `services::billing::exact_tests::sub_micro_grant_funds_zero_wallet_end_to_end_and_44608_tokens` | called Result::unwrap() on an Err value: BillingProviderUnavailable("Exact accounting cutover is pending; billing is temporarily unavailable") |
| `services::billing::exact_tests::voice_seconds_reconcile_uses_allowance_then_grant_then_wallet_once` | called Result::unwrap() on an Err value: BillingProviderUnavailable("Exact accounting cutover is pending; billing is temporarily unavailable") |
| `services::billing::grants::tests::reconcile_recovers_missing_issue_and_terminal_ledger_entries` | assertion left == right failed;   left: 0;  right: 2 |
| `services::billing::ledger::tests::corrupt_checkpoint_does_not_poison_group_commit` | assertion failed: replies.next().unwrap().await.unwrap().is_ok() |
| `services::billing::exact_tests::rollup_normalization_interleaves_with_fold_without_losing_increment` | Durable rollup batch write failed. |
| `services::billing::funding::tests::actual_usage_consumes_allowance_then_grant_then_wallet` | Ledger append remains pending. |
| `services::billing::meter::tests::credential_restriction_meters_every_class_and_charges_only_eligible_ones` | Ledger append remains pending. |
| `services::billing::meter::tests::component_materialization_rereads_concurrently_finalized_rows_without_double_debit` | Ledger append remains pending. |
| `services::billing::meter::tests::persisted_settlement_intent_recovers_when_live_apply_never_starts` | assertion left == right failed;   left: 0;  right: 1 |
| `services::billing::meter::tests::recovery_after_settle_debit_gap_does_not_debit_wallet_twice` | Ledger append remains pending. |
| `services::billing::meter::tests::failed_live_settlement_is_durable_and_recovers_without_double_debit` | assertion left == right failed;   left: 0;  right: 1 |
| `services::billing::provisioning::tests::create_topup_checkout_returns_hosted_url_and_reuses_idempotency_key` | MongoDB 8+ client bulk-write operation unsupported. |
| `services::billing::meter::tests::settle_moves_wallet_once_and_blocks_double_spend_before_lago_sync` | Ledger append remains pending. |
| `services::billing::meter::tests::settle_and_recovery_sweep_debit_wallet_exactly_once` | Ledger append remains pending. |
| `services::billing::meter::tests::settle_first_apply_appends_exactly_one_ledger_entry` | Ledger append remains pending. |
| `services::billing::provisioning::tests::concurrent_wallet_provisioning_persists_one_owner_wallet` | MongoDB 8+ client bulk-write operation unsupported. |
| `services::billing::provisioning::tests::topup_rejects_idempotency_reuse_with_different_amount` | MongoDB 8+ client bulk-write operation unsupported. |
| `services::billing::provisioning::tests::ensure_owner_wallet_is_idempotent_and_persists_lago_ids` | MongoDB 8+ client bulk-write operation unsupported. |
| `services::billing::provisioning::tests::topup_provider_failure_marks_session_failed_and_allows_retry` | assertion failed: matches!(error, crate::errors::AppError::BillingProviderUnavailable(_)) |
| `services::billing::provisioning::tests::wallet_provider_failure_leaves_provisioning_retryable` | MongoDB 8+ client bulk-write operation unsupported. |
| `services::billing::targets::tests::issued_member_grants_snapshot_provenance_and_keep_bounded_ledger_activation` | assertion left == right failed;   left: 0;  right: 2 |
| `services::billing::tests::billing_enabled_with_lago_auto_provisions_missing_wallet` | MongoDB 8+ client bulk-write operation unsupported. |
| `services::billing::topup_expiry::tests::expiry_recovery_discovers_provider_debit_without_voiding_twice` | recovery ledger exists |
| `services::billing::topup_expiry::tests::expiry_sweep_voids_updates_history_and_ledgers` | expiry ledger entry exists |
| `services::billing::webhook::tests::deferred_refresh_services_newer_request_on_next_pass` | NYXID_TEST_DATABASE_URL is configured but MongoDB is not reachable and writable; refusing to fall back to a different test database |
| `services::billing::webhook::tests::deferred_refresh_preserves_successor_lease` | MongoDB 8+ client bulk-write operation unsupported. |
| `services::billing::tests::lane_prices_sync_retry_clear_and_fund_verified_ledger_entries` | Ledger append remains pending. |
| `services::billing::webhook::tests::deferred_topup_refresh_uses_effective_balance_without_flip_flop` | MongoDB 8+ client bulk-write operation unsupported. |
| `services::billing::webhook::tests::wallet_webhook_refreshes_balance_and_clears_accounted_pending_debits` | MongoDB 8+ client bulk-write operation unsupported. |
| `services::billing::webhook::tests::wallet_webhook_keeps_pending_debits_when_unacked_usage_exists` | MongoDB 8+ client bulk-write operation unsupported. |
| `services::billing::tests::mixed_lane_allowances_fund_only_the_charged_metric` | Ledger append remains pending. |
| `services::billing::grants::tests::large_batch_defers_activation_until_ledger_recovery` | assertion left == right failed;   left: 0;  right: 50 |
| `services::billing::usage_rollup::tests::benchmark_daily_fixture_scales_decimal_money_and_quantities` | Durable rollup batch write failed. |
| `services::billing::usage_rollup::tests::bootstrap_claims_obey_count_and_byte_limits_and_finish_every_source` | Durable rollup batch write failed. |
| `services::billing::usage_rollup::tests::aborted_transaction_leaves_sources_and_increments_uncommitted` | Durable rollup batch write failed. |
| `services::billing::usage_rollup::tests::concurrent_replicas_fold_once_and_handle_edges_live_rows_and_late_settlement` | Durable rollup batch write failed. |
| `services::billing::usage_rollup::tests::crash_after_increment_replays_without_double_count_on_standalone_protocol` | Durable rollup batch write failed. |
| `services::billing::usage_rollup::tests::current_hour_exact_unacked_folds_while_legacy_and_unforwarded_remain_live` | Durable rollup batch write failed. |
| `services::billing::usage_rollup::tests::daily_crash_between_tiers_and_before_source_mark_replays_once` | Durable rollup batch write failed. |
| `services::billing::usage_rollup::tests::legacy_integer_hourly_and_daily_buckets_accept_exact_increments_once` | Durable rollup batch write failed. |
| `services::billing::usage_rollup::tests::pre_tier_hourly_history_bootstraps_after_legacy_batch_recovery_and_raw_expiry` | Durable rollup batch write failed. |
| `services::billing::usage_rollup::tests::raw_claims_reaggregate_smaller_prefixes_when_increments_exceed_byte_budget` | Durable rollup batch write failed. |
| `services::billing::tests::component_precision_funding_recovery_and_cleanup` | Ledger append remains pending. |
| `services::channel_adapters::aurinko::tests::aurinko_individual_delete_busy_ingress_keeps_bot_visible_until_retry` | called Result::unwrap() on an Err value: Elapsed(()) |
| `services::channel_x_tests::billing::x_billing_allowance_then_grant_then_wallet` | called Result::unwrap() on an Err value: BillingProviderUnavailable("Exact accounting cutover is pending; billing is temporarily unavailable") |
| `services::channel_x_tests::billing::x_billing_account_verification_reserves_before_lookup` | durable usage settlement: Elapsed(()) |
| `services::channel_x_tests::billing::x_billing_incoming_duplicate_uses_owner_shared_oauth_price_and_one_ledger_entry` | durable usage settlement: Elapsed(()) |
| `services::channel_x_tests::billing::x_billing_no_agent_route_still_accounts_for_received_event` | durable usage settlement: Elapsed(()) |
| `services::channel_x_tests::billing::x_billing_preserves_picocredits_and_obeys_cutover_before_send` | durable usage settlement: Elapsed(()) |
| `services::channel_x_tests::billing::x_billing_split_reply_charges_each_success_and_releases_rejected_chunk` | durable usage settlement: Elapsed(()) |
| `services::billing::exact_tests::two_hundred_concurrent_wallet_settlements_leave_no_locks` | Ledger append remains pending. |
| `services::destination_routing::tests::auto_activation::google_concurrent_sync_inserts_editors_and_updates_old_contracts_once` | bounded concurrent sync: Elapsed(()) |
| `services::device_code_service::approve::tests::approve_extends_near_ttl_expiry_for_delivery_window` | approve near-expiry row: DeviceCodeExpired |
| `services::machine_access_tests::machine_access_legacy_revocation_resumes_bounded_batches_before_acknowledgement` | called Result::unwrap() on an Err value: MachineAuthorityStale |
| `services::channel_x_tests::review::oversized_backlog_advances_with_notice_and_does_not_fail` | assertion left == right failed;   left: Some("100");  right: Some("2000") |
| `services::usage_workspace_service::tests::workspace_handlers_enforce_admin_writes_and_operator_reads` | assertion left == right failed;   left: false;  right: true |
| `services::user_token_service::tests::user_api_key_refresh_is_single_flight_across_replicas` | called Result::unwrap() on an Err value: Conflict("OAuth credential refresh is still in progress") |

## Serial backend reruns

Twenty-eight failed full-suite cases were rerun individually against the same healthy replica set using the compiled final test binary:

```text
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27049/?replicaSet=toolsfinalrs' \
  target/debug/deps/nyxid_server-eeed0b98e0c8954e --exact <test-name> --nocapture --test-threads=1
```

Each invocation ran exactly one test with 7,486 filtered out. Twenty-one passed and seven remained failed. These are additional diagnostic runs, not replacements for the unfiltered result. The failed webhook rerun explicitly returned `the bulk write feature is only supported on MongoDB 8.0+`; the other persistent reruns report failed billing admission/settlement/finalization or analytics assertions. The required unfiltered command therefore did **not** pass in this MongoDB 7 environment. This is a verification deviation, not a claim that all 132 failures were confirmed against an independently compiled baseline. The unchanged subsystem diff, explicit server incompatibilities and serial results are the evidence available.

| Exact rerun test | Result |
| --- | --- |
| `handlers::auth_agent_key::tests::traces_record_hashed_ip_identifiers_and_outcomes_without_secrets` | PASS (exit 0) |
| `handlers::channel_relay::tests::channel_media_http_auth_and_route_body_limits` | PASS (exit 0) |
| `handlers::nyxbot::tests::org_group_bots_moved_to_a_specialist_keep_answering` | PASS (exit 0) |
| `handlers::devices::tests::onboard_handler_creates_payload_and_audits_without_secrets` | PASS (exit 0) |
| `handlers::devices::tests::approve_handler_returns_before_slow_notification_dispatch_finishes` | PASS (exit 0) |
| `handlers::service_pool_proxy_tests::pool_proxy_attempt_deadline_bounds_headers_first_body_and_rejection_drain` | PASS (exit 0) |
| `handlers::service_pool_proxy_tests::pool_proxy_overall_deadline_stops_before_backup_dispatch` | PASS (exit 0) |
| `handlers::service_pool_proxy_tests::runtime::pool_all_cooled_uses_standard_error_and_retry_after` | PASS (exit 0) |
| `handlers::service_pool_proxy_tests::runtime::pool_hung_403_body_keeps_configured_health_and_retry_policy` | PASS (exit 0) |
| `handlers::trigger_scheduler::tests::schedule_fake_agent_latency_authority_and_event_streak` | PASS (exit 0) |
| `handlers::trigger_webhooks::tests::webhook_target_acks_before_delivery_finishes` | PASS (exit 0) |
| `handlers::service_pool_proxy_tests::runtime::pool_proxy_known_429_with_hung_first_body_keeps_rejection_evidence` | PASS (exit 0) |
| `handlers::service_pool_proxy_tests::runtime::pool_health_403_requires_explicit_trigger_on_repeated_requests` | PASS (exit 0) |
| `services::auth_device_service::tests::approve_pending_row_encrypts_tokens_shortens_expiry_and_audits` | PASS (exit 0) |
| `services::channel_adapters::aurinko::tests::aurinko_individual_delete_busy_ingress_keeps_bot_visible_until_retry` | PASS (exit 0) |
| `services::destination_routing::tests::auto_activation::google_concurrent_sync_inserts_editors_and_updates_old_contracts_once` | PASS (exit 0) |
| `services::device_code_service::approve::tests::approve_extends_near_ttl_expiry_for_delivery_window` | PASS (exit 0) |
| `services::machine_access_tests::machine_access_legacy_revocation_resumes_bounded_batches_before_acknowledgement` | PASS (exit 0) |
| `services::channel_x_tests::review::oversized_backlog_advances_with_notice_and_does_not_fail` | PASS (exit 0) |
| `services::usage_workspace_service::tests::workspace_handlers_enforce_admin_writes_and_operator_reads` | FAIL (exit 101) |
| `services::user_token_service::tests::user_api_key_refresh_is_single_flight_across_replicas` | PASS (exit 0) |
| `billing_integration_tests::buffered_route_preserves_success_when_settlement_failure_is_replayed` | FAIL (exit 101) |
| `handlers::service_pool_proxy_tests::billing::pool_proxy_billing_preserves_reported_consumption_from_both_attempts` | FAIL (exit 101) |
| `handlers::service_pool_proxy_tests::billing::pool_proxy_billing_releases_proven_unsent_hold_before_byok_admission` | FAIL (exit 101) |
| `handlers::service_pool_proxy_tests::billing::pool_proxy_billing_unknown_timeout_releases_hold_without_inventing_usage` | PASS (exit 0) |
| `handlers::service_pool_proxy_tests::billing::pool_proxy_revoked_platform_grant_excludes_member_before_byok_dispatch` | FAIL (exit 101) |
| `handlers::service_pool_proxy_tests::billing::pool_proxy_billing_releases_rejected_platform_hold_before_byok_admission` | FAIL (exit 101) |
| `services::billing::webhook::tests::deferred_refresh_services_newer_request_on_next_pass` | FAIL (exit 101) |

## Final evidence

- Final Rust source: `6037e566`; subsequent commit contains verification documents only.
- Formatting and Clippy: final `*-verified` logs, exit 0.
- Backend: `/tmp/nyxid-tools-phase1-backend-stable.log`, exit 101; all 7,487 tests completed.
- Serial diagnostics: `/tmp/nyxid-tools-phase1-retries.log` and `/tmp/nyxid-tools-phase1-extra-retries.log`.
- CLI: `/tmp/nyxid-tools-phase1-cli-final.log`, exit 0.
- Frontend: `/tmp/nyxid-tools-phase1-lint.log`, `/tmp/nyxid-tools-phase1-frontend-tests.log`, `/tmp/nyxid-tools-phase1-frontend-build.log`, all exit 0.
- Manual proof: checked-in full transcript; final proof exit 0. The documented script invocation ran twice with the second run unchanged.
