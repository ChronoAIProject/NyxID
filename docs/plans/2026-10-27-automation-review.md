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
| Implementation | GPT-6.1-Sol | Exact Astra-approved revision 3 plan | **NOT STARTED** | The single required Sol session will be created only after this ledger/status revision is committed and the Heca settings are verified. |
| Astra implementation review | GPT-6-Astra PM | Exact final implementation commit | **NOT STARTED** | Astra will read the complete diff and relevant surrounding code, run the acceptance matrix, and resolve every valid finding including nits through the same Sol session. |
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

These are additive compatibility changes. The coordinator and permissions PM have been
notified of the boundary; neither sibling branch is imported.

## Acceptance items remaining

- Create and verify exactly one GPT-6.1-Sol implementer in this worktree.
- Implement every acceptance-matrix row from the approved revision 3 plan.
- Astra personally reviews the complete implementation diff and surrounding code.
- Resolve every Astra finding, including low-risk issues and nits, through the same Sol
  session and re-run affected checks.
- Record `ASTRA_IMPLEMENTATION_APPROVED` only for the exact final commit after acceptance
  evidence and required checks pass.
- Keep any missing live client/destination/production evidence explicitly OPEN; it must not
  be replaced by a caveat or treated as completed.
