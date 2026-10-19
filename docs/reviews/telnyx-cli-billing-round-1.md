# Native Telnyx CLI coverage: round 1

Base: `618c627b98f4a9362dd7c8b0a942638fc5aa2d4c` (`origin/main`).
Branch: `cli-admin-billing`. Scope: plan Phases 0–5, with no backend source
changes or paid live run. The source plan and Python references remain in the
other worktree. This report records acceptance evidence for review.

## General acceptance

| Status | Requirement | Evidence |
|---|---|---|
| DONE | Fresh main base, no preexisting branch commits | Fetch and ancestry/count checks; base above; baseline CLI test and clippy runs passed before feature edits. |
| DONE | Real response fixtures, sanitized identities and secrets | `cli/tests/fixtures/billing/README.md`; config/rates/overview/service/catalog/empty-executions captures use GET only. |
| DONE | Local write-path option without Lago | `docs/TELNYX_PLATFORM_BILLING.md`, “Local backend without Lago”; wiremock resync tests exercise failed synchronization. The optional manual backend exercise was documented, not started. |
| DONE | Leaf auth flags, profiles, URL selection, JSON stdout, admin table convention | `AuthArgs` flattened on leaves; binary tests `saved_human_profile_and_explicit_base_url_read_config_without_token_override`, `config_json_and_table_redact_secret_values_and_support_saved_profile`; integration helper supplies `NYXID_URL`; common table renderer uses the admin comfy_table preset on stderr. |
| DONE | Local human-session gate before any request | Unit `agent_key_credentials_reject_admin_and_billing_leaves_before_any_request`, `agent_key_profile_rejected_even_with_explicit_human_override`; binary `saved_agent_profile_rejects_every_admin_billing_leaf_with_exit_one_and_zero_requests` covers every new leaf plus existing billing leaves and catalog authoring. |
| DONE | Slug and UUID resolution; ambiguous/missing hints | `slug_fallback_lists_catalog_then_uses_uuid`, `ambiguous_and_missing_slugs_fail_with_catalog_hint`; shared `catalog_admin::fetch_catalog_service`. |
| DONE | JSON HTTP error status/path/body, including axum text | `server_json_and_axum_text_errors_are_http_errors_and_writes_not_retried`, binary `proxy_pending_exits_two_and_http_errors_keep_status_path_and_body`. |
| DONE | No credential output or credential values in argv | Sentinels asserted absent in auth, billing-file, probe-source, config and proxy tests. Upstream credentials remain environment-only; fixtures contain no real credentials. Production read subprocesses received the saved human token only through the environment. |
| DONE | No automatic write replay; grant confirmation | New writes disable refresh replay; HTTP client retry policy is `never`; single-request expectations in config/update/set-managed/resync/grant tests; `grant_issue_without_yes_refuses_nonterminal_and_never_posts`. MCP permits one replay only after explicit session rejection. |
| DONE | Success 0, pending/not-ready/broken 2, errors 1; existing auth 3/4 | Binary tests below prove 0/1/2. Existing `ReauthRequired`/`RefreshUnavailable` handling and authentication suites remain passing. Uses existing `PendingReadiness`. |
| DONE | Exact money and rate handling | Shared `billing_units` validation; checked i128 picocredits; golden ceiling and exact-rate tests; `usage_prints_token_classes_and_all_exact_costs_without_float_conversion`. No f64 money/rate code added. |
| DONE | Required formatting, lint, test and generated-plugin gates | Final check outputs are reported with the local commit. Existing ignored tests remain ignored; no test was disabled. |

## Command acceptance

Test names below are unique suffixes in the CLI suite. Most admin billing tests
are in `cli/src/commands/admin_billing/tests.rs`; binary output tests are in
`cli/tests/billing_commands.rs`.

| Status | Command / behavior | Evidence |
|---|---|---|
| DONE | Config show: enabled/Lago/default effective/source/env and secret presence | `config_show_reads_sanitized_fixture_and_redacts_secret_values`; binary config JSON/table test asserts the secret value is absent. |
| DONE | Config set: exact number or null; local 0..=1e12 validation | `config_set_sends_exact_number_or_null_once`, `config_set_rejects_out_of_range_locally`. |
| DONE | Overview: all effective class and price details | `overview_renders_every_price_class_and_sync_field`; sanitized production fixture and successful production read. |
| DONE | Service show: effective plus raw contract; RFC3339 date | `service_show_joins_effective_and_raw_and_formats_bson_date`; production read succeeded. Table includes sync errors, cached rate, stale/retired and class charge/reason. |
| DONE | Rates: exact configured codes; retired opt-in | `rates_filters_exact_configured_codes_and_retired_rows`; binary `rates_output_has_exactly_the_services_nonretired_codes`; production filtered GET succeeded. |
| DONE | File update: bare/wrapped input, wrapping, server-field stripping, merged dry run and no PUT | `billing_updates_wrap_strip_merge_and_dry_run_without_put`; `billing_update_invalid_json_prices_and_secret_fields_fail_before_requests`. Dry run prints body and field diff; warnings go to stderr. |
| DONE | Set-managed: golden contract, exact prices, clears incompatible billing, reservation ceiling | `set_managed_body_matches_golden_bytes_and_exact_ceiling`, `set_managed_posts_once_and_clears_incompatible_previous_operation_prices`. Explicit empty operations prevent inheritance of incompatible request prices. |
| DONE | Set-managed bounds and context/output relation | `set_managed_every_server_rule_rejects_locally_without_requests`: all lower/upper bounds and output >= context. |
| DONE | Set-managed tariff URL and age validation | Same zero-request test: non-HTTPS/non-Telnyx, URL credentials/query/fragment, future and >30-day timestamps. |
| DONE | Set-managed decimal price validation | Same zero-request test: zero input/output, negatives, >12 fractional digits, exponent notation and >1m credits/unit. Cache-read zero is accepted. |
| DONE | Probe evidence validation | `probe_rejects_missing_cached_tokens_model_tools_totals_and_modalities`, `set_managed_dry_run_has_no_put_and_probe_failure_has_no_get`: explicit cached count, exact model/provider id, integer counts and totals, one text assistant choice without tool_calls, unsupported modalities rejected. |
| DONE | Resync: one POST, row rendering, exit 2 despite changed=true when unsynced | `resync_is_once_and_boolean_does_not_mask_unsynced_rows`. |
| DONE | Readiness: named checks and ready 0 while access disabled | `managed_readiness_accepts_disabled_access_and_exact_equivalent_rates`; binary `managed_verify_ready_exits_zero_while_access_is_disabled`. Readiness prints its evidence limits. |
| DONE | Readiness: every specified failure | Separate `verify_unsynced_component`, `verify_cache_rate_mismatch`, `verify_stale_cache`, `verify_retired_cache`, `verify_stale_tariff`, `verify_public_audience`, `verify_missing_credential` unit tests and corresponding binary exit-2 tests. Additional tests cover disabled billing/Lago, inactive or wrong-category service, user credential requirement, missing contract/prices and wrong effective charging reason. |
| DONE | Readiness against deployed api-telnyx | Production GET-only run returned exit 2; failed checks included `managed_contract_present`, `configured_prices_present`, `three_required_prices`, `tariff_verification_fresh`, `dedicated_internal_offering` and `effective_platform_billing_charged`. |
| DONE | Ledger verify: bounded flags, --all cursors, broken/invalid anchor exit 2 | `ledger_all_follows_cursors_and_detects_broken_or_invalid_anchor`; binary `execution_wait_polls_until_settled_and_ledger_broken_exits_two`. Non-advancing cursors are rejected. |
| DONE | Execution list: metadata and client-side --since | `execution_list_and_since_filter_use_the_self_scoped_route`; binary `execution_since_filters_stdout_and_wait_timeout_has_exit_two`; production empty list GET succeeded. |
| DONE | Execution show: UUID validation; settled/released 0; pending/nonterminal timeout 2 | `execution_show_wait_settled_released_and_pending_timeout`, `executions_reject_invalid_uuid_since_and_agent_key_before_requests`; binary timeout and polling tests. |
| DONE | Admin usage: local window exclusivity, 31-day bound and pagination; slug/UUID/user query | `usage_periods_and_invalid_windows_are_validated_locally`, `usage_sends_bounded_window_service_uuid_user_and_pagination`; Clap enforces mutually exclusive window flags. |
| DONE | Admin usage: service/class token quantities and four exact funding costs | `usage_prints_token_classes_and_all_exact_costs_without_float_conversion`; binary `usage_table_renders_flat_service_and_credential_class_exact_costs`. Rendering follows backend serde flattening; distinct row costs prevent totals from masking missing service/class output. |
| DONE | Proxy: unchanged stdout bytes, execution stderr, JSON envelope | `proxy_unchanged_stdout_is_byte_identical_and_execution_goes_to_stderr`, `proxy_envelope_allowlists_headers_and_preserves_provider_json`. Envelope exposes only explicitly allowed NyxID headers. |
| DONE | Proxy: 409/12800 pending 2 with parsed id; JSON HTTP errors | `proxy_pending_exits_two_and_http_errors_keep_status_path_and_body`; corrected `error_format::parse_request_path`. |
| DONE | MCP tools/call: handshake, session/protocol headers, JSON/SSE | `mcp_tools_and_call_handshake_json_and_sse_keep_schemas_and_session_headers`. Arguments accept JSON objects and @file; invalid inputs fail locally. |
| DONE | MCP: at most one reinitialize on 404/-32002; no replay for HTTP/RPC/tool errors | `mcp_reinitializes_once_for_http_404_and_rpc_session_expiry`, `mcp_repeated_expiry_and_http_rpc_errors_stop_without_call_replay`, `mcp_is_error_and_pending_have_distinct_exit_codes_and_no_replay`. Stateless initialization is tracked independently of a session header. |
| DONE | MCP: service discovery and call work in fresh lazy human sessions | `mcp_fresh_human_session_discovers_lazy_service_tools_without_execution`, `mcp_args_file_is_forwarded_through_universal_dispatcher`. Tools/list is supplemented by read-only `nyx__search_tools` for --service; service calls use `nyx__call_tool`. |
| DONE | MCP: isError 1 or managed pending 2 with id | Unit error/pending test plus binary `mcp_json_and_sse_errors_have_binary_exit_one_or_two`. No successful MCP execution id is fabricated. |
| DONE | Grants: issue body, reason, confirmation/--yes, batch/activation output, email lookup | `grant_issue_resolves_email_scope_and_posts_once_with_activation_states`, `grant_issue_without_yes_refuses_nonterminal_and_never_posts`; binary `grant_issue_json_prints_batch_id_and_each_activation_state`. |
| DONE | Grants: list user filter and revoke UUID/DELETE; failure no replay | `grant_list_user_and_revoke_routes`, `grant_errors_are_not_retried_and_invalid_revoke_is_local`. |
| DONE | Wallet: owner_id on GET as well as provisioning | `wallet_get_sends_owner_id_query`; binary `wallet_owner_id_is_forwarded_on_get_in_the_binary`. |
| DONE | Telnyx template: one restricted internal offering, access off, one pinned operation; no billing/key | `telnyx_template_pins_one_disabled_restricted_operation_with_typed_variables`; typed model `org/model:1` and integer output maximum are asserted. |
| DONE | Typed variables preserve default string restrictions | `typed_variables_preserve_default_charset_and_reject_unknown_types`; bad integer bounds/decimal/exponent inputs are rejected by the template test. |
| DONE | Catalog billing diff hypothesis tested before normalization | `catalog_billing_sync_and_bson_dates_do_not_create_drift` failed against the original comparison, demonstrating the perpetual diff; passed after normalization. The initial failure is retained in the external check log. |
| DONE | Applying the same catalog manifest twice is unchanged | Binary `catalog_apply_same_manifest_twice_is_unchanged_with_server_sync_and_bson_dates`. Normalization strips synchronization fields and reconciles extended-JSON date forms. Additional tests preserve explicit clears and exact authored strings. |
| DONE | Catalog warns before disabling a live platform key or clearing allowlist | `catalog_apply_warns_before_disabling_or_clearing_live_platform_access`; warning is emitted before applying the service update and during its dry run. |

## Documentation and release metadata

| Status | Item | Evidence |
|---|---|---|
| DONE | Skill reference map and admin reference | `skills/nyxid/SKILL.md`, `skills/nyxid/references/admin.md`. |
| DONE | Dedicated billing reference and installed reference list | `skills/nyxid/references/billing.md`, `cli/src/commands/ai_setup.rs::REFERENCE_FILES`; existing installation/reference tests pass. |
| DONE | Plugin version and generated metadata | `.claude-plugin/plugin.json` 0.8.8 -> 0.8.9; `scripts/sync-plugins.py` regeneration and --check. |
| DONE | Native operator walkthrough | `docs/TELNYX_PLATFORM_BILLING.md` replaces Python assembly with template/dry-run/apply, set-managed, credentials, readiness/rates/resync, pilot provisioning/grant, restricted activation, HTTP/MCP, execution polling, usage and ledger verification. Paid/mutation steps are described, not executed. |
| DONE | Other billing/admin command documentation | Billing configuration guide, Lago spec CLI inventory, ENV operational reference and CLAUDE command inventory updated. |
| DONE | No CLI/backend/frontend release-version bump | Cargo/package manifests and lockfiles unchanged. No backend source changes. |
| NOT DONE (outside authorized scope) | Delete other-worktree Python helpers/templates | Explicit user instruction prohibits touching the other worktree; references remain there, no helper was copied here. |
| NOT DONE (outside authorized scope) | Phase 6 paid/live acceptance and backend follow-ups | No production mutation, paid request or production MCP handshake/call was made. Vendor invoice parity, live settlement and replica inventory remain Phase 6/deployment evidence. |

## Deliberate differences and verification limits

The fresh human MCP session contract requires lazy discovery and universal
dispatch; filtering tools/list alone and calling a service tool directly would
fail against the current backend. The implementation uses the existing meta-tool
contracts and performs no speculative execution to activate a service.

Set-managed emits `operations: []` to clear existing incompatible operation
prices. File updates explicitly merge current authored billing because the
backend has different omission handling for legacy fields and lane extras.
Catalog normalization handles server fields/dates without suppressing explicit
clears or changing authored strings.

One existing proxy test's intentional expected error changed from a generic
message to the path-bearing HTTP error. Existing request constructors gained
`envelope: false`. No existing test was deleted or disabled.

There are no known outstanding CLI implementation defects from this round's
audit. Automated tests validate local behavior and mocked response contracts;
successful Lago synchronization and paid HTTP/MCP settlement were not exercised.
Readiness states those limits, and the native walkthrough preserves the attended
live verification steps for the separately authorized Phase 6 run.
