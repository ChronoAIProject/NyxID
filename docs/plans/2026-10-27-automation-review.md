# Automation review ledger (#2040)

Workstream: `automation`  
Plan: [`2026-10-27-automation-implementation.md`](2026-10-27-automation-implementation.md)  
Plan revision: 2  
Plan SHA-256: `0633146fbe464aa94fd6a3a98ba7c0a2b4ef0ce00da147bc5bc2741cbbfb96d4`  
Frozen snapshot: [`2026-10-27-automation-implementation.freeze-0633146f.md`](2026-10-27-automation-implementation.freeze-0633146f.md) (same SHA; read-only)  
Base commit: `b1dd6548ade04ec4f0051203e1353796331c7f87`  
Branch: `fix/automation-completion-2040`

## Gate ledger

| Gate | Agent | Exact input | Result | Evidence |
| --- | --- | --- | --- | --- |
| Plan review | Claude Opus 5.5 (`01a17aa2-d74e-7151-9efa-b40638c3befb`, provider `claude`, model `claude-opus-5-5`, effort `xhigh`, mode `default`) | Frozen plan revision 2, SHA `0633146fbe464aa94fd6a3a98ba7c0a2b4ef0ce00da147cb5bc2741cbbfb96d4` | **OPEN** | Two read-only prompts for prior plan bytes were sent with `notify_on_finish=true`; both runs failed before review with provider errors `Failed to authenticate: OAuth session expired and could not be refreshed` and `401 OAuth access token has expired. Re-authenticate to continue.` Heca activity sequences 1–10. The coordinator verified the current file SHA and shared status; this immutable snapshot is now the exact Opus input. No `PLAN_APPROVED` or `PLAN_CHANGES_REQUIRED` marker exists. |
| Implementation | GPT-6.1-Sol | Exact approved plan | **NOT STARTED** | Creation is forbidden until Opus returns `PLAN_APPROVED` for the exact plan SHA. |
| Astra code review | GPT-6-Astra PM | Final implementation commit | **NOT STARTED** | No implementation exists. |
| Final implementation review | Same Claude Opus 5.5 session | Exact final commit | **NOT STARTED** | Depends on implementation and Astra zero-finding review. |

The plan gate is intentionally still open. The authentication failure is an external provider dependency; it is not treated as approval, and no model substitution is permitted. The same persistent Opus agent will be retried after authentication recovers.

## Findings

No substantive Opus findings have been returned because the reviewer has not authenticated. The provider failure itself remains an open gate dependency.

### Coordinator pre-review findings resolved in plan revision 2

| Finding | Resolution in revision 2 | Acceptance evidence required |
| --- | --- | --- |
| Trigger continuations bypassed the existing hourly/daily run budget and generic event guards without a persisted spend bound. | Each run snapshots `continuation_limit` from `max_auto_continuations`; a fenced claim increments `continuation_count`, and the limit produces `continuation_limit_exceeded` without starting a turn. | Scheduler/continuation tests cover restart, duplicate claims, limit exhaustion, and truthful terminal reason. |
| Optional fields alone do not make old binaries refuse v2 events. | The repo has no authoritative replica/version registry, so the plan now makes `AUTOMATION_CONTINUATION_V2_ENABLED` an explicit operator drain assertion. The runbook pauses dispatch, stops all scheduler workers, waits two lease intervals, verifies no `trigger_work` leases, runs fenced `trigger_schedule::migrate_continuation_v2`, starts only upgraded workers, then enables v2; the durable `trigger_migrations/continuation-v2` marker gates scheduler admission; rollback drains v2 work first. Follower claims also carry a lease/fence and trigger followers are never evicted by capacity slices. | The readiness harness exercises the real all-worker lease-expiry, migration-marker, follower-claim/capacity, and flag-off checks. It does not claim automatic old-process detection. |
| Deterministic event IDs could collide across runs. | Event identity now includes group, conversation, trigger run (or legacy marker), follower `since_seq`, and upper group sequence. | Cross-run duplicate/out-of-order tests assert distinct IDs and one event per exact run. |
| Completion could race with handoff creation, active turns, terminal/stale runs, or user stop. | Added exact run/turn/work predicates and explicit race cases; terminal cleanup drops stale handoffs and never transfers them to a later run. | Integration tests exercise each interleaving and assert one of exact wait or terminal cleanup. |

## Shared runtime contract proposed for coordinator review

The plan proposes additive, server-derived continuation fields and semantics:

- `TriggerRun`: a serde-defaulted coordinator-work wait discriminator, source, and bounded deadline.
- `TriggerRun`: persisted `continuation_count`, immutable `continuation_limit`, and
  `continuation_protocol_version` for bounded spend and rollout fencing.
- `GroupFollower`: optional `trigger_run_id` plus durable notification claim/sequence state.
- `AgentEvent`: optional `trigger_run_id` and deterministic event identity for replay-safe delivery.
- `ChatAuthority`: server-derived optional `trigger_run_id` propagated from the active trigger turn.
- `TurnClaim`/`TurnStart`: an exact continuation claim for `origin: event` turns, with trigger run/work/lease fencing.
- `assistant_team::wake`: trigger-aware selective event draining; unrelated events remain queued.
- Activation: `AUTOMATION_CONTINUATION_V2_ENABLED` is an operator drain assertion;
  the runbook and readiness harness pause dispatch, stop old workers, verify lease
  expiry, migrate existing active schedules/runs, and drain v2 work before rollback.

These changes preserve legacy rows through optional/defaulted fields and keep normal event guards unchanged. The coordinator and sibling PMs must review changed signatures before implementation.

## Acceptance items remaining

- Obtain `PLAN_APPROVED` for the exact plan SHA from the required Opus session.
- Create exactly one Sol implementer with the required provider/model/effort/mode and exact worktree.
- Implement the plan and record executable evidence for every acceptance-matrix row.
- Perform Astra's complete diff/code review; resolve every finding, including nits.
- Obtain `IMPLEMENTATION_APPROVED` from the same Opus session for the exact final commit.
