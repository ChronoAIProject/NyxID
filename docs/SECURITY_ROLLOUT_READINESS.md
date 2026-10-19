# Security rollout readiness and required approval

Security rollout has two boundaries: what the serving deployment supports, and
what a stored policy authorizes. New configuration must not be mistaken for
proof that every serving process can enforce it. Stored operation scopes,
machine assignments and live ACLs keep their enforcement even when their
configuration flags are off. This change does not rewrite grants or turn every
feature flag on.

## Deploy and declare the serving roster

Drain all old or unregistered serving backend replicas and relevant writers.
Upgrade every replica, including replicas reachable through alternate ingress
or internal routing, before relying on required web approval or enabling the
listed security features. Each process must use a unique, stable
`INSTANCE_NAME`, the same complete comma-separated `SECURITY_ROLLOUT_REPLICAS`,
and `SECURITY_ROLLOUT_DRAINED=true`. A single local deployment must explicitly
declare its own one-name roster too. Restart a process to change its local
roster or drain declaration.

The drain declaration and roster are operator assertions. Mongo observations
cannot discover an old binary hidden behind a load balancer, prove that an
omitted process was drained, or prevent an old binary from serving. A green
report is meaningful only after that operational cutover. Do not restart old
binaries against the cutover deployment.

Every upgraded process registers its actual startup generation before serving
and renews its observation every 10 seconds. Readiness accepts observations up
to 45 seconds old and rejects timestamps more than five seconds in the future.
It is anchored to the executing process's immutable generation and roster:
another fresh singleton cannot certify a process whose own observation expired.
Missing, duplicate, undeclared, unsupported, or differently configured replicas
block readiness. Restarts retain distinct generations; an old fresh generation
with the same name blocks readiness until it expires. Observation reads use an
`observed_at` index and fail closed above 256 live generations. A seven-day TTL
removes old diagnostic rows; TTL deletion is not required for freshness.

Use the admin/operator endpoint `GET /api/v1/admin/security-readiness` to inspect
per-capability `ready`, `blockers`, expected roster and live generation count.
The public `/health` endpoint remains a process health check. Neither endpoint
establishes OS user separation, node key-service custody, or externally
witnessed audit integrity.

## Feature configuration and execution

The following effective feature gates require deployment readiness:

| Capability | Existing boundary retained |
| --- | --- |
| `assistant:org-agents` | Creation/configuration and use-oriented organization-agent discovery gate (`delegation_summaries`); direct use retains live membership/resource ACLs. |
| `assistant:operation-scopes` | Scope editing gate; stored scopes are enforced during authentication and execution with the flag off. |
| `assistant:machine-capabilities` | Capability editing and deliberate legacy-assignment materialization behavior; stored assignments and live machine ACLs retain their execution checks. |
| `assistant:machine-contexts` | Separated-context configuration; backend observations do not certify the node's OS implementation. |
| `assistant:live-turn-gate` | Conditional execution/route gate; channel and platform turn fences and conversation-key management denial retain their stronger unconditional checks. |

The ordinary feature resolver still reports configured precedence. Actual
callers use `personal_flag_enabled`, which rechecks readiness whenever the
effective value is true. This includes existing overrides, clearing a false
user/org override, membership changes, and defaults at restart. Unready effective
activation returns HTTP 503 (`security_readiness_unavailable`, code 12900),
without converting it to false and executing a legacy path. An effective false
value keeps the existing flag-off behavior; it does not erase stored policy.

The `approval:required-web-v1` capability is **diagnostic**, not an enforced
activation switch. Required approval is implemented by every upgraded process
without waiting for the readiness report. Drain and upgrade every serving
replica before relying on the new default and web fallback. A capability row
does not fence an old binary and is not a substitute for that cutover.

## Platform operation-policy cutover

`PLATFORM_REQUIRE_OPERATION_POLICY` starts false. The first startup observation
with it true persists `security_rollouts/platform-policy-v1` before registering
that process or serving traffic. This marker is sticky and does not expire.
All actor-addressed catalog master-credential authorization then requires the
executing process and the entire declared roster to support this capability
and have the setting true. A false-local process must refuse even after the
true sibling's heartbeat expires.

Readiness also inventories active master-capable catalog rows: authenticated
rows with explicit platform-key configuration or `requires_user_credential=false`.
This conservative superset excludes inactive and no-auth rows. Every selected
row needs an explicit `proxy_operation_policy`; no credential bytes are read.
Missing/null policy blocks readiness. An explicit empty rules array is counted
separately, is ready as configured, and denies all actual operations through
the existing operation evaluator. Server-chosen master-credential consumers
retain their separate existing contract.

Prepare policies and the exact serving roster before enabling this variable.
Once cut over, restore a complete supported roster and policies to recover
readiness. Changing an env value to false, losing a heartbeat, or waiting for a
lease does not downgrade enforcement. A deliberate rollback requires full
maintenance drain and an explicit deployment/data recovery plan; the product
does not automatically clear this marker or provide a live downgrade command.

## Required approval and delivery

A user with no settings row requires approval on the first ordinary
programmatic request, before a settings GET materializes the row. Newly created
settings use `approval_required=true` and a 300-second timeout. Existing stored
false values stay false. Legacy documents missing the preference still read
false, and existing timeouts remain unchanged (legacy missing timeout reads
30 seconds). Explicit per-service/org policies, auto-connected exemptions,
session bypasses and existing grant semantics retain their precedence.

When approval is required, a pending request is persisted before notifications
are attempted. Missing registration, absent provider credentials, invalidated
tokens or transient send failure leave it pending. Users can approve or reject
in `/approvals/history`; the normal and empty views expose **Refresh** to
discover requests created after the page opened. Expiry or rejection does not
execute the operation. Registration and last-channel removal never suspend a
stored required preference.

Notification settings distinguish these facts:

| Field | Meaning |
| --- | --- |
| `approval_required` | Stored global preference, independent of delivery. |
| `approval_suspended` | Always false on this implementation; retained for older clients. |
| `approval_web_available` | Explicit true confirms this server implements required web decisions without delivery. |
| `approval_delivery_configured` | An enabled registration has matching initialized provider configuration. Delivery may still fail. |

Clients must preserve unknown values when talking to older servers. An absent
web capability or a true legacy suspension flag must not be displayed as a
guarantee of web-only enforcement. Keep a working notification channel until
the upgraded server explicitly confirms the new contract. Ordinary client UI
changes do not alter the separate mobile dependency remediation constraints.
