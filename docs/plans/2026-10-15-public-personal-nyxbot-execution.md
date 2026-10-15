# Public and personal NyxBot implementation team

Created 2026-10-15 at the user's request for five parallel agents. The
[workstream allocation](2026-10-15-public-personal-nyxbot-workstreams.md) defines
exclusive ownership; the [design](2026-10-15-public-personal-nyxbot.md) and its
Opus convergence verdict define behavior.

Shared source baseline: `origin/main` at `b6f11c1d3`. Shared plan commit:
`7ff461c92` on `nyxbot-manager/base`. All five worktrees were verified at this
commit before their first edits. The earlier `guide-setup-simplify` branch and
PR #1896 are separate.

## Assignments and opening schedule

The session permits four active agents including the primary. Five workstream
agents therefore run in overlapping waves with at most three active workers.
Idle agents do not start a provider turn until the primary schedules them.

| Part | Agent ID | Branch | Opening assignment |
| --- | --- | --- | --- |
| 1 Identity, consent and controls | `01a13e66-2441-7e41-8293-28c93acc8f13` | `nyxbot-manager/identity` | Active: source review and concrete identity/HTTP/transaction interface proposal |
| 2 Personal runtime | `01a13e67-00b6-7770-a21d-8f8deb137947` | `nyxbot-manager/personal` | Created, verified idle: next available slot for engine contract review and implementation |
| 3 Public policy, NyxAgent and billing | `01a13e66-b6f1-78c0-84ba-69a6a35fab59` | `nyxbot-manager/public` | Active: auth/proxy/billing interface proposal and local upstream-contract evidence review |
| 4 Frontend and setup | `01a13e67-0146-7bc1-b2d9-9cb33d434283` | `nyxbot-manager/frontend` | Created, verified idle: next available slot for HTTP contract review and frontend implementation |
| 5 Contracts and Telegram integration | `01a13e65-a3c6-7fe2-8b5a-69ad79b77be4` | `nyxbot-manager/integration` | Active: shared concrete Rust/HTTP contract commit before production implementation |

Worktree directories are siblings of this checkout named
`NyxBot-manager-identity`, `NyxBot-manager-personal`, `NyxBot-manager-public`,
`NyxBot-manager-frontend`, and `NyxBot-manager-integration`.
All agents use the configured Codex provider defaults. Each was created with an
explicit worktree ID and verified placement in the NyxID project, never Loose.
All have completion notifications enabled. No worker may spawn subagents.

## Coordination

1. Collect Parts 1 and 3 interface proposals and pass them to Part 5. Their
   opening turns end after local proposal commits; they do not wait or poll.
2. Schedule Parts 2 and 4 into available slots to review the execution and HTTP
   seams. Freeze the contract with producer acknowledgement before dependent
   production edits. Resolve signature changes through Part 5.
3. Share the contract commit with all branches. Schedule implementation slices
   across at most three active workers. Part 3 lands local auth/origin hooks
   independently of the unresolved upstream public-runtime evidence.
4. Part 5 integrates compatible producer commits and owns shared registrations,
   lifecycle entry points, transport and combined validation. It returns defects
   in other owners' files to those owners.
5. Only one heavyweight Rust build runs on this host at a time. The primary
   grants that build slot; initial contract/proposal tasks use source inspection
   and targeted formatting/whitespace checks only. Frontend and source work can
   overlap with the build.

Keep all incomplete execution paths disabled/unavailable. Public activation
requires proven NyxAgent native-tool containment, model-call funding propagation
and session cleanup. Personal work is independent of that proof. Neither mock
results nor isolated branch checks satisfy combined acceptance A1–A17.

The user subsequently asked the primary to drive this through full completion
as PM. Continue implementation, integration, independent review, PR preparation
and CI without asking again at routine handoffs. Prepare any deployment/live
test request only after the integrated change is concrete and reviewable.
Production deployment, real-user Telegram messages and cross-repository changes
still need applicable authorization; prior release approvals are not reused for
this new feature. Track actual completion in the
[acceptance evidence](2026-10-15-public-personal-nyxbot-acceptance.md).

## Evidence

- Design and five-workstream allocation committed; whitespace check passed.
- Five worktrees verified against the same base commit.
- Five named agents created with explicit placement; three opening turns sent,
  two queued agents verified idle.
- No product implementation, Rust build or feature acceptance result is claimed
  at this opening checkpoint.

## Contract checkpoint

- Part 5 produced contract candidate `1b5aea084`; the primary applied it as
  `8e670134b` alongside the execution/acceptance records. It defines service
  values and producer signatures and mounts no new route or runtime.
- Full workspace `cargo +1.98.1 fmt --all -- --check` passed on that candidate.
  No compile or runtime-test claim follows from formatting.
- Part 2 started its engine/contract review. Parts 1 and 3 are reconciling their
  proposals; Part 5 is idle after the candidate commit. Part 4 remains queued.
- Some provider reloads emitted duplicate finish notifications while Parts 1
  and 3 resumed their original work. The primary verified actual status and
  stopped Part 4's just-started review to restore the three-worker limit. Future
  slot changes require a confirmed idle state and a concrete handoff artifact.
- Primary review identified the existing broad Stop cancellation, the optional
  ordinary live-turn flag, first-use missing NyxBot identity, initial runtime
  creation semantics and pre-router token scrubbing as concrete integration
  points. These have been sent to the relevant owners.

## Implementation handoff

Parts 1, 2 and 3 completed their source/interface proposals. The primary integrated
them through `921411d5e`; these are design evidence, not completed feature code.
Part 5 is reconciling their concrete corrections into the shared contract. Part 4
is completing the bounded HTTP review. Part 1 has begun its durable models and
consent/control services; independent work does not wait for another design round.
The active wave is Parts 1, 4 and 5. Parts 2 and 3 are queued for implementation.

The primary approves the narrow scope additions in Part 2's interface proposal:
`handlers/nyxbot.rs::owner_thread`, settled message origin, image/steering/title
execution helpers, machine job issuing-turn provenance and exact cancellation,
and async operation delivery provenance. Part 2 is their sole writer. Part 3
owns the new `services/assistant_nyxagent_egress.rs` adapter and existing proxy
core extraction. Part 5 retains all shared registration and composition edits.

Initial **Enable bot** must initialize both personal and public lanes atomically
after public readiness, without requiring a second hidden personal toggle.
Explicit personal-only opt-in remains possible while public proof is unavailable.
Subsequent public-only settings do not alter personal controls. An enforcing
server with no configuration reports `supported: true, runtime: null`; explicit
initialization uses zero absence preconditions and the live legacy profile
generation, then stores positive epoch/revisions. Ordinary updates use positive
expected versions. Trusted backend time governs expiry; client time never does.

The host briefly fell to approximately 200 MiB free while unrelated worktrees
were building. The primary verified no compiler used this checkout's target,
then removed only its disposable `target/debug/incremental` cache, recovering
approximately 5 GiB. Other build artifacts/processes were not touched. Rust
builds remain serialized and subject to a fresh capacity check.
