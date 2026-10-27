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
| Implementation | GPT-6.1-Sol | Exact Astra-approved revision 3 plan | **SOURCE DRAFT; BACKEND VERIFICATION DEFERRED** | Verified Heca session `01a17b03-c22c-7c50-a038-27bf595a74fd`, model `gpt-6.1-sol`, exact worktree/branch; no agents spawned. |
| Astra implementation review | GPT-6-Astra PM | Exact final implementation commit | **IN PROGRESS; APPROVAL PENDING** | Astra is personally reviewing the draft and has returned F-1/F-2/F-3 to this same Sol session. Astra will read the complete diff and relevant surrounding code, run the acceptance matrix, and resolve every valid finding including nits through the same Sol session. |
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
- Keep any missing live client/destination/production evidence explicitly OPEN; it must not
  be replaced by a caveat or treated as completed.

## Sol implementation evidence

- Verified active plan and frozen revision 3 SHA-256 `5e3469b2754b8ca8127519c19ba1349049288f70371c4d9a2de5a1a761799fc7` before source edits. The plan header's historical “revision: 2” is preserved because the exact approved bytes are authoritative.
- Pre-edit shared runtime notification attempted via Heca `heca_agent_send_prompt` with `notify_on_finish=true` to coordinator `01a17a8a-e916-77a0-a565-083ca25e2aa3` and Astra PM `01a17a9b-50b4-7e41-bb3b-dcbbf6204ad3`. Both rejected by Heca caller ACL: “a caller may act only on itself, agents it spawned, or (as an orchestrator or team lead) the fleet / its team.” Notification remains undelivered; no approval inferred and no additional agent created. Contracts are documented above and in session commentary.
- Implementation approval remains **NOT STARTED**, reserved for Astra's personal complete-diff review.

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
| End-to-end progress | `automation_v2_fake_agent_research_edit_review_fixture_publish_readback` drives real native group posting through a fake model, delayed/out-of-order researchers, editor, independent reviewer, local publish/readback | Compile/run full workflow; live client/destination evidence remains OPEN |
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
tests are already authorized and require no new user approval. Use normal Cargo
locking, the shared target and two jobs. Tests invoking assistant
admission require `AUTOMATION_CONTINUATION_V2_ENABLED=true`; test database setup
seeds a fresh completed marker, while the migration fixture removes/rebuilds its
own marker and injects flag-off checks without changing process environment.

```sh
CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 AUTOMATION_CONTINUATION_V2_ENABLED=true cargo test -p nyxid automation_v2_ -- --nocapture
CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 AUTOMATION_CONTINUATION_V2_ENABLED=true cargo test -p nyxid trigger_scheduler::tests -- --nocapture
CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 AUTOMATION_CONTINUATION_V2_ENABLED=true cargo test -p nyxid assistant_group::tests -- --nocapture
CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 AUTOMATION_CONTINUATION_V2_ENABLED=true cargo test -p nyxid async_service_operation -- --nocapture
CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
```

After the first focused results, run the remaining applicable ordinary-event,
channel, authority and lifecycle regressions, and re-run only affected checks
for fixes. Astra's `ASTRA_IMPLEMENTATION_APPROVED` gate is **NOT STARTED** and
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
