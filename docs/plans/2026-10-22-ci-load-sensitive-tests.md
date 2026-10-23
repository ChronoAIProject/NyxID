# CI stability follow-up: issue #2015

Investigation base: `6f4b94709d4feb7bb4034d8f831384db4fd8902a` (PR #1986).
Latest main integration base: `1dbf3b32c` (includes the Assistant launch).
Scope: the four remaining load-sensitive tests, failed-run diagnostics, and
backend image cache reuse. This builds on the four-shard backend workflow;
the test inventory verifier, coverage merge, 73% gate, and existing retry
policy remain in force.

## Root causes and changes

| Failure | Cause | Change |
| --- | --- | --- |
| Trigger confirmation continuation | A terminal run is committed before `finish_record` writes `schedule_state.last_run`. The fixture treated the earlier write as completed settlement and could race the later projection with an exact-source owner edit. | The completion helper waits for the matching terminal projection before returning. It keeps its existing bounded wait and outcome checks. |
| Machine Git credential drift | Proxy credential resolution starts an asynchronous usage update. That update changed `updated_at`, which the credential replacement source predicate compares. | Usage advances only `last_used_at`, using `$max` to preserve ordering. Actual edits keep `updated_at`; encrypted fields, ownership, credential epochs and the full existing source predicate remain fenced. A regression forces the usage write between the source read and the edit predicate, and checks that a real epoch change still invalidates that predicate. |
| FAQ limit/reset journey | Twenty admissions and a rejection assertion could span two real minute windows. | A task-local test clock drives the existing MongoDB `$dateTrunc` pipeline. The test fills the last millisecond of a window, crosses the boundary by one millisecond, and checks admission recovery. It still covers transaction abort, live mode-change refusal, sender/profile caps and absence of rejected receipts. Production uses `$$NOW`. |
| Migration lease renewal | The observer slept while the migration was free to finish or lose its shortened lease. Cleanup removed the fields the observer then read. | A notification holds the operation open. A task-local clock and explicit heartbeat ticks drive four actual database renewals past the original lease lifetime. The test retains the production lease duration, verifies ownership/readiness on each renewal, then releases work and checks that late renewal cannot alter the ready marker. Production clock/timer behavior is unchanged. |

## Workflow improvements

- `backend_test_summary.py` writes counts, every failure/retry/skip, missing or
  malformed reports, and the 20 slowest JUnit cases to a JSON artifact and the
  job summary. Displayed Markdown is capped at 900,000 UTF-8 bytes with a
  truncation notice; JSON retains every failure/retry/skip and raw JUnit retains
  every reported test result. It runs before the
  execution-result gate, so one failed shard no
  longer hides the evidence from the others. The workflow also retains raw
  per-shard JUnit reports in a 14-day diagnostic artifact. This helper is
  informational and never authorizes coverage publication.
- Ordinary archive builds install nextest without coverage-only tools.
- The Docker commit identity is declared after dependency compilation. A new
  commit still rebuilds/stamps the application, while identical manifests and
  lockfiles can reuse the dependency layer.
- Trusted image builds export backend compiler layers to
  `ghcr.io/<owner>/nyxid/backend:buildcache-linux-{amd64,arm64}`. PR image checks
  read the amd64 reference without registry write credentials. Existing GHA
  imports remain as a transition fallback; PR exports use `mode=min` so
  intermediate compiler layers do not compete with Rust caches. Cache export
  is best-effort and release tags remain gated by CI.

## Verification

The local evidence below was collected before integration with the newer main
base above. It validates the fixes at the investigation base; full hosted CI
and coverage must validate the integrated PR before merge.

- 52 CI helper unit tests passed, including failure-plus-retry visibility,
  missing/malformed shards, invalid durations, safe name rendering, and bounded
  slow-test output without truncating failures.
- All workflow files passed actionlint. The reusable-workflow permissions
  validator passed all three caller/callee checks.
- Replayed all 10,161 JUnit cases from hosted run `39657789719`, attempt 2:
  both failures were preserved in JSON and Markdown; the existing inventory
  verifier still rejected the failed run and produced no publishable report.
- A forced live MongoDB predicate probe missed the existing credential in
  10/10 iterations with the old usage write and 0/10 with the new write.
  This probe exercises database semantics, separately from Rust service tests.
- Static Docker instruction inspection confirms that the commit hash is out
  of scope for the dependency build and in scope for the application build.
  Anonymous HEAD of the existing backend `latest` manifest returned HTTP 200,
  confirming the registry read prerequisite for fork PRs.
- The Rust test binary built successfully using Rust 1.98.1 and `-Dwarnings`.
  The macOS linker emitted its large-unwind-table warning (a linker diagnostic,
  not a denied Rust warning). Runtime tests use nextest 0.9.146, the CI profile,
  four slots, an 8 MiB test stack, and explicit `--retries 0`. Nextest consumes
  metadata for that built binary; changed Rust source fingerprints are checked
  to ensure the artifact still matches the working tree.
- The first stress run passed seven complete rounds before local MongoDB
  aborted with WiredTiger `Too many open files` at `15:44:38Z`. Its log is
  retained separately. The local server had the default 300-second snapshot
  history and 8,192-file limit; it was replaced with a fresh disposable replica
  set using CI's zero snapshot-history window and a 65,536-file limit. No Rust
  code or assertion changed for that infrastructure rerun.
- The final stress run passed all 60 executions: ten iterations of the four
  reported cases, the related skill-confirmation variant, and the new usage/edit
  regression. Nextest reported `10/10 stress run iterations: 10 passed` in
  142.768 seconds, with retries disabled.
- The first surrounding run passed 191/192 tests. The remaining usage-timestamp
  assertion expected `updated_at` to advance and compared the persisted
  millisecond timestamp with the higher-precision create result. It now checks
  persisted values, requiring usage to be recorded while `updated_at` stays
  unchanged. The preceding stress run's production code and six test cases did
  not change.
- After rebuilding the final source, all 193 related tests passed in 92.187
  seconds with retries disabled: scheduler, machine transport, migration, FAQ,
  service history, credential management, and the ordinary MongoDB-time
  fixed-window counter test. Inventory inspection confirmed the renamed
  timestamp test was present in the rebuilt binary. This is a selected suite,
  not a full backend or instrumented coverage run.

Hosted cache export/import and end-to-end Docker timing still require a run
of the changed workflow. The first trusted build populates the new registry
cache; no speedup is claimed from a static configuration check.
The production Docker embed guard also requires the release build's dep-info;
that artifact was unavailable locally, so this guard remains a hosted check.

After main integration and the final summary-size safeguard, all 53 CI helper
unit tests passed, including a mass-failure case with multibyte names that
verifies bounded, intact Markdown and complete structured results. Workflow
lint, reusable-workflow permissions, Rust formatting and whitespace checks
also passed on the integrated source.

To repeat the focused stress check, configure a disposable MongoDB replica set
as in `.github/scripts/start-test-mongodb.sh`, set `NYXID_TEST_DATABASE_URL` to
its connection string, and use nextest 0.9.146 with retries explicitly disabled:

```sh
RUST_MIN_STACK=8388608 cargo nextest run -p nyxid --profile ci \
  --retries 0 --test-threads 4 --stress-count 10 --no-tests fail \
  -E 'test(=handlers::trigger_scheduler::tests::schedule_confirmation_waits_and_resumes_without_another_budget) |
      test(=handlers::trigger_scheduler::tests::scheduled_skill_allow_keeps_work_wake_and_durable_nyxbot_event) |
      test(=services::machine_transport_tests::machine_git_final_send_rejects_live_catalog_and_credential_drift) |
      test(=services::platform_manager_transport::journey_tests::faq::faq_limits_reset_and_admission_abort_or_mode_change_cannot_send) |
      test(=services::platform_entry_migration::tests::owned_lease_is_renewed_during_an_operation_longer_than_its_initial_lifetime) |
      test(=services::service_history::tests::usage_touch_preserves_pending_credential_edit_but_rotation_invalidates_it)'
```

## Remaining CI work

The earlier measurements put the single backend test-crate compilation at
roughly 16 minutes and document extensive full-schema fixture creation. The
next substantial improvements are measured crate extraction and narrower
database fixtures, while preserving real index/migration integration tests.
During this follow-up, local compiler samples showed LLVM code generation
with reported peak physical footprints of 29.6 GB and 30.5 GB on a 16 GB macOS machine.
This is a local resource observation, not a hosted timing or a before/after
performance comparison.

The main publish gate inspected during this investigation (`39792090328`)
passed both backend suites but failed because `oracle-worker-test` was cancelled.
PR #2017 addresses that cold browser setup timeout but merged into
`chore/retire-builtin-oracle`, not main. The first hosted run of this follow-up
(`39846645748`, job `120061721689`) reproduced the same 15-minute timeout:
Chromium setup took 10m53s, syntax and unit tests passed, and GitHub cancelled
the still-running browser suite after 4m09s. This follow-up therefore also
applies #2017's 30-minute job budget to main. It changes neither browser test
assertions nor their individual deadlines.

Cache usage observed during this follow-up: 165 entries, 11,934,166,536 bytes.
That is a point-in-time measurement, not a post-change performance result.

## Additional hosted regression

After integrating main `68636647f`, run `39856482241` exposed a separate
fixture race in `cma_agent_input_waits_for_activity_before_settling_ready`.
Ordinary shard 3 failed both attempts because it expected a completed watch
after one sweep. The fixture creates two watches against a service with one
execution slot, while four sweep workers may poll concurrently. Capacity
refusal legitimately defers a watch, including between status and result
calls, so one sweep need not settle both watches. The fixture also read an
unspecified row between phases.

The test now resolves both idempotency keys to stable watch IDs and waits,
with a bounded deadline, for each to observe `ready`, then `working`, then
completed `ready`. Only outstanding watches are made due again. Initial
readiness requires an actual status observation and no terminal result;
unexpected terminal results fail immediately. The service capacity, production
sweeper, retry policy and test assertions remain intact, with both watches now
checked through the complete transition.


## Analytics fixture follow-up

Hosted run `39866095746` passed all 10,357 instrumented backend tests without
retries and reported 89.27% line coverage. Ordinary shard 1 exposed two existing
analytics setup failures: the active uncertainty repair fixture failed while
creating its 32-row backlog through bounded admissions, and the independent
telemetry client test failed its healthy-admission precondition before an outage
check. The MongoDB log records a consent read rejected with `MaxTimeMSExpired`
at 199 ms at `2026-10-22T23:32:18.297Z`; the backlog failure coincided with slow
transaction work and unrelated index/drop activity. Diagnostics retained both
the repeated failure and the retry, and inventory verification rejected the run.

The repair fixture now seeds historical uncertain rows and their account
checkpoint directly, verifies 32 unresolved rows, then exercises real cap refusal,
operator reconciliation, and a single real resumed admission. It checks the
32-to-31-to-32 transition and the new capture generation, retaining stale consent,
stale generation, late settlement, audit and provider-drain assertions. It joins
the existing exclusive analytics scheduling group. The neighboring bound test
continues to exercise repeated real admission and settlement.

The telemetry fixture refreshes its existing read-only responsiveness probe
immediately before positive admission prerequisites. Queue-capacity assertions
prove both replicas and both permission-change cases actually admitted work
before withdrawal/deletion. Negative cases remain single-shot. Production
200/250 ms deadlines and fail-closed behavior are unchanged; unknown admission
outcomes are never retried.

Main `3a5e4175d` separately split CMA receipt deduplication into its own test.
Integration preserves that test and explicitly creates the second watch needed
by this PR's bounded lifecycle observation. The merged source must be validated
by fresh local tests and a new full hosted run before merge.


Run `39873605977` then passed 10,375 ordinary tests with zero retries, including
telemetry and both CMA tests. The one failure was an added setup probe in the
repair fixture: successful majority-write plus linearizable-read pairs took
about 103 ms and missed its synthetic 100 ms threshold. That compound probe
measures a different workload from the real 250 ms snapshot transaction. The
two added probe calls were removed; durable fixture writes, exact count and
generation reads, exclusive scheduling, and real single-shot cap/refill calls
remain. The shared helper and its unrelated caller are unchanged.

Main `1dbf3b32c` integrated cleanly without touching this PR's implementation
files. The superseded local test-binary build was stopped after approximately
44 minutes; it produced no fresh test result. The corrected integrated revision
requires the full hosted suites and gates before merge.
