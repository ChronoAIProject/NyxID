# Service pool routing architecture

`ServicePool` extends the existing UserService routing boundary. It stores an
owner, stable slug, embedded member IDs and configuration; credentials and
endpoints remain in their established stores. Node routing selects transport for
a concrete member and does not replace pool selection. Oracle is unrelated.
See [Service pools](SERVICE_POOLS.md) for the management and request contract.

## Request boundary

The slug entrance preserves personal service, personal pool, legacy and ordered
organization precedence. Organization pools with no caller-eligible member do not
hide a later authorized organization. Gateway `pool:<slug>` enters the same small
pool boundary with trusted LLM admission metadata; REST scopes are not broadened.

Priority ingress checks agent admission once and captures one pool ID/revision.
The body is read once within ingress limits. Candidate planning reloads live
metadata and must match that snapshot, preventing old retry policy from executing
a changed or recreated pool. Candidate filtering applies owner/platform grant,
service and node scopes, operation policy, transport capability and request
compatibility before the attempt cap. Planning never decrypts a credential.

The executor reserves a visited tier's counter, then carries the exact member
through read-only native operation and approval gates. It materializes only after
those gates and compares execution authority again before dispatch. Approval
waits therefore fence endpoint/key/configuration drift. Legacy strategies reserve
one selection once and use the same exact-member boundary, without priority
cooldown or retries. Legacy generic resolution rejects priority pools rather than
silently choosing round robin.

The direct and node transports share typed dispatch evidence. Priority direct
requests do not automatically follow redirects, so a failed redirect connection
cannot masquerade as a proven unsent original request. One absolute overall
deadline and one attempt deadline include preparation, dispatch, first-data gating
and drain. Node correlation guards remove pending work and send `proxy_cancel`
on cancellation, timeout, overflow or consumer loss; owner relay fencing remains
in force. The CLI tracks request tasks and aborts actual HTTP work on cancellation.

The first nonempty response data item, or clean EOF, is the commitment boundary.
Empty frames do not commit. Before commitment, known rejection status and
Retry-After survive a broken body. After commitment, errors remain body errors.
No handler starts a backup after caller cancellation or appends backup output to
a partially delivered stream. Unsupported execution entrances fail before effects.

## Contract and authority

`service_pool_contract` validates saved merged configuration and live candidates.
Same API declarations cannot override known protocol mismatch. AI models use
catalog inference and catalog provider hints, with the resolved UserEndpoint URL
preserved. `pool_ai_service` validates request features, selects the native path
and transforms request/response data once for both entrances. The bounded SSE
parser handles incremental UTF-8, LF/CRLF/CR framing, provider errors and missing
completion. Native bytes feed accounting before conversion.

Response ownership retains normal default/header forwarding, asynchronous
continuations, destination diagnostics, connection attribution and usage audits.
Async Location values use the chosen member slug plus the existing authorized
`_nyxid_via` selector, so following a job URL does not rebalance it.

## Durable health

`service_pool_member_health` has a unique immutable scope including pool/member,
configuration revision, credential identity/epoch, destination and operation/model,
plus pool/member reset generations. Issuing a ticket creates health only within
that scope. Outcome writes do not upsert. Reset updates only durable generation
state; old rows expire through TTL and old tickets cannot roll back the fence.

Each ticket has a separate expiring typed observation row. Terminal observation
and health mutation commit in one MongoDB transaction, so retries/interleavings
cannot double-count and cancellation cannot leave a claimed-but-unapplied result.
The hot health document has no unbounded observation ID list. Sequence fencing
prevents an older success from clearing a newer failure. Active provider
Retry-After minima cannot be shortened by a concurrent weaker failure.

## Billing and stream ownership

A dispatched attempt owns an independent meter/request identity and durable lease.
Pool admission atomically opens meter rows and reserves exact funding. Before
fallback, the executor awaits acknowledged known settlement or unknown cleanup;
a spawned settlement task alone is not that acknowledgment. Reported usage is
cached before settlement awaits and uses the existing durable intent/finalization
pipeline, including components and resale. Known intent wins over release.

Unknown cleanup atomically releases wallet holds and marks all request rows,
including the coordinator, so intent and release cannot write-skew. Existing
benefit recovery completes allowance/grant release idempotently. Recovery pages
at most 100 indexed expired rows using a durable keyset cursor, rechecks captured
expiry in the release transaction, and advances past known quantities/intents.
Ordinary historical forwarded rows are not eligible for this recovery.

A lifecycle-owned heartbeat renews leases independently of stream polling,
bounds its database work by authoritative expiry, distinguishes unmetered traffic
from a lost lease, and closes provider transport on loss. Dropping the client or
stream cancels owned work without recursively spawning cleanup guards.

## Configuration and observability

Merged edits compare `config_revision` and commit settings/members together.
Inspection is owner-scoped, read-only and paginated without leaking excluded IDs
in cursors. Health reads saved members directly rather than sampling an inventory
page. Draft strategy/contract/declarations apply only to candidate inspection.
Inventory inspection explicitly opts out of operation checks; old REST defaults
remain unchanged. Selected draft rows use an ID-bounded query independent of
inventory pagination. Draft candidates do not inherit saved member disable or
cooldown state; explicit saved operation checks retain both, and legacy strategies
never load priority cooldown. No inspection mode materializes credentials.
A deleted connection remains removable/resettable; Disable does not require stale
metadata to become executable again.

One attempt audit owner emits once across preparation, transport, body, timeout
and cancellation. Events use the standard tamper-evident audit path. Public
exhaustion summaries contain only ordered safe status/reason information. Secret
credential identity and API key references are never exposed through Debug or
inspection.

The implementation has real MongoDB/HTTP acceptance tests in
`handlers/service_pool_{proxy,runtime,billing,ai}_tests.rs`, configuration tests in
`handlers/service_pools_tests.rs`, transactional health and recovery tests, node
manager/dispatch and CLI cancellation tests, and CLI/UI request construction tests.
Execution results belong to the run logs; this architecture document does not
claim a particular unexecuted build is green.
