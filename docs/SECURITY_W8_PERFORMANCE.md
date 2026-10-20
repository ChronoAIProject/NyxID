# W8 audit performance investigation

This investigation compares test execution separately from compilation and
profiles the production audit path without changing durability, canonical bytes,
admission, retry budgets or test timeouts. W8-5 implementation is paused for this
CI regression. Measurements are development-build results, not production
capacity estimates.

## Hosted regression

The pre-W8-4 hosted Backend Test run (`8589cfd8`, PM log
`/tmp/pm-job-118380570787.log`) started tests at 00:37:53 and completed 9,615 tests
in 3,356.245 seconds: 9,614 passed, one failed, 11 skipped by the existing suite.
The W8-4 run (`19549c364`, PM log `/tmp/pm-bt.log`) started tests at 07:40:54 and
was cancelled at the existing 90-minute job deadline. Both builds took about
20 minutes before tests began. Compilation time does not explain the execution
regression.

Matching the two logs by test name gives 4,848 server tests that passed in both
runs. Their summed individual durations rose from 7,950.471 to 15,344.472 seconds
(1.93x). Tests exceeding 60 seconds rose from four to 39. These sums are not job
wall time: nextest runs several processes concurrently. The matched evidence is
in `/tmp/nyxid-w8-ci-perf/hosted-common.csv`.

The largest increases include OAuth sender-binding tests (0.559 to 89.750 s
and 0.660 to 89.306 s) and admin role-change tests (1.114 to 89.742 s and
1.277 to 88.937 s). Among the matched tests, the bucket taking at least 20 s
accounts for 87.1% of the increase in summed durations. This long tail has a
different scale from the measured sub-millisecond incremental append cost.

## Mechanism and implementation

W8-4 made every fresh database initialize the durable audit head and indexes.
Its initializer also called the snapshot verifier on an empty chain. The
verifier owned a read-only transaction but returned without explicitly ending
it. `ClientSession::drop` schedules asynchronous abort; it cannot guarantee that
abort completes before a short Tokio runtime shuts down. Initialization error
returns had the same implicit cleanup dependency. A snapshot can therefore
retain locks until MongoDB's transaction expiry/reaper. Further DDL or teardown
then waits for those locks.

A command-metadata-only wire trace found a second source: two of 40 short
handler processes left incomplete audit **write** transactions after runtime
shutdown, one after its outbox insert. The preexisting fire-and-forget helper
now starts a real multi-document transaction; canceling the test runtime can
also cancel its asynchronous session abort. Explicit verifier cleanup alone
does not close these abandoned writer sessions. The fixture teardown now
retains only each test client's observed session IDs
and ends exactly those sessions, from the existing independent cleanup runtime,
before dropping that client's test database. `test_utils.rs` and common fixtures
are CORE-owned under the accepted inventory; the focused change is covered by
the existing W8-4 approval and urgent regression instruction. It requires no
additional approval. Explicit verifier closure alone cannot cover a canceled
writer whose runtime has already stopped.

The `killSessions` call and database drop share the original three-second attempt
budget; the existing exit budget, retry count and backoff are unchanged. No
server-wide or user-wide session kill is used. Only session IDs are tracked,
with no command or event bodies retained by the fixture. The existing observer
is forwarded with the driver's callback/async/channel semantics. Cleanup occurs
after the client's lifecycle guard is released, or at process exit; a live
Database clone still defers lifecycle cleanup. Tests cover committed-data
preservation, another client's live transaction, and session drop after its
Tokio runtime has already shut down. Final live results are recorded below
when available.

On the isolated MongoDB 8.0.17 replica, observed read-only transactions spent
less than one millisecond doing work and 72–86 seconds inactive before abort.
Two `dropDatabase` operations were observed waiting for locks for 59 seconds.
A separate causal probe retained a read-only snapshot, observed DDL waiting for
its lock after 400 ms, then explicitly aborted: DDL completed about 32 ms after
release. No transaction-lifetime setting or timeout was changed.

The fix explicitly awaits snapshot abort after successful, broken and error
verification results. Initialization uses its existing snapshot for legacy
verification and explicitly aborts rejected initialization, while preserving
unknown-commit uncertainty. An empty chain is established by the same snapshot's
tail query and needs no nested chain walk. Every nonempty adopted chain still
gets the complete HMAC/link walk and the existing drained-replica gate.

The eight existing audit indexes are built in five commands, grouped by
collection, with the same names, key order, uniqueness and partial filters.
There is no process readiness/index cache: a later call still repairs a removed
index or recreated database. The local head and outbox remain unconditional in
unwitnessed deployments. The append path still validates the live tail/HMAC,
checks immutable replay, performs the head CAS, inserts row/outbox atomically,
and uses snapshot reads plus majority+journal commit.

## Local comparison method and instrumentation correction

Preserved pre-W8-4 and W8-4 binaries from the required shared Cargo target have
9,605 and 9,695 server tests respectively. All 9,605 original names survive in
the W8-4 binary. Select 150 names at evenly spaced positions in each sorted
common namespace (`handlers::`, `services::`), for 300 tests. Run each exact test
in its own process with four concurrent processes, matching nextest isolation.
Alternate the pre/W8 order in successive blocks of 30; retain all selected tests.
All 300 passed in each arm; zero failed or ignored.

| Cached binary | Summed process time | Summed libtest body time | Median process time |
| --- | ---: | ---: | ---: |
| Pre-W8-4 | 1,614.376 s | 1,547.370 s | 0.359 s |
| W8-4 | 1,731.088 s | 1,644.220 s | 0.658 s |

A subsequent Mach-O section check found an instrumentation mismatch: the cached
pre-W8 binary has `__llvm_prf_*` coverage sections; W8 and the fixed binaries do
not. The cached pre binary also lacks both KMS feature flags. The pass/fail counts
remain valid, but **the 7.2% sum and 83% median changes are descriptive only and
must not be used as a clean estimate of W8 overhead**. This correction was
reported to PM as soon as discovered. The hosted exact-source comparison and
Mongo causal evidence are independent of this local mismatch.

The replacement pre-W8 build is an isolated read-only source snapshot of exact
hosted baseline `8589cfd8`, built under the mandated environment with both KMS
features and no coverage flags. The shared Cargo target serializes it with the
other streams. The final W8/fixed arms also have matching uninstrumented build
profiles. Common test names and plain CSV/log evidence remain under
`/tmp/nyxid-w8-ci-perf`; no evidence archive is required.

The fixed measurement copy is committed source `31873a467` plus the four
CORE audit implementation/test files and the exact owned `test_utils.rs` diff.
It holds caller source at that commit rather than incorporating later lanes'
uncommitted conversions. The cached W8 executable is not an exact-source rebuild
of hosted head `19549c364`; the hosted logs provide that exact-head comparison.
It uses the mandated target/jobs/incremental/debug settings and both KMS
features; no profile overrides or weaker Mongo write concern.

## Validation and append profile

Final fixed-run timings and validation results are recorded in the W8 batch log
when available. The manual `audit_lifecycle_command_profile` benchmark records
fixture/restart time, command names/counts/durations, HMAC computation and direct
appends. It records no command/event bodies. The existing previous-versus-durable
benchmark runs three alternating 256-append rounds at one and 16 tasks, checks
every resulting chain, and preserves both implementations' retry budgets.

An initial append benchmark run concurrent with the 300-test workload completed
serial rounds at a 1.70x median slowdown, then failed on WriteConflict after the
existing 32-body budget in the 16-task durable round. This failure is retained
in `/tmp/nyxid-w8-ci-perf/append-before.log`; it is not a passing contention
result or a clean capacity measurement. Subsequent measurements run without a
second workload against this replica.

The clean pre-fix append benchmark passed (one test, zero failed/ignored) with
1,536/1,536 appends in each implementation across all rounds. Median previous /
durable throughput was 198.21 / 168.36 appends/s at one task (1.18x slowdown),
and 261.65 / 166.09 at 16 tasks (1.58x). Serial development-build latency was
5.045 / 5.940 ms per append: approximately 0.895 ms extra for the stronger path.
These figures include every validation/CAS/outbox/commit step and exclude fixture
setup and final chain verification. The replica had no concurrent test workload;
other streams continued compiling on the host. The log is
`/tmp/nyxid-w8-ci-perf/append-clean-before.log`.

The first fixed-code command profile passed (one test, zero failed/ignored):
256 direct appends took 1,565.386 ms (163.54/s), with exactly 256 body attempts
and 256 commit calls. The summed commit-command time was 909.317 ms, or
3.552 ms/append; total latency was 6.115 ms/append. The other command counts
were 256 state aggregates, 512 audit lookups, 256 head updates and 256 inserts
into each of audit log and outbox. Ten thousand standalone HMAC calculations
took 218.669 ms (21.867 microseconds each). Tail cryptography is a small fraction
of this path and is preserved. An initialized restart took 2.981 ms.

Fresh fixture initialization took 8,051.288 ms in that invocation, including
7,757.788 ms in the existing shared readiness-probe write. The five audit index
commands totalled 172.136 ms. This profile was started after the preceding test
process exited; its cleanup could still leave server transactions and DDL in
flight. Do not present that fixture duration as normal index or append cost.
Evidence: `/tmp/nyxid-w8-ci-perf/fixed-profile.log`.

A repeat after service-history validation also passed, with the same 256 body
attempts/commits and command counts. Setup took 1,722.166 ms (five index builds:
825.842 ms); restart took 9.658 ms. Direct appends took 5,093.758 ms (50.26/s),
with 4,119.908 ms in commit commands, while HMAC remained 23.163 microseconds.
The replica had no second test workload, but the shared host was compiling;
durable I/O timing varied substantially. Both profiles are retained. These
numbers support the command/CPU breakdown and preservation of tail HMAC, not a
production capacity promise. After the run, serverStatus showed zero open,
active or inactive transactions; four databases remained including Mongo's
system databases and the fixture coordination database. Evidence:
`/tmp/nyxid-w8-ci-perf/fixed-profile-quiet.log`.

A further fixed-code throughput run passed, with all durable appends succeeding.
The serial median previous/durable rates were 202.03/164.69 per second (1.23x).
At 16 tasks, the previous algorithm exhausted four of its eight-attempt budgets;
the durable algorithm exhausted none. That concurrent comparison is explicitly
not an all-success throughput ratio and cannot replace the clean comparison
above. Evidence: `/tmp/nyxid-w8-ci-perf/append-clean-fixed.log`.

The shared-tree executable containing these four CORE files passed the full
backend `audit_` filter: 166 passed, zero failed, two requested manual benchmarks
ignored, 163.18 s. This also executes the preceding machine-principal and
service-history retry coordinator tests. A frozen committed-source copy is used
separately for the representative timing comparison; the shared executable also
contains other lanes' current caller conversions.
The complete service-history filter also passed: 25 passed, zero failed/ignored,
23.63 s (`/tmp/nyxid-w8-ci-perf/shared-fix-history.log`).

The first frozen-source build (before adding fixture cleanup) also passed the
exact snapshot-closure regression: one passed, zero failed/ignored, 2.71 s.
Build plus target-lock wait took 41m28s; that is build contention, not the
measured test time. The complete frozen copy adds only the owned fixture diff
to this source; shared-tree integration may additionally include other lanes.


## Integration status after fixture cleanup

The compatibility follow-up `af55e0602` passed backend all-target Clippy with
both KMS features and 13 focused detail tests. The complete cleanup fixture
filter passed 27/0/0. A subsequent broad audit run returned 166 passed, two
failed and two manual benchmarks ignored: the existing 32-body contention
budget exhausted under host load, and the runtime-shutdown cleanup regression
raced its own lifecycle drop against its explicit exit-recovery drop. The latter
is a test-ownership race, observed as Mongo `DatabaseDropPending` (215), not a
three-second timeout. Its focused correction retains the database guard through
manual recovery while still dropping the owning runtime and session first.
An isolated contention recheck passed all 256 appends from 16 tasks in 2.25 s,
with 691 body attempts, maximum 16 body attempts/one commit attempt per operation
and zero integrity conflicts. This rerun does not replace the broad-run failures.

PM accepted the mechanism, measurements and fixes `aeebc7d16` / `c103bc99f`
pending hosted end-to-end proof on `82e7605bb`. The replacement matching-profile
300-test local comparison is incomplete and no final before/after ratio is
claimed; its idle runner has been stopped while hosted validation and the urgent
caller handoffs take precedence. The original instrumented baseline's timing
remains disqualified. No test timeout, transaction lifetime, durability setting
or retry budget has been increased.

The coordinator handoff reuses the shared both-KMS executable compiled from
19:34:14, with mtime 19:46:01. The corrected fixture filter passed 27/0/0 in
4.44 s. Its broader `audit_` run completed 190/1/2 in 347.96 s; only the
256-append/16-task contention test failed, with raw Mongo code 112 and
`TransientTransactionError` after the unchanged body-attempt ceiling. The
shutdown-cleanup test passed. A single isolated contention recheck also failed
0/1/0 in 13.48 s. This reproduces contention independently of the broad filter;
the earlier isolated success does not establish reliable contention handling.
No passing broad-suite or restored end-to-end runtime claim is made from this
run. Logs are `/tmp/nyxid-w8-ci-perf/handoff-{audit_,contention-isolated}.log`.

The CORE-B RSA guard refresh procedure is now committed. Both the refresh and
the working-tree reference/private-operation validation pass; retaining the
cleanup guard changes no bound RSA reference or private-operation line, so
there is no policy pin delta to include with this focused test correction.

## Hosted 82e7605bb follow-up (2026-10-20)

The first fixes did **not** restore the hosted Backend Test job: its 90-minute
deadline fired after 8,740/9,786 tests, including about 23 minutes of compilation.
Coverage completed 9,749 tests in 3,591.6 s, versus 9,615 tests / 3,356 s in
the pre-W8 job. These jobs are distinct measurements, not interchangeable proof.

Parsing the supplied nextest logs gives 8,728 Backend passing-test durations
summing to 15,941.925 s (median 0.059 s, p95 12.176 s). Coverage's 9,739 passing
durations sum to 14,305.287 s; the baseline's 9,614 sum to 13,412.114 s. The
Coverage log contains repeated failure summaries, which are not extra executed
tests. For common passing names only: 8,552 Backend/baseline tests total
15,755.136 / 13,091.276 s (1.203x), while 9,554 Coverage/baseline tests total
14,036.676 / 13,357.413 s (1.051x). These sums are aggregate process time, not
wall-clock runtime; parallel tests overlap.

The timed-out Backend host also recorded substantially more I/O pressure than
Coverage. From resource samples at elapsed >=1,500 s through the last sample,
Backend spent 678.0 s with some I/O stall and 586.9 s with full I/O stall in
3,843.0 s; Coverage recorded 172.3 / 121.1 s in 3,542.8 s. Memory-pressure
stalls in those windows were 0.14 / 0.001 s. This is an observed host/job
difference; it does not establish that all remaining cost is environmental.

Before the reboot, a same-machine command profile measured fixture initialization at
250.638 ms, including the writable readiness probe at 6.322 ms and five audit
index commands totalling 159.146 ms. The earlier 7,757.788 ms probe was a
contended sample, not a fixed per-test sleep. Restart took 2.463 ms; 256 direct
appends took 1,443.204 ms (177.38/s), including 969.701 ms in majority+journal
commit commands. HMAC alone took 15.979 microseconds per invocation. The index
commands are independent across collections and can overlap, but all results
must succeed before initialization or writer activation; this optimization is
in validation. Every index definition, tail HMAC check, snapshot, head CAS and
majority+journal guarantee stays intact.

An alternating direct-Mongo DDL experiment used eight trials per ordering
on the same replica, with the same five collection/index commands and index
verification after every trial. Serial median was 311.391 ms; concurrent median
was 268.552 ms, saving 42.839 ms (13.8%). This is a modest isolated DDL saving,
not proof that the hosted timeout is resolved. Subsequent Rust profiles under
shared host load varied from 1,031.771 to 1,468.275 ms per fixture and 45.10 to
70.12 appends/s, so they are not comparable throughput improvements against
the quiet sample above. Evidence: `hosted82-index-ddl-causal.log` and
`hosted82-profile-index-parallel-{0,1,2}.log` in the same local evidence directory.

Fault scripts were not isolated across nextest processes. The hosted delivery
script armed two unscoped insert failures at 12:27:56; another process's probe
failed at 0.008 s and the script then observed two bodies rather than three.
Other scripts used a process-local mutex, so their server-global configurations
could overwrite one another even with app-name restrictions. The shared guard
now owns the cross-process kernel lock, unique client/appName, arming and joined
panic-safe disarming. The source scan now finds configureFailPoint only in
the shared helper; the lane migrations use its documented API. This is a test
isolation correction, not a production retry-budget change.

Contention is independently reproducible: the unchanged cached executable ran
the 256-append/16-task test alone 20 times on own Mongo :50012, with 17 passes
and three failures (rounds 0/12/18). No fault script ran on that replica during
this measurement. Errors contain the natural server WriteConflict message and
TransientTransactionError, which the runner already classifies as retryable.
A second isolated diagnostic series stopped after two passes and one failure.
That failure reached body attempt 32 with zero commits, Mongo code 112 and the
`TransientTransactionError` label. The existing classifier recognizes both:
this reproduced case exhausts the bounded body budget under starvation; it is
not an unlabeled error escaping classification. Evidence:
`/tmp/nyxid-w8-ci-perf/hosted82-contention-diagnostic-2.log`. Local FIFO
scheduling before opening transaction snapshots is being evaluated separately;
no retry/timeout increase or passing contention claim follows from these failures.

The platform rotation test also reproduced locally (0/1/0 in 5.48 s). Its
fixture signed one chain with native-review-test-audit and later used AppState's
[2;32] key. W8's tail HMAC validation correctly rejected that mismatch. PM
approved exactly five fixture argument corrections; `d31615713` uses the fixture
key throughout and leaves production verification intact. The complete platform
service test filter passed 13/0/0 on the first post-reboot shared both-KMS binary.

The first post-reboot shared executable (compiler start 22:10:43, binary mtime
22:20:43) also passes the scoped helper 4/0/0, its cross-process lock 1/0/0,
the bounded core conflict/unknown-commit test 1/0/0 and service-history coordinator
tests 3/0/0. Fixture-owned heartbeat teardown `cc4a8e0e6` passes 2/0/0 without
heartbeat failure or panic diagnostics. Evidence now lives under
`~/Library/Caches/nyxid-w8-core/evidence/post-reboot-*.log`: the host reboot
cleared the earlier `/tmp` evidence, so earlier counts are recorded observations,
not surviving raw logs. None of these focused passes establishes restored
end-to-end runtime or resolves the separate natural contention failure.

The following top-40 tables include both jobs and their same-name pre-W8
durations. New/unavailable means no baseline passing result with that exact
name. Full paths: /tmp/pm-ci82/Backend_Test_.log,
/tmp/pm-ci82/Coverage__Backend__.log, /tmp/pm-job-118380570787.log. Local evidence:
/tmp/nyxid-w8-ci-perf/hosted82-{profile-before,platform-before}.log and
hosted82-isolated-contention-before.csv. Final post-fix numbers and any
sharding recommendation remain pending validation.

| Job | Test | Seconds | Pre-W8 seconds | Ratio |
| --- | --- | ---: | ---: | ---: |
| backend | `services::remote_machine_runtime::tests::journeys::local_journey_paid_http_manager_billing` | 219.929 | 212.422 | 1.04 |
| backend | `handlers::service_pool_ai_tests::operation_scopes::service_pool_operation_scope_checks_translated_backup_on_every_entry` | 97.773 | 82.336 | 1.19 |
| backend | `handlers::service_pool_proxy_tests::operation_scopes::service_pool_credential_fallback_checks_selected_member_scope` | 90.490 | 78.938 | 1.15 |
| backend | `handlers::service_pool_ai_tests::pool_ai_custom_api_prefixes_match_actual_native_dispatch` | 85.611 | 63.169 | 1.36 |
| backend | `services::platform_public_contract_service::tests::handshake::expired_consent_and_changed_signed_body_prevent_funding_and_provider_effects` | 82.291 | 54.892 | 1.50 |
| backend | `services::channel_x_tests::review::oversized_backlog_advances_with_notice_and_does_not_fail` | 81.439 | 56.722 | 1.44 |
| backend | `handlers::assistant_nyxagent::tests::capacity_refusals_recover_after_old_retry_limit_and_slow_proof_respects_first_byte_bound` | 81.167 | 88.698 | 0.92 |
| backend | `services::mfa_service::tests::w8_id_mfa_enable_disable_rollback_and_secret_free_events` | 68.766 | new/unavailable | — |
| backend | `handlers::assistant_nyxagent::tests::capacity_wait_beyond_31_seconds_keeps_heartbeat_and_owner_slot` | 61.865 | 61.933 | 1.00 |
| backend | `services::destination_routing::tests::auto_activation::google_concurrent_sync_inserts_editors_and_updates_old_contracts_once` | 59.162 | 58.940 | 1.00 |
| backend | `handlers::login_approval::tests::app_identity_creates_a_browser_session_only_for_the_persistent_choice` | 56.915 | 51.139 | 1.11 |
| backend | `handlers::login_approval::tests::app_identity_creates_no_session_or_extra_mfa_challenge_for_request_only_verification` | 56.534 | 52.231 | 1.08 |
| backend | `services::destination_routing::tests::auto_activation::google_incompatible_requirements_and_custom_policies_remain_unchanged` | 53.855 | 45.957 | 1.17 |
| backend | `handlers::service_pool_ai_tests::pool_ai_codex_catalog_with_custom_connection_slug_uses_local_destination` | 53.573 | 50.532 | 1.06 |
| backend | `handlers::catalog_editor_tests::key_updates::key_put_revocation_during_transaction_leaves_no_partial_write` | 51.752 | 40.123 | 1.29 |
| backend | `handlers::service_pool_ai_tests::operation_scopes::service_pool_alias_discovery_intersects_native_member_operations` | 51.673 | 51.403 | 1.01 |
| backend | `handlers::service_pool_proxy_tests::pool_proxy_attempt_deadline_bounds_headers_first_body_and_rejection_drain` | 48.367 | 42.495 | 1.14 |
| backend | `services::billing::machine_runtime::tests::fractional_windows_finalize_atomically_once_and_drop_the_final_fraction` | 47.807 | 41.751 | 1.15 |
| backend | `services::platform_manager_transport::journey_tests::default_and_configured_signup_login_mfa_review_and_same_chat_confirmation` | 45.501 | 55.793 | 0.82 |
| backend | `services::platform_public_contract_service::tests::handshake::concurrent_ingress_refresh_shares_one_durable_probe_and_failed_refresh_stays_unavailable` | 45.450 | 30.719 | 1.48 |
| backend | `handlers::service_pool_ai_tests::pool_ai_slug_and_gateway_preserve_destination_and_translate_each_member` | 45.350 | 37.070 | 1.22 |
| backend | `handlers::service_pool_ai_tests::pool_ai_native_stream_error_never_becomes_an_empty_success` | 44.575 | 26.290 | 1.70 |
| backend | `handlers::service_pool_proxy_tests::runtime::pool_hung_403_body_keeps_configured_health_and_retry_policy` | 41.912 | 31.115 | 1.35 |
| backend | `handlers::service_pool_proxy_tests::operation_scopes::service_pool_selected_member_does_not_intersect_sibling_scopes` | 41.902 | 40.585 | 1.03 |
| backend | `services::channel_adapters::aurinko::tests::aurinko_batch_timeout_cursor_allows_later_messages_progress` | 41.427 | 40.764 | 1.02 |
| backend | `handlers::service_pool_proxy_tests::runtime::pool_proxy_denied_earlier_org_service_cannot_hide_later_priority_pool` | 40.990 | 42.129 | 0.97 |
| backend | `services::billing::exact_tests::account_check_uses_constant_work_after_many_postings` | 40.531 | 25.891 | 1.57 |
| backend | `handlers::service_pool_proxy_tests::runtime::machine_buffered_pool_request_fails_over_with_declared_pool_authority` | 39.931 | 26.138 | 1.53 |
| backend | `handlers::login_approval::tests::persistent_mfa_creates_session_only_after_second_factor` | 38.889 | 55.041 | 0.71 |
| backend | `services::platform_key_service::tests::inactive_personal_rows_preserve_disabled_connections_and_replace_tombstones` | 38.534 | 25.719 | 1.50 |
| backend | `services::platform_key_service::tests::inactive_org_rows_preserve_disabled_connections_and_replace_tombstones` | 38.496 | 26.335 | 1.46 |
| backend | `handlers::service_pool_proxy_tests::runtime::pool_health_403_requires_explicit_trigger_on_repeated_requests` | 38.132 | 28.941 | 1.32 |
| backend | `handlers::login_approval::tests::request_only_mfa_is_required_before_authorization` | 37.643 | 53.456 | 0.70 |
| backend | `handlers::service_pool_proxy_tests::runtime::pool_encoding_identity_is_final_and_compressed_rejection_still_retries` | 37.401 | 27.645 | 1.35 |
| backend | `handlers::service_pool_proxy_tests::runtime::pool_529_default_post_cools_without_replay_and_opt_in_retries` | 37.287 | 26.669 | 1.40 |
| backend | `handlers::service_pool_proxy_tests::runtime::pool_committed_caller_drop_does_not_cool_but_upstream_failure_does` | 37.111 | 27.266 | 1.36 |
| backend | `services::platform_public_contract_service::tests::handshake::failed_readiness_recovery_releases_real_funded_and_pre_funding_holds` | 36.752 | 21.991 | 1.67 |
| backend | `services::exact_service_approval_service::destination_approval_tests::google_corrected_contracts_drift_pending_exact_approvals_then_reapprove_normally` | 35.821 | 29.067 | 1.23 |
| backend | `services::platform_public_contract_service::tests::handshake::rejected_or_mismatched_model_never_persists_ready` | 34.795 | 23.948 | 1.45 |
| backend | `services::mfa_service::tests::test_verify_totp_setup_with_valid_code` | 33.972 | 46.828 | 0.73 |
| coverage | `services::remote_machine_runtime::tests::journeys::local_journey_paid_http_manager_billing` | 209.041 | 212.422 | 0.98 |
| coverage | `services::mfa_service::tests::w8_id_mfa_enable_disable_rollback_and_secret_free_events` | 101.071 | new/unavailable | — |
| coverage | `handlers::assistant_nyxagent::tests::capacity_refusals_recover_after_old_retry_limit_and_slow_proof_respects_first_byte_bound` | 87.960 | 88.698 | 0.99 |
| coverage | `handlers::service_pool_ai_tests::operation_scopes::service_pool_operation_scope_checks_translated_backup_on_every_entry` | 81.917 | 82.336 | 0.99 |
| coverage | `handlers::service_pool_proxy_tests::operation_scopes::service_pool_credential_fallback_checks_selected_member_scope` | 79.123 | 78.938 | 1.00 |
| coverage | `services::destination_routing::tests::auto_activation::google_concurrent_sync_inserts_editors_and_updates_old_contracts_once` | 74.465 | 58.940 | 1.26 |
| coverage | `services::channel_x_tests::review::oversized_backlog_advances_with_notice_and_does_not_fail` | 69.913 | 56.722 | 1.23 |
| coverage | `handlers::service_pool_ai_tests::pool_ai_custom_api_prefixes_match_actual_native_dispatch` | 66.881 | 63.169 | 1.06 |
| coverage | `handlers::assistant_nyxagent::tests::capacity_wait_beyond_31_seconds_keeps_heartbeat_and_owner_slot` | 61.109 | 61.933 | 0.99 |
| coverage | `services::platform_manager_transport::journey_tests::default_and_configured_signup_login_mfa_review_and_same_chat_confirmation` | 60.459 | 55.793 | 1.08 |
| coverage | `services::destination_routing::tests::auto_activation::google_incompatible_requirements_and_custom_policies_remain_unchanged` | 59.302 | 45.957 | 1.29 |
| coverage | `handlers::login_approval::tests::app_identity_creates_a_browser_session_only_for_the_persistent_choice` | 58.964 | 51.139 | 1.15 |
| coverage | `handlers::login_approval::tests::app_identity_creates_no_session_or_extra_mfa_challenge_for_request_only_verification` | 58.691 | 52.231 | 1.12 |
| coverage | `handlers::login_approval::tests::persistent_mfa_creates_session_only_after_second_factor` | 55.195 | 55.041 | 1.00 |
| coverage | `handlers::login_approval::tests::request_only_mfa_is_required_before_authorization` | 52.519 | 53.456 | 0.98 |
| coverage | `handlers::service_pool_ai_tests::pool_ai_codex_catalog_with_custom_connection_slug_uses_local_destination` | 52.399 | 50.532 | 1.04 |
| coverage | `services::platform_public_contract_service::tests::handshake::expired_consent_and_changed_signed_body_prevent_funding_and_provider_effects` | 52.344 | 54.892 | 0.95 |
| coverage | `handlers::service_pool_ai_tests::operation_scopes::service_pool_alias_discovery_intersects_native_member_operations` | 49.907 | 51.403 | 0.97 |
| coverage | `services::mfa_service::tests::test_verify_totp_setup_already_verified` | 48.266 | 45.161 | 1.07 |
| coverage | `services::mfa_service::tests::test_verify_totp_login_with_valid_code` | 48.030 | 44.872 | 1.07 |
| coverage | `services::mfa_service::tests::test_verify_totp_setup_with_valid_code` | 47.427 | 46.828 | 1.01 |
| coverage | `handlers::catalog_editor_tests::key_updates::key_put_revocation_during_transaction_leaves_no_partial_write` | 43.018 | 40.123 | 1.07 |
| coverage | `services::mfa_service::tests::w8_id_mfa_pending_login_has_one_winner_and_rechecks_factor` | 41.968 | new/unavailable | — |
| coverage | `services::billing::machine_runtime::tests::fractional_windows_finalize_atomically_once_and_drop_the_final_fraction` | 41.665 | 41.751 | 1.00 |
| coverage | `handlers::service_pool_proxy_tests::pool_proxy_attempt_deadline_bounds_headers_first_body_and_rejection_drain` | 41.371 | 42.495 | 0.97 |
| coverage | `services::channel_adapters::aurinko::tests::aurinko_batch_timeout_cursor_allows_later_messages_progress` | 40.902 | 40.764 | 1.00 |
| coverage | `handlers::service_pool_proxy_tests::operation_scopes::service_pool_selected_member_does_not_intersect_sibling_scopes` | 40.832 | 40.585 | 1.01 |
| coverage | `handlers::service_pool_ai_tests::pool_ai_slug_and_gateway_preserve_destination_and_translate_each_member` | 39.496 | 37.070 | 1.07 |
| coverage | `handlers::service_pool_proxy_tests::runtime::pool_proxy_denied_earlier_org_service_cannot_hide_later_priority_pool` | 37.340 | 42.129 | 0.89 |
| coverage | `services::exact_service_approval_service::destination_approval_tests::google_corrected_contracts_drift_pending_exact_approvals_then_reapprove_normally` | 36.550 | 29.067 | 1.26 |
| coverage | `handlers::service_pool_proxy_tests::billing::pool_proxy_paused_consumer_renews_lease_and_lease_loss_closes_provider` | 34.425 | 32.734 | 1.05 |
| coverage | `services::assistant_service_execution::tests::compaction::org_stateful_model_transport_full_access_and_auto_resume_keep_exact_consent_without_tool_grant` | 34.343 | 27.581 | 1.25 |
| coverage | `services::assistant_skill_authoring::tests::pending_source_read_keeps_one_retryable_proposal_without_a_card` | 31.951 | 31.872 | 1.00 |
| coverage | `services::assistant_skill_authoring::tests::pending_source_read_timeout_keeps_retryable_reservation_without_a_card` | 31.722 | 31.691 | 1.00 |
| coverage | `handlers::nyxbot::tests::late_delivery_tests::image_delivery_tests::nyxbot_unknown_image_skips_rest_and_claim_survives_retry` | 31.534 | 31.353 | 1.01 |
| coverage | `services::token_service::tests::w8_id_session_and_mfa_c2_tier_cells` | 31.459 | new/unavailable | — |
| coverage | `services::platform_public_contract_service::tests::handshake::concurrent_ingress_refresh_shares_one_durable_probe_and_failed_refresh_stays_unavailable` | 31.368 | 30.719 | 1.02 |
| coverage | `services::assistant_service_execution::tests::managed::actual_managed_http_and_mcp_success_settles_once_and_records_one_receipt` | 30.873 | 27.818 | 1.11 |
| coverage | `services::platform_key_service::tests::inactive_org_rows_preserve_disabled_connections_and_replace_tombstones` | 29.954 | 26.335 | 1.14 |
| coverage | `services::platform_key_service::tests::inactive_personal_rows_preserve_disabled_connections_and_replace_tombstones` | 29.759 | 25.719 | 1.16 |
