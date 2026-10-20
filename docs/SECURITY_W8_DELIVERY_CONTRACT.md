# W8 delivery caller contract

CORE-B owns this additive delivery interface in
`services::notification_service::delivery`. It uses the frozen
[audit caller contract](SECURITY_W8_CORE_CONTRACT.md) and its existing runtime;
it introduces no alternative admission, claim, retry or recovery authority.
CORE-B-0 publishes the interface. Workflow 42 remains pending until its producers
and provider sinks have been converted and their complete boundaries tested.

## Ownership and identities

ID owns account, connection-expiry, connect-link and OAuth-broker producers.
ASSIST owns channel/platform/automation producers and their adapter effects.
NODE owns approval, Oracle and machine producers. CORE-B owns the shared
email/push/Telegram/generic-webhook sinks. Each owning producer still enforces
its live authentication, owner ACL, authority generations and business dedupe.
This interface never grants access based on a supplied owner or a loaded queue row.

Prepare one `DeliveryKey` outside the transaction retry callback, and persist it
with the product event so workers, retries and recovery reuse exactly these IDs:

| Field | Meaning |
| --- | --- |
| `operation_id` | UUID v4 for one actual send to one destination/channel. This is also the core effect operation ID and product queue `_id`. |
| `product_event_id` | Stable UUID v4 for the source event/dispatch cycle. Preserve an existing cycle identity; retries cannot manufacture another cycle. |
| `destination_id` | Stable UUID v4 for the server-resolved destination record. Never an email address, push token, chat identifier or callback URL. |
| `channel` | Closed `Channel` enum: Email, Fcm, Apns, Telegram or Webhook. |

The unique `(product_event_id, destination_id, channel)` index prevents another
operation ID from delivering the same logical send. Reusing an operation with
changed destination, channel, payload reference, actor, owner or event conflicts.
Do not mint fresh IDs to escape a claim or terminal outcome. A notification sent
to three devices is three effects with three operations and six reserved slots.
Do not share a permit across recipients or wrap a multi-send loop in one permit.
Transport wrappers consume the same permit; they do not reserve nested intents.

## Product payload storage and safe evidence

`PayloadRef::new(source, record_id, revision_id)` accepts a closed `PayloadSource`
and two UUID v4 references. `source` selects the producer-owned resolver. The
producer persists the immutable referenced payload version in the same business
transaction as enqueue, under its existing encryption and access controls.
For an existing durable payload, join the transaction's read/revision fence.
The destination mapping must be stable or revision-fenced too; changing an address
under the same ID cannot redirect already admitted work.

The referenced version must remain recoverable until the product's reviewed
delivery/reconciliation retention permits deletion. Missing or changed versions
fail closed. A mutable domain row plus a newly rendered body does not satisfy
this contract. Materialized bodies, template arguments, credentials, tokens,
provider responses and destination addresses stay in protected product storage
or process memory. `PayloadRef` accepts no arbitrary JSON or extra fields.

The immutable `notification_deliveries_v1` product queue record contains this
reference, the delivery identities and a safe event projection. The core receives
only the caller's typed `AuditEvent` through `EffectReservation`. It never receives
the payload reference, destination address, body or template arguments. Product
references are not copied into `audit_pending_events`, audit intents, chained
rows or witness-delivery evidence. There is no automatic queue TTL or deletion
that could erase dedupe evidence and permit another send.

The event uses stage Intent and a reviewed static action. Its operation ID must
equal `DeliveryKey.operation_id`. Its target is an optional safe domain UUID v4.
Use the exact verified principal: Person, Node, ServiceAccount, OracleWorker,
ApiKey (including its verified owner), or a named System actor. Preserve the
resource's server-resolved Person/Org/Platform owner independently. Background
delivery never invents a human principal; an affected owner is not its actor.
The existing core validator checks these identities and safe event fields.

## Producer and sink API

| API | Contract |
| --- | --- |
| `ensure_indexes(db)` | Startup DDL before producers are enabled, outside business transactions. Creates the required unique product/destination/channel index. B owns startup registration; request that hook through PM. Enqueue requires the named index and fails if it is absent. |
| `DeliveryRequest::new(key, payload_ref, effect_reservation)` | Prepare outside retries. Validates UUIDs, safe event, Intent stage, matching operation and distinct intent/terminal event IDs. Uses the core's `EffectReservation` directly. |
| `enqueue_in_session(db, session, runtime, owning_db_tier, &request)` | Join the producer's complete business transaction. Persist the immutable product envelope and call `begin_effect_in_session` for its two slots. Return a `QueuedDelivery`; neither commit nor dispatch occurs here. |
| `load(db, operation_id)` | Recover an immutable product reference. This returns data, with no dispatch authority or authorization decision. |
| `queued.authorize(db, runtime)` | After confirmed commit and live domain checks, delegate to `authorize_effect`. Returns a private `DeliveryPermit` containing the core permit and the exact queued context. |
| `permit.run(\|queued\| async move { ... })` | Consume that permit and context once. The core rechecks its process/attempt claim and live admission before entering the closure. Resolve/fence product inputs and perform one provider effect. Never detach the send or return the permit as a boolean. |
| `finish_in_session(db, session, runtime, operation_id, outcome)` | Join the product receipt/outcome transaction and call the core's terminal helper. Uses its reserved slot, even after new admission closes. |

Use the serving process's existing `AuditRuntime`. Do not bind effects, create
an unconfigured runtime or synthesize readiness in a producer or sink.
The enqueue tier is the owning DB operation's Standard, Availability or Narrowing
tier, fixed by service policy. Effect is refused as an enqueue tier; dispatch has
its own Effect admission. Preserve local revocation even when its remote notice
or provider revoke must defer.

The producer's transaction callback includes all live authority/revision checks,
the mutation, the immutable payload write/fence and enqueue. Use
`audit_runtime::transaction::run`, or the approved complete service-history owner.
Propagate errors without continuing an aborted session. Do not perform network
calls, spawn required work or generate fresh identities in the callback.
An unknown commit is not permission to send: resolve the committed product
receipt using the original identities before obtaining a permit.

For example, the integration order is:

1. Prepare stable product/delivery/event IDs, payload version and typed actor/owner.
2. Commit mutation + payload + `enqueue_in_session` in one retryable DB transaction.
3. After definite commit, validate the current product/destination authority and
   obtain `queued.authorize(db, runtime)` outside that transaction.
4. Transfer the returned permit to the single sink. Inside `run`, enforce the
   final destination/SSRF/configuration fences and send once. Existing unconverted
   helpers that internally retry HTTP requests cannot be used as that one send.
5. Commit the protected product receipt and `finish_in_session` together through
   the bounded DB transaction owner.

## Outcomes and recovery

`DeliveryOutcome` contains only Succeeded, DefinitiveFailure, Cancelled or
Uncertain. Succeeded means positive provider acceptance according to that sink's
contract; it does not claim that a person received/read the notification.
DefinitiveFailure requires positive rejection or proof that no effect occurred.
A timeout, dropped connection, lost response or crash after entering an effect
is Uncertain. Cancelled is valid only before any claim, as the core enforces.
No provider error text, response body or raw receipt enters the audit event.
Preserve existing non-secret forensic assertions during sink conversion. Use
the core contract's `detail: Option<AuditDetail>` with a CORE-reviewed closed
variant whenever legacy evidence carried an enumerable security decision value.
Omit detail for identifiers/content; this interface adds no free-form field.
Request needed variant names, fields, types, bounds and example sites through
PM. Mark conversions `detail-pending` until the reviewed variants are available,
then backfill them without dropping the corresponding legacy assertions.

Core completion is authoritative. The product queue is immutable and deliberately
does not maintain a competing claim/lease/outcome state machine. A product receipt
may summarize the outcome in the same transaction as core completion. Identical
terminal replay is idempotent; changing a terminal outcome conflicts.

Recovery loads the same operation and delegates authorization to the core.
A recovered unresolved claim becomes durably Uncertain and yields no permit.
A paused sender's permit is rechecked before its closure; an already entered
provider call may still complete. No lease expiry, restart, restored Mongo state,
retrieved receipt or missing product response authorizes another send. Preserve
real provider idempotency and positive reconciliation under the original logical
identity; further reconciliation transitions require the reviewed core/domain
API, never a fresh operation or overwritten terminal result.

In configured mode the existing independent witness protects claims across
Mongo rollback. Witness-free C1 mode keeps local intent/claim controls and remains
explicitly unwitnessed; this adapter adds no independent rollback protection.
Worker scheduling, provider-specific resolvers, sink conversion, startup wiring
and product reconciliation belong to the following workflow-42 implementation
unit and its participating lanes.
