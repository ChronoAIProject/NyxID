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

The current authorization covers local implementation, isolated worktrees and
local commits. No new production deployment, real-user Telegram messages,
cross-repository changes or merge is included. Follow-up release decisions use
the concrete integrated result and actual validation evidence.

## Evidence

- Design and five-workstream allocation committed; whitespace check passed.
- Five worktrees verified against the same base commit.
- Five named agents created with explicit placement; three opening turns sent,
  two queued agents verified idle.
- No product implementation, Rust build or feature acceptance result is claimed
  at this opening checkpoint.
