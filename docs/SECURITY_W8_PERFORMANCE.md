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
does not close these abandoned writer sessions. A focused fixture teardown
extension has been proposed to PM: retain only each test client's session IDs
and end exactly those sessions, from the existing independent cleanup runtime,
before dropping that client's test database. This must preserve transaction
rollback, observer behavior, cleanup budgets and isolation from other clients;
no production transaction limit or cryptographic control may be weakened.
The proposed session-ending call and database drop share the original
three-second attempt budget; the existing exit budget, retry count and backoff
are unchanged. Only session IDs are tracked, with no command or event bodies
retained by the fixture.

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

## Local comparison method

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

The same-host comparison reproduces added median setup cost (+83%) but only a
7.2% increase in summed process time, not the entire hosted factor. Long,
preexisting paths and other shared-host activity make these distributions
variable. It would be incorrect to claim that this local run alone reproduced
the hosted twofold slowdown. Cached binary identities, common names, runner and
plain CSV/log evidence are under `/tmp/nyxid-w8-ci-perf`; no receipt archive is
required.

The fixed measurement copy is committed source `31873a467` plus only the four
CORE audit implementation/test files. It excludes other lanes' uncommitted
caller conversions, so their changes cannot determine the before/after result.
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
