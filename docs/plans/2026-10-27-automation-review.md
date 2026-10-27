# Automation review ledger (#2040)

Workstream: `automation`  
Plan: [`2026-10-27-automation-implementation.md`](2026-10-27-automation-implementation.md)  
Plan revision: 3  
Plan SHA-256: `5e3469b2754b8ca8127519c19ba1349049288f70371c4d9a2de5a1a761799fc7`  
Frozen snapshot: [`2026-10-27-automation-implementation.freeze-5e3469b2.md`](2026-10-27-automation-implementation.freeze-5e3469b2.md) (same SHA; read-only)  
Historical frozen snapshot: [`2026-10-27-automation-implementation.freeze-0633146f.md`](2026-10-27-automation-implementation.freeze-0633146f.md), SHA `0633146fbe464aa94fd6a3a98ba7c0a2b4ef0ce00da147bc5bc2741cbbfb96d4`  
Base commit: `b1dd6548ade04ec4f0051203e1353796331c7f87`  
Branch: `fix/automation-completion-2040`

## Gate ledger

| Gate | Agent | Exact input | Result | Evidence |
| --- | --- | --- | --- | --- |
| Historical Opus plan review | Claude Opus 5.5 (`01a17aa2-d74e-7151-9efa-b40638c3befb`, provider `claude`, model `claude-opus-5-5`, effort `xhigh`, mode `default`) | Historical revision 2, SHA `0633146fbe464aa94fd6a3a98ba7c0a2b4ef0ce00da147bc5bc2741cbbfb96d4` | **SUPERSEDED_BY_USER** | Two read-only prompts were sent with `notify_on_finish=true`; both failed before review with `Failed to authenticate: OAuth session expired and could not be refreshed` and `401 OAuth access token has expired. Re-authenticate to continue.` Heca activity sequences 1–10. No approval was inferred. The reviewer ID and failures remain historical evidence; do not resume or retry it under the amendment. |
| Astra plan assessment | GPT-6-Astra PM (`01a17a9b-50b4-7e41-bb3b-dcbbf6204ad3`) | Active revision 3, SHA `5e3469b2754b8ca8127519c19ba1349049288f70371c4d9a2de5a1a761799fc7`, frozen at `freeze-5e3469b2.md` | **ASTRA_PLAN_APPROVED** | Astra personally re-read the charter, prepared plan, coordinator findings, relevant scheduler/group/event source, and shared ownership boundary. The amendment records the user's pure Astra-Sol workflow and closes the owner-wide continuation-budget gap by charging continuation claims against the existing persisted hourly/daily trigger budget atomically with the per-run continuation claim. |
| Implementation | Existing GPT-6.1-Sol agent `01a17b03-c22c-7c50-a038-27bf595a74fd` | `6df83d87b826141ec025ce2276f66cb147cbfe33` | **LOCAL_VALIDATION_COMPLETE** | 136 successful executions / 134 unique passes; required all-target/all-feature Clippy and final formatting pass. Heca session and provider session remain separately recorded. |
| Astra implementation review | GPT-6-Astra PM | `6df83d87b826141ec025ce2276f66cb147cbfe33` | **SOURCE_REVIEW_COMPLETE; FINAL APPROVAL PENDING** | Cumulative review plus exact final committed diff and actual results personally reviewed at 09:25:14 UTC. Local F-1–F-11 resolved. Combined I-01–I-04/I-06 remain OPEN; no implementation approval. |
| Historical Opus final implementation review | Same Claude Opus 5.5 session | Exact final commit | **SUPERSEDED_BY_USER** | The user explicitly removed the Opus final gate. This is not an approval and the historical reviewer will not be resumed. |

The user's amendment supersedes both earlier Opus requirements. Claude authentication is
therefore no longer a dependency, but the failed historical attempts remain recorded and
are not converted into approval. The active workflow is Astra plan sign-off, one Heca Sol
implementation session, and Astra's complete personal implementation review.

## Findings

No substantive Opus findings were returned because the reviewer never authenticated.
The provider failures remain historical evidence only.

### Coordinator findings and Astra resolutions

| Finding | Resolution in active revision 3 | Acceptance evidence required |
| --- | --- | --- |
| Trigger continuations bypassed the existing hourly/daily run budget and generic event guards without a persisted spend bound. | Each run snapshots an immutable `continuation_limit` from `max_auto_continuations`, capped at 32, and fenced claims increment `continuation_count`. Astra identified the remaining owner-wide gap and amended the plan so every trigger-bound continuation also checks and increments the existing persisted `trigger_run_budgets` hourly/daily counters atomically. A temporary owner-budget miss leaves the run/event waiting until the next UTC reset; deadline and per-run exhaustion produce truthful terminal reasons. | Scheduler/continuation tests cover restart, duplicate claims, per-run limit, owner hourly/daily exhaustion, reset retry, deadline expiry, and truthful outcomes. |
| Optional fields alone do not make old binaries refuse v2 events. | The repository has no authoritative replica/version registry. `AUTOMATION_CONTINUATION_V2_ENABLED` is therefore an explicit operator drain assertion: pause schedule and webhook admission, stop all scheduler workers, wait two `LEASE_SECONDS` intervals, verify no active `trigger_work` lease, run fenced `migrate_continuation_v2`, start only upgraded workers, then enable the flag. The durable `trigger_migrations/continuation-v2` marker gates admission; rollback drains v2 work first. Existing schedules, confirmation cards, and ambiguous waits are classified explicitly. | Readiness harness exercises real lease expiry, migration marker, flag-off refusal, existing schedule/webhook migration, confirmation preservation, ambiguous-wait failure, and rollback drain. It does not claim automatic old-process detection. |
| Deterministic event IDs could collide across runs. | Event identity includes group ID, conversation ID, trigger run or legacy marker, follower `since_seq`, and upper group sequence. | Cross-run duplicate/out-of-order tests assert distinct IDs and one event per exact run. |
| Completion could race with handoff creation, active turns, terminal/stale runs, or user Stop. | Exact run/turn/work predicates, durable follower claims, selective drain, and terminal cleanup cover handoff-before-wait, active coordinator turn, terminal/stale run, and post-Stop interleavings. | Integration tests assert each race converges to one exact wait/continuation or terminal cleanup, never later-run consumption or duplication. |

## Astra personal assessment

Astra checked the prepared root-cause evidence against `trigger_scheduler::settle_result`,
`trigger_schedule::admit_turn`, group follower registration/quiet notification,
`append_events_in_session`, and `assistant_team::wake`. The proposed ownership boundary is
preserved: automation owns trigger run identity, waiting state, follower/event binding,
continuation admission, and selective wake; permissions owns bounded stateless
`request_identity` client metadata and leaves proxy marker/classification ownership
unchanged. No permissions or artifacts branch is imported.

The only concrete plan defect found during this assessment was the owner-wide spend hole
created by skipping the owner trigger budget on continuation claims. Revision 3 closes it
with an atomic reuse of the existing budget document and explicit reset/deadline behavior.
The per-run immutable allowance remains as a second fence. Existing generic event-hour and
streak guards remain unchanged for ordinary unbound event turns; only exact trigger-bound
continuations use the trigger-specific path.

## Shared runtime contract

The active plan proposes additive, server-derived fields and semantics:

- `TriggerRun`: coordinator-work wait discriminator/source/deadline, persisted
  `continuation_count`, immutable `continuation_limit`, and
  `continuation_protocol_version`.
- `GroupFollower`: optional `trigger_run_id` plus durable notification claim/sequence
  state, with trigger-bound followers non-evictable under capacity pressure.
- `AgentEvent`: optional `trigger_run_id` and continuation protocol version with
  deterministic replay-safe identity.
- `ChatAuthority`: server-derived `trigger_run_id` propagated from the active trigger turn.
- `TurnClaim`/`TurnStart`: exact trigger continuation claim for `origin: event` turns,
  including run/work/lease fencing.
- `assistant_team::wake`: trigger-aware selective event draining; unrelated events remain
  queued for ordinary wake logic.
- Activation: `AUTOMATION_CONTINUATION_V2_ENABLED` is an operator drain assertion with
  migration-marker fencing, existing schedule/webhook migration, and rollback drain.

These are additive compatibility changes. The plan records the coordinator/permissions boundary. Sol attempted the pre-edit
notification with `notify_on_finish=true`, but Heca rejected delivery as documented
below; neither sibling branch is imported.

## Acceptance items remaining

- Exactly one GPT-6.1-Sol implementer was verified in this worktree; no additional
  agents or reviewers were invoked by Sol.
- Implement every acceptance-matrix row from the approved revision 3 plan.
- Astra personally reviews the complete implementation diff and surrounding code.
- Resolve every Astra finding, including low-risk issues and nits, through the same Sol
  session and re-run affected checks.
- Record `ASTRA_IMPLEMENTATION_APPROVED` only for the exact final commit after acceptance
  evidence and required checks pass.
- #2040 requires fake NyxAgent and local publisher/readback evidence. It has no live X
  publication or deployed-client gate. Keep applicable combined I-01–I-04/I-06 OPEN
  until executed on the exact integration candidate.

## Historical Sol implementation evidence

The sections through F-7 below preserve the state at each earlier review. Current
execution and acceptance status is in the gate table and F-8–F-11/final evidence
sections; old hold/unexecuted statements are not the present result.

### Original implementation handoff

- Verified active plan and frozen revision 3 SHA-256 `5e3469b2754b8ca8127519c19ba1349049288f70371c4d9a2de5a1a761799fc7` before source edits. The plan header's historical “revision: 2” is preserved because the exact approved bytes are authoritative.
- Pre-edit shared runtime notification attempted via Heca `heca_agent_send_prompt` with `notify_on_finish=true` to coordinator `01a17a8a-e916-77a0-a565-083ca25e2aa3` and Astra PM `01a17a9b-50b4-7e41-bb3b-dcbbf6204ad3`. Both rejected by Heca caller ACL: “a caller may act only on itself, agents it spawned, or (as an orchestrator or team lead) the fleet / its team.” Notification remains undelivered; no approval inferred and no additional agent created. Contracts are documented above and in session commentary.
- Astra implementation review is **IN PROGRESS**; approval remains **PENDING** and reserved for Astra's personal complete-diff review.

### Shared build resource window

User instruction received during implementation: Telegram's initial normal-profile Rust compilation is warming the shared target with `CARGO_BUILD_JOBS=2`; defer all new heavy backend build/link commands until Telegram PM reports completion. No backend compile, link, target change, lock deletion, or installation was started. `df -h .` reported **1.8 GiB available** (down from the user's approximately 7.7 GiB). Source, fixture and formatting work continue. The required focused tests/clippy remain recorded for the coordinator/shared-window handoff, not queued or passed.

Initial lightweight `cargo fmt --all --check` found a syntax error in newly expanded fixture constructors; corrected before proceeding. Log: `/tmp/nyxid-automation-2040-format-initial.log`.


## Draft handoff under the shared-window instruction

The source and fixtures below are a **draft for the same Sol session and Astra's
personal review**. They are not backend-validated implementation approval. The
latest user instruction reserves the shared backend window for another stream,
asks for a lightweight handoff now, and explicitly prohibits starting or queuing
`cargo check`, `cargo test`, `cargo build` or clippy pending coordinator clearance
through the shared-window handoff. Local tests are already authorized; no new
user approval is needed to run them once the shared resource window is available.
No such command was started. No required check was weakened or recorded
as passed without execution. `notify_on_finish=true` remains recorded for Sol;
the prior Heca ACL rejection still prevents direct coordinator notification.

Source cutoff: no additional feature scope was added during handoff. Static
corrections narrowed readiness to assistant delivery admission, preserved
unmodeled run fields during migration, tracked migrated started/confirmation
runs for Stop, required exact Event/Trigger admission origins, cleared capacity
retry state on a successful follower registration, and made the privacy fixture
query the real audit collection with a nonempty count assertion.

### Drafted behavior and fixtures

- Server-derived run identity flows from the live unstopped coordinator turn
  through `ChatAuthority`, group followers and versioned events. Quiet followers
  have independent notification leases/fences and UUID-v5 identities containing
  group/thread/run/since/upper sequence. Event acceptance and follower removal
  share a transaction. Trigger events/followers are non-evictable.
- Completion checks exact attributable work before terminal projection. Waiting
  runs retain a completion deadline and immutable protocol/allowance snapshot;
  exact event claims drain only that run. Successful claims atomically consume
  both the per-run allowance and the existing owner hourly/daily trigger budget.
  Temporary owner exhaustion defers to the UTC reset; timeout/limit/Stop/deleted
  thread paths retain bounded terminal reasons.
- The operator migration checks elapsed drain time and real work leases, uses a
  fenced marker and bounded batches, preserves confirmation waits and terminal
  history, fails ambiguous waits truthfully, and stamps assistant triggers. The
  server exposes migration and rollback-check invocation modes before workers
  start. Readiness gates assistant admission; ordinary delivery types retain
  their path. The runbook is in `docs/NYXBOT_SCHEDULES.md`.
- Run response/schema/UI expose waiting kind, completion deadline and bounded
  continuation counters. The UI explicitly says “Waiting for coordinator work”.
- Seven fixtures are drafted in
  `backend/src/handlers/trigger_scheduler_completion_tests.rs` (included from
  the existing scheduler test module), using unique managed test databases and
  local ephemeral HTTP ports. The publish/readback fixture writes only to a
  local fixture endpoint. These Rust tests have **not run**.

### Commands and lightweight results

All commands ran in this exact worktree, with frontend commands in `frontend/`.
The active plan and frozen revision-3 hashes were rechecked and both remain
`5e3469b2754b8ca8127519c19ba1349049288f70371c4d9a2de5a1a761799fc7`;
the preserved revision-2 snapshot still hashes to
`0633146fbe464aa94fd6a3a98ba7c0a2b4ef0ce00da147bc5bc2741cbbfb96d4`.

| Command/check | Result | Evidence |
| --- | --- | --- |
| `git diff --check` | PASS, exit 0 | `/tmp/nyxid-automation-2040-diff-check.log`; repeated before commit |
| `CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 cargo fmt --all --check` | PASS, exit 0 | `/tmp/nyxid-automation-2040-format-final.log`; no compiler/linker |
| `rustfmt --edition 2024 --check backend/src/handlers/trigger_scheduler_completion_tests.rs` | PASS, exit 0 | `/tmp/nyxid-automation-2040-include-format-final.log`; explicit include-file coverage |
| `npm test -- --maxWorkers=2 src/pages/automations.test.tsx src/schemas/automations.test.ts` | PASS, 2 files / 17 tests | `/tmp/nyxid-automation-2040-frontend-test-final.log` |
| `npm run lint -- --quiet` | PASS, exit 0 | `/tmp/nyxid-automation-2040-frontend-lint-final.log` |
| `npm run build` | PASS, exit 0 | `/tmp/nyxid-automation-2040-frontend-build-final.log`; TypeScript, Vite, prerender, credential-accept and footprint assertion completed; large-chunk advisory remains |
| Constructor and changed-signature inspection | PASS for static field/call placement | 37 literals mechanically checked for duplicate/additive fields in `/tmp/nyxid-automation-2040-constructor-audit.log`; four further literals manually checked as detailed below |
| Backend focused tests / compile / clippy | DEFERRED BY USER, not started | Shared backend window held by another stream; coordinator/shared-window handoff required, with no new user approval |

Earlier lightweight failures are preserved rather than treated as product
failures: the first formatter invocation found misplaced fields from scripted
fixture expansion (`/tmp/nyxid-automation-2040-format-initial.log`), corrected
before the successful checks. An initial frontend test invocation at repository
root failed with package-file ENOENT; it was rerun successfully from `frontend/`.
An initial constructor scanner reported unrelated `Trigger` enum variants and
function return bodies as false positives; its refined scan and manual audit
found no remaining duplicate/misplaced fields.

The constructor audit covered every changed stored struct/authority fixture and
new `TurnClaim`, `RunResponse`, `CoordinatorWait` and `FollowerNotification`
literal. Manual supplemental inspection covered
`assistant_acknowledgement_service.rs`'s policy fixture and live authority
constructor, `mcp_service.rs`'s chat-authority fixture, and
`assistant_nyxagent.rs`'s new-conversation constructor. Struct-update fixtures
inherit the remaining fields and have no duplicate overrides. All known
`admit_turn` calls include the origin argument, `post_and_follow` calls include
the optional server run ID, and `append_following` calls use the new optional
(thread, run) tuple; ordinary callers explicitly use `None`. Rustfmt parsed the
changed sources, but this is not type checking.

Resource evidence: `df -h .` sampled 1.8 GiB, then 4.7 GiB and finally about
11 GiB available during source work. No install or large backend command was
started. Frontend checks reused an ignored local `node_modules` symlink to
`/Users/chronoai/Desktop/projects/nyxid/frontend/node_modules`; generated build
output is ignored. No sibling source or branch was imported. The shared target
and normal Cargo locks were preserved; no clean, lock removal or alternate
target was used.

### Acceptance matrix at handoff

Every backend row remains OPEN until executable evidence passes. A fixture name
below describes drafted coverage, not a passing result.

| Active plan area | Draft evidence / passing lightweight evidence | Remaining evidence |
| --- | --- | --- |
| Reproduction | `automation_v2_completion_barrier_claim_restart_and_selective_drain` asserts waiting/no terminal result when handoff arrives before settlement | Compile/run fixture; assert external terminal delivery remains suppressed |
| End-to-end progress | `automation_v2_fake_agent_research_edit_review_fixture_publish_readback` drives real native group posting through a fake model, delayed/out-of-order researchers, editor, independent reviewer, local publish/readback | Compile/run full local fixture workflow; no live publication gate belongs to #2040 |
| Binding | Barrier/selective-drain fixture checks wrong thread, other run, ordinary event and exact active-turn identity | Compile/run; extend explicit second real run/thread interleavings if needed |
| Crash/retry | `automation_v2_follower_lost_claim_queue_full_expiry_and_cross_run_ids` covers persisted snapshot, lost claimant, new fence, queue failure, dedup and cross-run identity | Compile/run; real replica/process restart and concurrent-claim convergence still OPEN |
| Guards | `automation_v2_owner_budget_wait_deadline_limit_and_stop`, `automation_v2_follower_capacity_never_evicts_or_completes`; existing scheduler confirmation/overlap tests retained | Compile/run; independent hourly/daily boundaries, pool-full, busy thread and high ordinary event-hour/streak cases require evidence |
| Lifecycle | Draft fixture checks Stop, completion deadline, per-run limit and thread deletion; existing generation/owner/agent/lifecycle fences retained | Execute lifecycle regressions, terminal/Stop/late settlement races, pause/edit/delete, owner inactivity and membership/grant revocation |
| Compatibility | `automation_v2_legacy_bson_and_unbound_event_guards`; `automation_v2_migration_readiness_real_leases_confirmation_and_ambiguity` checks real lease timestamps, missing marker, flag-off refusal, idempotence, confirmation/ambiguity classification and rollback | Compile/run; explicit existing webhook and exact coordinator migration, ordinary event, channel routing, async service watches and non-assistant delivery regressions still OPEN |
| Observability/privacy | Frontend tests PASS including coordinator wait/deadline/thread link; draft full workflow asserts three metadata-only waiting audits and excludes fixture text/fences/Bearer values | Run backend audit/history assertions; negative logs/payload/secret evidence remains OPEN |
| Performance | Group owner/follower run+thread index added; exact thread/run lookups use IDs; unrelated Stop returns before trigger queries when no run markers | Explain/index and read-count/latency measurements have NOT run |
| Required checks | Formatting, diff check, focused frontend tests, frontend lint and build PASS | Focused Cargo tests then required clippy remain explicitly deferred |

### Known compile and review risks

- No Rust type checking or test linking has run. Highest compile-risk areas are
  MongoDB `and_run2` transaction closures and session lifetimes, boxed `Send`
  futures through wake/settlement recursion, the included test module's inherited
  imports/visibility, and all-target fixture constructors/call signatures. A
  format pass does not prove these types or trait bounds.
- The large fake-agent workflow and audit-count assertions have not executed.
  Its turn-text classification, timing and history-role assertions may need
  correction after the first focused run. Local publication is fixture evidence
  only; it is not external delivery evidence.
- Migration now preserves raw additive run fields and tracks started and waiting
  migrated runs for Stop. Exact webhook/coordinator classification, interrupted
  migration recovery and terminal projection of ambiguous legacy waits still
  need execution and Astra inspection. The operator assertion cannot detect an
  old worker process automatically.
- Deadline/allowance changes racing admission can reject the turn and leave the
  work for its bounded scheduler retry before terminalization. The scheduler's
  work reschedule/wake timing and follower lease-expiry/queue-full interleavings
  need executable concurrency evidence.
- Stop, late completion, terminal cleanup and generation/owner/credential/pool
  fences have source changes but incomplete race/regression evidence. No claim
  is made that all acceptance rows are satisfied.
- No performance measurements or explain plans were obtained. Full rollout and
  implementation acceptance remain pending required checks and Astra review.

### Backend commands reserved for the later window

These commands are recorded for continuation, **not queued or executed**. Resume
them after coordinator clearance through the shared-window handoff; the local
tests are already authorized and require no new user approval. The coordinator's
latest policy keeps automation on **GLOBAL BACKEND HOLD**, with Telegram alone
authorized for the next exclusive retry. No Cargo command may start or queue in
this stream until coordinator clearance. Preserve normal Cargo locking, the
shared target and two jobs; use the shared `--config` file below for every later
test/build/check/clippy invocation. Its package-only nyxid dev/test overrides set
`debug=0` and `incremental=false`, preserving optimization, assertions, overflow
checks, dependencies and test coverage. Do not change profiles or reclaim outputs
in this stream. Tests invoking assistant
admission require `AUTOMATION_CONTINUATION_V2_ENABLED=true`; test database setup
seeds a fresh completed marker, while the migration fixture removes/rebuilds its
own marker and injects flag-off checks without changing process environment.

```sh
CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 AUTOMATION_CONTINUATION_V2_ENABLED=true cargo --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' test -p nyxid automation_v2_ -- --nocapture
CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 AUTOMATION_CONTINUATION_V2_ENABLED=true cargo --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' test -p nyxid trigger_scheduler::tests -- --nocapture
CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 AUTOMATION_CONTINUATION_V2_ENABLED=true cargo --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' test -p nyxid assistant_group::tests -- --nocapture
CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 AUTOMATION_CONTINUATION_V2_ENABLED=true cargo --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' test -p nyxid async_service_operation -- --nocapture
CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 cargo clippy --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' --manifest-path backend/Cargo.toml --all-targets --all-features -- -D warnings
```

After the first focused results, run the remaining applicable ordinary-event,
channel, authority and lifecycle regressions, and re-run only affected checks
for fixes. Astra's `ASTRA_IMPLEMENTATION_APPROVED` gate is **PENDING** and
must identify the exact accepted commit after all required evidence passes.

### Exact changed files in the draft handoff

This list includes both new Rust files and handoff documentation. The session
handoff reports the containing commit SHA and `git show --stat` totals.

- `CLAUDE.md`
- `backend/src/handlers/agent_skills.rs`
- `backend/src/handlers/assistant_group.rs`
- `backend/src/handlers/assistant_group_tests.rs`
- `backend/src/handlers/assistant_team.rs`
- `backend/src/handlers/mcp_transport.rs`
- `backend/src/handlers/trigger_scheduler.rs`
- `backend/src/handlers/trigger_scheduler_completion_tests.rs`
- `backend/src/handlers/trigger_scheduler_tests.rs`
- `backend/src/handlers/trigger_webhooks.rs`
- `backend/src/handlers/triggers.rs`
- `backend/src/main.rs`
- `backend/src/models/assistant_conversation.rs`
- `backend/src/models/assistant_group.rs`
- `backend/src/models/trigger.rs`
- `backend/src/models/trigger_run.rs`
- `backend/src/services/agent_operation_scope_tests.rs`
- `backend/src/services/assistant_access_policy_tests.rs`
- `backend/src/services/assistant_acknowledgement_service.rs`
- `backend/src/services/assistant_agent_learning_review_tests.rs`
- `backend/src/services/assistant_authority_tests.rs`
- `backend/src/services/assistant_group_service.rs`
- `backend/src/services/assistant_nyxagent.rs`
- `backend/src/services/assistant_nyxagent_tests.rs`
- `backend/src/services/assistant_team_service.rs`
- `backend/src/services/async_service_operation.rs`
- `backend/src/services/mcp_service.rs`
- `backend/src/services/trigger_schedule.rs`
- `backend/src/services/trigger_schedule/continuation.rs`
- `backend/src/services/trigger_service.rs`
- `backend/src/test_utils.rs`
- `docs/NYXBOT_SCHEDULES.md`
- `docs/plans/2026-10-27-automation-review.md`
- `docs/plans/2026-10-27-automation-status.json`

- `frontend/src/pages/automations.test.tsx`
- `frontend/src/pages/automations.tsx`
- `frontend/src/schemas/automations.ts`


## Astra personal review revision: F-1, F-2 and F-3

Revision base: `2f57fbaf0f3f7387ff8f23ff6027876a67b0b5bd`. The containing
fix commit is reported in the same-session handoff. Astra implementation review
is **IN PROGRESS** and approval remains **PENDING**; these fixes still need
Astra's verification.

- **F-1:** `trigger_scheduler::defer` now caps an exhausted-owner-budget reset by
  `coordinator_deadline` for `wait_kind=Coordinator`. Initial occurrence admission
  retains its original grace deadline. Confirmation retries retain the separate
  expiry/admission path and their existing short backoff. A legacy/malformed
  coordinator with no completion deadline keeps the existing conservative
  occurrence-deadline fallback; the scheduler's exhaustion check treats a missing
  coordinator deadline as terminal timeout.
- **F-1 regression:** extended
  `deferred_runs_wait_for_backoff_or_budget_window` to inspect persisted
  `trigger_work.at` for both hourly-only and daily-only owner exhaustion with an
  expired occurrence deadline. Deadlines on either side of the UTC reset assert
  `at=min(reset, coordinator_deadline)` and that immediate work re-claim fails.
  Synthetic deadlines isolate this retry-bound regression; production completion
  window initialization remains in `new_run`. A confirmation case asserts its
  retry remains in the future despite expired occurrence/coordinator deadlines.
  This Rust regression is drafted and **not executed** under the shared resource
  restriction.
- **F-2:** replaced the status/ledger's user-relay wording with coordinator
  clearance through the shared-window handoff. Already-authorized local tests
  require no additional user approval. The resource restriction still defers
  backend execution.
- **F-3:** the three readiness checks in `trigger_service::create` and `update`
  explicitly borrow `&input.delivery`, `&delivery` and `&next.delivery`, preserving
  their later reuse. Static inspection checked those exact sites; no redundant
  implementation-mirroring unit test was added. Rust type checking remains
  deferred.

The latest instruction for this revision permits **lightweight formatting and
Git diff checks only, with no Cargo commands**. Accordingly, this revision uses
direct `rustfmt`, and does not repeat Cargo, frontend tests/lint/build or any
backend execution. Earlier handoff check results above apply to that earlier
commit; they are not presented as newly rerun checks for this revision.

| Revision command/check | Result | Evidence |
| --- | --- | --- |
| `rustfmt --edition 2024 --check backend/src/handlers/trigger_scheduler.rs backend/src/handlers/trigger_scheduler_tests.rs backend/src/services/trigger_service.rs` | PASS, exit 0 | `/tmp/nyxid-automation-2040-f1-f3-rustfmt.log` |
| `git diff --check` and staged/committed diff checks | PASS, exit 0 | `/tmp/nyxid-automation-2040-f1-f3-diff-check.log`; repeated at commit |
| Static review of three borrowed delivery checks and touched constructor/signature sites | PASS for source placement | All three checks borrow; the affected Trigger constructor has no duplicate/misplaced fields, and no runtime signature changes |
| Cargo / Rust execution / frontend checks | NOT RUN for this revision | Latest instruction: formatting/diff checks only; backend awaits coordinator clearance |

Exact files changed in this revision:

- `backend/src/handlers/trigger_scheduler.rs`
- `backend/src/handlers/trigger_scheduler_tests.rs`
- `backend/src/services/trigger_service.rs`
- `docs/plans/2026-10-27-automation-review.md`
- `docs/plans/2026-10-27-automation-status.json`

Remaining concerns are unchanged backend type/fixture/race verification and open
acceptance rows above. The focused regression should run first when the
coordinator/shared-window handoff permits backend work. No backend process was
started or queued, and no lock, shared target, approval gate or agent topology
was changed.

## Astra personal review revision: F-4, F-5 and F-6

Revision base: `4683f9d76874768300ebb01d4b5c724f661b43c6`. The containing
commit's exact SHA is reported in the same-session handoff. The active plan and
frozen revision-3 bytes retain SHA-256
`5e3469b2754b8ca8127519c19ba1349049288f70371c4d9a2de5a1a761799fc7`.
Astra review remains **IN PROGRESS**, implementation approval **PENDING**.

**F-4 source fix.** `mark_coordinator_waiting` now delegates to one shared
transaction body that fences both dispositions, including no source. Before
checking `source`, it performs actual `$inc` writes on the exact owner's thread
(`automation_handoff_version`) and the exact started run/turn (`handoff_version`),
checks each matched count, and seals that turn's registrations with the internal
`coordinator_handoff_closed` field. Follower registration writes these same
documents, so a handoff committed after the barrier's read snapshot conflicts
with its write. The existing `and_run2` wrapper preserves Mongo transient error
labels through `transaction_result` and retries the entire decision.

The seal remains set when an empty-source barrier commits, closing the gap
before terminal projection. `follow_in_session` rejects sealed, result-claimed,
or terminal-projected runs, rolling back the group message, allocated sequence,
and thread writes with its transaction. A late group-capacity marker follows the
same seal/result predicates. Only the existing exact run/turn/work admission
transaction reopens the seal for a new turn. The barrier's live-run predicate
also excludes terminal-projected runs; a stale or terminal run suppresses further
settlement without creating a wait. No existing runtime signature or client
field changed. Owner/thread/run/turn checks, Stop checks, origin admission,
security fences and Permissions ownership remain in place.

**F-4/F-6 regression.**
`automation_v2_completion_barrier_fences_registration_and_stop` uses the existing
transaction-topology guard and unique managed fixture databases. Its ordering
is controlled by real Mongo sessions and completed writes; it contains no
sleep-based synchronization. These are drafted assertions, **not executed
results**:

| Ordering | Assertion in the focused regression |
| --- | --- |
| Empty-source snapshot, then follower registration commits, then stale barrier writes | An explicit snapshot transaction receives `TransientTransactionError`; a fresh production settlement finds the follower and records coordinator waiting with no terminal result or delivery claim. |
| Empty-source barrier commits, then registration attempts before terminal projection | Registration fails and its message count/follower changes roll back; normal completion subsequently records a terminal result. |
| Stop or terminal cleanup commits, then the barrier rechecks | The earlier read transaction is ended and a fresh session returns `(true, false)` at the live-run predicate. No transient error is required for these committed terminal cases (F-6). Late registration rolls back and late barriers cannot revive the run. |
| Exact continuation admission, then another group registration | Admission reopens the seal and the next turn successfully registers a follower for the same run. |
| Waiting barrier commits, then Stop cleans up, then a captured follower claim and completion arrive | Stop records `failed/cancelled`, removes the follower, queued event and tracked run ID; the old claim cannot requeue work and late completion cannot revive the run. |

Static ordering review: registration and the barrier write the same thread/run
documents. Stop's cancellation and terminal cleanup write those same identities
transactionally. A transaction that retains an older snapshot must conflict when
it attempts these writes; a fresh recheck sees the terminal outcome and returns
without a barrier decision. A no-work barrier that wins first keeps its seal
until terminalization or the next exact admission. The fixture keeps the
registration conflict assertion separate from the already-committed terminal
recheck, as requested in F-6.

**F-5 source fix and regression.** `trigger_service::update` now includes
`continuation_protocol_version` in its existing transactional `$set` under
`exact_document_predicate(&source)`. The active-assistant readiness decision's
version therefore reaches MongoDB and the returned committed generation.
`automation_v2_trigger_update_persists_protocol_and_preserves_other_states`
reloads raw BSON after a real update, asserts the field is present and the
returned/reloaded versions agree, checks the resulting status/delivery/generation,
and rejects a stale-current update. Six cases cover an absent legacy field
upgraded to v2 for active assistant delivery, disabled assistant versions 0/2,
active and disabled notification version 0, and retention of an existing v2
version when changing to notification delivery. This regression is also
**unexecuted**.

### Lightweight validation for F-4 through F-6

No Cargo command or backend test/build/link process was run or queued. The
shared backend window remains held by another stream. Direct formatting uses
the repository's Rust 2024 edition; workspace Cargo formatting, focused tests
and clippy remain for coordinator clearance through the shared-window handoff.
No frontend file changed in this revision, so earlier frontend results are not
reported as newly rerun checks.

| Command / inspection | Result | Evidence |
| --- | --- | --- |
| `rustfmt --edition 2024 --check backend/src/services/trigger_schedule/continuation.rs backend/src/services/assistant_group_service.rs backend/src/services/trigger_schedule.rs backend/src/handlers/trigger_scheduler_completion_tests.rs backend/src/services/trigger_service.rs` | PASS, exit 0 | `/tmp/nyxid-automation-2040-f4-f6-rustfmt.log` |
| `git diff --check`; staged and committed equivalents | PASS, exit 0 | `/tmp/nyxid-automation-2040-f4-f6-diff-check.log`; repeated at commit |
| Status JSON parse and `astra_review` / `notify_on_finish` assertions | PASS | `/tmp/nyxid-automation-2040-f4-f6-status-check.log` |
| Static source inspection | No duplicate/misplaced fields found in touched constructors; new `CoordinatorWait` and `UpdateInput` literals match their declarations; existing `ChatAuthority`, `AgentEvent`, `GroupFollower`, and changed append/follower call sites rechecked | Manual inspection of the cumulative automation diff and surrounding constructors; not Rust type checking |
| Transaction/type/lifetime inspection | Session borrows use established reborrow patterns; production retry closure owns cloned database/run; borrowed thread comes from a live local run; raw seal survives field-preserving migration updates; F-5 retains the exact-document predicate | Manual inspection of service/fixture code and Mongo driver/error helpers; no compiler or runtime result claimed |

Remaining validation: execute the two new regressions and the earlier F-1 test
first when the coordinator releases a backend window, then the existing focused
automation/confirmation/event/Stop checks and required clippy. All backend types,
`Send` bounds, transaction retry behavior, fixture assumptions and open acceptance
rows still require executable evidence. No implementation approval is recorded.
`notify_on_finish=true` remains enabled in status and on the completion
notification attempt; delivery outcome is reported in the session handoff.

Exact files changed in this revision:

- `backend/src/handlers/trigger_scheduler_completion_tests.rs`
- `backend/src/services/assistant_group_service.rs`
- `backend/src/services/trigger_schedule.rs`
- `backend/src/services/trigger_schedule/continuation.rs`
- `backend/src/services/trigger_service.rs`
- `docs/plans/2026-10-27-automation-review.md`
- `docs/plans/2026-10-27-automation-status.json`

## Astra personal review revision: F-7

Revision base: `193a6f38f71f45928e1facd99f33c8071f45b054`. The containing
commit's exact SHA is reported in the same-session handoff. Active and frozen
revision-3 plan SHA-256 remains
`5e3469b2754b8ca8127519c19ba1349049288f70371c4d9a2de5a1a761799fc7`.
Astra review remains **IN PROGRESS**, implementation approval **PENDING**.

**Terminal semantics.** The ambiguous legacy-wait branch now persists only its
migration-owned protocol/allowance/classification fields, then calls the normal
`finish_update` in that same session with the exact owner, trigger, run, turn,
waiting outcome and v2 predicate. That path sets the failed outcome, reason,
finish time and duration, claims `terminal_result_recorded`, evaluates the normal
failure streak/pause policy under its existing live trigger-generation fence,
and performs normal v2 terminal cleanup. Missing generations are initialized as
before; an existing stale generation is preserved and cannot mutate the current
generation's failure streak. Trigger lookup and last-run projection also match
the run owner.

`finish_record` now shares a session-aware body with migration for work removal,
ordered `schedule_state.last_run` projection, metadata-only outcome audit and
bounded history trimming. The ordinary completion path keeps its best-effort
audit behavior. Migration passes its existing session, so its audit is required
and its history trim joins the transaction without opening a nested transaction.
The former direct work deletion is removed: only the shared terminal record body
deletes that run's work row. Terminal effects, the v2 stamp, ambiguity metadata
and progress commit or abort together.

The production per-run migration uses the existing `run_db_only` helper with
snapshot/majority concerns. It retries definite Mongo conflicts, owner-fence
creation races and chained-audit head collisions. An unknown commit retries
COMMIT on the same transaction, without replaying side effects. The nonterminal,
non-v2 predicate makes a committed classification ineligible on migration replay;
its audit, failure evaluation and metadata counters therefore commit once. The
marker retains its 1,000-ID recent-ambiguity bound, and each stable ambiguity row
contains only run ID and reason. No shared runtime signature, Permissions code,
profile, lock or target was changed.

**Focused regression assertions, drafted and NOT EXECUTED:**

| Fixture | Assertions added |
| --- | --- |
| `automation_v2_migration_readiness_real_leases_confirmation_and_ambiguity` | Ambiguous run becomes `failed/legacy_wait_unclassified` with terminal flag, finish time and duration; real work row disappears; public history and `last_run` contain the failed terminal row; tracked thread run ID is removed; exactly one metadata-only outcome audit contains the truthful reason. Confirmation IDs and readiness checks remain. Migration replay preserves terminal timestamps and audit/progress counts. |
| `automation_v2_migration_terminalization_is_atomic_and_generation_fenced` | Uses unique managed databases and a real Mongo session to execute the production classification body, inspect its provisional terminal row, and abort. Outside reads then assert that protocol/outcome/terminal flag, work row, projection, history trim, audit and ambiguity progress/metadata all rolled back. Expiring only the fixture lease permits full migration retry, followed by replay and a late ordinary completion attempt. Assertions cover one terminal audit, stable finish time, no revival, no work row, normal failure-policy evaluation for a current generation, preservation of the streak for a stale generation, and trimming 1,001 history rows to the existing 1,000-row bound. No sleep-only mock or model call is used. |

The generation fixture reloads its trigger before constructing its run, so its
generation comparisons use BSON millisecond precision. Static review covered
new `CoordinatorWait`/`AuditActor` literals, moved candidate IDs, optional session
reborrows, boxed future boundaries and both terminal-record call paths; no
duplicate/misplaced fields or nested transaction were found. Existing
`ChatAuthority`, `AgentEvent`, append/follower contracts and F-4 handoff seals
are unchanged by this revision. This inspection is not Rust type checking.

**Lightweight evidence:**

| Command / inspection | Result | Evidence |
| --- | --- | --- |
| `rustfmt --edition 2024 --check backend/src/services/trigger_schedule.rs backend/src/services/trigger_schedule/continuation.rs backend/src/handlers/trigger_scheduler_completion_tests.rs` | PASS, exit 0 | `/tmp/nyxid-automation-2040-f7-rustfmt.log` |
| `git diff --check`; staged and committed equivalents | PASS, exit 0 | `/tmp/nyxid-automation-2040-f7-diff-check.log`; repeated at commit |
| Status JSON parse, active/frozen plan hash, approval-pending and `notify_on_finish=true` assertions, shared config existence | PASS, exit 0 | `/tmp/nyxid-automation-2040-f7-status-check.log` |
| `df -h .` | 23 GiB available, rounded | Read-only observation; no installation, cleanup or build started |
| Cargo formatting, compile, tests and clippy | NOT RUN / NOT QUEUED | Coordinator GLOBAL BACKEND HOLD; Telegram alone receives the next exclusive retry |
| Frontend checks | Not rerun | No frontend files changed in F-7 |

The coordinator reported reclaiming 16.49 GiB from superseded inactive outputs,
with 23.40 GiB free. This stream performed no cleanup. The reserved-command
section above now records the concrete shared resource config for every later
backend invocation. Direct `rustfmt` does not substitute for the required
workspace Cargo formatting/check/test/clippy evidence after coordinator
clearance. Already-authorized local tests need no new user approval.

Remaining verification: execute both migration regressions first after the
shared-window handoff, then the earlier F-1/F-4/F-5 regressions, focused
automation/confirmation/event/Stop checks, workspace formatting and required
clippy. Rust types/`Send` bounds, transaction retries, audit-chain concurrency
and fixture assumptions remain unverified by execution. Open acceptance-matrix
rows and Astra's personal complete-diff review remain outstanding. No
implementation approval is recorded. Completion notification attempts retain
`notify_on_finish=true`; their delivery result is reported in the session handoff.

Exact files changed for F-7:

- `backend/src/handlers/trigger_scheduler_completion_tests.rs`
- `backend/src/services/trigger_schedule.rs`
- `backend/src/services/trigger_schedule/continuation.rs`
- `docs/plans/2026-10-27-automation-review.md`
- `docs/plans/2026-10-27-automation-status.json`

## Astra personal review bookkeeping: Heca runtime identity

The Sol implementer remains the original Heca agent and session
`01a17b03-c22e-7503-ae21-ef4d38050be9` in the required worktree. Heca status
verified provider `codex`, model `gpt-6.1-sol`, `x_high` effort, `full-access`
mode, and provider session `01a17b03-c43f-74f3-b98c-beb00d71c256` at
`2026-10-27T03:58:24Z`. The provider session identifier is recorded separately
from the Heca session identifier; no session replacement was inferred.

## Coordinator Cargo policy correction

The frozen plan SHA is unchanged. For the eventual shared backend window, built-in
Cargo test/build/check commands use `cargo --config <shared-resource-profile>
<subcommand>`. Clippy uses the coordinator-verified argv:
`cargo clippy --config <shared-resource-profile> --manifest-path backend/Cargo.toml
--all-targets --all-features -- -D warnings`. This stream has not started or queued either form;
actual process arguments will be checked in the clippy evidence before approval.


## Astra targeted review and executed validation — F-8 through F-11

This current record supersedes the historical unexecuted/hold snapshots above.
Automation received the exclusive backend window from the coordinator at 06:31 UTC.
The same Sol remains codex / gpt-6.1-sol / x_high / full-access in the assigned
worktree; status was personally reverified at 09:06 UTC. No Opus gate was resumed.

### F-8 — migration safety and executable acceptance

Personally reviewed implementation `2bf59012f80dc82b0bf113977562d9617c278990`
and its surrounding migration, admission, follower, scheduler and transaction code.
The cumulative earlier F-1–F-7 review is carried forward. Concrete corrections:

- Migration cannot seal readiness for a missing, changed, wrong-owner or unsupported
  nonterminal target. It preserves reachable non-assistant pending work instead of
  stamping it as assistant v2 work.
- A follower is attributable only to the exact live coordinator thread; a dangling
  follower cannot make migration infer safe resumable work.
- Protocol comparisons handle Mongo BSON int32 and int64 representations. Terminal
  bookkeeping, audit, work removal, history trimming, generation fences and migration
  progress share the real transaction and are asserted after abort and replay.
- Legacy discovery uses the bounded indexed production query. Read-count/explain
  fixtures inspect actual Mongo commands and assert a nonempty intended workload;
  no local predicate copy substitutes for the production query.
- Real concurrency tests cover competing work/follower claims, last-owner-budget-slot
  contention, independent hourly/daily exhaustion and reset, exact two-run selection,
  completion-registration/Stop conflicts, and ordinary event-hour/streak safeguards.

The strict transaction fixture verifies Mongo topology, logical sessions and an actual
transactional write/abort. The acceptance suite fails if Mongo is unavailable; it cannot
silently return a pass. The fake NyxAgent test drives delayed/out-of-order research,
edit, independent review, local publish and readback through production orchestration.
No external publisher write is attempted or required.

Five successful logs contain 107 executions / 106 unique passed / zero failures or
ignored tests. Source/binary/log binding and all exact test names are in
[`automation-2bf59012-validation.json`](2026-10-27-automation-evidence/automation-2bf59012-validation.json).
The five actual commands were recovered from this Astra session's Heca tool records in
[`commands-2bf59012.json`](2026-10-27-automation-evidence/commands-2bf59012.json), not reconstructed from names.
Historical binary SHA256: `a692860b8edbbd09c48a71f1199e56e0751ec0ea1d3300f1579aa64e39844a3a`.
Source manifest SHA256: `48dc9c951134f23c27050951f760184c25dbc3b039f5042a3c2b0a12632efdec`.
This binary was subsequently replaced by the required correction rebuild; old passes
remain bound to the old identity.

Measured S7 results: 10,000 schedules/one due yields a covered query with zero documents
and one key examined; discovery 0.220 ms/query, 100-schedule list 7.815 ms/request,
fenced CAS 9.471 ms/claim, idle tick 0.406 ms, confirmation-to-completed 228.004 ms,
scheduled-to-started 296 ms, and three-day catch-up 49.893 ms. These are local fixture
measurements. They do not assert production throughput or network latency.

### F-9 — dead helper and follower deadline proof

Required all-target/all-feature Clippy found the unused `followers_live_at` helper.
Deleting it is a narrow production cleanup. Moving its filter into a copied test closure
would prove only that closure, so Astra rejected that attempted replacement. The prior
`automation_v2_follower_lost_claim_queue_full_expiry_and_cross_run_ids` test covers
notification-lease expiry; it does not establish a follower deadline expiring after claim.

The corrected regression obtains a real live quiet claim, persists/reloads its follower
with an expired deadline while keeping the live notification fence/lease, calls production
`deliver_follower`, and requires false acceptance, unchanged event queue/no event ID,
and removal of the expired claimed follower. Existing raw legacy expiry cases remain.
The actual contract freezes upper sequence/event identity at claim and rechecks follower
expiry at delivery. It does not freeze expiry classification at an acknowledged take.
No production expiry defect was demonstrated. The rebuilt group suite passed all 13 cases, including this regression; final all-target/all-feature Clippy passed.

### F-10 — related normative documentation

The engine and orchestrator docs now explain Event continuation claims, selective run
selection, durable follower delivery, protected queue entries, owner hourly/daily and
immutable run allowance limits, deadline outcomes, confirmation semantics and the real
operator-drain/migration-marker gate. They link the existing schedule runbook. Astra also
returned the Rule14 sentence that said all continuations were free: only confirmation
resumes avoid another occurrence/coordinator debit. The complete committed doc diff was personally reviewed and matches these contracts.

### F-11 — test filter omitted required scheduler evidence

Personal name-by-name reconciliation found 16 tests in `trigger_scheduler_tests.rs`
absent from the 106-pass record. They include the changed F-1 budget deferral test,
confirmation/skill wake and terminal failure/generation/finalizer cases. The same Sol
ran only the missing cases from the correction binary: 16 passed, child exit 0, with
exact names/count verified independently. Existing successful suites were not broadly rerun.
This is an execution coverage gap, not a newly alleged production defect.

### Preserved failures and their causes

- Initial focused compile: five malformed `json!` calls; corrected before retry.
- Retry1: two missing `bson::doc` imports in `assistant_team`; corrected before retry.
- Retry2: 18 passed / 3 failed; ordinary wake timing, fake research completion ordering,
  and BSON integer representation assumptions were corrected with production semantics
  preserved. Retry3 passed all 21.
- Initial schedule batch: 17 passed / 16 failed with missing v2 activation environment;
  the single diagnostic rerun failed for the same reason. Explicit-env retry passed 33.
- Initial all-feature Clippy: unused follower helper; F-9 correction and final rerun passed.
- First helper-only rebuild: compilation succeeded, then all 13 group cases failed
  because the wrapper omitted `NYXID_TEST_DATABASE_URL` (exit 101). The strict harness
  refused missing Mongo. The next wrapper explicitly sets Mongo, v2, serial test count,
  FD limit, shared target/jobs/config, and an owned-process 2 GiB disk guard.
- An intermediate formatter check found only formatting in the newly edited expiry
  assertions. Formatting is required again after the completed correction.

Failed logs are retained beside successful logs; no failed attempt is counted as a pass.
No exit status absent from the original record is invented.

### Shared-contract personal review

Read-only comparison used Artifacts source `47a2f3f1845b1896e172e68ec077bb8270ee2e33`
and Permissions source `493b5c2dc012cc4de80c00f33f7cddf489cb316d`; no sibling feature
was imported. Preserve Artifacts' `bind_in_session(db, keys, row, session)`, original
Web/User provenance, exact non-trigger completion identity, encrypted result association
and Stop cancellation alongside Automation's exact trigger identity/selective drain.
Permissions' bounded seven-scalar `client_metadata` classifier and independent guest
HTTP callback guard remain outside Automation ownership. No proxy-marker bypass exists.
F-7 itself did not touch artifact-shared files; cumulative Automation changes do.

I-01/I-02 require mixed event queues to preserve and later consume only the original
Web watch. I-03 requires both exact run cleanup and conversation Stop transactions.
I-04 must refuse trigger/channel/voice/unrelated materialization and recovery. I-06
must retain owner callback behavior, no guest egress/cards and exact cancelled denial
identity. These are OPEN executable combined gates, not established by source review.
I-05/I-07 belong to the coordinator/other workstreams and are not #2040 blockers.


## Final local review record — 2026-10-27 09:25:14 UTC

- **SOURCE_REVIEW_COMPLETE** for implementation
  `6df83d87b826141ec025ce2276f66cb147cbfe33`.
- **LOCAL_VALIDATION_COMPLETE**: 136 successful backend executions / 134 unique passed /
  zero failures or ignored in the successful runs; required all-target/all-feature
  warnings-denied Clippy, final formatting and whitespace checks passed. The unchanged
  frontend retains its 17-test, lint and build passes.
- **#2040 BLOCKED** only on combined I-01/I-02/I-03/I-04/I-06. No
  `ASTRA_IMPLEMENTATION_APPROVED` is granted. No live X publication gate exists.

Astra personally read the entire final five-file commit after reviewing its working
source, verified all 42 source-manifest entries against exact committed/current bytes,
and inspected the raw final logs, runner exit/guard records and actual compiler/Cargo
arguments. Final source manifest SHA256:
`25a9c5f4151a28dc49b7de051f93b39b415177debb01dbab8c0bacdeb0f4c1a0`.
Final binary SHA256:
`5c9a6b0c9c549009ffc700f08b329fc747f54ee495ac557d14408bc216a90057`.
Final Clippy log SHA256:
`7947200a011ef009cff4ca56235d674491d5fa6389bc9721627ab21fd6deaf05`.
The [final evidence manifest](2026-10-27-automation-evidence/final-verification.json)
contains source/base/plan/prerequisite identities, every successful test name and duplicate,
source/binary/log bindings and remaining combined gates.

The historical 106 unique passes at 2bf59012 are retained for unchanged behavior. The
correction rebuild passed 13 group tests and the unchanged binary passed the 16 omitted
scheduler cases. Two overlapping executions account for 136 executions / 134 names.
The scheduler test child exited 0; its original wrapper returned 126 only because a
nocapture timing line split one result. Raw metadata remains unchanged; the separate
verification record and Astra's independent parse establish all 16 exact passes without
rerunning them or presenting the wrapper as exit 0.

Final Clippy exited 0 in 6m19s with config after the subcommand, all targets/features and
warnings denied. Actual backend cfg includes AWS/GCP features, with no independent
profile/optimization/assertion change. The 2 GiB owned-process guard did not fire;
its minimum free space was 15.18 GiB. The guard returned only after owned PGID 40304
was empty; the coordinator independently verified global heavy-process quiescence and
17.21 GiB free and handed the window to Artifacts. No heavy work is queued here.

All identified local findings F-1–F-11 are resolved. The next dependency is the
coordinator-owned isolated integration commit containing all individually reviewed
sources; Automation must inspect the resolved shared changes and actual results for
its five applicable rows. No sibling feature source was imported into this worktree.
