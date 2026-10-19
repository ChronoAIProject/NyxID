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


## Review round 1
Recorded 2026-10-09T02:11:30.601332+00:00. The Phase 1 scope remains unchanged. PR #1821 stays draft.
### Commits and changed files

<details><summary>d3aaf068 fix(tools): publish operations atomically and reject ambiguous drafts (2 files)</summary>

```text
backend/src/handlers/endpoints.rs
backend/src/services/tool_publication_service.rs
```

</details>

<details><summary>d2710d62 fix(tools): timestamp catalog twin provenance (2 files)</summary>

```text
backend/src/handlers/tools_tests.rs
backend/src/services/tool_twin_service.rs
```

</details>

<details><summary>633c5e02 refactor(tools): isolate transactional publication updates (1 files)</summary>

```text
backend/src/services/tool_publication_service.rs
```

</details>

<details><summary>bdc3ab24 fix(tools): allow clearing supplier and import provenance (2 files)</summary>

```text
backend/src/handlers/services.rs
backend/src/handlers/tools_tests.rs
```

</details>

<details><summary>6315f4de fix(keys): include tool bindings through one authorized server query (5 files)</summary>

```text
backend/src/handlers/keys.rs
backend/src/handlers/service_account_key_reads.rs
backend/src/services/unified_key_service.rs
frontend/src/hooks/use-keys.test.tsx
frontend/src/hooks/use-keys.ts
```

</details>

<details><summary>08ae70a0 fix(tools): list credential presence and represent disabled limits (4 files)</summary>

```text
backend/src/handlers/tools.rs
backend/src/services/tools_service.rs
frontend/src/pages/tools.tsx
frontend/src/schemas/tools.ts
```

</details>

<details><summary>4767dfe5 fix(cli): document tool commands and format offering tables (4 files)</summary>

```text
cli/src/cli.rs
cli/src/commands/catalog_tools.rs
cli/src/commands/service.rs
cli/src/commands/service/catalog_admin.rs
```

</details>

<details><summary>18605146 style(frontend): format tool changes without rewriting legacy files (14 files)</summary>

```text
frontend/src/components/assistant/assistant-platform-service-fields.tsx
frontend/src/components/assistant/nyxbot-agent-forms.tsx
frontend/src/components/cli-wizard/access-scope-card.tsx
frontend/src/components/dashboard/api-key-create-dialog.tsx
frontend/src/components/dashboard/api-key-detail/service-scope-card.tsx
frontend/src/components/dashboard/endpoint-form-dialog.test.tsx
frontend/src/components/dashboard/endpoint-list.tsx
frontend/src/components/layout/dashboard-layout.tsx
frontend/src/hooks/use-endpoints.ts
frontend/src/lib/endpoint-changes.ts
frontend/src/pages/api-key-detail.test.tsx
frontend/src/pages/api-key-detail.tsx
frontend/src/pages/keys.tsx
frontend/src/schemas/endpoints.ts
```

</details>

<details><summary>1b200d84 fix(frontend): integrate tool metadata into the service form (12 files)</summary>

```text
frontend/src/components/dashboard/endpoint-form-dialog.tsx
frontend/src/components/services/catalog-tool-metadata.tsx
frontend/src/components/services/service-tool-fields.tsx
frontend/src/components/services/tool-publication.tsx
frontend/src/pages/admin-edit-forms.test.tsx
frontend/src/pages/admin-tools.tsx
frontend/src/pages/service-edit.helpers.ts
frontend/src/pages/service-edit.test.tsx
frontend/src/pages/service-edit.tsx
frontend/src/pages/tools.test.tsx
frontend/src/schemas/services.ts
frontend/src/types/api.ts
```

</details>

<details><summary>3f74d359 fix(tools): own publication transaction inputs for async dispatch (1 files)</summary>

```text
backend/src/services/tool_publication_service.rs
```

</details>

<details><summary>b65cbdb4 fix(keys): document tool binding opt-in and preserve default dispatch (2 files)</summary>

```text
backend/src/handlers/keys.rs
backend/src/handlers/service_account_key_reads.rs
```

</details>

<details><summary>deb681fe docs(tools): describe review updates to keys metadata and CLI output (2 files)</summary>

```text
docs/API.md
docs/TOOLS.md
```

</details>

<details><summary>8f72a891 merge: origin/main into enrich-ai-search-tools (691 files)</summary>

```text
.agents/plugins/marketplace.json
.claude-plugin/marketplace.json
.claude-plugin/plugin.json
.github/workflows/ci.yml
CLAUDE.md
Cargo.lock
DESIGN.md
README.md
backend/Cargo.toml
backend/specs/catalog/chrono-sandbox.openapi.json
backend/specs/catalog/cloudflare.openapi.json
backend/specs/catalog/github.openapi.json
backend/specs/catalog/lark.openapi.json
backend/specs/catalog/railway.openapi.json
backend/specs/catalog/stripe.openapi.json
backend/specs/catalog/supabase-management.openapi.json
backend/src/api_docs.rs
backend/src/billing_integration_tests.rs
backend/src/billing_integration_tests/usage.rs
backend/src/crypto/jwt.rs
backend/src/db.rs
backend/src/errors/access_denial.rs
backend/src/errors/mod.rs
backend/src/errors/skill_draft.rs
backend/src/handlers/admin.rs
backend/src/handlers/admin_anonymous_endpoints.rs
backend/src/handlers/admin_ownership.rs
backend/src/handlers/agent_bindings.rs
backend/src/handlers/agent_skills.rs
backend/src/handlers/assistant_action_effects_endpoints.rs
backend/src/handlers/assistant_action_effects_keys.rs
backend/src/handlers/assistant_action_effects_services.rs
backend/src/handlers/assistant_agent_learning.rs
backend/src/handlers/assistant_group.rs
backend/src/handlers/assistant_group_tests.rs
backend/src/handlers/assistant_nyxagent.rs
backend/src/handlers/assistant_nyxagent_steering.rs
backend/src/handlers/assistant_nyxagent_steering_tests.rs
backend/src/handlers/assistant_nyxagent_tests.rs
backend/src/handlers/assistant_nyxagent_upload_tests.rs
backend/src/handlers/assistant_team.rs
backend/src/handlers/assistant_team_tests.rs
backend/src/handlers/assistant_uploads.rs
backend/src/handlers/assistant_voice.rs
backend/src/handlers/async_service_operation_tests.rs
backend/src/handlers/async_service_operations.rs
backend/src/handlers/billing.rs
backend/src/handlers/catalog.rs
backend/src/handlers/channel_activity_tests.rs
backend/src/handlers/channel_relay.rs
backend/src/handlers/channel_x_public_tests.rs
backend/src/handlers/delegation.rs
backend/src/handlers/endpoints.rs
backend/src/handlers/exact_service_approvals.rs
backend/src/handlers/key_updates.rs
backend/src/handlers/keys.rs
backend/src/handlers/llm_gateway.rs
backend/src/handlers/login_client_context.rs
backend/src/handlers/machine_access.rs
backend/src/handlers/machine_discovery_tests.rs
backend/src/handlers/machine_mcp_tests.rs
backend/src/handlers/machine_tools.rs
backend/src/handlers/mcp.rs
backend/src/handlers/mcp_chat_authority_tests.rs
backend/src/handlers/mcp_config_routes_tests.rs
backend/src/handlers/mcp_delegation_tests.rs
backend/src/handlers/mcp_path_parameter_tests.rs
backend/src/handlers/mcp_proxy_parity_tests.rs
backend/src/handlers/mcp_transport.rs
backend/src/handlers/mod.rs
backend/src/handlers/nyxbot.rs
backend/src/handlers/nyxbot_chats.rs
backend/src/handlers/nyxbot_gateway_thread_tests.rs
backend/src/handlers/nyxbot_gateway_threads.rs
backend/src/handlers/nyxbot_late_delivery.rs
backend/src/handlers/nyxbot_tests.rs
backend/src/handlers/nyxbot_thread_controls.rs
backend/src/handlers/nyxbot_thread_follow.rs
backend/src/handlers/nyxbot_transport.rs
backend/src/handlers/nyxbot_transport_tests.rs
backend/src/handlers/oauth.rs
backend/src/handlers/oauth_incremental_tests.rs
backend/src/handlers/oauth_registration_tests.rs
backend/src/handlers/org_group.rs
backend/src/handlers/proxy.rs
backend/src/handlers/proxy_concurrency_tests.rs
backend/src/handlers/public_mcp.rs
backend/src/handlers/public_proxy.rs
backend/src/handlers/service_concurrency.rs
backend/src/handlers/service_insights.rs
backend/src/handlers/service_insights_tests.rs
backend/src/handlers/service_pool_billing_tests.rs
backend/src/handlers/service_pool_inspection_tests.rs
backend/src/handlers/service_pool_proxy_tests.rs
backend/src/handlers/service_preference.rs
backend/src/handlers/service_preference_tests.rs
backend/src/handlers/services.rs
backend/src/handlers/ssh_exec.rs
backend/src/handlers/ssh_tunnel.rs
backend/src/handlers/ssh_web_terminal.rs
backend/src/handlers/tools_tests.rs
backend/src/handlers/trigger_scheduler.rs
backend/src/handlers/trigger_scheduler_tests.rs
backend/src/handlers/user_api_keys_external.rs
backend/src/handlers/user_endpoints.rs
backend/src/handlers/user_services_handler.rs
backend/src/handlers/user_tokens.rs
backend/src/handlers/users.rs
backend/src/models/assistant_acknowledgement.rs
backend/src/models/assistant_agent_learning.rs
backend/src/models/assistant_conversation.rs
backend/src/models/assistant_message.rs
backend/src/models/assistant_voice.rs
backend/src/models/async_service_operation.rs
backend/src/models/downstream_service.rs
backend/src/models/mcp_session.rs
backend/src/models/mod.rs
backend/src/models/nyxbot_channel.rs
backend/src/models/oauth_client.rs
backend/src/models/service_billing.rs
backend/src/models/service_concurrency.rs
backend/src/models/service_endpoint.rs
backend/src/models/service_preference.rs
backend/src/models/usage_meter.rs
backend/src/models/usage_rollup_hourly.rs
backend/src/models/user.rs
backend/src/models/user_api_key.rs
backend/src/mw/auth.rs
backend/src/mw/auth_denial_tests.rs
backend/src/routes.rs
backend/src/services/admin_usage_service.rs
backend/src/services/admin_usage_service/analytics.rs
backend/src/services/admin_usage_service/tests.rs
backend/src/services/admin_user_service.rs
backend/src/services/agent_binding_service.rs
backend/src/services/agent_operation_scope_service.rs
backend/src/services/anonymous_endpoint_service.rs
backend/src/services/assistant_account_tools.rs
backend/src/services/assistant_acknowledgement_service.rs
backend/src/services/assistant_agent_credential_service.rs
backend/src/services/assistant_agent_learning.rs
backend/src/services/assistant_agent_learning_review.rs
backend/src/services/assistant_agent_learning_review_tests.rs
backend/src/services/assistant_authority_tests.rs
backend/src/services/assistant_group_service.rs
backend/src/services/assistant_instruction_context.rs
backend/src/services/assistant_learning_publication.rs
backend/src/services/assistant_links.rs
backend/src/services/assistant_live.rs
backend/src/services/assistant_nyxagent.rs
backend/src/services/assistant_nyxagent_tests.rs
backend/src/services/assistant_oneshot_inference.rs
backend/src/services/assistant_skill_authoring.rs
backend/src/services/assistant_skill_authoring_tests.rs
backend/src/services/assistant_skill_authoring_validation.rs
backend/src/services/assistant_skill_authoring_validation_tests.rs
backend/src/services/assistant_steering.rs
backend/src/services/assistant_team_service.rs
backend/src/services/assistant_team_tools.rs
backend/src/services/assistant_title_service.rs
backend/src/services/assistant_voice.rs
backend/src/services/async_service_operation.rs
backend/src/services/billing/funding.rs
backend/src/services/billing/funding/target_tests.rs
backend/src/services/billing/lago_client.rs
backend/src/services/billing/meter.rs
backend/src/services/billing/mod.rs
backend/src/services/billing/reconcile.rs
backend/src/services/billing/route_inventory.rs
backend/src/services/billing/usage_rollup.rs
backend/src/services/billing/usage_rollup/tests.rs
backend/src/services/billing/webhook.rs
backend/src/services/catalog_spec_registry.rs
backend/src/services/catalog_spec_sync.rs
backend/src/services/channel_thread_follow_service.rs
backend/src/services/channel_thread_follow_service/turns.rs
backend/src/services/channel_thread_service.rs
backend/src/services/channel_thread_service/gateway.rs
backend/src/services/channel_thread_service/resolution.rs
backend/src/services/connect_link_service.rs
backend/src/services/connection_expiry_service.rs
backend/src/services/destination_approval_tests.rs
backend/src/services/destination_routing_tests.rs
backend/src/services/durable_operation_grant_service.rs
backend/src/services/exact_service_approval_service.rs
backend/src/services/feature_flag_service.rs
backend/src/services/gcp_sa_service.rs
backend/src/services/google_auto_activation_tests.rs
backend/src/services/identity_service.rs
backend/src/services/llm_usage_service.rs
backend/src/services/machine_tools.rs
backend/src/services/mcp_service.rs
backend/src/services/mod.rs
backend/src/services/oauth_app_source.rs
backend/src/services/oauth_client_dcr_tests.rs
backend/src/services/oauth_client_service.rs
backend/src/services/oauth_flow.rs
backend/src/services/oauth_resource_service.rs
backend/src/services/oauth_revocation.rs
backend/src/services/oauth_service.rs
backend/src/services/openapi_parser.rs
backend/src/services/operation_path.rs
backend/src/services/org_group_service.rs
backend/src/services/ownership_transfer_tests.rs
backend/src/services/permission_policy_service.rs
backend/src/services/permission_policy_service_tests.rs
backend/src/services/platform_key_service/tests.rs
backend/src/services/provider_service.rs
backend/src/services/proxy_authorization.rs
backend/src/services/proxy_service.rs
backend/src/services/reporting_identity_service.rs
backend/src/services/scope_catalog.rs
backend/src/services/service_concurrency_service.rs
backend/src/services/service_concurrency_service/tests.rs
backend/src/services/service_endpoint_service.rs
backend/src/services/service_history/projection.rs
backend/src/services/service_history/tests.rs
backend/src/services/service_insights_activity.rs
backend/src/services/service_insights_billing.rs
backend/src/services/service_pool_service.rs
backend/src/services/service_preference_service.rs
backend/src/services/ssh_service.rs
backend/src/services/telegram_new_service.rs
backend/src/services/tool_publication_service.rs
backend/src/services/tools_service.rs
backend/src/services/unified_key_service.rs
backend/src/services/user_api_key_service.rs
backend/src/services/user_credentials_service.rs
backend/src/services/user_preferences_service.rs
backend/src/services/user_service_service.rs
backend/src/services/user_token_service.rs
backend/src/services/voice/credentials.rs
backend/src/services/voice/grok_runtime.rs
backend/src/services/voice/receipt.rs
backend/src/services/voice/runtime.rs
backend/src/services/voice/tests.rs
backend/src/services/voice/transcript.rs
backend/src/test_utils.rs
backend/tests/fixtures/skills/community-workflow-zh.md
cli/Cargo.toml
cli/src/api.rs
cli/src/auth.rs
cli/src/auth/login_exchange.rs
cli/src/auth/login_exchange/tests.rs
cli/src/auth/login_input.rs
cli/src/auth/login_input/tests.rs
cli/src/cli.rs
cli/src/commands/ai_setup.rs
cli/src/commands/service.rs
cli/src/credential_guidance.rs
cli/src/error_format.rs
cli/src/main.rs
cli/src/net_diagnostics.rs
cli/src/wizard/assets/index.html
cli/src/wizard/bundle-meta/index.hash
cli/src/wizard/bundle-meta/index.manifest
cli/tests/login_code_input.rs
cli/tests/service_preference.rs
cli/tests/service_preference_transport.rs
docs/AGENT_ISOLATION.md
docs/AGENT_LEARNING.md
docs/AGENT_SKILLS.md
docs/AI_SERVICES_ARCHITECTURE.md
docs/API.md
docs/API_DISCOVERY.md
docs/BILLING_ANALYTICS.md
docs/BILLING_EXACT_ACCOUNTING.md
docs/BILLING_UI_GLOSSARY.md
docs/CHANNEL_EVENT_GATEWAY.md
docs/CHANNEL_THREAD_FOLLOW.md
docs/CHANNEL_THREAD_FOLLOW_GATEWAY_CONTRACT.md
docs/CLAUDE_PLUGIN.md
docs/CLOUD_PLATFORM_OAUTH.md
docs/CODEX_PLUGIN.md
docs/DEPLOYMENT.md
docs/MACHINE_AGENT_ISOLATION.md
docs/MACHINE_NODES.md
docs/MANAGED_OAUTH_CONNECTORS.md
docs/MCP_DELEGATION_FLOW.md
docs/OIDC.md
docs/ORG_AGENTS.md
docs/PLUGINS.md
docs/POSTHOG_CONNECTIONS.md
docs/POSTHOG_OAUTH.md
docs/SERVICE_CONCURRENCY_LIMITS.md
docs/SERVICE_CONFIGURATION.md
docs/SERVICE_POOL_ROUTING_PROOF.md
docs/STRIPE_OAUTH.md
docs/chat/08-nyxagent-engine.md
docs/chat/09-nyxbot-orchestrator.md
docs/chat/10-uploads.md
docs/connecting-services/README.md
docs/plans/ai-service-connection-user-flow.md
docs/plans/consolidated-services-flow.md
docs/plans/local-routing-preview.md
docs/plans/posthog-oauth-fable-review.md
docs/plans/posthog-oauth.md
docs/plans/references/services-card-reference.html
docs/plans/service-billing-labels.md
docs/plans/service-route-resolution-flow.md
docs/plans/service-tool-preference-order.md
docs/plans/service-tool-preference-review.md
docs/plans/services-consolidated-fable-review.md
docs/plans/slug-connection-resolution-proposal.md
docs/plans/slug-routing-fable-review.md
docs/quickstarts/supabase.md
docs/site/cli/getting-started/authenticate.md
docs/site/cli/guides/connect-a-service.md
docs/site/shared/concepts/oauth-oidc.md
frontend/Dockerfile
frontend/dev/routing-preview.ts
frontend/e2e/assistant-account-menu.spec.ts
frontend/e2e/assistant-mobile-layout.spec.ts
frontend/e2e/billing-page.spec.ts
frontend/e2e/nyxagent-authority.spec.ts
frontend/e2e/service-card-scroll.spec.ts
frontend/e2e/service-preference.spec.ts
frontend/e2e/usage-token-breakdown.spec.ts
frontend/e2e/wizard-scope.spec.ts
frontend/nginx.conf.template
frontend/package-lock.json
frontend/package.json
frontend/public/nyxid-coloured-icon.png
frontend/scripts/build-version.ts
frontend/src/app.css
frontend/src/components/admin-credits/credit-pickers.tsx
frontend/src/components/assistant/assistant-account-menu.tsx
frontend/src/components/assistant/assistant-account-panel.tsx
frontend/src/components/assistant/assistant-chat-page.tsx
frontend/src/components/assistant/assistant-drawer-context.ts
frontend/src/components/assistant/assistant-link-modals.test.tsx
frontend/src/components/assistant/assistant-link-modals.tsx
frontend/src/components/assistant/assistant-shell.tsx
frontend/src/components/assistant/assistant-sidebar.test.tsx
frontend/src/components/assistant/assistant-sidebar.tsx
frontend/src/components/assistant/authored-skill-card.test.tsx
frontend/src/components/assistant/authored-skill-card.tsx
frontend/src/components/assistant/automation-preferences.tsx
frontend/src/components/assistant/chat-composer.test.tsx
frontend/src/components/assistant/chat-composer.tsx
frontend/src/components/assistant/chat-message.tsx
frontend/src/components/assistant/machine-capabilities.test.tsx
frontend/src/components/assistant/machine-capabilities.tsx
frontend/src/components/assistant/nyxagent-acknowledgement-card.tsx
frontend/src/components/assistant/nyxagent-composer.test.tsx
frontend/src/components/assistant/nyxagent-composer.tsx
frontend/src/components/assistant/nyxbot-agent-details.test.tsx
frontend/src/components/assistant/nyxbot-channel-chats.tsx
frontend/src/components/assistant/nyxbot-channels.tsx
frontend/src/components/assistant/nyxbot-group-view.tsx
frontend/src/components/assistant/nyxbot-home.tsx
frontend/src/components/assistant/nyxbot-settings-button.tsx
frontend/src/components/assistant/nyxbot-settings-content.test.tsx
frontend/src/components/assistant/nyxbot-settings-content.tsx
frontend/src/components/assistant/nyxbot-settings-dialog.test.tsx
frontend/src/components/assistant/nyxbot-settings-dialog.tsx
frontend/src/components/assistant/upload-composer.test.tsx
frontend/src/components/assistant/upload-composer.tsx
frontend/src/components/auth/auth-flow.tsx
frontend/src/components/auth/device-approval.tsx
frontend/src/components/auth/mfa-setup-dialog.tsx
frontend/src/components/auth/mfa-verify-form.tsx
frontend/src/components/billing-analytics/analytics-canvas.tsx
frontend/src/components/billing-analytics/controls.test.tsx
frontend/src/components/billing-analytics/sample-data.ts
frontend/src/components/billing-analytics/token-metric-picker.test.tsx
frontend/src/components/billing-analytics/token-metric-picker.tsx
frontend/src/components/billing-analytics/token-metrics.ts
frontend/src/components/billing-analytics/token-usage-value.test.tsx
frontend/src/components/billing-analytics/token-usage-value.tsx
frontend/src/components/billing-route-guard.test.tsx
frontend/src/components/billing/billing-activity.tsx
frontend/src/components/billing/billing-allowance-details.tsx
frontend/src/components/billing/billing-benefits-card.tsx
frontend/src/components/billing/billing-benefits.test.tsx
frontend/src/components/billing/billing-funding-chart.test.tsx
frontend/src/components/billing/billing-funding-chart.tsx
frontend/src/components/billing/billing-metric-picker.tsx
frontend/src/components/billing/billing-multi-select.tsx
frontend/src/components/billing/billing-page.css
frontend/src/components/billing/billing-quantity-chart.tsx
frontend/src/components/billing/billing-usage-details.tsx
frontend/src/components/billing/billing-usage-explorer.tsx
frontend/src/components/billing/credits-denied-dialog.tsx
frontend/src/components/billing/credits-denied-host.tsx
frontend/src/components/billing/credits-denied.test.tsx
frontend/src/components/build-update-banner.test.tsx
frontend/src/components/build-update-banner.tsx
frontend/src/components/cli-wizard/access-scope-mode-a.test.tsx
frontend/src/components/cli-wizard/client.ts
frontend/src/components/cli-wizard/shell.tsx
frontend/src/components/dashboard/add-key-dialog.test.tsx
frontend/src/components/dashboard/add-key-dialog.tsx
frontend/src/components/dashboard/agent-plugins-section.test.tsx
frontend/src/components/dashboard/agent-plugins-section.tsx
frontend/src/components/dashboard/api-key-detail/bindings-card.tsx
frontend/src/components/dashboard/api-key-detail/callback-url-card.tsx
frontend/src/components/dashboard/api-key-detail/details-card.tsx
frontend/src/components/dashboard/api-key-detail/node-scope-card.tsx
frontend/src/components/dashboard/api-key-detail/platform-card.tsx
frontend/src/components/dashboard/api-key-detail/rate-limit-card.tsx
frontend/src/components/dashboard/api-key-detail/service-scope-card.tsx
frontend/src/components/dashboard/api-key-detail/usage-stats-card.tsx
frontend/src/components/dashboard/api-key-detail/verify-key-card.tsx
frontend/src/components/dashboard/api-key-dialog.tsx
frontend/src/components/dashboard/connection-delete-action.test.tsx
frontend/src/components/dashboard/connection-delete-action.tsx
frontend/src/components/dashboard/device-code-dialog.tsx
frontend/src/components/dashboard/grouped-service-cards.tsx
frontend/src/components/dashboard/notification-setup-card.tsx
frontend/src/components/dashboard/pool-connections-editor.tsx
frontend/src/components/dashboard/pool-controls.tsx
frontend/src/components/dashboard/pool-editor.tsx
frontend/src/components/dashboard/pool-health-dialog.tsx
frontend/src/components/dashboard/pool-labels.ts
frontend/src/components/dashboard/provider-card.test.tsx
frontend/src/components/dashboard/provider-card.tsx
frontend/src/components/dashboard/routing-section.tsx
frontend/src/components/dashboard/sa-device-code-dialog.tsx
frontend/src/components/dashboard/service-agent-order-panel.tsx
frontend/src/components/dashboard/service-avatar-stack.test.tsx
frontend/src/components/dashboard/service-avatar-stack.tsx
frontend/src/components/dashboard/service-billing-summary.test.tsx
frontend/src/components/dashboard/service-billing-summary.tsx
frontend/src/components/dashboard/service-card-motion.tsx
frontend/src/components/dashboard/service-connection-table.tsx
frontend/src/components/dashboard/service-filter-multiselect.tsx
frontend/src/components/dashboard/service-history.tsx
frontend/src/components/dashboard/service-insight-panels.tsx
frontend/src/components/dashboard/service-insights.test.tsx
frontend/src/components/dashboard/service-order-actions.tsx
frontend/src/components/dashboard/service-order-keyboard.ts
frontend/src/components/dashboard/service-order-rows.tsx
frontend/src/components/dashboard/service-owner-avatar.tsx
frontend/src/components/dashboard/service-pool-cards.tsx
frontend/src/components/dashboard/service-pool-icons.tsx
frontend/src/components/dashboard/service-pool-routing-panel.test.tsx
frontend/src/components/dashboard/service-pool-routing-panel.tsx
frontend/src/components/dashboard/service-pool-summary.tsx
frontend/src/components/dashboard/service-pools-tab.tsx
frontend/src/components/dashboard/service-routing-preview.test.tsx
frontend/src/components/dashboard/service-routing-preview.tsx
frontend/src/components/dashboard/service-saved-views.test.tsx
frontend/src/components/dashboard/service-saved-views.tsx
frontend/src/components/dashboard/service-view-toolbar.tsx
frontend/src/components/dashboard/sidebar.tsx
frontend/src/components/dashboard/user-credentials-dialog.tsx
frontend/src/components/data-table/data-table-columns.tsx
frontend/src/components/data-table/data-table-controls.tsx
frontend/src/components/developer-apps/developer-app-detail.test.tsx
frontend/src/components/developer-apps/developer-app-detail.tsx
frontend/src/components/layout/breadcrumb-context.test.tsx
frontend/src/components/layout/breadcrumb-context.ts
frontend/src/components/layout/dashboard-layout.tsx
frontend/src/components/layout/studio-breadcrumb-trail.test.tsx
frontend/src/components/layout/studio-breadcrumb-trail.tsx
frontend/src/components/orgs/org-developer-apps-tab.tsx
frontend/src/components/providers/codex-connection.tsx
frontend/src/components/service-accounts/service-account-detail.test.tsx
frontend/src/components/service-accounts/service-account-detail.tsx
frontend/src/components/service-icon.test.tsx
frontend/src/components/service-icons/_shared.tsx
frontend/src/components/service-icons/api-airtable.tsx
frontend/src/components/service-icons/api-asana.tsx
frontend/src/components/service-icons/api-attio.tsx
frontend/src/components/service-icons/api-bitbucket.tsx
frontend/src/components/service-icons/api-box.tsx
frontend/src/components/service-icons/api-calendly.tsx
frontend/src/components/service-icons/api-capsule-crm.tsx
frontend/src/components/service-icons/api-clickup.tsx
frontend/src/components/service-icons/api-cloudflare.tsx
frontend/src/components/service-icons/api-crowdin.tsx
frontend/src/components/service-icons/api-dialpad.tsx
frontend/src/components/service-icons/api-dropbox.tsx
frontend/src/components/service-icons/api-eventbrite.tsx
frontend/src/components/service-icons/api-figma.tsx
frontend/src/components/service-icons/api-gitlab.tsx
frontend/src/components/service-icons/api-hubspot.tsx
frontend/src/components/service-icons/api-intercom.tsx
frontend/src/components/service-icons/api-jira.tsx
frontend/src/components/service-icons/api-linear.tsx
frontend/src/components/service-icons/api-miro.tsx
frontend/src/components/service-icons/api-pagerduty.tsx
frontend/src/components/service-icons/api-posthog-eu.tsx
frontend/src/components/service-icons/api-posthog.tsx
frontend/src/components/service-icons/api-productboard.tsx
frontend/src/components/service-icons/api-railway.tsx
frontend/src/components/service-icons/api-sentry.tsx
frontend/src/components/service-icons/api-shippo.tsx
frontend/src/components/service-icons/api-square.tsx
frontend/src/components/service-icons/api-supabase-management.tsx
frontend/src/components/service-icons/api-supabase.tsx
frontend/src/components/service-icons/api-todoist.tsx
frontend/src/components/service-icons/api-zoom.tsx
frontend/src/components/service-icons/index.tsx
frontend/src/components/services/service-concurrency.test.tsx
frontend/src/components/services/service-concurrency.tsx
frontend/src/components/settings/display-settings.test.tsx
frontend/src/components/settings/display-settings.tsx
frontend/src/components/shared/add-cta-button.tsx
frontend/src/components/shared/copyable-url-callout.tsx
frontend/src/components/shared/machine-settings.test.tsx
frontend/src/components/shared/machine-settings.tsx
frontend/src/components/shared/machine-summary.tsx
frontend/src/components/shared/status-badge.tsx
frontend/src/components/shared/twitter-oauth-guidance.tsx
frontend/src/components/ui/button.test.tsx
frontend/src/components/ui/button.tsx
frontend/src/components/ui/date-picker.tsx
frontend/src/components/ui/dialog-focus-return.tsx
frontend/src/components/ui/dialog.tsx
frontend/src/components/ui/input.tsx
frontend/src/components/ui/select.tsx
frontend/src/components/ui/tabs.tsx
frontend/src/features/blog/blog-index-page.tsx
frontend/src/features/blog/components/article-body.tsx
frontend/src/features/blog/components/article-card.tsx
frontend/src/features/blog/components/article-not-found.tsx
frontend/src/features/blog/components/article-view.tsx
frontend/src/hooks/service-order-transport.test.tsx
frontend/src/hooks/use-account-panel.test.tsx
frontend/src/hooks/use-account-panel.ts
frontend/src/hooks/use-agent-bindings.ts
frontend/src/hooks/use-api-keys.ts
frontend/src/hooks/use-assistant-viewport.test.tsx
frontend/src/hooks/use-assistant-viewport.ts
frontend/src/hooks/use-billing.ts
frontend/src/hooks/use-build-updates.test.tsx
frontend/src/hooks/use-build-updates.ts
frontend/src/hooks/use-card-sequence.ts
frontend/src/hooks/use-feature-flag.ts
frontend/src/hooks/use-keys-identity.test.tsx
frontend/src/hooks/use-keys.test.tsx
frontend/src/hooks/use-keys.ts
frontend/src/hooks/use-service-concurrency.ts
frontend/src/hooks/use-service-group-order.test.tsx
frontend/src/hooks/use-service-group-order.ts
frontend/src/hooks/use-service-insights.ts
frontend/src/hooks/use-service-preference.test.tsx
frontend/src/hooks/use-service-preference.ts
frontend/src/hooks/use-service-routing-pools.test.tsx
frontend/src/hooks/use-service-routing-pools.ts
frontend/src/hooks/use-service-view.test.tsx
frontend/src/hooks/use-service-view.ts
frontend/src/hooks/use-theme.test.tsx
frontend/src/hooks/use-usage-analytics.test.tsx
frontend/src/hooks/use-usage-analytics.ts
frontend/src/lib/agent-grant.test.ts
frontend/src/lib/agent-grant.ts
frontend/src/lib/agent-plugins.ts
frontend/src/lib/api-client.ts
frontend/src/lib/assistant/account-panel-availability.ts
frontend/src/lib/assistant/account-panel-search.test.ts
frontend/src/lib/assistant/account-panel-search.ts
frontend/src/lib/assistant/assistant-link-target.test.ts
frontend/src/lib/assistant/assistant-link-target.ts
frontend/src/lib/assistant/chat-types.ts
frontend/src/lib/assistant/nyxagent-http-fixtures.ts
frontend/src/lib/assistant/nyxagent-steering.ts
frontend/src/lib/assistant/nyxagent-transport.test.ts
frontend/src/lib/assistant/nyxagent-transport.ts
frontend/src/lib/assistant/panel-focus.ts
frontend/src/lib/assistant/search.ts
frontend/src/lib/assistant/shell-routes.ts
frontend/src/lib/assistant/uploads.ts
frontend/src/lib/billing-plain.test.ts
frontend/src/lib/billing-plain.ts
frontend/src/lib/build-update-navigation.test.ts
frontend/src/lib/build-update-navigation.ts
frontend/src/lib/build-updates.test.ts
frontend/src/lib/build-updates.ts
frontend/src/lib/build-version.ts
frontend/src/lib/connection-access.ts
frontend/src/lib/credits-denial.test.ts
frontend/src/lib/credits-denial.ts
frontend/src/lib/feature-flags.test.ts
frontend/src/lib/feature-flags.ts
frontend/src/lib/mock-data.ts
frontend/src/lib/overlay-layer.ts
frontend/src/lib/provider-branding.ts
frontend/src/lib/routing-preview-gateway.test.ts
frontend/src/lib/service-billing-config.ts
frontend/src/lib/service-card-summary.test.ts
frontend/src/lib/service-card-summary.ts
frontend/src/lib/service-groups.ts
frontend/src/lib/service-insights-compat.test.tsx
frontend/src/lib/service-insights-compat.ts
frontend/src/lib/service-insights.ts
frontend/src/lib/service-pool-display.test.ts
frontend/src/lib/service-pool-display.ts
frontend/src/lib/service-preference.ts
frontend/src/lib/service-routing-preview.test.ts
frontend/src/lib/service-routing-preview.ts
frontend/src/lib/service-usage.test.ts
frontend/src/lib/service-usage.ts
frontend/src/lib/service-view.test.ts
frontend/src/lib/service-view.ts
frontend/src/lib/studio-breadcrumbs.test.ts
frontend/src/lib/studio-breadcrumbs.ts
frontend/src/lib/url-tabs.ts
frontend/src/main.tsx
frontend/src/pages/admin-audit-log.test.tsx
frontend/src/pages/admin-audit-log.tsx
frontend/src/pages/admin-credits-safety.test.tsx
frontend/src/pages/admin-credits.tsx
frontend/src/pages/admin-edit-forms.test.tsx
frontend/src/pages/admin-invite-codes.tsx
frontend/src/pages/admin-oauth-clients.test.tsx
frontend/src/pages/ai-setup.tsx
frontend/src/pages/api-key-detail.tsx
frontend/src/pages/assistant-workspace.router.test.tsx
frontend/src/pages/assistant.tsx
frontend/src/pages/automations.tsx
frontend/src/pages/billing.test.tsx
frontend/src/pages/billing.tsx
frontend/src/pages/channel-conversation-detail.tsx
frontend/src/pages/dashboard.tsx
frontend/src/pages/developer-apps.tsx
frontend/src/pages/key-detail.test.tsx
frontend/src/pages/key-detail.tsx
frontend/src/pages/keys.test.tsx
frontend/src/pages/keys.tsx
frontend/src/pages/lazy.ts
frontend/src/pages/login.tsx
frontend/src/pages/nyxbot-onboarding.test.tsx
frontend/src/pages/oauth-consent-access.tsx
frontend/src/pages/oauth-consent-preview-data.ts
frontend/src/pages/oauth-consent-preview.tsx
frontend/src/pages/oauth-consent.test.tsx
frontend/src/pages/oauth-consent.tsx
frontend/src/pages/org-developer-app-detail.tsx
frontend/src/pages/org-service-account-detail.tsx
frontend/src/pages/provider-edit.tsx
frontend/src/pages/service-detail.tsx
frontend/src/pages/service-edit.test.tsx
frontend/src/pages/service-edit.tsx
frontend/src/pages/service-overview.test.tsx
frontend/src/pages/service-overview.tsx
frontend/src/pages/settings.test.tsx
frontend/src/pages/settings.tsx
frontend/src/pages/triggers.tsx
frontend/src/router.tsx
frontend/src/schemas/admin-usage.test.ts
frontend/src/schemas/admin-usage.ts
frontend/src/schemas/agent-skills.ts
frontend/src/schemas/assistant-nyxagent.ts
frontend/src/schemas/billing.ts
frontend/src/schemas/machine-access.ts
frontend/src/schemas/machines.ts
frontend/src/schemas/service-concurrency.test.ts
frontend/src/schemas/service-concurrency.ts
frontend/src/schemas/service-insights.ts
frontend/src/schemas/service-preference.test.ts
frontend/src/schemas/service-preference.ts
frontend/src/schemas/service-view.ts
frontend/src/stores/auth-store.ts
frontend/src/stores/build-update-store.test.ts
frontend/src/stores/build-update-store.ts
frontend/src/stores/service-card-view-store.ts
frontend/src/stores/theme-store.test.ts
frontend/src/stores/theme-store.ts
frontend/src/test-setup.ts
frontend/src/test/admin-usage-fixture.ts
frontend/src/test/billing-fixture.ts
frontend/src/types/admin.ts
frontend/src/types/api.ts
frontend/src/types/keys.ts
frontend/test/build-version.test.ts
frontend/test/routing-preview-gateway.test.ts
frontend/vite.config.ts
integrations/claude-plugin/.claude-plugin/plugin.json
integrations/claude-plugin/.mcp.json
integrations/claude-plugin/LICENSE
integrations/claude-plugin/README.md
integrations/claude-plugin/assets/icon.svg
integrations/claude-plugin/skills/nyxid/SKILL.md
integrations/codex-plugin/LICENSE
integrations/codex-plugin/README.md
integrations/codex-plugin/assets/composer-icon.svg
integrations/codex-plugin/assets/logo.svg
integrations/codex-plugin/mcp.json
integrations/codex-plugin/plugin.json
integrations/codex-plugin/skills/nyxid/SKILL.md
integrations/plugin-source/README.md
integrations/plugin-source/plugin.json
integrations/plugin-source/skills/nyxid/SKILL.md
scripts/build-codex-plugin.sh
scripts/check-catalog-spec-drift.py
scripts/sync-plugins.py
scripts/validate-claude-plugin.py
skills/INSTALL.md
```

</details>

### Main merge resolution
- `CLAUDE.md`: Kept main’s discovery-preference rule and Tools execution rule; restored main’s released concurrency reservation and assigned the next free block to tool publication in review round 2.
- `backend/src/handlers/delegation.rs`: Kept main’s consent behavior and async endpoint field alongside all defaulted Tools endpoint fields.
- `backend/src/handlers/endpoints.rs`: Kept hosted async discovery contracts and Tools admin authorization, draft defaults, metadata and transactional publication.
- `backend/src/handlers/keys.rs`: Kept editable-configuration metadata and authorized discovery ranks; combined them with offering_kind and the scoped include_tool_bindings query.
- `backend/src/handlers/mcp_chat_authority_tests.rs`: Kept main’s authority tests and async endpoint field, adding the Tools endpoint defaults to fixtures.
- `backend/src/handlers/mcp_transport.rs`: Kept main’s read-only, concurrency and discovery behavior alongside unpublished Tool rejection and combined endpoint fixtures.
- `backend/src/handlers/mod.rs`: Registered both main’s preference/concurrency handlers and Tools handlers.
- `backend/src/models/downstream_service.rs`: Combined main’s optional concurrency policy with defaulted offering kind, topics, supplier and import provenance.
- `backend/src/models/service_endpoint.rs`: Combined main’s optional async operation contract with defaulted Tools data scope, cost class, execution and publication.
- `backend/src/services/catalog_spec_sync.rs`: Kept URL-aware hosted async contracts and Tools additive draft discovery; remote Tool documents cannot enable hosted-only async contracts.
- `backend/src/services/destination_routing_tests.rs`: Kept routing tests and async fixtures alongside Tools default fields.
- `backend/src/services/durable_operation_grant_service.rs`: Kept main’s grant logic and async field alongside Tools endpoint defaults.
- `backend/src/services/mcp_service.rs`: Kept main’s discovery preference/read-only behavior and Tools discovery/publication filtering; combined endpoint fixtures.
- `backend/src/services/mod.rs`: Registered main’s async/preference services alongside all four Tools services.
- `backend/src/services/proxy_service.rs`: Combined catalog concurrency policy and offering kind in loaded authorization; preserved both during target materialization.
- `backend/src/services/service_endpoint_service.rs`: Kept async-contract generation comparisons/updates alongside Tools metadata, draft inserts and preserved publication.
- `cli/src/wizard/assets/index.html`: Regenerated with npm run build:wizard from the combined frontend; did not hand-merge generated HTML.
- `cli/src/wizard/bundle-meta/index.hash`: Regenerated with npm run build:wizard from the combined source closure.
- `frontend/src/components/layout/dashboard-layout.tsx`: Kept main’s shared breadcrumb architecture; moved Tools breadcrumb labels into studio-breadcrumbs.ts.
- `frontend/src/hooks/use-keys.ts`: Kept main’s acting-identity request guard and query enablement; added Tool inclusion to the query key and single guarded server request.
- `frontend/src/pages/keys.tsx`: Kept main’s named views, grouping, preferences and reconnect components; carried Tool BYOK labels into shared cards and connection tables.
- `frontend/src/pages/service-edit.test.tsx`: Kept concurrency mocks/tests alongside Tools topic mocks and integrated-form tests.
- `frontend/src/pages/service-edit.tsx`: Kept main’s concurrency editor alongside Tools fields in the existing form and admin-only transport/credential/pricing controls.
- `frontend/src/types/keys.ts`: Kept main’s configuration-edit permission and preference fields alongside offering_kind.

The merge has two parents and includes origin/main at db0cee5f. The imported release versions are main’s versions; this review adds no separate version bump. The initial merge incorrectly moved the released concurrency code. Review round 2 restores the released concurrency code and assigns the next free block to tool publication.

### Verification
Checks use the isolated MongoDB 7 replica set at localhost:27049 (toolsfinalrs), one Rust test thread. The original detached build used one Cargo job; subsequent CLI, binary and round-2 runs use default Cargo parallelism. Frontend tests use two workers to bound local resource contention.

- `cargo fmt --all -- --check`: exit 0.

- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0.    Compiling nyxid-cli v0.68.0 (/Users/chronoai/Library/Application Support/heca/worktrees/ea204fe1/solar-spruce/cli)
   Compiling nyxid v0.68.0 (/Users/chronoai/Library/Application Support/heca/worktrees/ea204fe1/solar-spruce/backend)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5m 19s

- `cargo test -p nyxid -- handlers::keys handlers::endpoints handlers::mcp_transport services::mcp_service services::proxy_service services::service_endpoint_service services::catalog_spec_sync handlers::tools_tests services::tools_service services::unified_key_service services::tool_publication_service handlers::proxy::proxy_resolution_integration_tests services::service_concurrency_service models::service_endpoint models::downstream_service --test-threads=1`: exit 0. test result: ok. 785 passed; 0 failed; 0 ignored; 0 measured; 6956 filtered out; finished in 269.06s

- `cargo test -p nyxid-cli`: exit 0. test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s; test result: ok. 1455 passed; 0 failed; 6 ignored; 0 measured; 0 filtered out; finished in 9.75s; test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 23.49s; test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s; test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 11.21s; test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.40s; test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.28s; test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.69s; test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.64s; test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.54s; test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s; test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s; test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.38s; test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s; test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

- `cd frontend && npm run lint`: exit 0. Zero errors, 29 existing warnings.

- `cd frontend && npm test -- --maxWorkers=2`: exit 0. Test Files  485 passed (485); Tests  5033 passed (5033); Duration  260.96s (transform 22.41s, setup 59.02s, import 96.08s, tests 216.45s, environment 84.81s)

- `cd frontend && npm run build`: exit 0.
✓ built in 69ms
Mock scenario footprint assertion passed; credential-accept output is present.

- `cd frontend && npm run build:wizard`: exit 0; generated HTML, 174-file manifest and source hash installed.
- Focused keys/account-switch/Mode A transport tests: 3 files, 26 tests passed. Service editor: 1 file, 6 tests passed.

### Affected manual proof
The transcript is appended under Review rounds 1 and 2 in `docs/plans/2026-10-08-tools-manual-proof.txt`. A fresh backend uses rebuilt binaries, disabled per-user limits, dummy vendor credentials and a new isolated database. Default /keys hides platform Tool bindings; include_tool_bindings=true includes them; both retain the same BYOK UUID. CLI list/show use formatted pricing, limits, topics and operation rows; JSON retains raw shapes. Supplier and provenance null clears and the missing-source CLI error are also exercised.

### Review clarifications and deviations
- The first full post-merge frontend attempt had 2 failed / 5031 passed tests in 485 files: a Mode A cache assertion and global form-count assertion. Both were corrected to preserve main’s default guarded cache and separate concurrency form; focused reruns passed before the final full rerun. Initial merge type checks also caught an MCP session signature change, frozen legacy fixture fields, a Value/Arc comparison and a redundant URL borrow; all were corrected before final checks. The first MCP run had 167 passes and one failure: Tools preflight canonicalized non-Tool paths and rejected main’s valid GitHub trailing-slash root path. Canonicalization now happens only after selecting an eligible Tool row; the final batched run repeats all requested modules.
- B6 required a real change: `platform_key_service::credential_configured` decrypts at backend/src/services/platform_key_service.rs:23; Tools listing now checks only encrypted-byte presence.
- C1’s CatalogCommands::Endpoint/Discover/Publish/Topics variants already had /// help in the reviewed source (cli/src/cli.rs:846,851,857,867 in a613883c). Their fields and formatting were corrected along with the remaining requested help text; no item was skipped.
- B7’s CLI wording and C3’s exact table format conflict. The UI uses "No per-user rate limit"; the CLI uses C3’s explicitly requested "none". Both consume limits=null.
- The numeric-code resolution in the merge was incorrect and is superseded by review round 2. No Phase 2 code or seeds were added.

Requested post-merge module coverage (all passed):

- `handlers::keys`: 74 tests.
- `handlers::endpoints`: 6 tests.
- `handlers::mcp_transport`: 168 tests.
- `services::mcp_service`: 149 tests.
- `services::proxy_service`: 132 tests.
- `services::service_endpoint_service`: 20 tests.
- `services::catalog_spec_sync`: 11 tests.
- `handlers::tools_tests`: 2 tests.
- `services::tools_service`: 2 tests.
- `services::unified_key_service`: 170 tests.

The full CLI rerun completed during round 2 after correcting the test-only PID-file readiness race described below. It passed 1,533 tests across all targets, with six ignored. The first CLI attempt had 1,454 passes, one failure and six ignored in its binary target before stopping.


## Review round 2
Recorded 2026-10-09T02:17:45.650452+00:00. PR #1821 remains draft and mergeable.

### Commits and changed files

- 171fb61d fix(tools): preserve released concurrency error code: `CLAUDE.md`, `backend/src/errors/mod.rs`, `backend/src/handlers/mcp_transport.rs`, `backend/src/handlers/proxy.rs`, `backend/src/handlers/proxy_concurrency_tests.rs`, `backend/src/handlers/tools_tests.rs`, `docs/API.md`, `docs/SERVICE_CONCURRENCY_LIMITS.md`, `docs/TOOLS.md`, `docs/plans/2026-10-08-tools-implementation-plan.md`.

- f9a37612 style(frontend): restore admin form test formatting: `frontend/src/pages/admin-edit-forms.test.tsx`.

- 8cb74d44 test(cli): wait for driver PID before takeover: `cli/src/node/machine/runtime.rs`.

- f65cd4ae docs(tools): append review keys CLI and publication proof: `docs/plans/2026-10-08-tools-manual-proof.txt`.

- This verification-record commit changes only `docs/plans/2026-10-08-tools-verification.md` and appends both missing review sections. Every review commit includes the requested Co-Authored-By trailer.

### Error-code compatibility and repository audit
Concurrency retains the released `ServiceConcurrencyLimited` code **12600**, HTTP 429 and Retry-After behavior. Tool publication uses `ToolOperationNotPublished` code **12700**, HTTP 404. The reserved-code rule, active API/Tools documentation, implementation plan, HTTP/MCP literals and regression assertions agree.

`rg -n "12600|12700" backend cli frontend docs scripts CLAUDE.md` was run after the correction. All current implementation and contract references match those allocations. The original transcript lines 640, 644 and 894 and verification line 408 still contain obsolete publication numbers because the user explicitly required append-only historical records. These are archived observed outputs, superseded by the appended proof; they do not describe the current API. No historical output was rewritten to manufacture a passing result.

### Verification
Default Cargo build parallelism was used for every new round-2 run. The isolated MongoDB 7 replica set remains on localhost:27049; GitHub CI uses MongoDB 8 and is authoritative for the full suite.

- `cargo fmt --all -- --check`: exit 0. Passed.

- `cargo test -p nyxid -- handlers::tools_tests handlers::proxy_concurrency_tests handlers::mcp_transport handlers::public_proxy handlers::llm_gateway::tests::assistant_operation_scope_denies_llm_passthrough_gateway_and_inference_alias --test-threads=1`: exit 0. test result: ok. 181 passed; 0 failed; 0 ignored; 0 measured; 7560 filtered out; finished in 190.66s

- `cargo test -p nyxid assistant_operation_scope_denies_llm_passthrough_gateway_and_inference_alias -- --test-threads=1`: exit 0. test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7740 filtered out; finished in 3.54s

- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0. Passed.

- `cd frontend && npx prettier --check src/pages/admin-edit-forms.test.tsx`: exit 0. Passed.

The combined round-2 command had a nonexistent `llm_gateway::tests` selector. The real test lives in `llm_gateway::agent_operation_tests`; the separate exact-function run above selected and passed it, exercising both provider passthrough and gateway publication gates. Combined results: **182 passed, zero failed**.

- `cargo test -p nyxid-cli`: exit 0; **1,533 passed, zero failed, six ignored** across all targets, including the wizard freshness integration test.
- `cargo build -p nyxid -p nyxid-cli`: exit 0; rebuilt the server and CLI used in the fresh proof.

The first local CLI run exposed a PID-marker race in `takeover_cancels_a_thirty_second_cua_action_and_discards_its_result`: creating the file and writing the PID are separate observable events, so takeover could kill the driver between them. The test now waits for a parseable PID before triggering takeover; cancellation and process-exit assertions remain intact. The first binary target had 1,454 passes, one failure and six ignored. The full rerun passed as reported above. No production CLI behavior was changed.

### Manual proof
`docs/plans/2026-10-08-tools-manual-proof.txt` now appends 597 lines of command/output transcript under **Review rounds 1 and 2**, without rewriting prior records. The rebuilt server runs on localhost:4313 against a fresh isolated database, with limits disabled and dummy vendor credentials. Authentication tokens and passwords are omitted and the transcript was scanned for their presence before commit.

- `GET /api/v1/keys` excludes platform Tool bindings; `GET /api/v1/keys?include_tool_bindings=true` includes them with offering_kind=tool and credential_binding=platform. Both retain the same BYOK UUID and credential_binding=user.
- `nyxid tools list` and `nyxid tools show tools-firecrawl` use table mode: Free pricing, none for disabled limits, comma-joined topics, and one row per published operation. JSON output still reports limits=null and raw operation arrays.
- Draft Firecrawl proxy execution returns HTTP 404 with error_code=12700; `nyx__call_tool` returns HTTP 200 with isError=true and embedded error_code=12700.
- Supplier/provenance null clears and the local missing-source CLI validation also pass.

### CI monitoring

Snapshot before pushing the appended records:

- [Backend Image Inputs](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635841016): IN_PROGRESS.
- [Rust Clippy](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635841137): SUCCESS.
- [Coverage (CLI)](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635840933): SUCCESS.
- [Coverage (Frontend)](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635841065): IN_PROGRESS.
- [Frontend](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635840967): IN_PROGRESS.
- [Rust Features (aws-kms,gcp-kms)](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635840976): SUCCESS.
- [Coverage (Backend Base)](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635841043): SUCCESS.
- [Coverage (Backend)](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635840955): IN_PROGRESS.
- [CLI Wizard Bundle Freshness](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635841007): SUCCESS.
- [Rust Features (gcp-kms)](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635841054): SUCCESS.
- [Rust Features (aws-kms)](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635840989): SUCCESS.
- [CLI Test](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635840982): SUCCESS.
- [Machine Container E2E](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635841071): IN_PROGRESS.
- [Backend Billing Smoke](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635840947): IN_PROGRESS.
- [Backend Test](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635841019): IN_PROGRESS.
- [Claude Plugin Validate](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635840787): SUCCESS.
- [Rust Format](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635840806): SUCCESS.
- [SDK Build](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635842385): SKIPPED.
- [Oracle Worker Test](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635841914): SKIPPED.
- [Codex Plugin Validate](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635842430): SKIPPED.
- [Google Permissions](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635842257): SKIPPED.
- [Cursor Plugin Validate](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635842285): SKIPPED.
- [Mobile](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635842008): SKIPPED.
- [Detect Changes](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635645162): SUCCESS.
- [CodeQL](https://github.com/ChronoAIProject/NyxID/runs/113635754101): NEUTRAL.
- [Workflow Permissions](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042446/job/113635645040): SUCCESS.
- [CodeQL (javascript-typescript)](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042570/job/113635554528): SUCCESS.
- [CodeQL (actions)](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042570/job/113635554386): SUCCESS.
- [CodeQL (rust)](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042570/job/113635554235): IN_PROGRESS.
- [CodeQL (python)](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042570/job/113635554431): SUCCESS.
- [host](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042441/job/113635434267): SKIPPED.
- [announce](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042441/job/113635434498): SKIPPED.
- [Release Integrity Manifest](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042441/job/113635335602): SUCCESS.
- [build-global-artifacts](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042441/job/113635337006): SKIPPED.
- [build-local-artifacts (${{ join(matrix.targets, ', ') }})](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042441/job/113635336591): SKIPPED.
- [plan](https://github.com/ChronoAIProject/NyxID/actions/runs/37873042441/job/113635206413): SUCCESS.

After pushing this round, `gh pr checks 1821` is watched to completion; any failing job is investigated and fixed forward. The final response reports the final pushed-head outcome. The PR remains draft.
