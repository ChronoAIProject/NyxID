# Automation completion repair (#2040)

Plan revision: 2  
Workstream: automation  
Branch: `fix/automation-completion-2040`  
Base: `b1dd6548ade04ec4f0051203e1353796331c7f87`  
Issue: https://github.com/ChronoAIProject/NyxID/issues/2040  
Owner: ctkm-aelf  
PM: GPT-6-Astra (Heca workspace `01a17a96-a01b-7f10-a98b-646d494dc1d5`)

This is the implementation plan for the concrete scheduled NyxBot group-workflow
failure. It is intentionally narrower than the broader collaboration proposal in
#1975 and does not introduce an unbounded workflow engine.

## Evidence and current root cause

The issue body and its 2026-10-26 comment document two consecutive scheduled X
workflow runs. In both runs the trigger run was recorded `completed` after roughly
64–68 seconds, although the coordinator's editor/group work had left two or three
updates waiting, the independent reviewer and publisher were never started, and
`create_tweet` was never called. Read-only X calls succeeded, so the failure is in
NyxID orchestration before any X write.

The current source confirms the race:

* `backend/src/handlers/trigger_scheduler.rs::settle_result` treats the first
  settled assistant reply as the trigger result and calls `terminal_result` and
  `finish`, without checking for attributable group followers, queued trigger
  events, or pending specialist work.
* `post_to_group` uses `assistant_group::post_and_follow`; it atomically registers a
  `GroupFollower` for the coordinator conversation, starts addressed members, and
  lets `notify_followers` later push a `group_settled` event. The follower currently
  has no trigger-run identity.
* `assistant_group::notify_followers` takes/removes the follower before pushing the
  event. A crash or failed queue write in that gap loses the handoff.
* `assistant_team::wake` starts an ordinary `origin: event` turn from all queued
  events. There is no trigger-run claim on that turn, so
  `after_turn` cannot route its reply to `trigger_scheduler::settled`. The event
  loop guard and event rate window also apply to a scheduled continuation even
  though scheduled automation is supposed to have its own budget.
* `trigger_schedule::admit_turn` already has the transactional run/work/trigger
  fences needed to bind a continuation, but it currently only admits
  `origin: trigger` turns and the wake path never supplies a continuation claim.

This combination makes a coordinator reply look terminal while the durable group
handoff is still live, and it leaves the queued result without an owner that can
resume the trigger run.

## Final behavior

An automation run is terminal only after its current coordinator turn settles and
there is no attributable coordinator work remaining. Attributable work includes a
group follower or queued event bound to that exact trigger-run ID. A coordinator
may therefore go through this bounded sequence repeatedly:

```text
pending --admit--> started --reply with attributable work--> waiting(coordinator)
waiting(coordinator) --bound event wake--> started (new turn)
started --final reply, no work--> completed/failed
```

Confirmation waits keep their existing `waiting(confirmation_required)` path and
acknowledgement semantics. A coordinator wait has its own bounded deadline and
truthful terminal reason (`coordinator_timeout`, `thread_deleted`, or the existing
failure code). It never becomes `completed` merely because the prior turn ended.

The exact trigger run is carried end to end:

1. A trigger coordinator turn exposes its run ID through `ChatAuthority`.
2. `post_to_group` stores that run ID in the follower created with the group post.
3. Quiet-group notification creates a deterministic, deduplicated `AgentEvent` that
   carries the same run ID and remains retryable until the event is durably queued.
4. `wake` claims the waiting run's work lease and starts an event turn with a
   `TurnClaim { continuation: true }`. The event turn drains only events for its
   run; unrelated events remain queued for their normal owner turn.
5. The existing trigger settlement hook sees the new turn ID and applies the same
   completion barrier again. A later group post can put the run back into the same
   coordinator-wait state.

No prompt, event payload, instruction text, specialist response, credential, or
external provider body is written to run metadata, audit, or logs. Run history
continues to contain IDs, timing, outcome and bounded reason codes only.

## Concrete data and interface changes

### Trigger run state

* Add a serde-defaulted `CoordinatorWait`/`coordinator_wait` field to
  `models::trigger_run::TriggerRun` containing the coordinator thread ID, the
  attributable source kind (`group` or `event`), and a bounded
  `deadline_at`. Keep `waiting_for` exclusively for acknowledgement IDs.
* Add a serde-defaulted `RunWaitKind` (confirmation versus coordinator work) or an
  equivalent explicit discriminator. Existing rows with no field retain current
  confirmation behavior; old trigger rows remain readable.
* Add serde-defaulted `continuation_count`, `continuation_limit`, and
  `continuation_protocol_version` fields. Version `0` and absent fields are legacy;
  version `2` is written only after the rollout activation/drain fence is enabled.
* Initialize the coordinator deadline in `trigger_schedule::new_run` to one
  bounded completion window after the dispatch/grace deadline (one hour, capped by
  the run's existing 90-day retention). This is a completion bound, not a second
  schedule occurrence deadline. A waiting run is failed with
  `coordinator_timeout` when it reaches that bound; it is never silently skipped.
* Snapshot a bounded continuation allowance into each run. The run stores
  `continuation_count` (default `0`) and `continuation_limit`, derived from the
  owner's `max_auto_continuations` setting and clamped to the existing hard maximum
  of 32. A successful trigger-bound continuation claim increments the count in the
  same fenced update that changes `waiting -> started`; a claim at the limit
  records `continuation_limit_exceeded` and never starts a model turn. This is the
  spend/turn bound for scheduled continuations when ordinary event-hour and streak
  guards are not charged. The bound is immutable for the run and survives restart.
* Add `mark_coordinator_waiting`, fenced by run ID, current turn ID and outcome
  `started`, to atomically set `outcome=waiting`, the discriminator, reason
  `coordinator_pending`, clear acknowledgement IDs, and make the run work item
  immediately eligible. Emit a metadata-only `trigger_run_waiting` audit record.
* Extend `run_job_once` with an explicit coordinator-wait branch. It checks the
  exact run's thread for a queued event carrying that run ID and invokes the
  trigger-aware wake path when one exists; otherwise it keeps the work item alive
  with a short bounded backoff until the coordinator deadline or records the
  timeout/continuation-limit outcome. It must not synthesize an acknowledgement
  continuation or start an unbound/duplicate turn. Existing confirmation waits
  remain unchanged.

### Durable run identity on asynchronous group work

* Extend `models::assistant_group::GroupFollower` with optional
  `trigger_run_id` and a durable notification state/sequence marker. Missing fields
  mean a legacy non-automation follower. A follower identity includes
  `(conversation_id, trigger_run_id, since_seq)`; a newer follower may replace an
  older one only for the same trigger identity. Distinct runs that share a
  conversation remain separate until each handoff is settled.
* Extend `models::assistant_conversation::AgentEvent` with optional
  `trigger_run_id` and `continuation_protocol_version`, plus a
  constructor/helper that preserves the existing bound report semantics. Old
  events deserialize unchanged; a run ID is written only for a version-2 run.
* Add `trigger_run_id` to `assistant_acknowledgement_service::ChatAuthority`,
  populated from the live `ActiveTurn`, and pass it through `post_to_group` to
  `post_and_follow`/`append_following`. This is a server-derived value; it is never
  accepted from tool arguments or client JSON.
* Preserve the exact run ID when a newer follower for the same conversation replaces
  an older follower. A follower for a later run cannot replace a different run's
  settled handoff without a newer group post.

### Crash-safe follower notification

* Change group quiet handling from “remove follower then send event” to a small
  durable claim state: a quiet follower is marked with the upper group sequence and
  deterministic event ID but remains retryable until the event is accepted into the
  conversation queue. The claim has a short lease/fence so concurrent sweeps can
  work on different followers while a crashed claimant is retryable. The existing
  follower expiry remains authoritative.
* Use an event ID derived from the complete handoff identity `(group ID,
  coordinator conversation ID, trigger run ID or a legacy marker, follower
  since-sequence, and upper group sequence)` (UUID-v5 or an equivalent stable
  digest). Replaying the sweep or a replica after a lost response therefore hits
  `append_events_in_session`'s existing event-identity deduplication without
  allowing two runs to collide.
* Queue the event and finalize/remove the claimed follower transactionally where the
  existing Mongo transaction helpers permit it. If the queue write is unavailable,
  leave the claimed follower for the 15-second group sweep. If the conversation was
  deleted, remove the stale follower and record no successful run result.
* Trigger-bound followers are never evicted by the `MAX_FOLLOWERS` slice or by a
  generic follower replacement. If the bounded follower capacity is exhausted,
  group posting returns a retryable `group_follower_capacity` result and the run
  remains waiting; it cannot be converted to a successful terminal result.
  Multiple trigger and legacy followers are claimed independently when a group
  becomes quiet, with one upper-sequence snapshot per follower.
* Keep generic non-trigger group followers behavior-compatible: they receive the
  same `group_settled` text and retry semantics, but no trigger continuation claim.

### Trigger-aware event wake

* Add a trigger continuation claim helper in `services::trigger_schedule` that
  atomically claims the run's `WORK` row only when the run is
  `waiting(coordinator)`, the work is due, and the owner/thread identity matches.
  It returns the current trigger `updated_at` and a fresh work fence for
  `TurnClaim`.
* In `assistant_team::wake`, inspect queued events for their optional trigger IDs.
  For a trigger event, claim the matching run and construct an event `TurnStart`
  with `trigger: Some(TurnClaim { continuation: true })`. The claim increments
  `continuation_count` only when the exact waiting run, thread, work fence and
  deadline all match. If the claim is busy, expired, terminal, stopped, or at its
  continuation limit, leave the event queued for the scheduler/sweep (or drop it
  only under the terminal cleanup rule); never start it as an unbound owner event.
* Add selective event draining for trigger continuations. A trigger continuation
  consumes only events with its exact run ID; unrelated events remain in
  `pending_events` and are processed by ordinary wake logic. Existing invocation,
  channel-question, queue-capacity and identity dedup rules remain in force. A
  trigger-bound event is non-evictable: queue capacity returns a retryable error
  and leaves the follower claim intact rather than evicting it as an ordinary
  notice.
* Permit a trigger claim on an `origin: event` turn only for the exact continuation
  path. Such turns set `ActiveTurn.trigger_run_id`, skip the generic
  `EVENT_TURNS_PER_HOUR` admission and `MAX_EVENT_STREAK` increment, and retain
  the existing owner pool, trigger overlap, trigger-generation and lease fences
  plus the persisted per-run continuation allowance above. Normal unbound event
  turns retain both generic guards. A continuation that cannot claim its allowance
  is durably failed with `continuation_limit_exceeded` and its event is not
  executed.
* `after_turn` then reaches `trigger_scheduler::settled` through the existing
  trigger-run hook. A coordinator event turn does not deliver a terminal automation
  result until the completion barrier passes.

### Compatibility and shared contracts

* All new model fields are `serde(default)`/optional. Existing webhook-only rows,
  ordinary group followers, direct user turns, service watches and channel routing
  keep their current wire/storage behavior.
* New trigger events are additive, but an old binary cannot safely consume them
  because serde ignores unknown fields and the old wake path would treat one as an
  ordinary event. This repository has no authoritative replica membership or
  worker-version registry, so `AUTOMATION_CONTINUATION_V2_ENABLED=true` is an
  explicit operator drain assertion, not an automatic attestation. The activation
  runbook requires: deploy the new API revision everywhere; pause schedule and
  webhook automation admission; stop **all** scheduler workers (old and new); wait at least two
  `LEASE_SECONDS` intervals; verify the `trigger_work` collection has no lease with
  `at > now`; run the idempotent `trigger_schedule::migrate_continuation_v2`
  migration below; start only the upgraded workers; then enable the flag. The flag
  remains false by default, and trigger creation/update refuses continuation-capable
  automation while it is off. Rolling back requires disabling the flag, pausing
  dispatch, and draining all v2 work before old binaries return. The executable
  readiness harness tests these real lease and migration checks plus the flag-off
  refusal; it does not mock replica membership or claim to detect an old process
  automatically.
* `migrate_continuation_v2` records a durable
  `trigger_migrations/continuation-v2` marker only after every nonterminal active
  assistant trigger/run has been classified and updated. Scheduler admission
  fails closed when the flag is true but this marker is absent or incomplete, so a
  partial migration cannot silently dispatch v2 work. The migration is idempotent,
  processes bounded batches, and keeps its counts and ambiguous-row IDs in the
  marker for the readiness harness and operator rollback review.
* Activation runs a fenced migration for existing active assistant triggers
  (scheduled and webhook assistant deliveries) and their nonterminal runs before
  new schedule dispatch or webhook admission resumes. The operator drain pauses
  both ingress paths. It stamps
  `continuation_protocol_version=2`, snapshots each run's continuation limit, and
  initializes missing count/wait fields while preserving pending confirmation
  cards. A nonterminal row with `waiting_for` is explicitly classified as a
  confirmation wait and keeps its acknowledgement continuation; a waiting row
  with no acknowledgement IDs is classified as coordinator work only when its
  exact thread has a matching follower/event, otherwise it is durably failed with
  `legacy_wait_unclassified` for operator review. New occurrences from previously
  created triggers therefore use the v2 completion barrier; already-terminal legacy
  rows remain historical and are never rewritten as successful. Rows with version 0
  and absent fields remain readable for rollback/legacy history, but no v2 event is
  emitted until the migration and operator drain have completed.
* The shared assistant runtime contract changes in `AgentEvent`,
  `GroupFollower`, `ChatAuthority`, `TurnStart` admission and event selection. I
  will notify the coordinator before implementation with these additive fields and
  the trigger-continuation semantics. No permissions or artifacts implementation is
  imported.
* Trigger run history APIs/UI expose the existing outcome plus the bounded reason
  (`coordinator_pending`, `coordinator_timeout`, or terminal reason) and the thread
  link. They do not expose prompts, payloads or internal fences. A waiting run is
  visibly “Waiting for coordinator work” with its deadline; it is not displayed as
  completed.

## Failure, retry, and recovery rules

* **Duplicate wake/event:** deterministic event identity, the run work fence, and
  `append_events_in_session` dedup make retries idempotent. Only one continuation can
  move `waiting -> started`, and that transition consumes exactly one persisted
  continuation allowance.
* **Out-of-order specialist/group replies:** group sequence bounds the summary;
  follower `since_seq` and the quiet snapshot exclude earlier/later messages. The
  trigger event remains tied to the exact run, so a later schedule cannot consume it.
* **Busy thread/pool:** `wake` leaves the event and run waiting; the existing queued
  sweep and owner pool retry it. No event budget is charged until an actual
  continuation admission succeeds.
* **Loop/rate guards:** scheduled/webhook continuations use the run's budget and
  completion deadline; ordinary event guards remain unchanged. A run that reaches
  its completion bound fails with `coordinator_timeout` and the run history records
  the reason.
* **Crash/restart:** a claimed work lease expires; the scheduler or group sweep
  reclaims it. A follower claimed before a process crash remains in retry state. A
  committed event is deduplicated on replay. A committed trigger turn is fenced by
  exact run/turn IDs and is never started twice.
* **Cancellation/deletion:** deleting/stopping the coordinator thread or trigger
  causes the existing fenced settlement to record `thread_deleted` or
  `cancelled`; pending follower/event state is dropped only after the run is
  terminal and never converted to success.
* **Settlement races:** tests and update predicates cover a group completion that
  arrives before the waiting transition, while the coordinator turn is still
  active, after the run has become terminal or stale, and after a user stop. Each
  case converges through the run/turn/work fence: the handoff is either attached to
  the waiting run exactly once or is dropped as terminal cleanup, never consumed by
  a later run.
* **Disabled target/membership/credential changes:** existing scheduler checks and
  trigger-generation fences remain authoritative. A target/owner that becomes
  unavailable pauses/skips or fails with the existing truthful reason; a queued
  continuation cannot bypass a live grant or credential check.
* **Next occurrence/overlap:** a coordinator-waiting run remains an active run for
  the trigger's existing overlap policy. `skip` and one-successor `queue` behavior
  therefore remain intact; a new occurrence cannot consume the old run's event.
* **Follower capacity:** a full follower array defers the exact trigger run with a
  bounded retry and preserves every trigger-bound follower; capacity pressure never
  evicts another run's handoff or marks a run completed.

## Implementation order

1. Add the plan/status/review records and coordinator notification to the Heca
   coordinator before source edits.
2. Implement model fields and serialization/unit tests first (`TriggerRun`,
   `AgentEvent`, `GroupFollower`, `ChatAuthority` derivation).
3. Implement crash-safe follower notification and deterministic event IDs, retaining
   legacy follower behavior; add group-service tests for retry, duplicate and
   expiry cases.
4. Implement trigger wait state, completion-deadline handling and scheduler
   recovery/timeout branches, persisted continuation allowance, the fenced
   existing-run migration and operator readiness/drain checks; add scheduler
   unit/integration tests for the state machine, overlap/budget behavior and
   flag-off/migration readiness harness.
5. Implement trigger-aware wake, selective event draining and event-turn admission;
   add tests for delayed, mixed, duplicate and out-of-order events plus pool-full
   retry.
6. Add the end-to-end fake NyxAgent scenario: scheduled coordinator posts to a
   group, two or more members finish out of order, the coordinator is resumed,
   posts the next stage, then finishes only after the final result. Assert no
   premature `completed`, no duplicate continuation, and no external publisher
   write; use a local fixture for any publisher/readback.
7. Update `docs/NYXBOT_SCHEDULES.md`, `docs/chat/08-nyxagent-engine.md`,
   `docs/chat/09-nyxbot-orchestrator.md`, and the relevant CLAUDE/AGENTS trigger
   sections with the durable completion and rollout contract.
8. Run focused tests, then the required Rust format/clippy/test checks using the
   shared target (`CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target`,
   `CARGO_BUILD_JOBS=2`), and frontend checks only if a UI contract is touched.

## Acceptance matrix and executable evidence

The implementation is accepted only when every row has a command/result recorded in
the review ledger:

| Area | Evidence required |
| --- | --- |
| Reproduction | A test leaves a scheduled coordinator reply with a follower/queued event and proves the run stays `waiting`, with no terminal projection or delivery. |
| End-to-end progress | A fake NyxAgent drives research → edit → independent review → fixture publish/readback, with delayed and out-of-order specialist completions; the run becomes terminal only after the final coordinator result. |
| Binding | A second run/thread and unrelated queued event cannot consume the first run's event; exact trigger IDs and thread IDs are asserted. |
| Crash/retry | Follower claim before process loss, failed queue write, lease expiry, duplicate event and replica restart all converge to one continuation. |
| Guards | Pool-full, busy thread, event/hour cap, `MAX_EVENT_STREAK`, persisted per-run continuation limit, trigger budget, overlap skip/queue, follower capacity, confirmation continuation and completion timeout retain truthful outcomes. |
| Lifecycle | Cancellation, deadline, trigger pause/edit/delete, disabled assistant/agent, owner inactivity, membership/grant revocation and stale worker fences are covered. |
| Compatibility | Legacy trigger/group/event documents deserialize; the operator drain/readiness harness verifies all-worker lease expiry before upgraded workers restart, v2 migration of existing active schedule/webhook assistant runs (including confirmation and ambiguous waits), rollback drain, and flag-off refusal; ordinary event turns, channel routing, async service watches and non-assistant deliveries remain green. |
| Observability/privacy | Run history/UI reason/deadline and metadata-only audit are asserted; no prompt, payload, secret, fence or upstream body appears in logs/audit. |
| Performance | Explain/index checks cover the new run/group predicates; focused measurements show no extra reads for owners without automation and bounded wake/retry work for owners with it. |
| Required checks | `cargo fmt --check`; focused `cargo test`; `cargo clippy --all-targets --all-features -- -D warnings`; relevant frontend lint/test/build when touched. |

## Rollout and rollback

Deploy all backend replicas and scheduler workers containing the additive model and
event readers before creating new trigger-bound group automations. Monitor counts of
`coordinator_pending`, `coordinator_timeout`, stale follower retries, trigger wake
conflicts, and terminal outcomes. Rollback is safe only after draining new
trigger-bound runs or leaving the feature disabled; legacy triggers and ordinary
group events remain readable. Never roll back to a binary that can mark a run
terminal while it ignores the new binding fields.

## Known risks to resolve during review

* MongoDB transaction boundaries span the group follower claim and assistant event
  append; the implementation must avoid a lost handoff or a second event after an
  uncertain commit.
* Selective event draining must preserve invocation-event fences and channel
  question ordering; a broad change to `select_events` would be unsafe.
* A coordinator can post multiple group stages. The barrier must re-enter waiting
  after each stage without resetting the trigger run budget or occurrence identity.
* A run may be waiting when the user sends a direct message or stops the thread.
  User ownership and Stop must win without accidentally allowing a late event to
  resume a cancelled run.
* Old replicas and mixed rows must fail closed for new trigger-bound events while
  still serving legacy schedules; this needs a concrete deserialization/rollout
  test, not a comment alone.
* UI text and history must explain waiting/blocked states without exposing internal
  event or run payloads.

