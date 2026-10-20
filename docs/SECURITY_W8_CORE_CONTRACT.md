# W8 caller contract

W8-4 is PM-accepted. This is the frozen service API for the ID, ASSIST, NODE and
CORE conversions. Request interface changes from CORE; do not edit shared audit
modules or split an existing business transaction to accommodate this API.
Workflow ownership and completion evidence remain in
[SECURITY_W8_INVENTORY.md](SECURITY_W8_INVENTORY.md). Component acceptance does
not complete any pending caller workflow.

Pass the serving process's existing `AppState.audit_runtime` (or the same
startup/worker-owned runtime) through the service boundary. Do not construct an
unconfigured runtime per operation, bind a second effects identity, synthesize
freshness/readiness, or choose a tier from request input. `main` binds the single
process generation and optional relay before business writers start.

## Transaction ownership and contention

Every audit append/reservation shares the single persisted `audit_chain_state`
document. Contention is expected. Mongo snapshot transactions return
`WriteConflict` (112), usually labeled `TransientTransactionError`; this remains
an `AppError::DatabaseError`, never `AuditIntegrityConflict`.

Use `audit_runtime::transaction::run(db, &context, callback)` to own a new
DB-only transaction. It uses snapshot reads and majority+journal commits. The
context holds immutable operation/event IDs, observed revisions and prepared
inputs **outside the callback**. The callback receives the same context and a
new transaction snapshot on each body retry. All business reads, live authority
fences, mutations and required audits join its supplied session.

| Failure | Required handling |
| --- | --- |
| Body/definite commit `TransientTransactionError`, or server WriteConflict 112 even without a label | Abort the failed transaction and retry the **whole business callback**, including its authorization/revision checks. The shared runner permits at most 32 body attempts, including the first, with randomized backoff capped at 160ms. |
| `UnknownTransactionCommitResult` | Retry **only commit**, with the same session/transaction number, at most 8 commit calls. Never rerun the callback or issue an abort because the outcome is unknown. An unknown result takes precedence over a simultaneous transient label. |
| Exhausted body budget | Return the database failure. Do not translate contention to integrity failure, pretend the operation committed, or perform effects after a failed/uncertain transaction. This bound changes no request/server/Mongo timeout. |
| Exhausted unknown-commit budget | Return database uncertainty; retain the logical operation identity and reconcile its durable receipt/state. Never mint another operation UUID or resend an effect. A possibly committed callback is not replayed by this runner. |
| `AuditIntegrityConflict` | Abort and return immediately. It means conflicting immutable identity, invalid hash/boundary/reservation, missing required evidence, overflow or other invariant failure. It is never a retry signal. |

`append_chained_entry` owns its transaction and uses this runner internally.
`append_chained_entry_in_session`, `log_actor_event_in_session`,
`log_actor_event_in_transaction`, enqueue and intent helpers cannot retry a
partial business transaction: propagate their errors to its owner. Existing
driver `and_run2` transaction owners already retry labeled transient errors and
unknown commits within the driver's finite deadline; retain the raw Mongo error
through `api_key_mutation_service::transaction_result` / `map_transaction_error`.
Lane conversions should use the explicit bounded runner above (including raw
112 handling), or request CORE review of an existing equivalent retry owner.
Never catch a write conflict and continue on that aborted session.

`service_history::transaction::run(db, async |transaction| { ... })` is an
equivalent bounded owner for existing journal-coupled mutations. Keep its
`Transaction` wrapper and **complete** domain/journal/audit callback; passing
`&mut *transaction` to core helpers joins that same Mongo session. Its former
deadline-only loops now share core contention classification, backoff and the
32-body/8-commit budgets, snapshot reads and majority+journal commits. The
existing 120-second retry cutoff remains an additional ceiling. Unknown commit
has priority over a simultaneous transient label and never causes abort/body
replay. `ConcurrentHeadCreation` additionally retries the whole journal callback
because its first-writer insert race is safe to retry. Implicit service-history
writes invoke the core runner directly with the same typed first-writer retry
classification, instead of maintaining a separate retry implementation. EventIds
and history context stay outside body retries; do not replace a larger caller
transaction with individual auto-owned journal writes.

The CAS `matched_count` guard remains an integrity check. Under a valid snapshot
transaction, another committed writer causes Mongo's write conflict rather than
a successful stale overwrite. Do not turn the guard into a surviving-tail
fallback, a fresh allocator, or an unconditional update.

The callback may execute repeatedly. It must contain **no network/provider/node
effect, secret response delivery, spawned required work, or fresh retry identity**.
Keep existing business replay receipts, unique constraints, funding IDs,
credential epochs, generation fences and first-writer rules. Transaction retry
does not supply cross-request business idempotency by itself.

## Replica fault-injection ownership

MongoDB's `failCommand` state (including its app-name selector) is server-global.
A process-local mutex does not isolate nextest processes. Tests must use
`crate::test_utils::fail_command::Guard::acquire(&fixture_db, command_handler)`
(`command_handler` is `None` or the existing Mongo command observer). Keep the
guard through every fault assertion. Execute the tested operations through
`guard.database().clone()`; that client has a unique guard-owned `appName`.

Arm with `guard.arm(doc! {"times": 2}, doc! {"failCommands": ["update"],
"errorCode": 112, "errorLabels": []}).await?`, or `"alwaysOn"` as the mode.
The guard inserts its own app-name restriction, rejects caller-defined scope,
and holds the shared kernel file lock across the complete script. Use
`guard.disarm().await?` between phases when useful. Scope exit and panic also
reset the server on a joined independent runtime before releasing ownership;
do not rely on a spawned reset that a short-lived Tokio runtime can cancel.
The lock serializes fault scripts; it does not change normal test parallelism.
An unrelated client's readiness/business writes must succeed while it is armed.
Do not add raw `configureFailPoint` calls or a new local mutex in lane files.

For an exact command boundary in a synchronous `CommandEvent::Started`
observer, install `guard.controller()` in the observer's `Arc<OnceLock<_>>`
before executing the tested operation. Its cloneable `FailCommandController`
offers blocking `arm(mode, data)` and `disarm()` with the same guard-owned
scope. Control commands run on a separate thread/runtime and a dedicated
single-connection client, without the tested client's callback or pool. The
monitored client also has exactly one pooled connection: a Started observer
arms while its target owns that connection, and concurrent operations queue
behind it. An asynchronous arm before racing two matching commands does not
select one by identity; use the target's Started boundary for that case. Calls
serialize with each other and teardown; Drop tracks even uncertain arming,
resets and joins the controller before unlocking. Controller clones retained
after Drop return an error and cannot arm another script. The controller does
not extend the guard's lifetime or transfer the kernel lock to a lane.

## Safe event and actor/owner construction

`audit_outbox_service::AuditEvent` accepts only:

- `operation_id`: stable UUID v4 for this logical operation;
- `stage`: `AuditStage` enum;
- `principal`: `AuditPrincipal::Person { id }` for a verified human;
  `Node { id }`, `ServiceAccount { id }`, `OracleWorker { id }` or
  `ApiKey { id, owner_id }` for an authenticated machine/key identity; or
  `System { actor: SystemAuditActor }` for Authentication, Startup, Maintenance,
  Reconciliation, Expiry or Delivery;
- `owner`: `AuditOwner::Person { id }`, `Org { id }` or fixed `Platform`;
- `action`: reviewed `&'static str`, bounded to 96 identifier characters;
- `target_id`: optional UUID v4, never a label or provider reference string;
- `detail`: optional, closed `AuditDetail`, attached with `event.with_detail(detail)` (Decision 6).

`System { actor: SystemAuditActor::Authentication }` records an account-authority mutation performed by the authentication service on an unauthenticated, request-driven input: self-registration, invitation consumption during registration, and password-reset-token issuance. `owner` is the server-created or server-loaded `Person` row the mutation affects; the requester is never attributed, and no email, IP or submitted identifier is stored. Any operation that verified a credential or redeemed a server-issued token (password login, MFA, reset redemption, email verification, forced reset by an admin) records `Person { id: verified subject }` (or the admin's principal), never `Authentication`.

The enqueue/intent path validates every UUID/action before reservation and again
before materialization. There is no arbitrary metadata/payload map. Never copy
credentials, wrapped keys/ciphertext, tokens, prompts, commands, transcripts,
message/upload/request/response bodies, provider errors or bearer-bearing URLs
into audit/outbox/intent records. Existing product payload storage remains
separate and follows its existing protection. Preserve reviewed legacy direct
event projections and v1 canonical bytes when keeping an existing direct audit.

Resolve polymorphic owner from the server-loaded domain row. For org ownership,
call `org_service::resolve_owner_access(db, verified_actor, stored_owner)` and
apply the operation's existing `can_write`, `can_proxy`, resource scope and other
live policy checks. `can_read` alone does not authorize a mutation; membership or
an event's owner field never grants authority. Preserve the verified actor
separately from the org owner. The `Person` principal enum records the verified
acting ID; it must not substitute the affected user for a background actor or
invent a human behind an org-owned credential.

Authenticated-machine attribution is explicit:

| Verified caller | Principal and source | Materialized compatibility fields |
| --- | --- | --- |
| Human session/access/delegated token | `Person { id: verified subject }`; the affected user is not the actor | `user_id = person id` |
| Node token or redeemed registration identity | `Node { id: server-resolved node UUID v4 }`; use the newly allocated node ID for successful registration, never the stored owner | `user_id = null` |
| Oracle worker | `OracleWorker { id: "{pool_uuid}:{worker_label}" }` from the authenticated/validated worker binding | `user_id = null` |
| Service-account token | `ServiceAccount { id: verified service-account UUID v4 }`; use the token subject, never `approval_owner_user_id` | `user_id = null` |
| API/agent key, including child credentials | `ApiKey { id: verified api_key_id, owner_id: verified key owner }`; retain the verified parent API-key identity used by `AuthUser` | `user_id = null`, `api_key_id = key id`, `api_key_name = null` |
| Unauthenticated account-lifecycle request | `System { actor: Authentication }`; owner = affected person row | `user_id = null` |
| Unauthenticated rejection or background work | Closed named `System` actor; never manufacture a verified identity from submitted IDs | `user_id = null` |

API-key owner can be a person **or org** and is recorded independently of the
resource's `event.owner`; no stored owner implies a human caller. This explicit
key principal is mandatory for new typed events instead of `Person` plus a lost
key attribution. Existing direct transactional audits retain their reviewed
`AuditActor`/`AuditLog.api_key_id`/name projection and v1 bytes. Machine/key
materialization does not copy user-authored key names or worker tokens. Principal
UUIDs (including key owner) validate as v4. Oracle's existing row identity is not
a UUID: require a canonical hyphenated v4 pool UUID, one colon, then 1–64 ASCII
letters/digits/underscore/hyphen, exactly as its protocol permits; no arbitrary
labels, URL, token or payload. Authority still comes from live authentication and
fences; syntactic validation is not authentication. An unverified submitted node
or worker ID belongs in neither a verified principal nor an invented `Person`.

These are additive, tagged JSON variants; old Person/System serialized bytes and
the existing v1 HMAC encoder remain unchanged. The typed event's principal is
already part of its canonical JSON and immutable replay comparison; changing a
principal conflicts. New version/key/checkpoint metadata and its length-framed
encoding remain W8-5b work; do not rewrite or re-sign historical v1 entries.

For system work use the named system principal and owner from the authorized
domain row (or Platform for platform maintenance). Do not impersonate the
affected user. HTTP IP attribution continues to use strict `resolve_client_ip`;
do not add a header/legacy fallback or an arbitrary string field to this event.

## Closed security decision detail (Decision 6)

`SafeAuditEvent.detail` is optional and omitted entirely when absent. Historical
six-field producer literals, JSON/BSON bytes, v1 canonical bytes and their replay
semantics remain unchanged. `AuditEvent::with_detail` returns
`DetailedAuditEvent`; both producers use the same sealed `AuditEventSource`
enqueue interface. `EffectReservation<E = AuditEvent>` also accepts the detailed
producer. No stored model or arbitrary JSON implements this producer interface.

Detail is **required** when the legacy event carried an enumerable security
value: mode, role, approval mode, scope/allowlist delta, security flag or expiry.
Omit it when legacy metadata contained only already-projected identities or
content. Never copy the latter. Compute before/after values from the **same
transaction snapshot** as the mutation, not the raw request patch. Detail is
part of immutable replay equality; a changed value for the same operation/stage
is `AuditIntegrityConflict`. It persists through materialization and effect
terminal events, and contributes to the new row's hash. No historical row is
rewritten or re-signed; the witness still receives hashes only.

| Variant | Fields / meaning |
| --- | --- |
| `AccessModeChange` | `scope: Workspace/Agent`, `old/new: Option<assistant_settings::AccessMode>` (`autopilot/nyxmode/ask_every_time`). **None = inherited from workspace** for agent scope. Workspace requires both Some; equal old/new (including None/None) is rejected. This is the current settings enum, not the retired conversation Ask/Full enum. |
| `NodeBindingPriorityChange` | `old/new: i32`, the binding model's full -2,147,483,648–2,147,483,647 range (no narrower product bound). Lower values sort first; equal values preserve idempotent binding updates. Derive the old value from the binding loaded inside the transaction. |
| `OrgRoleChange` | Existing `OrgRole` old/new (`admin/member/viewer`). |
| `ApprovalModeChange` | `AuditApprovalMode` old/new (`writes_only/all/off`); `writes_only` preserves the global stored tri-state None, rather than treating it as off. |
| `ServiceApprovalModeChange` | Existing per-service `ApprovalMode` old/new (`per_request/grant`). |
| `SecurityFlagChange` | Closed `SecurityFlag`: `MfaEnabled`, `EmailVerified`, `SkipDestructiveConfirmation`, `AllowAutoConnectedServices`, `BrokerEnabled`; bool old/new from effective stored policy. |
| `ScopeSetChange` | `added/removed: Vec<ScopeIdent>`, `truncated: bool`; identifiers must belong to the server OIDC/delegation registries or the reviewed write/admin/account:read/sandbox:execute extensions. Use `audit_outbox_service::scope_ident`. |
| `ResourceSetChange` | `kind: ResourceSetKind` (`Services/Nodes/Agents/OrgMembers`), UUID-v4 added/removed, truncated. JSON calls the resource field `resource_kind` because the enum's internal `kind` tag already identifies `resource_set_change`. |
| `ExpiryChange` | Optional old/new BSON datetimes, millisecond precision; None = no expiry. |
| `SessionRevocationCount` | `count: u64 <= 1_000_000`, `saturated: bool`. Use `session_revocation_count(update_many.modified_count)` from `revoke_all_user_sessions_in_session`, never request input. Actual counts above the cap become count=1_000_000/saturated=true; saturated implies count=cap. Zero is valid. |
| `NotificationSettingsChange` | `old/new: NotificationSettingsDecision`: `telegram_enabled`, `push_enabled`, `approval_suspended`, `approval_delivery_configured` booleans; `global_approval_mode: AuditApprovalMode`; `approval_timeout_secs: u32` in 10–300; `grant_expiry_days: u32` in 1–365. Derive both snapshots inside the authorized mutation transaction. Preserve legacy global None as `WritesOnly`. No destinations or credentials. |
| `WebhookDeliveryFailure` | `event: WebhookEventKind` (`ConnectionExpired/ConnectLinkCompleted/ConnectLinkCancelled/ConnectLinkExpired`); `reason: WebhookFailureKind` (`TenantIdentityMissing/ConfigurationLoadFailed/RateLimitUnavailable/AppRateLimited/KeyIdLoadFailed/KeyIdStoreFailed/SecretDecryptFailed/SerializationFailed/BodyTooLarge/DestinationUnavailable/SendFailed/NonSuccessStatus`); `attempts: u8` in 0–3; `last_http_status: Option<u16>` in 100–599. Only Failed/Uncertain terminal events. The bounded counter records evidence; it grants no retry authority. No provider text, URLs or response bodies. |
| `AcknowledgementDecision` | `decision: AcknowledgementDecision` (`Allow/Deny`); `card_kind: AcknowledgementCardKind` (`Action/Service/Account/Operations/Skills/Machine/SavedLogin`); `approver_kind: AcknowledgementApproverKind` (`Owner/OwnerChannel/Nyxbot/ApiKey`). Use the server-loaded card and verified deciding authority for HTTP, NyxBot, channel and voice paths; never user-authored labels or an unknown-kind fallback. |

Every variant denies unknown kinds/fields. Scope identifiers use only
`[a-z0-9:_.-]`, 1–64 bytes, and must be server-registered; free-form provider
scopes, wildcard scopes, user-authored labels/names/emails/URLs and body content
are unsupported. Each vector has at most 32 sorted unique entries; added and
removed must be disjoint. Compute the complete delta first, sort, retain bounded
prefixes and set `truncated=true` when clipping. Total serialized detail must
also fit 2 KiB: shorten the retained sorted prefixes further if necessary and
keep truncated=true. A truncation flag never permits oversized records. Enqueue
and materialization both enforce all bounds; materialization checks persisted
input again. Expiry values must already have millisecond precision to preserve
JSON/BSON/replay equality.

For any additional optional override, request an Option-valued variant with
explicit None/inheritance semantics; do not coerce inheritance into a concrete
mode or false. Lanes request new centrally owned variants through an inventory
row (name, fields/types/bounds, example site), never define local variants or
reintroduce free-form event data. Affected conversions remain
`detail-pending: <Variant>` until the lane wires and tests them; adding this
contract alone does not complete those workflows.

Example inside the transaction, after loading and authorizing the before/after
settings:

```rust
let event = AuditEvent { /* existing six fields */ }.with_detail(
    AuditDetail::AccessModeChange {
        scope: AccessModeScope::Workspace,
        old: Some(before.access_mode),
        new: Some(after.access_mode),
    },
);
audit_outbox_service::enqueue_admitted_in_session(
    db, session, runtime, tier, &event_id, &event,
).await?;
```

## T and deferred O

Preferred entrypoint:

```rust
audit_outbox_service::enqueue_admitted_in_session(
    db, session, runtime, tier, &event_id, &event,
).await?;
```

This validates replay identity, checks admission for **one new slot**, and
reserves the typed event in the same transaction as the caller's mutation.
Identical replay returns the original event ID without a new reservation or new
admission, even if new work is now closed. Changed content is an integrity error.
Local materialization later commits audit row/head/delivery marker atomically;
it cannot release witness backlog. Only authenticated external progress does.

A minimal callback shape (all IDs and inputs prepared before entering `run`):

```rust
let context = (db, runtime, event_id, &event);
audit_runtime::transaction::run(db, &context, |session, (db, runtime, id, event)| {
    Box::pin(async move {
        // Revalidate and perform the actual business mutation using session.
        audit_outbox_service::enqueue_admitted_in_session(
            db, session, runtime, AuditTier::Standard, id, event,
        ).await
    })
}).await?;
```

For **existing direct transactional audits**, keep the original audit and
business session. Call `runtime.require_new_in_session(db, session, tier, slots)`
for the complete new bundle before the direct append(s). That check is not a
reservation: the actual append/outbox insert must follow in the **same**
transaction. Do not add a second queued copy of an already-atomic direct audit.
Ensure a durable business replay is recognized before charging new capacity.

The low-level `enqueue_in_session` is reserved for producers that already own a
correct complete admission/reservation boundary (notably two-slot I). It is not
a caller-lane admission bypass. No critical `log_async`/postcommit task counts as
T/O. A deferred business job additionally needs its own durable job/product
identity and recoverable payload reference in the business transaction; the
safe audit outbox deliberately cannot store the job payload.

## I, consumed dispatch permit and completion

Allocate one `intent::EffectReservation` outside retry loops: a safe `event`
with stage `Intent`, `event_id: Uuid`, and distinct `terminal_event_id: Uuid`.
Keep the logical operation UUID stable across request, worker and process
recovery. Commit the business mutation with:

```rust
runtime.begin_effect_in_session(db, session, owning_db_tier, &reservation).await?;
```

This reserves **two slots atomically**: the intent and its future terminal event.
Choose the DB tier of the owning operation; dispatch is separately Effect-gated.
The empty terminal slot participates in pending count/age before it has an audit
sequence. Witnessing the intent does not release that slot.

After a confirmed commit, obtain authorization outside the retry callback:

```rust
let permit = runtime.authorize_effect(db, &operation_id).await?;
let result = permit.run(|| async { /* one provider/node effect */ }).await?;
```

Configured mode requires materialized and independently witnessed intent plus
an authenticated operation/attempt/process claim. `DispatchPermit` has private
construction, is not Clone/Serialize, and `run(self, ...)` consumes it while
rechecking durable attempt, terminal state and live admission immediately before
the effect. No wrapper may extract a reusable boolean, clone authority, detach
the send, or rebuild a permit from a retrieved receipt. Do not nest duplicate
intents at producer and shared transport; pass the same context/permit to the
single effect owner. C1 local mode remains explicitly unwitnessed and still has
durable local intent/attempt controls.

Commit domain outcome plus
`runtime.finish_effect_in_session(db, session, operation_id, stage)` using the
bounded transaction runner. Terminal stages are Succeeded, Failed, Cancelled or
Uncertain; the helper consumes the pre-reserved slot without new admission, even
after closure. Identical terminal replay is idempotent; conflicting outcomes
fail. Cancelled requires an unclaimed pending intent; Succeeded requires the
claim owned by this process. A timeout, lost response, crash or recovered claim
without positive terminal proof is **Uncertain**, never automatic Failed/retry.

For webhook failure evidence known only after dispatch, use
`runtime.finish_effect_with_detail_in_session(db, session, operation_id, stage, Some(detail))`
in that same product-receipt transaction. Its `detail` parameter is
`Option<AuditDetail>`: `Some` accepts only `WebhookDeliveryFailure` on Failed or
Uncertain, and only when the original intent has no detail. It cannot replace
the original intent's policy decision. `None` preserves the intent's existing
detail and is the exact behavior of `finish_effect_in_session`. The immutable
intent remains unchanged; separate `terminal_detail` evidence and the terminal
outbox event commit atomically with the outcome. Replay must supply the same
stage and detail, including presence/absence, and match the complete terminal
event and reserved slot. Changed or omitted recorded detail conflicts. Enqueue
and materialization validate the terminal detail again. No terminal addition
creates a claim, reopens a terminal result or permits a second dispatch. Request
additional terminal variants centrally; ordinary policy variants cannot be
introduced through this post-dispatch API.

The shared [delivery interface](SECURITY_W8_DELIVERY_CONTRACT.md) uses this same
runtime. Startup registers its product/destination/channel dedupe indexes after
the database DDL and before migrations, workers or producers; a missing index
continues to fail closed at enqueue. Sink owners join the product receipt and
core terminal completion in one transaction rather than introducing a second
outcome state machine.

The ID account payload store uses `account_delivery_payloads_v1`. Startup DDL
creates `account_delivery_user_kind_revision`, unique on
`(user_id, kind, revision_id)`, and `account_delivery_expiry`, a nonunique scan
index on `(expires_at, _id)`. Neither is a TTL index: expiry does not authorize
automatic removal of payloads needed by unsettled delivery or reconciliation.
Until the producer model lands, the collection name is available as
`db::ACCOUNT_DELIVERY_PAYLOADS_COLLECTION_NAME` for its model to re-export;
the startup commit does not depend on that producer module.

Dropping a permit, lease expiry, Mongo restore or a new process does not reopen
dispatch. The protected claim prevents reassignment; recovery records uncertainty
without resending. Preserve provider-supported idempotency and positive provider
reconciliation. A later authoritative reconciliation needs a reviewed domain
transition/evidence API; do not overwrite terminal uncertainty or issue a new
operation UUID to evade the old claim.

## Tier and lane acceptance

Narrowing always passes witness-policy admission. Availability and Standard
gate on persisted pending age/count at their separate hard ceilings; contact
freshness only alarms DB-only work. Divergence permits bounded Availability and
Narrowing, refuses Standard, and supplies no Effect claim. Effect requires live
intent/claim evidence. Preserve the approved C2 defaults, generic 13003/13004,
operator alarms and explicit unwitnessed state; see
[SECURITY_AUDIT_WITNESS.md](SECURITY_AUDIT_WITNESS.md).

Split mixed operations at the actual boundary: local revoke/disable commits as
Narrowing even when a downstream provider revoke/control message must defer as
Effect. Do not make the local revocation wait for a witness claim or move a
credential/consent grant into Narrowing to avoid Standard admission.

Every lane batch must test abort/commit/replay, its tier/state cells and authority
fences, explicit human/machine/system attribution, safe records, and HTTP/non-HTTP
entrypoints. I/O conversions additionally cover before/after intent, claim,
effect and outcome; restored Mongo; paused old sender; duplicate delivery; and
honest uncertain reconciliation. Update inventory workflow status only after
the complete caller/effect boundary has been implemented and tested. No pending
critical workflow can be waived by passing infrastructure tests.

The manual ignored benchmark is
`audit_append_throughput_previous_vs_durable`; run it with both KMS features,
`-- --ignored --nocapture --test-threads=1`. It compares a test-only copy of the
previous direct appender at `866f289df` with the current public path, on the same
Mongo replica and machine: 256 appends, three alternating-order rounds each at
one and sixteen tasks. Setup and full verification are outside measured time;
the new path retains all head/outbox/HMAC/durability controls. The previous
algorithm retains its original eight-attempt budget: record exhausted baseline
operations separately, verify its surviving contiguous chain, and never retry
them externally to manufacture success. The durable path must complete every
operation. A baseline with failures has no all-success throughput comparison;
label its successful-throughput ratio accordingly. Record medians and any
slowdown above roughly 2x in the W8 log before PM fanout. Debug/local single-node
figures from the shared development host describe this comparison, not
production capacity.
