# NyxID automation ticket sign-off (#2040)

**Verdict: BLOCKED — combined acceptance remains open.**

**SOURCE_REVIEW_COMPLETE. LOCAL_VALIDATION_COMPLETE.**

Issue [#2040](https://github.com/ChronoAIProject/NyxID/issues/2040), **[Bug][NyxBot][Automations] Scheduled group workflow reports Completed before queued specialist updates resume the coordinator**.

Astra personally reviewed the exact final source commit
`6df83d87b826141ec025ce2276f66cb147cbfe33` and the actual evidence. Local verification
is complete: **134 unique backend tests passed**, required all-target/all-feature
warnings-denied Clippy passed, formatting passed, and the unchanged frontend has 17
passing tests plus lint/build passes. The backend window has been released to Artifacts.

The remaining ticket gate is combined I-01/I-02/I-03/I-04/I-06 on the coordinator's
integration commit. No `ASTRA_IMPLEMENTATION_APPROVED` is recorded. #2040 has no live
X publication, production callback or deployed-client acceptance gate.

## Reviewer and source identities

| Identity | Exact value |
| --- | --- |
| Reviewer | GPT-6-Astra PM, Heca `01a17a9b-50b4-7e41-bb3b-dcbbf6204ad3` |
| Review record time | 2026-10-27 09:25:14 UTC; exact final source and evidence reviewed |
| Worktree / branch | `01a17a96-a01b-7f10-a98b-646d494dc1d5` / `fix/automation-completion-2040` |
| Common base | `b1dd6548ade04ec4f0051203e1353796331c7f87` |
| Active approved plan SHA256 | `5e3469b2754b8ca8127519c19ba1349049288f70371c4d9a2de5a1a761799fc7` |
| Preserved historical frozen plan SHA256 | `0633146fbe464aa94fd6a3a98ba7c0a2b4ef0ce00da147bc5bc2741cbbfb96d4` |
| Original feature candidate | `e0e53834ae94ae838631195ec63826426f348617` |
| Previously validated feature/acceptance correction | `2bf59012f80dc82b0bf113977562d9617c278990` |
| Final implementation commit | `6df83d87b826141ec025ce2276f66cb147cbfe33`: unused helper removal, production follower-deadline regression, chat 08/09 and CLAUDE budget documentation |
| Distributed lint prerequisite | `336405f8e667ea2eb2c6513effec3d15fd960b02` |
| Applied prerequisite/source snapshot | `d151005983937693df0b716f87d1b78058e0e518` |
| Historical report-only commit | `6d0cb44c5e1bbfb1de4f2f922485fa48670019d6`; not an implementation identity |
| Persistent Sol agent | `01a17b03-c22c-7c50-a038-27bf595a74fd` |
| Sol Heca session | `01a17b03-c22e-7503-ae21-ef4d38050be9` |
| Sol provider session | `01a17b03-c43f-74f3-b98c-beb00d71c256` |

Astra reverified Sol's actual runtime as codex / `gpt-6.1-sol` / `x_high` /
`full-access` in the required worktree at 09:06 UTC. The same Sol implements all fixes;
no extra reviewer or subagent is used. Historical Opus agent
`01a17aa2-d74e-7151-9efa-b40638c3befb` and its OAuth failures remain in the ledger,
marked **SUPERSEDED_BY_USER**, never passed.

## Issue criteria and personal review

The body and October 26 comment describe two scheduled group runs incorrectly recorded
Completed after roughly 64/68 seconds: researchers/editors finished, queued specialist
updates remained, no independent reviewer or publisher started, and `create_tweet`
remained zero. A textual specialist mention was not a durable handoff. Neither X API
failures nor local client memory pressure is established as the orchestration cause.

The resulting contract retains the run while attributable group work or an exact queued
result remains, resumes only the correct owner/run/thread, and completes after the
coordinator's final result. Deadline, allowance, cancellation and unclassifiable legacy
states have bounded truthful outcomes. An explicit recorded failure or skip is distinct
from successful completion. The required publication check uses a local fixture.

Astra personally read the complete cumulative implementation diff and surrounding
scheduler, admission, transaction, follower, event selection, migration, Stop, authority,
DTO and UI code. The earlier F-1–F-7 review is carried forward. The latest targeted review
covered the entire continuation module, actual terminal bookkeeping and abort/replay
fixtures, concurrent claims and persisted owner budgets, production query/explain/read
bounds, fake-agent workflow, and final follower-expiry correction. Sol's summary is not
used in place of source or log inspection.

The four-file prerequisite was separately reviewed as equivalent boolean transformations
before its exact import. No sibling feature branch was imported. Read-only combined
review used Artifacts `47a2f3f1845b1896e172e68ec077bb8270ee2e33` and Permissions
`493b5c2dc012cc4de80c00f33f7cddf489cb316d`. Automation preserves trigger run/claim/drain
ownership; integration must retain Artifacts' `bind_in_session(db, keys, row, session)`
and original Web/User provenance, plus Permissions' metadata classifier and independent
guest HTTP callback guard. No proxy-marker authorization shortcut is introduced.

## Executed evidence and exact commands

Durable evidence is in [2026-10-27-automation-evidence](2026-10-27-automation-evidence/).
The [historical validation manifest](2026-10-27-automation-evidence/automation-2bf59012-validation.json)
contains all test names, duplicate reconciliation and original log SHA256 values.
[Recovered commands](2026-10-27-automation-evidence/commands-2bf59012.json) preserve the
actual commands and original Astra Heca activity sequence IDs.

The historical binary was
`/Users/chronoai/Desktop/projects/nyxid/target/debug/deps/nyxid_server-a7bf57b9c4cb166a`,
SHA256 `a692860b8edbbd09c48a71f1199e56e0751ec0ea1d3300f1579aa64e39844a3a`.
Its source manifest has 42 entries, SHA256
`48dc9c951134f23c27050951f760184c25dbc3b039f5042a3c2b0a12632efdec`, all matched to
`2bf59012f80dc82b0bf113977562d9617c278990` by Astra and independently by the coordinator.
Subsequent rebuilds replaced the executable path; its new bytes do not retroactively
identify these earlier passes.

The final 42-entry source manifest matches every corresponding committed/current byte
at `6df83d87b826141ec025ce2276f66cb147cbfe33`, SHA256
`25a9c5f4151a28dc49b7de051f93b39b415177debb01dbab8c0bacdeb0f4c1a0`.
The corrected binary SHA256 is
`5c9a6b0c9c549009ffc700f08b329fc747f54ee495ac557d14408bc216a90057`.
The [final verification record](2026-10-27-automation-evidence/final-verification.json)
binds the exact source tree, individual test names, final logs, duplicates and Clippy
command/argv evidence. The binary was built from these bytes before the final source
commit and can embed pre-commit build metadata; that is distinct from source identity.


All backend commands use the assigned shared target and jobs=2. The shared config is
`/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml`.
It changes only package `nyxid` dev/test debug information and incremental compilation;
assertions, overflow behavior, optimization and coverage retain manifest defaults.
The isolated fixture is Mongo8 on loopback port 27440, replica set `nyxidAutomation2040`,
with unique managed test databases. Required transaction tests verify topology/sessions
and perform an actual transactional write/abort; they fail if Mongo is unavailable.

| Check | Actual command/filter | Result / archived log |
| --- | --- | --- |
| Focused build and automation acceptance | `cargo --config <shared-config> test -p nyxid automation_v2_ -- --nocapture` | PASS: 21; `automation-rust-focus-retry3.log` |
| Scheduler and performance | Historical binary `schedule_ --test-threads=2 --nocapture` | PASS: 33; `automation-scheduler-regressions-retry1.log` |
| Group routing | Historical binary `group_messages_ --test-threads=2 --nocapture` | PASS: 2; `automation-group-regressions.log` |
| Async watches and routing | Historical binary `async_service_ --test-threads=2 --nocapture` | PASS: 33; `automation-async-regressions.log` |
| Source queue/Stop | Historical binary `source_queue --test-threads=2 --nocapture` | PASS: 18; `automation-source-queue-regressions.log` |
| Final group correction | `cargo --config <shared-config> test --manifest-path backend/Cargo.toml --bin nyxid-server handlers::assistant_group::tests:: -- --nocapture`, `RUST_TEST_THREADS=1` | PASS: 13, exit 0; `automation-followers-expiry-test-20261027T090413Z.log` |
| Missing scheduler checks | New unchanged binary, exact 16 previously absent scheduler tests, serial | PASS: 16, test process exit 0; `automation-followers-expiry-scheduler-20261027T091503Z.log`; original wrapper exit 126 explained below |
| Required Clippy | `cargo clippy --config <shared-config> --manifest-path backend/Cargo.toml --all-targets --all-features -- -D warnings` | PASS, exit 0 in 6m19s; `automation-followers-expiry-clippy-20261027T091635Z.log`. Actual argv retains config/all-target/all-feature; backend cfg includes `aws-kms` and `gcp-kms`. Package scope is `nyxid` |
| Frontend tests | From `frontend/`: `npm test -- --maxWorkers=2 src/pages/automations.test.tsx src/schemas/automations.test.ts` | PASS: 2 files/17 tests; `nyxid-automation-2040-frontend-test-final.log` |
| Frontend lint/build | From `frontend/`: `npm run lint -- --quiet`; `npm run build` | PASS; archived lint/build logs, including TypeScript/Vite/prerender/credential-accept/footprint checks |
| Final formatting/whitespace | `cargo fmt --all --check`; `git diff --check` | PASS, both exit 0 at 09:18:55 UTC; `automation-final-fmt.log.json` and `automation-final-diff-check.log.json` |

Five successful historical backend logs reconcile to **107 executions / 106 unique
passed / zero failures or ignored**. The sole overlap is the schedule/webhook migration
fixture. The broad `schedule_` filter also selects existing credit-schedule regressions;
those are counted as executed tests, not mislabeled as newly added automation tests.
The changed frontend bytes are unchanged since their passing checks.

The final correction adds 13 successful group executions and 16 previously omitted
scheduler executions. Combined accounting is **136 successful executions / 134 unique
passed / 0 failed / 0 ignored**: the migration case overlaps the old focus/schedule logs,
and one group-routing case overlaps the old/new group logs. These are carried-forward
plus affected-test results, not 134 rerun cases on the final binary. Deleting an unused
production helper and updating its test/docs does not change the other tested behavior.

The scheduler child exited 0 and its summary reports 16 passed. Its first log verifier
returned 126 because a timing print separated one test name from `ok`; the original
exit/metadata remain intact. A separate verifier record and Astra's independent parse
confirm all 16 exact requested cases. No suite was rerun to repair a reporting parser.

## Acceptance and finding disposition

| Criterion / finding | Actual evidence | State |
| --- | --- | --- |
| Premature completion / exact workflow progress | Completion barrier and fake-agent research → edit → independent review → local publish/readback tests pass; delayed/out-of-order results remain bound to the original run | Local PASS |
| Exact binding and durable follower notification | Two real runs, selective drain, competing follower claims, queue full/retry, lost notification lease and cross-run event IDs pass | Local PASS; combined I-01/I-02 OPEN |
| Persisted budget limits / F-1 | Atomic per-run and owner hourly/daily charges; independent reset and last-slot contention pass. Changed deferred-run test passed in the final missing-cases batch | Local PASS |
| Barrier/registration/Stop races / F-4/F-6 | Real transaction conflict/abort paths, terminal cleanup, stale claims and post-Stop admission pass | Local PASS; combined I-03 OPEN |
| Protocol persistence / F-5 | Real trigger update reloads persisted protocol across active/disabled/non-assistant variants and rejects stale updates | Local PASS |
| Migration terminal semantics / F-7/F-8 | Real abort/replay checks work, audit, history, projection, timing, failure policy, generation and progress; missing/changed/wrong-owner/unsupported targets cannot seal readiness | Local PASS |
| Old-reader compatibility | Real lease/drain/marker/flag-off checks, existing schedule/webhook/confirmation classification and rollback tests pass. Operator assertion is required; flag/marker do not attest absence of old processes | Local PASS |
| Performance and query cost | Actual Mongo command monitoring/explain asserts indexed bounded production predicates, no trigger read on ordinary admission, bounded migration pages/readiness/history/work discovery | Local PASS |
| History, waiting UI and privacy | Backend history/metadata-only audit assertions and 17 frontend page/schema cases pass; waiting reason/deadline/counters reflect durable state | Local PASS |
| Dead helper / follower deadline coverage F-9 | Unused helper removed; copied-filter test rejected. New real claim → expired follower with live notification lease → production delivery rejection/cleanup personally reviewed and executed | Local PASS |
| Normative documentation F-10 | Chat08/chat09 and CLAUDE now distinguish budgeted coordinator Event continuations from confirmation resumes, and document real rollout/rollback mechanics | Source review COMPLETE |
| Omitted scheduler cases F-11 | Name-by-name comparison identified 16 absent cases; all 16 passed from the unchanged corrected binary | Local PASS |
| Combined permissions/artifacts behavior | Exact Web/User watch admission and original turn association, trigger selective drain, both Stop scopes, no invalid recovery/materialization and independent guest callback fence reviewed read-only | I-01/I-02/I-03/I-04/I-06 OPEN |

Measured local S7 performance: 10,000 schedules/one due produces a covered query with
zero documents and one key examined; discovery 0.220 ms/query, 100-schedule list 7.815 ms,
fenced CAS 9.471 ms, idle tick 0.406 ms, confirmation-to-completed 228.004 ms,
scheduled-to-started 296 ms and three-day catch-up 49.893 ms. These fixture measurements
do not establish deployed throughput or real-provider latency.

Failures remain archived: five initial malformed json macros, two missing doc imports,
18-pass/3-fail fixture retry before correction, missing-v2-env scheduler runs, unused-helper
Clippy, and the helper-only rebuild whose 13 group cases failed without the Mongo env
(exit 101). The latter proves the harness fails instead of silently skipping. None is
counted as a successful check. Details and fixes are in the
[review ledger](2026-10-27-automation-review.md).

## Remaining actions and ticket verdict

**SOURCE_REVIEW_COMPLETE** and **LOCAL_VALIDATION_COMPLETE** are recorded for
`6df83d87b826141ec025ce2276f66cb147cbfe33`. All identified local F-1–F-11 findings are
resolved with source or execution evidence; there is no open local implementation or
check defect. Opus remains superseded by the user.

Coordinator independently verified zero Cargo/compiler/backend-test/linker processes
and 17.21 GiB free after Clippy. The owned guard did not fire and its process group was
empty. Automation released heavy execution for Artifacts; no further heavy command is
started or queued. Documentation/evidence work does not retain the resource window.

The smallest remaining dependency is the coordinator's isolated integration candidate,
composed from the final individually reviewed sources. Automation must personally
inspect the resolved shared code and actual I-01/I-02/I-03/I-04/I-06 results bound to
that exact commit. Those rows cover mixed trigger/Web completion selection, subsequent
exact artifact consumption, Stop races, invalid-origin artifact refusal and independent
owner/guest callback/cancelled-denial behavior. I-05/I-07 are not #2040 gates. The
coordinator's `integration-candidates.json` marks Automation's final source ready to
freeze while other inputs remain provisional; no integration commit or executed
combined pass is implied.

**#2040: BLOCKED on combined acceptance.** Final approval requires
`ASTRA_IMPLEMENTATION_APPROVED` only after those applicable criteria are demonstrated.
No merge, deployment, external publication or issue closure is authorized by this report.
