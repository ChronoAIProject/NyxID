# NyxBot adapter implementation evidence

One implementation session, no delegation. Branch: `telnyx-nyxbot-integration`.
The proposal is preserved; the parent owns the separate review ledger and the
operator guide `docs/NYXBOT_SERVICE_ADAPTERS.md`.

## Session and authorized scope

The parent independently verified the effective turn context as `gpt-6.1-sol`,
`xhigh`, approval `never`, sandbox `danger-full-access`. No configuration,
environment or session contents are copied here. Validation uses local controlled
provider/NyxAgent/FAQ servers. Tests6 used the parent-managed isolated replica on
port 27116; final validation uses a separate task-owned MongoDB 8 replica on
port 27117 (see the infrastructure correction below).
No live provider effect, deployment, commit or push has been performed.

## Pre-rebase implementation validation checkpoint

The implementation source at this checkpoint matched the passing tests13
snapshot. The later PR rebase onto main is recorded in the primary review ledger
and requires fresh CI validation. All commands in this checkpoint exited 0:

| Check | Result | Evidence |
| --- | --- | --- |
| Selected backend acceptance and affected regressions | 881 passed, 0 failed, 0 ignored; compile 1m57s, execution 672.92s | `/tmp/nyxbot-adapter-tests13-final.log` |
| Focused signed carrier transport and canonical authorization | 20 passed; compile 8m44s, execution 5.02s | `/tmp/nyxbot-adapter-tests13-carrier.log` |
| Full workspace Clippy, all targets, `-D warnings` | Passed, 2m50s | `/tmp/nyxbot-adapter-clippy13.log` |
| `cargo fmt --all -- --check` | Passed | `/tmp/nyxbot-adapter-fmt-final13.log` |
| Backend RCI boundary script | Passed | `/tmp/nyxbot-adapter-boundary-final13.log` |
| `git diff --check` | Passed | `/tmp/nyxbot-adapter-diff-final13.log` |
| Worker-owned admin adapter UI and feature-flag tests | 10 passed | `/tmp/nyxbot-adapter-frontend-final13.log` |
| Full registry frontend schema tests, Node environment | 3 passed | `/tmp/nyxbot-adapter-catalog-schema13.log` |
| Prettier, six worker-owned frontend files including fixture | Passed | `/tmp/nyxbot-adapter-prettier-final13.log` |

The registry fixture contains all 15 compiled profiles and is checked against
the executable without regeneration in the final run. The primary completed the
final frontend harness correction: static JSON import, per-test cloning and
strict first-entry narrowing. All 26 owner/settings/admin/catalog tests pass in
the normal happy-dom environment, and the final TypeScript, catalog lint and
format checks pass (`/tmp/nyxbot-adapters-review-final-ui15.log` and the related
`review-tsc15`, `review-catalog-lint15`, `review-catalog-format15` logs). The
previous production frontend build remains valid; no product implementation
changed afterward. See the closed primary review ledger for the final evidence.

## Implementation checklist

The implementation, controlled acceptance and targeted validation items below
passed on the final frozen source. The primary completed the default-environment
test harness follow-up and personal review described above.

- [x] 1. Version 1 typed descriptors from the executable channel registry;
  legacy channel-platform projection preserved; per-operation binding readiness.
- [x] 2. Telnyx final-transcript Conversation Relay and SMS, signature/resource
  proof, durable session/generation/claim fences, normal specialist turns.
- [x] 3. Twilio signed form and WebSocket admission, active-call cap setup,
  ConversationRelay and SMS through the same coordinator.
- [x] 4. ElevenLabs authorized Custom LLM HTTP SSE with separately issued guest
  sessions, complete-history correlation and refusal of ambiguous retry/reset.
- [x] 5. Owner setup/readiness, admin account trust and allocation, safe NyxBot
  discovery tool, delivery evidence, feature-gated rollout and documentation.
- [x] Controlled transport/SSE/FAQ acceptance, authority, interruption, queue
  boundaries, duplicate ingress, lifecycle races and funded usage settlement.
- [x] Final formatting, Rust lint, backend acceptance and targeted frontend checks.

## Implemented boundaries

`service-adapters` remains the HTTP transformation crate. `channel_platform` and
`channel_adapters` retain the authoritative executable registry; the existing
messaging projection excludes the new typed call/SMS/Custom LLM variants.

Public callers use a personal owner-configured specialist. FAQ tools require its
ordinary service grant and `guest_access=read`. Main NyxBot guest turns still deny
all tools. Browser assistant voice/private owner threads remain human-only.
Provider system messages and caller ID grant no authority. Each turn rebuilds a
guest recap and refreshes trusted instructions; actual tool callbacks revalidate
the adapter binding/session generation and the ordinary tool authority.

Bindings reference existing connections or explicit catalog platform credentials.
Account trust, exclusive number/application allocation and cross-owner resource
claims are separate from platform-key ACL. Ingress verifies provider signatures
and proves actual resource membership; credential-bearing HTTP rejects redirects
and authorizes the exact canonical catalog operation before dispatch.

Session preparation uses the repository retryable transaction helper, write-fences
binding revision and inserts session atomically. Claims, turn receipts, command
fences and generations prevent automatic replay. Interrupted-prefix evidence is
correlated against bounded prior audible replies. Acceptance is never recorded
as proof of hearing or delivery. Adapter state is metadata only; conversation
text follows normal NyxBot retention.

Owner deletion durably closes adapter admission on the existing person record
before checking pending cleanup. Binding persistence/reverification, session
preparation/activation, turn admission and SMS effect-record insertion write the
same owner row in their transactions. The closed gate leaves authentication
available while cleanup is pending. SMS effect records also write-fence the live
binding/session; a closed session cannot acquire a late orphan send identity.
Noncall activation checks prepared state and generation and cannot reopen a
terminal session. Disable invalidates all session generations/leases before its
bounded immediate cleanup; fair recovery handles the remainder.

Call/SMS cleanup receipt insertion and SMS provider-message correlation also
write-fence the retained binding transactionally. Disabled bindings can retain
admitted terminal evidence, while removal or owner purge prevents an already
authenticated callback from recreating deleted event/phone data.

Management audit events use the existing audit append service and include actor,
record ID and revision where applicable. They contain no provider bodies, keys,
socket tickets, Custom LLM setup values or session tokens.

## Provider and billing limits

Platform Telnyx voice is intentionally unavailable with
`provider_limit_unavailable`: documented inbound Answer/relay operations cannot
prove a carrier-enforced duration bound. BYOK Telnyx voice and platform SMS are
separate supported configurations. ElevenLabs is the Custom LLM profile, not the
separately authenticated Speech Engine WebSocket composition.

Twilio ringing admission uses Pause plus a bounded no-retry Redirect to reach
active-call admission, with provider fallback URLs prohibited. It confirms the
active-call TimeLimit before returning relay TwiML. Established webhook retries
return identical authenticated TwiML and do not repeat the cap or hang up the
call. Ambiguous cap outcomes issue no relay ticket and are reconciled without
command replay. These behaviors require controlled and live-provider contract
validation; local fixtures do not prove live account eligibility.

Carrier lanes use existing funding/meters/ledger: Requests per SMS part,
VoiceSeconds per call. Components, operation-priced lanes and resale are rejected
with billing readiness failure instead of falling back to a base price. Platform
allocations require synchronized pricing and current complete tariff attestation.
The existing Telnyx text-inference pilot does not cover these costs.

Terminal quantity is frozen once; settlement resumes admitted meters after price
or credential changes. BYOK Telnyx owner-supplied verification keys cannot attest
billable quantity: authenticated provider reads establish terminal usage/parts.
SMS conservatively reserves bounded Unicode multipart costs and releases unused
parts; overflow produces an explicit bounded response rather than truncation.
Correlated signed callbacks can recover an accepted send after its POST response
is lost. Uncertain effects are never automatically replayed. If both acceptance
and all correlated callbacks are lost, evidence can remain pending; destructive
removal is blocked rather than inventing a delivery outcome or charge.

`nyxbot:service-adapters` is default off. Every API replica must be upgraded before
enabling it: old replicas ignore additive adapter-origin callback fences.

## Validation log

- `/tmp/nyxbot-adapter-check4.log`: earlier `cargo check -p nyxid --locked` passed;
  this predates the final authority and acceptance-test changes.
- `/tmp/nyxbot-adapter-tests3.log`: obsolete snapshot failed on an undefined
  fixture variable; corrected. No acceptance test result from this attempt.
- `/tmp/nyxbot-adapter-tests4.log`: failed to compile on a moved socket-ticket
  digest; corrected by borrowing it. No tests ran.
- `/tmp/nyxbot-adapter-tests5.log`: compilation failed on E0515 from a wallet
  collection temporary in a test closure. The collection is now bound outside
  the closure. No tests ran; the obsolete codegen was stopped using only this
  task's processes.
- `/tmp/nyxbot-adapter-check-tests6.log`: `cargo check -p nyxid --tests --locked`
  was cancelled (exit 143) after roughly 18 minutes without diagnostics. Its
  snapshot predated the owner deletion/activation/receipt and final SMS dispatch
  refinements. Only this task's Cargo/rustc processes were stopped. No passed
  type-check or acceptance result is claimed.
- `/tmp/nyxbot-adapter-tests6.log`: the earlier frozen snapshot completed:
  `cargo test -p nyxid --locked -- services::call_channels::tests services::admin_user_service::tests --test-threads=1`.
  It uses the isolated replica set, shared Cargo target and
  `NYXID_UPDATE_CONTRACT_FIXTURES=1` to generate the full executable catalog.
  Compilation took 179m53s. Test execution finished in 146.17s: 43 passed,
  15 failed, none ignored, exit 101. All 24 selected admin-user lifecycle tests
  passed. The compiler reported one `unused_mut`, removed in the later batch.
  This run completed without cancellation and is failed acceptance evidence.
- Tests6 failure investigation found two fixture/infrastructure prerequisites
  and one runtime wiring defect. Fixtures now initialize the existing process
  audit HMAC key before issuing relay tickets, Custom LLM setup/session values
  or platform credential fingerprints. Port 27116 reports MongoDB 7.0.37 and
  rejects the existing ledger's client `bulkWrite` with code 115. The separate
  task-owned MongoDB 8.0.34 replica (`nyxbotAdaptersWorker8`, port 27117,
  local-only, 256MiB WiredTiger cache) supports that production requirement;
  the parent-managed replica is untouched. Both funded settlement tests pass
  against MongoDB 8 using the old binary, recorded in
  `/tmp/nyxbot-adapter-tests6-mongo8-diagnostic.log` (2 passed, 6.94s).
  An old-binary run after the existing startup-key initializer also confirms
  Custom LLM setup succeeds; these are diagnostics, not final source acceptance.
  Coordinator execution now supplies the same `Metered(Proxy)` route policy
  used by existing platform turns; passing `None` correctly caused the ordinary
  egress to reject all NyxAgent dispatch. Turn assertions now retain settlement
  code/stage and upstream/callback counts. The duplicate-upgrade fixture yields
  while polling closed sockets so its timeout remains enforceable.
- While tests6 continued unchanged, the parent authorized a narrow source batch:
  zeroizing minted session/relay bearers, updating the native account-tool
  inventory to 23 with audited adapter discovery, removing the reported unused
  `mut`, and the bounded Twilio upgrade-signature scheme compatibility helper.
  New fixtures cover exact upgrade URL signatures and altered URL rejection,
  an actual HTTPS-signature/WSS upgrade behind simulated TLS termination,
  Telnyx/Twilio 302 responses with no follow-up at a second server, and captured
  diagnostics through the shared production HTTP span factory. These changes
  are present in working source but absent from the recorded tests6 snapshot;
  none has runtime verification yet. Direct `rustfmt` formatted the changed
  Rust files without starting another Cargo process.
- The same uncompiled source batch now rechecks complete channel turn, raw
  NyxAgent key and session authority after `mark_forwarded`, immediately before
  HTTP transport. Refusal at this proven pre-transport boundary settles zero
  usage through the admitted meter. The per-instance test gate exercises Stop,
  replacement of the live turn and key revocation, asserts the meter was already
  forwarded, then requires zero upstream requests/callbacks and finalized zero
  quantity. Readiness fixtures now require `resource_policy_unresolved` on both
  ReceiveCall and EndSession for Telnyx and Twilio configured call policies.
  These additions are also absent from tests6 and await the final source run.
- `/tmp/nyxbot-adapter-tests7.log`: the next snapshot completed with
  `cargo test -p nyxid --locked --config 'profile.test.package.nyxid.debug=0' -- services::call_channels:: services::admin_user_service::tests services::assistant_authority_tests mw::request_trace::tests --test-threads=1`.
  Environment selects the isolated MongoDB 8 replica on port 27117, shared
  Cargo target and `NYXID_UPDATE_CONTRACT_FIXTURES=1`. It includes the full adapter
  protocol module, actual production trace capture and audited native discovery
  alongside transport/SSE/funded/lifecycle tests. Compilation passed without
  compiler warnings in 40m18s; execution finished in 166.47s with 74 passed,
  11 failed, none ignored, exit 101. Trace capture, both funded settlement
  regressions, redirect refusal, audited discovery, lifecycle/receipt fencing,
  credential revocation and resource-policy readiness passed. Most remaining
  transport/FAQ failures returned the fixture's `Callback denied.` response;
  the missing proxy-route classification below is corrected in working source.
  Its source hashes are recorded in `/tmp/nyxbot-adapter-tests7-source.json`.
  A subsequent fixture-only correction mounts the actual FAQ proxy callback
  with the same `Metered(Proxy)` extension used by production routes. This is
  the second hop of the missing billing-classification failure; it preserves
  all normal proxy checks. Tests7 used its earlier snapshot; tests8 includes this
  correction. The Custom LLM
  failure fixture also now requires one actual upstream turn, one successful
  authenticated FAQ callback and one FAQ read before accepting its error-frame
  assertions; an earlier connection/configuration failure cannot satisfy that
  provider-failure regression.
- Tests7 additionally exposed a missing `Metered(Proxy)` classification in the
  direct R21 fixture, which prevented reaching its post-forwarded gate. That
  fixture now supplies the required classification. Platform webhook-key lookup
  now requires both the verified trusted-account revision and the stored public
  key to match the current trusted account; revision rotation invalidates prior
  callback verification while retained provider API recovery remains separate.
- `/tmp/nyxbot-adapter-tests8.log`: the corrected snapshot completed the same
  acceptance/admin/authority/trace filter and package-only debug override
  against MongoDB 8, with catalog generation enabled. Compilation passed without
  warnings in 19m25s; execution finished in 147.10s with 75 passed, 10 failed,
  none ignored, exit 101. The post-forwarded dispatch boundary and trusted-account
  rotation regressions now pass. Strengthened callback assertions identify HTTP
  404 in the remaining FAQ-dependent failures. The fixture incorrectly addressed
  the catalog-ID proxy route with a UserService ID; working source now uses the
  production slug route plus its exact `_nyxid_via` connection selector. Normal
  proxy authority and billing checks remain in place. This fixture correction
  requires the next current-source execution. Source hashes for tests8 are
  recorded in `/tmp/nyxbot-adapter-tests8-source.json`.
- The current test compile uses the package-only command override
  `--config 'profile.test.package.nyxid.debug=0'`. This changes test debug-symbol
  generation for NyxID only; repository profiles and dependency settings remain
  unchanged. The override was not applied to tests6.
- Parent frontend checks: owner/settings/admin suites 23 passed; existing admin
  platform credential suite 15 passed; transcript/schema/flag regression group
  197 passed; production build and TypeScript checks passed at their snapshots.
- Worker admin component plus flag checks: 10 passed,
  `/tmp/nyxbot-adapter-admin-tests2.log`.
- Worker frontend `/tmp/nyxbot-adapter-ui-final.log`: 23 tests passed across
  owner bindings, NyxBot settings and worker admin resources suites.
- Worker ESLint `/tmp/nyxbot-adapter-ui-lint-final.log`: passed for the five new
  worker implementation/test files.
- Worker TypeScript `/tmp/nyxbot-adapter-tsc8.log`: passed after the session-end
  action and formatting.
- Worker `/tmp/nyxbot-adapter-catalog-schema7.log`: the newly generated catalog
  test ran with 1 passed, 2 failed. Both catalog cases fail before parsing:
  Vite/happy-dom transforms `import.meta.url` to HTTP, which the parent-owned
  test's `readFileSync(new URL(...))` cannot load. The parent has been notified;
  its test file is unchanged by this worker. A direct Node invocation of the
  actual TypeScript Zod schema against the same JSON fixture succeeds for all
  15 compiled descriptors, OpenClaw's null documentation/unsupported outbound
  transport and additive operation/capability values:
  `/tmp/nyxbot-adapter-catalog-node7.log`. This separate contract assertion does
  not turn the failed Vitest invocation into a passing test run.
- Worker `/tmp/nyxbot-adapter-catalog-schema-node7.log`: the actual parent-owned
  schema suite passes all 3 tests when invoked with
  `npx vitest run src/schemas/channel-adapters.test.ts --environment node`
  (733ms, exit 0). No test edits were made. The default happy-dom loader still
  needs a parent-owned per-file environment annotation or static JSON import
  for the normal mixed frontend suite.
- Parent backend RCI boundary check passed at its latest reviewed snapshot.
- Worker `/tmp/nyxbot-adapter-fmt-final.log`: `cargo fmt --all -- --check`
  passed after the receipt transaction changes. `git diff --check` also passed.
- Worker `/tmp/nyxbot-adapter-fmt8.log`: `cargo fmt --all -- --check` passed
  after the startup/classification and stronger SSE fixture corrections.
- Worker `/tmp/nyxbot-adapter-boundary7.log`: the backend RCI boundary script
  passed after the coordinator billing-classification correction.
- Worker `/tmp/nyxbot-adapter-clippy9.log`:
  `CARGO_BUILD_JOBS=1 cargo clippy --workspace --all-targets --locked -- -D warnings`
  failed with 12 lint errors in changed backend code (nested conditions,
  duplicate disabled branches and one nonminimal boolean expression). No lint
  suppression was added. The conditions are simplified in working source.
- Worker `/tmp/nyxbot-adapter-clippy10.log`: the same full workspace command
  advanced to the backend test target, then failed with nine test-target lint
  errors: five field shorthands, two helpers after test modules and two nested
  iterator/condition expressions. These are corrected without suppressions;
  `/tmp/nyxbot-adapter-clippy11.log` records the corrected full workspace run:
  all targets passed with `-D warnings`, exit 0, in 7m43s. This includes the
  backend test target and every workspace crate, not a reduced package scope.
- Worker `/tmp/nyxbot-adapter-fmt9.log`: `cargo fmt --all -- --check` passed
  after the tests8 fixture correction and the 12 lint fixes. `git diff --check`
  also passed at that snapshot.
- Worker `/tmp/nyxbot-adapter-fmt11.log`: final `cargo fmt --all -- --check`
  passed after all reported lint fixes. The backend RCI boundary script passed
  in `/tmp/nyxbot-adapter-boundary11.log`, and `git diff --check` passed.
- Worker `/tmp/nyxbot-adapter-tests9-core.log`: final-source focused FAQ
  preflight passed (1 test covering both carriers, 4.36s), after compilation
  passed without warnings in 4m11s using the package-only test debug override.
  It requires successful authenticated callbacks, fresh FAQ reads and trusted
  instruction refresh between turns. Source hashes are recorded in
  `/tmp/nyxbot-adapter-tests9-source.json`.
- Worker `/tmp/nyxbot-adapter-tests9-final.log`: the same frozen source compiled
  without warnings in 1m17s, then started the combined 85-test acceptance/admin/authority/trace filter and
  775 existing NyxAgent/callback/recap/proxy/registry/auth/meter regressions.
  The exact command is `/tmp/nyxbot-adapter-final-command.txt`; it uses the
  shared target, one build job, MongoDB 8 on port 27117 and the package-only
  debug override. Catalog generation is unset so the full-registry contract
  compares the checked-in draft fixture to the executable. After 39 passing
  tests the process aborted (SIGABRT, exit 101 from Cargo) in the existing
  `nyxagent_admission_retries_charge_only_the_accepted_execution` test with a
  stack overflow. No complete acceptance result is claimed.
- Stack isolation reproduced the abort at the default stack:
  `/tmp/nyxbot-adapter-stack-default10.log` (exit 134). The same binary's exact
  regression passes with `RUST_MIN_STACK=8388608` (1 passed, 8.22s):
  `/tmp/nyxbot-adapter-stack-large10.log`. This override was diagnostic only.
  Working source boxes the large setup/turn/billing fixture futures, preserving
  all assertions. `/tmp/nyxbot-adapter-tests10-stack.log` records the rebuilt
  exact regression with `env -u RUST_MIN_STACK`: compilation passed in 4m27s,
  but execution still aborted with the same overflow. Source hashes are in
  `/tmp/nyxbot-adapter-tests10-source.json`; final validation uses the default
  stack, not the diagnostic larger stack.
- The debugger launch was bounded to 45 seconds and timed out before launch
  (`/tmp/nyxbot-adapter-stack-lldb11.log`); only its task-owned process group and
  orphan debugserver were stopped. A narrowly projected crash-report backtrace
  identifies the existing HTTP admission-refusal path:
  `relay_response -> BillingService::settle -> meter::settle -> settle_usage_funding`.
  Only function names are retained in `/tmp/nyxbot-adapter-stack-frames11.txt`,
  not the full diagnostic report. The earlier tests6 executable also overflows
  at the default stack (`/tmp/nyxbot-adapter-stack-earlier11.log`). The ineffective
  test-only boxing changes are removed; the response and funded-settlement
  futures are boxed at their actual HTTP boundaries without changing charge or
  retry semantics. `/tmp/nyxbot-adapter-tests11-stack.log` records the exact
  regression plus both shared HTTP response tests at the default stack:
  compilation passed in 4m19s, but the first test still aborted with a stack
  overflow before the response-core cases ran. No passing result is claimed.
  The final filter now contains 862 tests, adding these two response-core cases;
  its command is `/tmp/nyxbot-adapter-final11-command.txt` and source hashes are in
  `/tmp/nyxbot-adapter-tests11-source.json`.
- The next crash projection confirms funded release still runs on the deep
  proxy poll stack. Working source now polls that release in one awaited Tokio
  task, boxing its funding future. A provider retry is permitted only after the
  same durable zero-usage settlement completes; settlement errors and task
  failures return the existing internal refusal. The ineffective outer-response
  boxing is removed. `/tmp/nyxbot-adapter-tests12-stack.log` records the default
  stack preflight: compilation passed without warnings in 4m19s; all three tests
  passed in 11.79s, including the exact prior overflow regression. Source hashes and the full 862-test command are
  `/tmp/nyxbot-adapter-tests12-source.json` and
  `/tmp/nyxbot-adapter-final12-command.txt`.
- `/tmp/nyxbot-adapter-tests12-final.log`: the combined 862-test filter compiled
  without warnings in 1m14s and finished with 861 passed, 1 failed in 679.65s.
  It used `env -u RUST_MIN_STACK`, the package-only test debug override, one
  build job and the MongoDB 8 replica. The signed carrier transport fixture
  reproduced the remaining failure in `/tmp/nyxbot-adapter-carrier-fixture12.log`:
  Telnyx's padded call ID was safely encoded into the provider URL, but carrier
  policy authorization incorrectly used the literal-path constructor, which
  refuses percent escapes. The correction uses the existing server-built-path
  canonicalizer, preserving its single-decoding and ambiguity checks. The real
  transport fixture now asserts both exact encoded Answer/relay command paths.
  `/tmp/nyxbot-adapter-tests13-carrier.log` records the focused rebuilt fixture
  and shared canonical authorization tests: compilation passed without warnings
  in 8m44s; all 20 tests passed in 5.02s. Source hashes are recorded in
  `/tmp/nyxbot-adapter-tests13-source.json`; only binding authorization and its
  transport test changed after tests12.
- `/tmp/nyxbot-adapter-tests13-final.log`: the final combined filter adds all 19
  shared canonical authorization tests to the prior 862-test filter. It is
  complete on the same frozen source: compilation passed without warnings in
  1m57s; 881 passed, 0 failed, 0 ignored in 672.92s. This includes actual signed
  HTTP ingress and WebSocket setup/frames for both carriers, Custom LLM HTTP SSE,
  real authenticated FAQ proxy callbacks, funded meter/ledger settlement,
  interruption/disconnect, duplicate and deletion races, plus the existing admin
  lifecycle, NyxAgent callback/recap, proxy, registry and auth regression filters.
  It is a selected backend filter, not a claim that all 9,053 backend tests or
  live-provider validation ran. The 53-file source manifest's canonical JSON
  SHA-256 is `5c3f558372f84303d00a0940aa2676b278536f1d9a23ad82f0f4ec0c54c0b267`.
- `/tmp/nyxbot-adapter-clippy13.log`: final-source full workspace, all targets,
  `--locked -- -D warnings`, one build job; passed in 2m50s with exit 0. No
  final-source lint fixes or warning suppressions were needed.
- `/tmp/nyxbot-adapter-fmt-final13.log`,
  `/tmp/nyxbot-adapter-boundary-final13.log`, and
  `/tmp/nyxbot-adapter-diff-final13.log`: final formatting, backend RCI boundary,
  and whitespace checks all passed with exit 0.
- `/tmp/nyxbot-adapter-prettier-final13.log`: all six worker-owned new frontend
  files, including the full registry fixture, pass repository Prettier.
- `/tmp/nyxbot-adapter-frontend-final13.log`: worker-owned admin adapter UI and
  feature-flag regressions pass 10/10 in 3.12s on the current frontend source.
  `/tmp/nyxbot-adapter-catalog-schema13.log`: the complete generated catalog
  schema suite passes 3/3 in Node mode (258ms). The parent-owned test's file-URL
  loader failed under the default happy-dom environment at that worker checkpoint;
  the primary subsequently fixed it and passed all 26 focused tests in that
  environment. Its content/schema assertions also passed in Node mode.
- Worker `/tmp/nyxbot-adapter-prettier-final11.log`: all six new worker-owned
  frontend implementation/test/fixture files pass repository Prettier after
  formatting the generated JSON. `/tmp/nyxbot-adapter-catalog-schema11.log`:
  the full catalog schema suite passes 3/3 in Node mode (402ms); parent-owned
  test files remain unchanged.

New frontend implementation files have been formatted with repository Prettier.
The full registry fixture was generated by the passing tests6 executable
registry test at `frontend/src/schemas/fixtures/channel-adapter-catalog.json` and
is available for the parent's frontend contract check. Tests7 and tests8
regenerated and checked it against their compiled registries. The final 881-test
backend acceptance/regression filter and final-source full workspace Rust lint
pass. No resource-limited or pending run is counted as passed.
All provider transport tests use controlled local servers, not live provider
accounts.

## Acceptance fixture map

Every mapped case below passed in the final 881-test filter, including the
separate authority and production trace-capture regressions.

| Requirement | Controlled fixture |
| --- | --- |
| Truthful executable/legacy catalog | `full_registered_catalog_contract_and_legacy_truth` (also generates the frontend JSON fixture) |
| Fresh FAQ and trusted instruction updates | `coordinator_faq_is_fresh_and_specialist_instructions_refresh_for_both_carriers` |
| Signed ingress, actual relay setup/frames, FIFO, guest isolation and closure | `signed_carrier_ingress_websocket_finals_and_fifo_reach_real_faq_tools` |
| HTTPS-form Twilio upgrade signature | `actual_twilio_upgrade_accepts_https_signature_for_configured_wss_endpoint` |
| Real callback revocation | `disabling_after_nyxagent_acceptance_denies_actual_notion_callback` |
| Final NyxAgent dispatch authority after the last billing await | `nyxagent_dispatch_after_forwarded_mark_rechecks_turn_and_key_and_settles_zero` |
| Final carrier/SMS authority and fresh OAuth material | `final_call_dispatch_rechecks_revocation_and_exact_catalog_policy_after_proof`, `sms_final_dispatch_uses_refreshed_material_without_replacing_credential_epoch` |
| Actual interruption/disconnect and duplicate socket claim | `websocket_burst_interrupt_cancels_stale_output_without_repeating_faq_effect`, `websocket_disconnect_stops_accepted_turn_before_tool_callback`, `concurrent_duplicate_websocket_loser_cannot_cleanup_winner` |
| Queue boundary and SSE failure/disconnect | `terminal_output_survives_exact_queue_capacity_and_slow_consumer`, `custom_llm_failed_turn_sends_error_without_success_or_done`, `custom_llm_disconnected_body_stops_slow_tool_and_full_queue_is_bounded` |
| Custom LLM real HTTP SSE and durable history correlation | `custom_llm_http_sse_correlates_isolated_calls_refreshes_faq_and_refuses_replay`, `custom_llm_evidence_distinguishes_repeated_text_and_refuses_history_reset` |
| Call cap bootstrap/retry/failure | Signed carrier ingress fixture and `twilio_rejected_or_uncertain_cap_never_releases_relay_and_never_replays_cap` |
| Cross-owner resources and platform trust rotation/forgery | `global_resource_claim_rejects_cross_owner_account_alias_takeover`, `trusted_platform_key_rotation_and_forged_ingress_never_control_another_resource` |
| Policy readiness and paginated provider proof | `telnyx_proof_paginates_and_read_only_policy_blocks_call_and_sms_effects`, `twilio_call_policy_readiness_requires_unresolved_live_call_prerequisites` |
| Provider redirect refusal and ticket-free diagnostics | `provider_redirect_never_forwards_credentials_or_effect_to_second_server`, `mw::request_trace::tests::actual_request_trace_contains_method_path_and_no_query_or_bearer` |
| Funded call/SMS, frozen quantity and post-revocation settlement | `admitted_funded_call_settles_frozen_usage_once_after_price_and_credential_revocation`, `configured_funded_sms_multipart_releases_remainder_after_revocation_and_duplicate_receipts` |
| Signed multipart/receipt-first/lost-response recovery | `funded_twilio_sms_signed_ingress_real_turn_unknown_parts_receipt_and_lifecycle`, `receipt_recovers_response_lost_send_without_replay_and_rejects_second_join` |
| Noncall expiry/activation, disable, preparation races and deletion/late receipts | `durable_terminal_quantity_is_frozen_and_noncall_sessions_finalize`, `concurrent_prepare_retries_distinct_calls_and_deduplicates_same_call`, `noncall_activation_never_resurrects_closed_sessions_and_disable_expires_excess`, and the owner-deletion/SMS-send/late-receipt fixtures |
| Safe audited NyxBot discovery | `assistant_authority_tests::native_inventory_is_closed_and_schemas_exclude_secret_inputs` and both orchestrator native-inventory invocation tests |

## Reproduce the final backend filter

Run from the repository root. The package-only debug override limits test-binary
codegen memory without changing repository profiles or rebuilding dependencies
with a global debug override. The default thread stack is retained. The local
MongoDB 8 replica is required for the existing ledger bulk-write fixtures; the
parent's MongoDB 7 replica cannot execute those fixtures.

```sh
env -u RUST_MIN_STACK \
  CARGO_TARGET_DIR=/Users/chronoai/Desktop/aelf-frontend-work/NyxID/target \
  CARGO_BUILD_JOBS=1 \
  NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27117/?replicaSet=nyxbotAdaptersWorker8&directConnection=true' \
  cargo test -p nyxid --locked \
  --config 'profile.test.package.nyxid.debug=0' -- \
  services::call_channels:: \
  services::admin_user_service::tests \
  services::assistant_authority_tests \
  mw::request_trace::tests \
  services::assistant_nyxagent::tests \
  services::assistant_continuation \
  services::assistant_acknowledgement_service \
  handlers::mcp_transport::chat_authority_tests \
  handlers::assistant_nyxagent::tests \
  services::proxy_authorization::tests \
  services::proxy_service::tests \
  services::channel_platform::tests \
  services::channel_adapters:: \
  mw::auth::tests \
  services::billing::meter::tests \
  services::proxy_http_response::tests \
  --test-threads=1
```

The full-workspace lint command uses the same shared target and one build job:

```sh
CARGO_TARGET_DIR=/Users/chronoai/Desktop/aelf-frontend-work/NyxID/target \
  CARGO_BUILD_JOBS=1 \
  cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
bash scripts/check-rci-backend-boundary.sh
git diff --check
```

After final review, the primary gracefully stopped both task-owned MongoDB
replicas and verified process absence. MongoDB 8 data and logs remain under
`/tmp/nyxbot-adapter-mongo8-worker/`; restart that replica on loopback port 27117
with replica name `nyxbotAdaptersWorker8` before reproducing the commands above.
The initial MongoDB 7 data/logs remain under
`/tmp/nyxbot-adapters-mongo-20261016/`. Unrelated MongoDB and compiler processes
were not stopped. Cleanup evidence:
`/tmp/nyxbot-adapters-review-mongo-cleanup15.json`.
