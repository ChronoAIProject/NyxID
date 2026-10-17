# Oracle pool ownership transfer and deletion

This work implements issues #1527 and #1542 while keeping HTTP handlers,
business services, and plain MongoDB models separate. Pool ownership remains
the existing polymorphic `OraclePool.user_id`; it does not introduce a second
organization field or replace a pool during transfer.

## Contract and implementation plan

1. Add a manager-only ownership-transfer API and matching CLI operation. Verify
   management authority over the current pool and the destination owner using
   existing owner ACLs. Organization-to-organization moves require management
   rights at both ends; personal destinations must be the caller's own active
   person account. Preserve identity, slug, settings, workers, queued tasks,
   sessions, and history. Preserve the pool token by default and offer explicit
   rotation through existing one-time secret conventions. Platform visibility
   is an audience setting, not a distinct owner type; do not widen visibility
   implicitly. Resolve org visibility when moving to a person explicitly in the
   documented contract. Revalidate contributed-worker authorization against the
   new owner so previous membership cannot confer authority after transfer.
2. Add authenticated `DELETE /api/v1/oracle/pools/{pool}` and CLI deletion.
   Prefer a retained tombstone, reserve its slug, hide it from normal discovery,
   and invalidate both shared and installation worker credentials. Return an
   acknowledgement that needs no subsequent GET. An authorized repeated delete
   returns the same acknowledgement. Reject queued/dispatched tasks, pending
   commands, and other active work with a dedicated typed 409 conflict. Define
   online-worker and open-session behavior explicitly; do not force-abandon work.
3. Establish a database-backed serialization boundary for lifecycle changes
   and all relevant writers. A stale authenticated pool or worker snapshot must
   not create presence, submit/dispatch work, enqueue commands, or alter pool
   settings after deletion. Include enrollment, heartbeat, results, sessions,
   login profiles/snapshots, background expiry/recovery, and concurrent transfers
   in the write-path audit. Use existing MongoDB transaction/retry conventions;
   a process-local mutex is insufficient across replicas. Keep unrelated traffic
   free of additional database work.
4. Retain terminal task results under existing TTL and audit history with their
   tombstone reference; document every remaining collection's retention or
   cleanup. Commit lifecycle state and cleanup atomically, or use a durable,
   retryable state machine if bounded cleanup cannot be atomic. Audit metadata
   records actor, pool identity, and old/new owner only; never secrets or browser
   data. Route secret handling and delegated-auth policy through existing rules.
5. Add meaningful database and HTTP authorization/lifecycle regression tests,
   including races, retries, partial failure, stale worker requests, token
   invalidation, contributor membership changes, and collection retention.
   Document API status/response contracts, CLI examples, upgrade requirements,
   and release availability in the repository's existing release-note format.
6. The primary agent reviews the entire implementation directly, reports all
   actionable findings to the same Heca implementer, and repeats review and
   targeted verification until no known actionable findings remain.

## Verification

Run Rust formatting, relevant backend and CLI tests, database-backed lifecycle
tests against a transaction-capable MongoDB instance, and appropriate compile/
lint checks. Review all pool-related writes and authentication gates in source,
not only newly added routes. Record exact verification evidence and final
contract decisions here before completion.

## Final contract and serialization decisions

- Ownership remains `OraclePool.user_id`. Transfer requires current source and
  destination management, active actor/owner accounts and current admin
  membership, including existing Direct org-owned general API keys. A personal
  destination is the caller's own active Person. Org-to-person transfers require
  explicit private visibility when the source audience is org. Other visibility
  changes stay on PATCH. Destination creation/transfer counts exclude tombstones
  and contend on the same owner User inventory document.
- Shared tokens survive transfer unless explicit rotation is requested. New raw
  transfer tokens enter `Zeroizing` immediately after minting and remain there
  through transaction writes/audit/retries. Hashes alone are stored. A separate
  defaulted pool installation epoch invalidates all old installation credentials
  in constant space on owner change; re-enrollment needs a fresh credential and
  destination membership identity. Worker identity, affinity, tasks, sessions,
  profiles and history remain. A full updated pool response computes permissions
  and online count inside the transaction; acknowledgement needs no postcommit
  GET. Transfer accepts UUID owner IDs, caps bodies at 4096 bytes and uses no-store.
- Delete refuses queued/dispatched tasks and queued/delivered commands with the
  typed active-work conflict. Idle online workers and idle sessions do not block
  it. The transaction writes an inactive tombstone, invalid shared token, durable
  pending-cleanup marker, lifecycle version and mandatory hash-chained audit.
  Tombstones reserve slugs and disappear from normal discovery/authentication.
  The timestamp is normalized to BSON milliseconds before both storage/return.
- Delete's stable 200 acknowledgement is `{id,slug,status:"deleted",
  cleanup:"asynchronous",deleted_at}`. It confirms logical deletion, not physical
  cleanup completion. DELETE attempts sixteen bounded cleanup transactions after
  commit; failure logs a deferred retry without changing the acknowledgement.
  A five-second worker processes at most sixteen pending pools per pass, rotating
  by last attempt to avoid starvation. Each small transaction fences the retained
  pool and removes at most one worker, profile, snapshot and closes one session.
  Remaining rows are the durable cursor. The pending marker clears only after an
  empty chunk; no arbitrary pool deletion limit or in-memory progress is used.
  Scheduling and open-session indexes support this bounded traversal.
- Sessions/history remain. Open idle sessions gain `closed_at` asynchronously;
  they cannot accept work once the tombstone commits. Terminal task TTLs remain
  unchanged; terminal commands keep the existing seven-day TTL. Cleanup removes
  worker credentials/presence and encrypted login snapshots/profiles/bindings.
  Imported browser logins require upstream logout separately. Expired profile
  envelope cleanup projects IDs only, scans sorted ID pages of at most 256 rows,
  drains a full pass with yields, and never caps total expiry at 256 per hour.
- Pool writes reuse enrollment's `worker_enrollment_epoch` in Mongo transactions.
  Expected lifecycle version/owner/shared-token comparisons reject stale pool
  snapshots. Active user/org and membership writes conflict with disable/revoke.
  Worker snapshots additionally compare live credential/incarnation/membership
  identity. No process-local mutex supplies lifecycle correctness. Mandatory
  transfer/delete audit appends use `log_actor_event_in_session`; rollback also
  rolls back the audit chain head. Driver retries clone owned inputs per attempt,
  propagate database errors, and do not emit successful external effects before
  commit. Duplicate client refs resolve under the pool fence, after live submit
  authorization, before quotas/session creation; aborted duplicate inserts are
  never reused for fallback reads.
- Upgrade all replicas and drain old replicas plus their in-flight requests
  before using these operations. Old binaries ignore fences/tombstones and must
  not be restarted against lifecycle-mutated pools. MongoDB transactions require
  a replica set/sharded deployment. Availability is unreleased: the first tagged
  release containing these changes. The release workflow derives GitHub notes
  from commits; there is no checked-in CHANGELOG or assertion that 0.70.1 shipped
  these routes. Package version changes remain the shipping workflow's job.

## Writer audit inventory

| Writer boundary | Serialization/current authority |
|---|---|
| Pool create/update/rotate/transfer/delete | Transactional pool and/or owner inventory fence; live manager and active destination; lifecycle audit in same transaction |
| Worker enrollment/renewal | Existing transaction extended with captured pool owner/version/token; active person/org and membership identity writes |
| Worker allocation/commands/forget/cancel/command listing expiry | Expected pool plus current manager/contributor authority; retained generation worker lock; related state in same transaction |
| Worker presence, instance binding, command delivery/reports/cancellation reads | Expected pool, installed credential/incarnation and live membership; command leases and unique delivery index handled before queued delivery |
| Task submit/extract/attach | Expected pool plus live submitter visibility authority; session/task/quota/idempotency decisions in same snapshot |
| Task claim/ack/result/transcript/pin | Expected pool plus installed worker revalidation; related worker/session/task records use the supplied transaction |
| Task cancellation/session closure | Retained history lookup and current manager lock when not original submitter/owner; authorization denial keeps historical 404 masking, database errors propagate |
| Status recovery/lease expiry/stale affinity/capacity settlement | Runs inside caller's fenced pool transaction, including status GET's lazy writes |
| Snapshot fanout/profile save/bind/delete/unbind | Snapshot creation and each streamed worker command have separate fenced/current-manager transactions; profile/binding mutations retain their existing atomic contract |
| Profile worker refresh/current | Expected pool; pool credentials only at handler boundary, worker instance/capability validated in transaction |
| Profile envelope expiry | Bounded fenced transactions, including pending tombstones; ID-only pagination drains full pass |
| Tombstone physical cleanup | Durable pending marker and retained pool fence; bounded resumable transactions, fair scheduling |
| Startup default-model migration | Only pool-document updates, excludes tombstones and increments pool enrollment/lifecycle versions atomically |

Ordinary task/session history and listing reads stay lightweight; transaction
read helpers remain only for actual composed mutation snapshots. The new lifecycle paths and dedicated DTOs are registered in generated OpenAPI
(the first Oracle registrations). Lifecycle discovery is also documented in
`docs/API.md`, canonical `docs/ORACLE_RELAY.md`, and `AI_AGENT_PLAYBOOK.md` included
by the existing llms.txt handler.

## Final verification evidence

Implementation and the primary's source/fixture review are complete. All required
checks below passed. The implementation checkpoint was verified on
`implement-nyxid-issues-1527-1542` before the shipping phase below. One requested Heca implementation session used
`gpt-6.1-sol`, `xhigh`, full access; the primary personally reviewed and iterated
with that same session. No native subagents or additional implementation/review
sessions were created. No push, PR, deployment, or GitHub mutation was performed.

Owned test MongoDB: `mongodb://127.0.0.1:27279/?replicaSet=oracleLifecycle`, data
and logs under `/tmp/nyxid-oracle-lifecycle-mongo`, bound to loopback. This session
started and initialized the single-node replica set. Unrelated Mongo instances
and build processes were left alone. Tests used isolated random NyxID databases
with production idempotency, enrolled credential, one-delivered-command and
cleanup uniqueness/index definitions. After final verification, the primary
verified the PID, exact command and lock file, sent SIGTERM to this task-owned
instance only, and confirmed shutdown completed with exit code 0.

All Cargo compilation ran serially with `-j 1`. Test compilation used only the
package-specific debug overrides below; dependency profiles and unrelated Cargo
workloads were not changed.

Backend Oracle regression command:

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27279/?replicaSet=oracleLifecycle' \
cargo test -j 1 -p nyxid --bin nyxid-server \
  --config 'profile.test.package.nyxid.debug=0' \
  --config 'profile.test.package.nyxid-cli.debug=0' \
  oracle -- --test-threads=3
```

Actual result: **exit 0; 138 passed, 0 failed, 0 ignored**, 8084 filtered,
execution 55.18s, build 4m35s (`/tmp/oracle-tests-final-pass.log`). Coverage includes
lifecycle and real routing/JWT/general-key HTTP authentication, org-owned keys,
manager/member/viewer/outsider denial, no-store/body caps, deterministic held
transfer/delete/revocation races, audit rollback, stable duplicate deletion,
bounded cleanup/restart/fairness, command expiry beyond whole-request budget,
partial snapshot fanout, bounded task maintenance and existing Oracle behavior.

CLI Oracle regression command:

```sh
cargo test -j 1 -p nyxid-cli \
  --config 'profile.test.package.nyxid.debug=0' \
  --config 'profile.test.package.nyxid-cli.debug=0' \
  oracle -- --test-threads=3
```

Actual result: **exit 0; 52 passed, 0 failed, 0 ignored**, 1467 main-binary tests
filtered, execution 1.62s, build 4m18s (`/tmp/oracle-cli-tests-final.log`). Other
package test targets had zero matching tests and passed. Tests cover both
lifecycle commands in text/JSON modes, conflict propagation, explicit destination
parsing, org resolution and default token preservation.

Strict validation command (default dev profile; no test-profile overrides):

```sh
cargo clippy -j 1 -p nyxid -p nyxid-cli --all-targets -- -D warnings
```

Actual result: **exit 0, no diagnostics**, 5m08s
(`/tmp/oracle-clippy-final-pass.log`). All targets of both packages are covered
with their shared feature graph. `cargo fmt --all -- --check` also exited 0
(`/tmp/oracle-fmt-final.log`); `git diff --check` exited 0.

Independent primary verification against the same explicit Mongo URI and rebuilt
binaries, with isolated databases:

| Check | Actual result | Evidence |
|---|---|---|
| Lifecycle/HTTP subset | Exit 0; 16 passed, 0 failed, 0 ignored; 16.42s | `/tmp/oracle-primary-lifecycle.log` |
| Complete backend Oracle suite | Exit 0; 138 passed, 0 failed, 0 ignored; 55.94s | `/tmp/oracle-primary-backend.log` |
| CLI Oracle suite | Exit 0; 52 passed, 0 failed, 0 ignored; 1.81s | `/tmp/oracle-primary-cli.log` |
| Strict Clippy, both packages and all targets | Exit 0; no diagnostics; 3m18s | `/tmp/oracle-primary-clippy.log` |
| Formatting and diff whitespace | Both passed | Primary reported successful independent commands |

The primary's independent strict rerun passed. Personal review and correction
iterations are complete, with no known actionable findings remaining. The
coordinated upgrade/drain requirement above is the deployment contract.
Backend binary: `target/debug/deps/nyxid_server-8b528c998f501022`.
CLI binary: `target/debug/deps/nyxid-563da248eda465cd`.

Earlier checks are not represented as successful. Compilation exposed MongoDB
callback higher-ranked Send inference; converted operations now use explicit
boxed Send callbacks with owned retry context and per-attempt input clones.
Strict lint findings were fixed through natural input groups, removal of unused
wrappers/reexports, test-only annotations and mechanical cleanup, without lint
suppressions. The first executed full backend suite exited 101 with 134 passed
and four fixture failures (`/tmp/oracle-tests-green.log`, 19.14s). Those obsolete
fixtures used stale pool authority, an unseeded quota caller, or contributors for
manager-only operations. Corrected tests explicitly assert denial and use live
pool/active caller/current manager fixtures. Production gates were preserved.

After successful runtime runs, the only source changes were two `#[cfg(test)]`
annotations for genuinely test-only helpers. Their test behavior is unchanged;
the final all-target strict check compiled them in the test target and excluded
them from production. The primary explicitly confirmed no additional runtime
rebuild was necessary solely for those annotations. No source edits follow the
successful strict check; this final plan update records the completed evidence.

## Changed files

- `CLAUDE.md`
- `README.md`
- `docs/ARCHITECTURE.md`
- `backend/src/api_docs.rs`
- `backend/src/db.rs`
- `backend/src/errors/mod.rs`
- `backend/src/handlers/oracle_login_profiles.rs`
- `backend/src/handlers/oracle_pools.rs`
- `backend/src/handlers/oracle_worker.rs`
- `backend/src/handlers/oracle_workers.rs`
- `backend/src/main.rs`
- `backend/src/models/oracle_pool.rs`
- `backend/src/models/oracle_worker.rs`
- `backend/src/routes.rs`
- `backend/src/services/assistant_native_execution.rs`
- `backend/src/services/assistant_service_execution_authority_tests.rs`
- `backend/src/services/mod.rs`
- `backend/src/services/oracle_login_profile_service.rs`
- `backend/src/services/oracle_login_snapshot_service.rs`
- `backend/src/services/oracle_mutation_service.rs`
- `backend/src/services/oracle_pool_service.rs`
- `backend/src/services/oracle_pool_service/lifecycle.rs`
- `backend/src/services/oracle_pool_service/lifecycle/tests.rs`
- `backend/src/services/oracle_pool_service/lifecycle/tests/http.rs`
- `backend/src/services/oracle_session_service.rs`
- `backend/src/services/oracle_task_service.rs`
- `backend/src/services/oracle_worker_enrollment_service.rs`
- `backend/src/services/oracle_worker_enrollment_service/tests.rs`
- `backend/src/services/oracle_worker_service.rs`
- `backend/src/services/oracle_worker_service/management.rs`
- `backend/src/services/oracle_worker_service/management/tests.rs`
- `cli/src/cli.rs`
- `cli/src/commands/oracle.rs`
- `docs/AI_AGENT_PLAYBOOK.md`
- `docs/API.md`
- `docs/ORACLE_RELAY.md`
- `docs/plans/oracle-pool-lifecycle-1527-1542.md`

## Shipping integration

The user authorized PR creation, CI monitoring, squash merge into main, branch
removal and local build/service teardown. The branch is now
`feature/oracle-pool-lifecycle`, following CONTRIBUTING.md's feature prefix.
It incorporates `origin/main` at `0aceacb0`. The only merge conflict was in
`docs/API.md`; both independent documentation additions were retained. Upstream
Oracle input-validation visibility changes and assistant admission gates remain
intact. The final compatibility review identified a main-integration regression:
normal worker heartbeats advanced `worker_enrollment_epoch`, which main's native
Oracle consent identity hashed as execution state. The correction excludes only
that Oracle coordination counter from both consent issuance and held-admission
revalidation. Ownership, lifecycle/installation epochs and execution settings
remain bound; non-Oracle identity projections are unchanged. Regression coverage
uses the authenticated heartbeat and task-submit handlers, verifies successful
card redemption and held admission after a heartbeat, and verifies configuration
updates and ownership transfer still reject old approvals. Formatting and
whitespace checks passed; fresh full CI must execute these new regressions before
merge. No other actionable compatibility finding remains in the personal source
review. README and architecture documentation now link
and explain that contract as required by the contribution guide. No package
version or release tag is changed; releases use the repository's tag workflow.

Post-integration backend Oracle tests passed: **139 passed, 0 failed, 0 ignored**
(`/tmp/oracle-integrated-backend.log`). The CLI Oracle rerun also passed: **52
passed, 0 failed, 0 ignored** (`/tmp/oracle-integrated-cli.log`). The task-owned
MongoDB replica set was restarted solely for these tests with the contribution
guide's ephemeral-test snapshot-history setting. Its exact process identity and
lock file were verified before SIGTERM; shutdown completed with exit code 0.
Unrelated services and workspaces are outside teardown scope.

PR #1921 targets main with the Conventional Commit title
`feat: support Oracle pool ownership transfer and deletion`. Final GitHub check
results and the squash-merge record are authoritative in the PR. The PR remains
in draft until integration validation passes; merging is gated on settled CI,
current conflict checks, and the primary's final readiness review.
