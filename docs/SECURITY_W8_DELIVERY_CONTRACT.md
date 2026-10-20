# W8 delivery caller contract

CORE-B owns this additive delivery interface in
`services::notification_service::delivery`. It uses the frozen
[audit caller contract](SECURITY_W8_CORE_CONTRACT.md) and its existing runtime;
it introduces no alternative admission, claim, retry or recovery authority.
CORE-B-0 publishes the interface. Workflow 42 remains pending until its producers
and provider sinks have been converted and their complete boundaries tested.

## Ownership and identities

ID owns connection-expiry, connect-link and OAuth-broker producers.
ASSIST owns channel/platform/automation producers and their adapter effects.
NODE owns approval, Oracle and machine producers. CORE-B owns the shared
push/Telegram/generic-webhook sinks; email remains unconfigured as described
below. Each owning producer still enforces its live authentication, owner ACL,
authority generations and business dedupe.
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

## Email remains unconfigured

`Channel::Email` and `PayloadSource::Account` remain reserved interface values.
This remediation adds no SMTP transport, email consumer or mail dependency.
The existing `SMTP_*` configuration fields and deployment documentation do not
implement delivery: registration verification and password-reset email remain a
product gap. Implementing that transport is separate product work.

ID does not create an account email payload store, persist raw verification/reset
tokens for an absent consumer, or enqueue account email effects. There is no
`account_delivery` startup index registration or email dispatch/recovery worker.
The existing account-token verification and reset semantics remain domain-owned.

If an Email-channel envelope is ever encountered, its resolver must report the
explicit unconfigured outcome and send nothing. An absent consumer must not
obtain a dispatch claim, report provider acceptance, fall through to another
channel, log a token, or manufacture a delivery receipt. The generic
`execution::dispatch_one` API requires an explicitly supplied consumer; it does
not discover an email transport or turn SMTP configuration into send authority.
A future email feature must supply its reviewed immutable payload, destination,
consumer and startup/recovery wiring before enabling account mail producers.

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
only the caller's sealed `AuditEventSource` projection through
`EffectReservation<E>`. Both plain `AuditEvent` and `DetailedAuditEvent` (owned or
borrowed) are accepted. It never receives
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
| `DeliveryRequest<E = AuditEvent>::new(key, payload_ref, effect_reservation)` | Accepts `E: AuditEventSource` through the core's `EffectReservation<E>`, including `DetailedAuditEvent` and borrowed event producers. Validates UUIDs, the complete safe projection including detail, Intent stage, matching operation and distinct intent/terminal event IDs. |
| `enqueue_in_session(db, session, runtime, owning_db_tier, &request)` | Join the producer's complete business transaction. Persist the immutable product envelope and call `begin_effect_in_session` for its two slots. Return a `QueuedDelivery`; neither commit nor dispatch occurs here. |
| `load(db, operation_id)` | Recover an immutable product reference. This returns data, with no dispatch authority or authorization decision. |
| `queued.authorize(db, runtime)` | After confirmed commit and live domain checks, delegate to `authorize_effect`. Returns a private `DeliveryPermit` containing the core permit and the exact queued context. |
| `permit.run(\|queued\| async move { ... })` | Consume that permit and context once. The core rechecks its process/attempt claim and live admission before entering the closure. Resolve/fence product inputs and perform one provider effect. Never detach the send or return the permit as a boolean. |
| `finish_in_session(db, session, runtime, operation_id, outcome)` | Join the product receipt/outcome transaction and call the core's terminal helper. Uses its reserved slot, even after new admission closes. |

The bound execution adapter in `delivery::execution` consumes these interfaces
together. Producer-owned resolvers supply protected payload versions and receipts;
no automatic queue scan or recovered envelope grants dispatch authority:

| Interface | Required consumer and contract |
| --- | --- |
| `DeliveryBinding { destination_id: Uuid, payload: PayloadRef }` | ID's ConnectLink/Connection resolvers, and other producer-owned resolvers, return the exact server-loaded immutable payload/destination binding. It contains no dispatch authority. |
| `DeliveryPermit::run_bound(binding, channel, effect)` | The sink checks the queued destination, payload reference and channel against that independently resolved binding before consuming the existing one-shot permit. A mismatch refuses the send. |
| `SendOutcome<T>` | A sink returns `Accepted(T)` for positive provider acceptance, `Rejected { status: Option<u16>, invalid_destination: bool }` only for positive rejection, or `Uncertain` for ambiguous delivery. Provider receipt `T` stays in protected product storage; no free-form response enters audit detail. |
| `SendOutcome::delivery_outcome()` | The worker maps those outcomes to Succeeded, DefinitiveFailure or Uncertain and commits the product receipt plus the reviewed core terminal completion together. Webhook failure detail uses `finish_effect_with_detail_in_session`; mapping never grants retry authority. |

`execution::DeliveryConsumer` supplies `resolve`, `send`,
`record_outcome_in_session` and `terminal_detail`. `resolve` loads and authorizes
the immutable product version and independently constructs
`ResolvedDelivery { binding, channel, prepared }`; copying identity fields from
the queue is not a resolver. Each producer owns its protected version store
and returns the same `DeliveryBinding`; the shared sink has no dependency on
that product model. There is no Account/email consumer in this remediation.

After confirmed enqueue commit and materialization, invoke
`execution::dispatch_one(db, serving_runtime, operation_id, consumer)`. It
checks the resolved binding before claiming and again in `run_bound`, consumes
the existing core permit for one provider send, then runs a DB-only bounded
transaction that records the protected product result and calls the reviewed
terminal-detail helper together. `SendOutcome::delivery_outcome` determines the
terminal stage. Detail freezes before this transaction's retries. Consumers
record by stable operation ID and must refuse any changed receipt replay.

`execution::commit_outcome` retries the receipt/terminal transaction from known
provider or reviewed reconciliation evidence without sending. If completion is
unresolved, the durable claim remains reserved for reconciliation. A later
`dispatch_one` cannot resend a claimed or terminal operation; a lost response
never proves non-delivery. Approval/connection and other converted workers
retain their own live owner, revision, destination and revocation fences, and
must recheck any
mutable authority immediately before the network effect. There is no new
automatic worker that could dispatch an unconverted producer. Do not create a
second intent, bypass a permit, fan out inside one send, detach the effect, or
equate a timeout with positive rejection.

Use the serving process's existing `AuditRuntime`. Do not bind effects, create
an unconfigured runtime or synthesize readiness in a producer or sink.
The enqueue tier is the owning DB operation's Standard, Availability or Narrowing
tier, fixed by service policy. Effect is refused as an enqueue tier; dispatch has
its own Effect admission. Preserve local revocation even when its remote notice
or provider revoke must defer.

Prepare stable delivery/event IDs and immutable payload inputs before retries.
When detail contains before/after policy, construct the detailed event and
`DeliveryRequest` inside each transaction attempt from that attempt's loaded
before/after snapshot; reuse the prepared IDs. Never capture a pre-read policy
value or derive security decision detail from the request patch alone.

The producer's transaction callback includes all live authority/revision checks,
the mutation, the immutable payload write/fence and enqueue. Use
`audit_runtime::transaction::run`, or the approved complete service-history owner.
Propagate errors without continuing an aborted session. Do not perform network
calls, spawn required work or generate fresh identities in the callback.
An unknown commit is not permission to send: resolve the committed product
receipt using the original identities before obtaining a permit.

For example, the integration order is:

1. Prepare stable product/delivery/event IDs, payload version and verified actor/owner.
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
variant through `event.with_detail(detail)` whenever legacy evidence carried an
enumerable security decision value. The adapter preserves detail in its immutable
product envelope and passes the same sealed producer to core reservation. The
core preserves it through materialization and terminal completion; changing or
removing detail on replay conflicts. Absent detail retains today's event bytes.
Omit detail for identifiers/content; this interface adds no free-form field.
Request needed variant names, fields, types, bounds and example sites through
PM. Mark conversions `detail-pending` until the reviewed variants are available,
then backfill them without dropping the corresponding legacy assertions.

The following CORE variant requests are `detail-pending` for workflow 42,
not additional local variants or prerequisites for publishing this interface:

| Requested variant | Fields, types and bounds | Existing example site |
| --- | --- | --- |
| `NotificationSettingsChange` | `old/new: NotificationSettingsDecision`, each with `telegram_enabled`, `push_enabled`, `approval_suspended` and `approval_delivery_configured` booleans; `global_approval_mode: AuditApprovalMode`; `approval_timeout_secs: u32` in 10–300; `grant_expiry_days: u32` in 1–365. Preserve the legacy `approval_required` tri-state through the existing WritesOnly/All/Off mapping. Values come from the authorized transaction snapshot, not request input. | `handlers/notifications.rs::update_settings` currently records settings, effective approval mode and delivery readiness. |
| `WebhookDeliveryFailure` | `event: WebhookEventKind`, `reason: WebhookFailureKind`, `attempts: u8` in 0–3 and `last_http_status: Option<u16>` in 100–599. Event kinds: ConnectionExpired, ConnectLinkCompleted, ConnectLinkCancelled, ConnectLinkExpired. Failure kinds: TenantIdentityMissing, ConfigurationLoadFailed, RateLimitUnavailable, AppRateLimited, KeyIdLoadFailed, KeyIdStoreFailed, SecretDecryptFailed, SerializationFailed, BodyTooLarge, DestinationUnavailable, SendFailed, NonSuccessStatus. No provider text or identity/content fields. | `services/developer_webhook_service.rs::record_final_failure_for_app` and its existing failure-audit assertions. |

The webhook request requires CORE's reviewed terminal-detail API because the
failure class is known after the effect; today's terminal helper preserves the
intent's existing detail. The 0–3 bound preserves the legacy bounded attempt
counter and grants no authority to retry a consumed delivery permit. The
terminal stage already supplies Succeeded/Failed/Cancelled/Uncertain; do not
invent another outcome state machine. CORE reviews the requested variants and
their validation centrally before the conversion uses them.

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
