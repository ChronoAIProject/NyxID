# Service pool failover implementation plan

Issue: https://github.com/ChronoAIProject/NyxID/issues/1680

The pool exposes one stable route across compatible service connections. It can
prefer an available platform connection and fall back to an owner's BYOK
connection. The existing `ServicePool` is the durable construct. Routing,
attempt policy, and health are independent of Oracle; Oracle's queue and session
semantics do not enter this implementation.

Fable 5.1 reviewed the current code and proposed the initial design. This plan
incorporates the primary reviewer's corrections. Implementation is assigned to
GPT-6-astra at xhigh reasoning with full access. The primary agent reviews the
code and tests directly and returns findings to the same implementation agent.

## Design decisions

### Membership and authorization

Keep member references as `user_service_id`. Platform-bound and automatically
provisioned connections already use `UserService` and can coexist with BYOK
connections under one owner. Use their existing live platform grants and billing
classification. Do not introduce catalog credentials as a parallel member type.

Preserve personal, legacy-personal, and organization routing precedence. Candidate
planning filters by the caller's effective service and node allowlists, ownership,
org membership, admin-only flags, and operation policy. Before every attempt,
revalidate the exact member and the request's operation with the existing read-only
authority and approval facilities, then materialize credentials. The same resolved
authority must be the one executed. The first member must not supply approval or
authorization metadata for a later selected member.

An unavailable or out-of-scope member can be excluded before selection. A human
approval denial, approval timeout, or request-wide authorization failure terminates
the request. Approval for one member never authorizes a backup. Scheduled/durable
and delegated exact-target execution must remain bound to an exact service; refuse
multi-member execution when their existing authority cannot cover it, before any
provider effect. Tests must exercise the actual restricted request routes.

### Configuration and compatibility

Add `priority` to member configuration (lower first), per-member model mapping,
an explicit request contract, and bounded failover/cooldown configuration. Keep
round-robin and weighted pools' default single-attempt behavior. Introduce an
explicit `priority` strategy for new routing semantics, with a separate choice of
round-robin/weighted balancing within each tier. All new routing features require
the new strategy so older backends cannot silently ignore the contract or model
mapping and execute a request incorrectly. Update frontend/CLI enums together.
Operators must upgrade replicas and clients before authoring priority pools;
rollback requires converting or deleting every priority pool first. Disabling a
priority pool is insufficient because older backends also deserialize disabled
pools when listing them. Old documents retain
their existing defaults. Omitted fields in member updates preserve new settings;
explicit null clears nullable settings. Validate the merged effective configuration
on every mutation, including add/remove/set-members and strategy/contract changes.

Two supported contracts:

- Same API: raw slug proxy for members that accept the same operations and wire
  format. Reject known incompatible inference protocols. Custom endpoints need an
  explicit compatibility declaration, and eligibility must distinguish declarations
  from catalog metadata; protocol equality alone does not prove API equivalence.
- AI chat: a stable OpenAI chat-completions request/response contract, explicit model
  mapping per member, protocol adapters, and capability validation. Use existing
  inference metadata and translators, but validate their actual supported features.
  Unknown or lossy features must be rejected before dispatch, never silently dropped.
  Support tools/streaming/images only when the chosen adapter and member contract
  preserve them. Repair relevant translator deficiencies and test them. Existing
  provider slugs are hints, not authority to override a connection's destination.

Expose AI pools through the normal slug proxy and
`POST /api/v1/llm/gateway/v1/chat/completions` using `model: "pool:<slug>"`.
Both entrances use one pool planner and attempt policy. For the AI contract, the
slug route uses the same translation path as the gateway. Reject unsupported paths
instead of sending an AI body to an unrelated endpoint. Preserve member destinations,
node routing, defaults, agent credential overrides and current platform restrictions.
Ordinary gateway requests retain their provider/model behavior.

### Attempt execution

Create a reusable service-layer routing/attempt-policy module, with HTTP/gateway
adapters at the existing handler execution boundary. Reuse the established proxy,
authorization, billing, and node paths; avoid a second simplified proxy that bypasses
those facilities. A typed attempt result distinguishes terminal application errors,
retryable pre-commit outcomes, and the final response. Never infer an upstream
failure merely from the HTTP status of a local `AppError`.

Buffer the request once within existing ingress limits. A smaller configured replay
limit can restrict a request to one attempt. Count ingress agent rate limiting once;
member-specific platform limits remain per attempt. Each candidate is visited at
most once. Priority determines tier order; weights/round-robin determine the first
candidate and ordering within a tier. Cap member attempts (default 3, server cap 5),
per-attempt time, overall request preparation/attempt time, and nested node work.
Use one deadline propagated to node and direct transport; checks before a node
request alone do not bound a hanging request. Caller cancellation cancels downstream
work and never triggers fallback.

Retry policy must distinguish:

- Definitely unsent: proven connection-establishment failure or node rejection
  before dispatch. Retry only configured transport/unavailability causes.
- Explicit upstream rejection: e.g. upstream 429. The default AI quota fallback can
  retry this rejection before client commitment. It must not classify NyxID's own
  authorization, ingress-rate-limit or billing errors as upstream 429s.
- Ambiguous: timeout after possible dispatch, connection loss after send, and 5xx
  responses without stronger provider evidence, including 503. Unsafe methods
  require explicit replay opt-in; AI POST is not automatically idempotent. Preserve
  existing stricter node semantics. Optional 401/403 applies only to upstream
  credential rejection, never local access/approval denial.
- Terminal: invalid request, policy/approval denial, client cancellation, and any
  error after committing response headers/body to the caller.

The commitment boundary is returning the response to Axum/client, not allocating
a `Response` object. For streaming, inspect status and gate the first body data/error
within bounded time before returning, so a stream that fails before its first data
frame can fall back when replay is permitted. Once committed, propagate errors and
never concatenate another member's output. Do not discard lazy metered streams:
settlement/cancellation must run even if a response is retried or the caller leaves.
Handle raw direct, node, translated SSE and supported Codex transport consistently.
WebSocket sessions remain single-member; reject retry configuration for an operation
that cannot meet the replay contract rather than silently promising failover.

### Health and accounting

Persist cooldown in MongoDB with bounded retention and a TTL index. Use UUID-v4
document IDs and a unique compound scope. Scope includes pool/member and effective
credential identity/epoch, destination/configuration and model where relevant. A
failing agent credential override must not cool a different key, a replaced
credential, or unrelated org members. Bind health observations to a generation or
sequence so an older success cannot delete a newer failure. Use atomic updates and
await routing-relevant health writes. Do not unconditionally delete state on success
or fire-and-forget ordering-sensitive writes. Reset is owner-authorized and fenced
against stale in-flight observations. Member removal/pool deletion cannot resurrect
active routing state.

Honor bounded Retry-After seconds and HTTP dates; clamp exponential backoff safely.
Skip members still cooling. If all eligible members are cooling, return an explicit
unavailable result and retry information; do not bypass the cooldown automatically.
Eligibility/health display must not expose another caller's credential identities.

Create an independent billing identity/reservation per dispatched member attempt,
using its real credential class, actor and resource owner. Release unsent reservations
and settle known consumed usage through existing billing services before moving on.
Unknown usage retains a null quantity and additive attempt-outcome/lease metadata
on the existing usage status model. Ordinary reconciliation intentionally does not
release forwarded unknown rows, so pool-tagged attempts need explicit durable
lease recovery. Admission commits funding holds and meter rows together; recovery
rechecks the lease and known settlement intents transactionally before releasing
unresolved holds. Known consumption always settles first. Do not fabricate a zero-token success or charge a
full successful request for a proven rejection. Do not bypass the exact-credit ledger
or manipulate wallet/account collections in pool code. Admission, stream errors,
timeouts, cancellation and error-body truncation all need an explicit cleanup path.

Record attempt metadata without credentials or request/response bodies. Response
headers report the actual member and attempt count. Exhaustion preserves the final
upstream error where possible or returns a structured pool error for transport-only
failure; sanitization and limits still apply. Skipped candidates are distinct from
attempts and must not leak inaccessible member information to callers.

## Implementation sequence

1. Models, configuration validation, compatibility/candidate queries, pure retry
   policy, durable health and migration/default tests. Register new models/services,
   indexes, error codes, request/response DTOs and OpenAPI schemas as needed. Keep
   the error-code list authoritative in `errors/mod.rs`; follow repository rules.
2. Refactor the existing HTTP proxy boundary enough to execute exact member attempts
   safely. Add the common planner/loop, deadline/cancellation propagation, response
   commitment control, attempt billing and audit. Fix first-member preflight and
   selected-member mismatch for existing pools as part of this boundary.
3. Integrate AI contracts, per-member model/adapters and gateway aliases, including
   raw slug entry parity. Add discovery so clients can find configured AI aliases.
4. Complete REST, CLI and frontend creation/editing: priorities, tier balancing,
   model/contract configuration, retry/time/body limits, cooldown and reset, and
   candidate eligibility reasons. Candidate discovery must work before a pool has
   been created as well as while editing, with personal/org ownership explicit.
   Follow `DESIGN.md`, `useAppForm`, and current lifecycle naming.
5. Update user docs, architecture/routing proof and the NyxID skill reference with
   working platform-to-BYOK and gateway examples, retry/approval semantics and rollout.
   No Oracle migration is required. Do not add a release version bump unless required
   by the repository's actual release workflow.
6. Run targeted and required checks, then the primary agent reviews every changed
   production file and meaningful tests. Return all findings, including minor issues,
   to the same Astra agent and repeat until there are no unresolved known findings.

## Verification requirements

Use real local HTTP mock servers and the isolated MongoDB replica set, with upstream
request counts, received headers/bodies, final stream output, audits, billing rows
and ledger verification as appropriate. Cover:

- Upstream A=429/B=200; A=400/no B; exhausted HTTP and transport attempts; old pools
  remain single-attempt with their established balancing behavior.
- Priority ordering/weights, disabled/deleted/unowned members, replay size limits,
  all-cooled behavior, Retry-After dates/seconds, concurrent health ordering/reset,
  and credential-override/rotation/tenant isolation.
- Pre-first-frame stream failure and bounded fallback, mid-stream failure without
  fallback, ambiguous POST with opt-in off/on, safe methods, node offline and
  dispatched failure, nested deadline exhaustion, client cancellation and cleanup.
- Restricted service/node scopes, org viewer/admin-only/scope restrictions, live
  platform revocation, approval deny/timeout, exact/delegated/durable target fencing.
  Prove no unauthorized credential materialization with an instrumented key provider
  or equivalent execution observation; absence of a routing audit is insufficient.
- Actual platform and BYOK attempts with billing enabled: correct wallet owner,
  class and per-attempt identity, incurred usage retained, no duplicate settlement,
  no reservation orphaned by response preparation or cancellation, balanced ledger.
- AI alias OpenAI-to-Anthropic (and other supported adapters) request/model mapping,
  response and SSE conversion, tools/modalities, unsupported fields, custom
  destination preservation, and slug/gateway parity.
- API/CLI payload roundtrips, omitted/null update semantics, pre-create candidates,
  UI create/edit dirty state and visible validation, health rendering and reset.
- `cargo fmt`, relevant backend/CLI tests, workspace clippy, frontend types/lint/
  tests/build, billing route coverage and any other applicable CI gates.

## Local execution

Base: `e96a5078` on `evaluate-service-failover-pools`.
Mongo container: `nyxid-arctic-fjord-pools-mongodb`, host port 27021.
Use `NYXID_TEST_DATABASE_URL=mongodb://127.0.0.1:27021/?replicaSet=rs0&directConnection=true`.
Frontend dependencies are installed. Build with `CARGO_PROFILE_DEV_DEBUG=0` and
`CARGO_PROFILE_TEST_DEBUG=0` to reuse this worktree's warmed test artifacts.
The Fable source consultation is at
`/Users/chronoai/.claude/plans/consult-on-nyxid-issue-reactive-marble.md` and is advisory;
the corrected decisions in this document take precedence.
