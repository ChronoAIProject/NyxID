# Platform services v2

This document specifies the target design for NyxID-managed vendor credentials and
per-operation billing. It replaces the separate `platform-*` catalog-service model.
The existing catalog service is the provider identity for both bring-your-own-key
(BYOK) connections and NyxID-managed access. A platform credential is an alternate
credential source for that catalog identity, not a second service.

The `experimental:platform-services` feature flag remains the caller-facing rollout
gate. Admin configuration and reconciliation remain available while the flag is off.

## Data model

The two collections below organize the subsystem. Catalog rows describe providers,
`platform_credentials` holds shared secrets, and `platform_operations` describes the
only operations for which a shared secret may be selected.

### Platform credentials

`platform_credentials` is the only storage location for a NyxID-managed provider
credential:

```rust
PlatformCredential {
    _id: String,                    // UUID v4
    catalog_service_id: String,     // unique DownstreamService._id
    credential_encrypted: Vec<u8>,
    auth_method: String,
    auth_key_name: String,
    created_by: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
```

The collection has a unique index on `catalog_service_id`. Credential writes validate
that the referenced catalog row is active and is registered in the code-owned
platform-provider safety registry. The credential is encrypted with `EncryptionKeys`.
The model has a manually redacted `Debug` implementation, and no response object ever
contains the ciphertext, credential text, or a reversible key identifier.

No catalog, proxy, MCP, auto-provision, or key-list resolver reads this collection.
Those paths may ask `platform_credential_service` whether a credential is configured,
but they cannot fetch or decrypt it. The service exposes exactly two execution
authorizers:

```rust
authorize_endpoint(db, catalog_service_id, method, canonical_path)
    -> (AuthorizedPlatformCredential, PlatformOperationRow)

authorize_constrained(db, catalog_service_id, constrained_op)
    -> (AuthorizedPlatformCredential, PlatformOperationRow)
```

`AuthorizedPlatformCredential` is a non-serializable type with private fields. Its
secret is held in `Zeroizing`; its `Debug` output is redacted. It can be converted into
the bounded proxy target required by the caller, but callers cannot construct it or
decrypt a `PlatformCredential` directly.

Authorization and decryption are one operation. `authorize_endpoint` succeeds only
after an enabled endpoint row for the same catalog service matches the exact method and
canonical path. `authorize_constrained` succeeds only after an enabled constrained row
for the same catalog service and code-owned operation exists. Missing credentials,
disabled rows, mismatches, and invalid catalog associations fail before decryption.

`DownstreamService.credential_encrypted` remains for unrelated legacy/internal catalog
services, but is not a platform-vendor credential store. There are no live
`platform-*` provider rows, vendor-template records, vendor slug guards, or special
deny-all proxy policies in the v2 design.

### Platform operations

`platform_operations` contains one row per catalog provider and operation:

```rust
PlatformOperationRow {
    _id: String,                    // UUID v4
    catalog_service_id: String,
    kind_key: String,               // service-derived, not accepted from APIs
    enabled: bool,
    kind: PlatformOperationKind,    // serde tag: kind
    limits: OperationLimits,
    billing: OperationBilling,
    billing_cleanup_metric_code: Option<String>,
    created_by: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

PlatformOperationKind {
    Endpoint {
        method: String,
        path_template: String,
        name: String,
        description: Option<String>,
    },
    Constrained {
        op: ConstrainedOp,
        config: ConstrainedConfig,
    },
}

ConstrainedOp = Speak | CallAndSay | FlightSearch

OperationLimits {
    per_request: PerRequestCaps,
    per_user_per_day: Option<u32>,
}

OperationBilling {
    metric: BillingMetric,
    price_per_unit: String,
    base_fee_per_call: Option<String>,
    lago_metric_code: String,
    sync_status: PricingSyncStatus,
    sync_error: Option<String>,
}
```

`billing_cleanup_metric_code` is the durable cleanup marker for an obsolete Lago
charge. It is separate from the active billing tuple so an admin edit can make the new
price authoritative immediately while reconciliation keeps retrying removal of the old
metric. The marker is cleared only after Lago removal and local rate-cache cleanup both
succeed.

`kind_key` is persisted because MongoDB cannot create the required unique index from a
computed tagged-enum expression. It is derived on every write and never trusted from an
HTTP body:

```text
endpoint:{METHOD} {normalized_path_template}
constrained:{snake_case_op}
```

The unique `(catalog_service_id, kind_key)` index makes retries and migration upserts
converge on one row. Additional indexes support enabled rows by catalog service and
pending/failed Lago synchronization.

`PerRequestCaps` is tagged and must match the operation kind. Endpoint rows use the
ordinary authenticated proxy body and response bounds. Constrained rows retain typed
caps: speak text characters, call message characters and call-duration ceiling, and
flight-search result count. Config owns non-cap settings: speak model and voice
allowlist, Twilio destination-prefix allowlist and vendor identity, and the bounded
flight-search request contract. Hard maxima remain code-owned. Admin values may lower
them but never raise them. Unknown fields and a cap/config variant that does not match
the operation are rejected.

The allowlist for a provider is exactly its enabled `Endpoint` rows. It is empty by
default. Creating a credential does not create an endpoint row, and enabling a
constrained operation does not authorize the equivalent generic vendor endpoint.

Constrained rows can only be created from the code-owned registry. Administrators
cannot introduce a fourth constrained operation or change its provider binding. The
registry binds:

| Constrained operation | Catalog service | Why it remains constrained |
| --- | --- | --- |
| `speak` | `api-elevenlabs` | The server validates text and voice, composes the body, and bounds streamed audio. |
| `call_and_say` | `api-twilio` | The server owns caller identity and TwiML and excludes callbacks, recording, and arbitrary form fields. |
| `flight_search` | `duffel` | The server admits search only and projects a bounded response; booking and payment stay unreachable. |

X search is not a constrained operation. It migrates to the endpoint row
`GET /2/tweets/search/recent` on `api-twitter`.

### Billing extensions

`BillingMetric` adds `Characters` and `Seconds`. `PlatformUsage` adds non-negative
`characters` and `seconds` counters, and `platform_quantity` maps those variants just
as it already maps requests, bytes, and tokens. The stable legacy metric codes gain
`platform_characters` and `platform_seconds`.

Each operation receives a stable code:

```text
platform_op_{catalog_slug}_{kind_key_slug}
```

The slugging function is deterministic, bounded, collision-tested, and derived from
the normalized kind key. The operation row, rather than `DownstreamService.billing`,
is the NyxID price authority. Prices are normalized decimal credit strings with no
floating-point conversion. The normal save path attempts to upsert the Lago sum metric
and standard plan charge. Pending and failed writes are retried by billing
reconciliation. A stale completion cannot overwrite a newer admin edit. Removing or
replacing a metric retains a durable cleanup marker until the old plan charge and rate
cache entry are removed.

The execution adapter must supply a meaningful quantity for the selected metric.
`characters` counts Unicode scalar values in the validated request text and is known
before forwarding. `seconds` is supported by `call_and_say` and comes from Twilio's
completed-call duration. Invalid metric/operation combinations are rejected at admin
write rather than silently producing a zero quantity.

`UsageMeterRow` gains `base_fee_micros: Option<i64>` and a tagged deferred descriptor:

```rust
DeferredQuantity::TwilioCall {
    account_sid: String,
    call_sid: String,
}
```

The base fee and per-unit price are snapshotted when the meter opens. Reservation size
is:

```text
base fee + estimated quantity * unit price
```

For known quantities, the estimate is the already validated quantity. For a Twilio
call, the estimate is the configured, code-capped maximum duration and the server sends
the corresponding duration ceiling to Twilio. Quantity allowances cover quantity
units; the credit-denominated base fee is funded by grants and then wallet credits.

Settlement remains one `UsageMeterRow`, one transaction ID, and one eventual
`usage_settled` ledger entry. The charged amount is:

```text
base fee + actual quantity * unit price
```

The funding/wallet transition includes an idempotent base-fee-applied marker so a
Twilio row can settle its base fee when the vendor accepts the call while retaining the
same row and the bounded quantity hold. The row remains `forwarded`; no second meter or
ledger row is created. Final quantity settlement consumes the remaining hold, releases
unused reservation, and cannot apply the base twice. Billing-ledger canonical encoding
adds the new metric names and base-fee field only for the new encoding version, so
verification of historical entries remains byte-for-byte stable.

### Deferred Twilio seconds

After a successful Twilio create-call response, NyxID validates the returned call SID,
persists `DeferredQuantity::TwilioCall` on the forwarded meter row, and settles the
snapshotted base fee. The response can then return without waiting for call completion.

The existing billing reconcile sweep claims due deferred rows in bounded batches. For
each row it re-authorizes the `call_and_say` constrained operation through
`platform_credential_service` and polls:

```text
GET /2010-04-01/Accounts/{account_sid}/Calls/{call_sid}.json
```

The account and call SIDs are validated opaque identifiers and are never placed in
audit payloads. A completed response with a valid non-negative duration finalizes the
same row with `seconds = duration`. Non-terminal responses and transient failures leave
the descriptor in place with bounded retry scheduling. If the row is still unresolved
24 hours after forwarding, reconciliation atomically claims it, finalizes quantity
zero, retains the already settled base fee, clears the descriptor, and emits the
metadata-only `platform_call_duration_unresolved` audit event.

Every transition filters on the row ID, `status = forwarded`, and the exact deferred
descriptor. A replay therefore observes the completed state and becomes a no-op. If an
administrator disables the constrained row or replaces the credential while a call is
pending, polling fails closed and retries; the 24-hour base-only terminal rule still
prevents an immortal hold.

## Endpoint authorization

Endpoint authorization is intentionally source-specific. It controls access to the
platform credential; it does not modify `DownstreamService.proxy_operation_policy` and
does not restrict a user's own credential.

Methods are normalized to uppercase. The safe-method registry permits `GET` and `HEAD`
for registered platform providers. A `POST` is permitted only for an exact
code-registered safe provider/template pair, initially Duffel
`POST /air/offer_requests`. `PUT`, `PATCH`, `DELETE`, `OPTIONS`, WebSocket upgrades, and
unregistered POST templates are rejected at admin write. This registry is checked
again when loading an operation for authorization so a malformed database row cannot
broaden access.

Path templates use the existing canonical proxy-path semantics without using a catalog
row's proxy policy:

- Templates are root-anchored and query-free.
- Static segments are case-sensitive and exact.
- A full segment such as `{id}` matches exactly one non-empty canonical segment.
- Globs, regex, alternation, partial placeholders, empty segments, dot segments,
  backslashes, encoded separators, and ambiguous trailing slashes are rejected.
- The query string is forwarded but is not part of the path match.

REST UUID, REST slug, typed MCP, and generic MCP entry points first run their existing
source-specific canonicalization. They pass the same decoded, root-anchored canonical
path to `authorize_endpoint`. Parity tests must prove that equivalent REST and MCP
requests select the same row and forward byte-equivalent paths, and that ambiguous
encodings fail before credential decryption.

## Alternatives considered

| Decision area | Alternative | Result | Reason |
| --- | --- | --- | --- |
| Credential store | Reuse `DownstreamService.credential_encrypted` on separate `platform-*` rows | Rejected | It duplicates provider identities, mixes catalog and secret objects in `/keys` and Admin Services, and requires broad exclusion guards on every generic resolver. |
| Credential store | Put a system-owned `UserApiKey` behind a synthetic `UserService` | Rejected | User ACL, lifecycle, agent binding, and auto-provision rules would treat a shared platform secret as a user connection and make precedence harder to audit. |
| Credential store | Environment variables per provider | Rejected | It cannot support admin replacement, per-provider status, database migration, or multi-replica convergence without a separate control plane. |
| Credential store | Dedicated `platform_credentials` keyed by existing catalog service | Chosen | It gives one provider identity, one secret home, a unique association, write-only admin behavior, and an enforceable service-layer decryption boundary. |
| Allowlist matching | Keep an empty/non-empty `proxy_operation_policy` on the catalog row | Rejected | A catalog policy applies to BYOK traffic too; the new allowlist must constrain only selection of the platform credential. Existing policies serve unrelated services and stay untouched. |
| Allowlist matching | Authorize by `ServiceEndpoint._id` only | Rejected | Raw REST proxy requests have method/path but no endpoint ID, free-form admin entries would not fit, and a stale or regenerated OpenAPI row could silently change identity. |
| Allowlist matching | Regex or glob paths | Rejected | Expressive patterns create overlap, encoding, and review hazards and make REST/MCP parity difficult to prove. |
| Allowlist matching | Exact method plus canonical segment template | Chosen | It supports ordinary resource IDs while remaining anchored, deterministic, source-independent, and easy to deny by default. |
| Deferred seconds | Hold the HTTP request open until Twilio completes | Rejected | Calls can outlive request, proxy, and deploy lifetimes; disconnect behavior would make billing nondeterministic. |
| Deferred seconds | Charge the base and duration in two usage rows | Rejected | Two transactions complicate wallet holds, grant precedence, Lago drift, ledger interpretation, and idempotency for one vendor action. |
| Deferred seconds | Add a separate job collection or depend on Twilio webhooks | Rejected | A second durable queue duplicates the existing meter state machine, while webhooks add public ingress, secret verification, and delivery configuration for data NyxID can poll. |
| Deferred seconds | Keep one forwarded meter row with a tagged deferred descriptor | Chosen | The existing reconciliation and settlement machinery can retry and converge around one transaction, one funding record, and one ledger entry. |

## Resolution and precedence

Platform fallback is considered only after normal caller and connection authority is
known. The order is:

1. Check the platform-services rollout flag when the request would discover or select
   a platform credential.
2. Resolve owner/org access, agent-key service scope, and the ordinary UserService
   cascade.
3. Apply `AgentServiceBinding` before classifying an active own connection.
4. If a usable own server credential exists, use it and disable platform billing.
5. If the connection is absent, disabled, or invisible to the scoped agent key, ask the
   platform credential service to authorize the exact endpoint or constrained op.
6. If an active own connection is unusable, do not fall back. Return the existing
   source-specific platform-operation error where applicable, or the ordinary proxy
   resolver error on a normal proxy request.
7. Evaluate approvals with the same operation descriptor used by the existing door.
8. Only after authority succeeds, reserve billing and obtain the authorized platform
   credential.

A scoped agent key's out-of-scope connection is **denied**, not routed to the platform
credential. Falling back would mean narrowing a key's scope increased its ability to
spend the owner's credits, turning deny-by-scope into pay-by-scope. Execution returns
`PlatformOperationOwnConnectionOutOfScope` (11805, HTTP 409); discovery reports the row
as the owner's own connection with reason `out_of_scope`, because reporting it as the
platform source would promise a billed call that will in fact be refused.

An active revoked, expired, missing, or unreadable user credential is visible but
unusable and blocks fallback. Platform responses set `X-NyxID-Credential-Source`; MCP
results carry the same source in structured metadata.

An org-owned agent key authenticates as the org, so the org is both the resolution
identity and the payer. A member is never billed personally for work done under an org
key.

| Door | Own connection | Platform fallback |
| --- | --- | --- |
| `/proxy/{id}` and `/proxy/s/{slug}` HTTP | Existing direct or node path, agent binding, scope, and approval behavior stays authoritative. | Only authenticated HTTP requests, only after absent/disabled resolution, and only for an enabled endpoint match. |
| Generic/typed MCP proxy tool | Existing prepared-call, exact-operation, scope, binding, approval, and node behavior stays authoritative. | The prepared canonical method/path must match an enabled endpoint row before billing or decryption. |
| `speak`, `call_and_say`, `flight_search` HTTP and `nyx__*` | Server-held own credential wins and skips credits. Per-request approval requirements retain the existing fail-closed constrained-op behavior; session bypass remains unchanged. | Requires the enabled constrained row and uses its limits, config, and billing. |
| Node-routed own request | Ordinary generic proxy/MCP node execution remains available. Constrained operations retain their existing unsupported error. | Never selected after a node route exists, including when the node is offline or a fallback node fails. |
| Anonymous proxy and public MCP | Existing forced no-auth target only. | Never reads, tests, authorizes, or decrypts a platform credential. |

For ordinary proxy traffic, failure to authorize platform fallback is deliberately
indistinguishable from today's connect-first/not-found result. Credential presence and
allowlist contents are not exposed through error shape or timing-sensitive preflight
APIs.

## Discovery and user keys

`GET /api/v1/keys` is the single user-facing provider inventory. It never persists a
synthetic UserService for platform access.

For a catalog provider with at least one enabled platform operation:

- With no own row, return one synthesized `KeyInfo` using the catalog slug and identity,
  `platform_managed: true`, `auto_connected: true`, and platform operation summaries.
- With an own row, return that row once and attach the platform summary. Do not append a
  second vendor object.
- An active usable own row reports `own_connection` and explains that its credential
  powers the enabled operations without credits.
- A disabled own row reports `platform_fallback`, the current price summary, and the
  normal Enable action.
- An unusable, node-routed, or approval-required own row reports a typed reason and the
  View connection action; it does not claim that platform fallback will occur.

The additive response shape is:

```text
KeyInfo {
    platform_managed: bool,
    auto_connected: bool,
    platform: Option<PlatformKeySummary>,
    // existing KeyInfo fields remain
}

PlatformKeySummary {
    operations: Vec<{ name, kind, price_label }>,
    credential_source:
        platform
        | own_connection
        | platform_fallback
        | unusable(reason),
}
```

A synthesized row uses the catalog slug and a stable synthetic response ID, but is not
accepted by mutation handlers as if it were a `UserService`. Its primary treatment is
"Platform managed" with "Connect your own". An active usable own row shows
"Powers {N} platform operation(s) - your credential, no credits". A disabled own row
shows "Platform credential in use - {price}" and Enable. Unusable, node-routed, and
approval-required rows show their reason and View connection. The rendered separators
may use the design system's middle-dot glyph; the wording and state meanings are fixed.

Each operation summary includes `name`, `kind`, and a server-formatted `price_label`.
The backend remains authoritative for price wording so `/keys`, platform discovery,
and MCP descriptions cannot disagree.

The separate "Platform services" section and its `/keys` discovery query are deleted.
`GET /api/v1/platform-ops` remains for agents and MCP clients, but returns the same
resolved operation/source projections rather than the old four-operation cards.
Discovery is read-only: it does not decrypt credentials, refresh OAuth, mutate
last-used state, create approvals, or open meters.

## Admin surface

`/admin/platform-ops` is a table with Provider, Operation, Kind, Enabled, Metric,
Price, Limits, and Edit columns.

The header actions are:

- **Add endpoint**: choose a catalog service, then a stored `ServiceEndpoint`, a hosted
  OpenAPI operation, or a free method/path. Every choice is normalized and checked
  against the code-owned safe-method/POST registry before persistence.
- **Add operation**: choose one of the three code-owned constrained operations that is
  not already present for its provider.
- **Platform credential**: set or replace the credential for a provider. The response
  reports only configured state and timestamps; it never reads the secret back.

The edit modal groups Per-request caps, Per-user quotas, Allowlists, Vendor identity,
and Billing. Each cap shows its code-owned maximum. Endpoint rows have no voice or
destination allowlists. The billing group edits metric, price per unit, and base fee.
Its preset picker is the distinct normalized `(metric, price, base fee)` set computed
from other operation rows; presets have no collection and applying one just fills the
form.

Frontend mutations use `useAppForm`, Zod schemas in `schemas/platform-ops.ts`, and
TanStack Query hooks in `hooks/use-platform-ops.ts`. Admin Services shows only the
existing catalog row. If a platform credential exists, the row gains a
"Platform credential configured - N operations" badge linking to the operations table
(rendered with the design system's middle-dot separator).

## Execution order

A platform-credential request follows this side-effect order:

1. Rollout flag and caller class.
2. `platform:spend` scope, then agent-key rate and service scope.
3. Existing own-connection cascade and binding resolution.
4. Canonical operation construction.
5. Enabled exact endpoint or constrained-row authorization.
6. Approval evaluation, for the platform credential as well as an own connection.
7. Typed limits, allowlists, and daily-quota reservation.
8. Billing reservation using the operation price snapshot.
9. Platform credential decryption through the authorized wrapper.
10. Meter `mark_forwarded` immediately before provider dispatch.
11. Immediate settlement for known quantities, or persisted deferred Twilio state.

### Daily caps

Every constrained operation carries a per-user daily cap, including `speak`, which
previously had none. Speak is priced per character, so a per-call count is a coarse
bound; it is still the difference between a looping agent spending a bounded amount and
spending until the wallet stops it.

The cap is enforced by a conditional upsert against `platform_op_usage`. The unique
`(op, user_id, yyyymmdd)` index is load-bearing: without it a second reservation inserts
a second row instead of failing, and the cap silently does nothing.

### Shared-credential rate limiting

Platform execution applies `PLATFORM_SERVICE_RATE_LIMIT_PER_SECOND` per `(catalog
service, owner)` before any quota row or wallet reservation is taken. It is off by
default, matching the limiter's existing use on the master-credential proxy path.

This matters more here than on a BYOK path. A vendor rejection releases both the billing
reservation and the daily quota slot, so failed attempts are free and quota-neutral.
Without a limiter, one tenant could burn the shared vendor's rate allowance for every
other tenant at no cost to themselves.

The limit is per tenant, not aggregate across tenants. A ceiling on total platform
throughput against one vendor does not exist yet.

Known gap: the call site itself is not unit-tested, because the limiter is a
process-wide `OnceLock` and initializing it in one test would leak into every other
test in the process. `enforce_platform_user_limit` is unit-tested through its explicit
limiter seam. Making the call site testable means moving the limiter onto `AppState`,
which would also have to move the proxy path to avoid two mechanisms for one limit.

### Spend authority

Executing a platform operation requires the `platform:spend` API-key or access-token
scope. Discovery does not: an agent may learn that a service exists without being
allowed to pay for it.

The scope is separate from `proxy` on purpose. Reaching a service with a credential the
owner already holds, and directing NyxID to pay a vendor on the owner's behalf, are
different grants. A browser session is exempt because it is the owner acting directly.

`/api/v1/platform-ops` is exempt from the management write-scope gate in `mw/auth.rs`
because it is execution-shaped, not because it is ungated. The spend scope is its gate.

### Approval parity

Platform-funded execution evaluates the owner's approval policy exactly as an
own-credential call does. Because platform execution has no `UserService` row, the
policy is keyed on the catalog provider's service ID.

`is_auto_connected` is deliberately false on this path. It suppresses the owner's global
"require approval for everything" flag, which would otherwise leave the one path that
spends their money as the only unprompted one.

Any failure before `mark_forwarded` releases the reservation and quota without a
provider effect. A provider failure uses the existing forwarded-failure cleanup and
does not become a reconcile replay. Audit is metadata-only: catalog and operation IDs,
source, bounded sizes/counts, duration, status, and normalized reason. It never contains
text, speech, phone numbers, account SIDs, call SIDs, credentials, or provider bodies.

A successful call records `operation_id`, `catalog_service_id`, and, when the call was
platform-funded, `billing_request_id`. That last field is the join key into `usage_meter`
and `billing_ledger`, and it is what makes "which call produced this charge" answerable.

Audit deliberately records no credit amount. Settlement happens after the response
returns -- for Twilio, after the call ends -- so any amount written at audit time would
be an estimate that later disagrees with the ledger. Actor and API-key identity are
already carried by every `AuditLog` row.

A failed call records no attribution beyond `op` and outcome. It produced no charge, so
there is nothing to join to.

## Migration

Startup migration is idempotent and runs after indexes are available. For every old
active `platform-*` `DownstreamService` with a non-empty credential:

| Legacy slug | Catalog slug |
| --- | --- |
| `platform-x` | `api-twitter` |
| `platform-elevenlabs` | `api-elevenlabs` |
| `platform-twilio` | `api-twilio` |
| `platform-duffel` | `duffel` |

The migration resolves the active catalog row, inserts a `PlatformCredential` only if
that catalog service has none, and never overwrites an already configured v2 secret.
It converts old operation documents using unique kind-key upserts: `x_search` becomes
the X endpoint row and the other three become constrained rows. Billing is copied from
the old vendor row's platform pricing when present and normalized into the per-row
shape.

Only after the v2 credential and operation upserts succeed does migration set the old
vendor row `is_active: false` and `migrated_to_platform_credential: <id>`. The old
ciphertext is retained only as migration evidence on the inactive legacy document
until a separately approved cleanup; no runtime path reads it. A rerun observes the
unique credential and operation keys, repeats safe upserts, and converges without
credential replacement or duplicate rows.

The migration test fixture is serialized from the round-2 model shape, not assembled
from the v2 Rust type, so it proves compatibility with documents already deployed by
this branch.

## Verification contract

The implementation is complete only with tests that prove:

- only `platform_credential_service` can decrypt the new credential model;
- credentials and their `Debug` output are redacted;
- allowlists default empty and exact method/path matching is identical across REST
  UUID, REST slug, typed MCP, and generic MCP paths;
- unsafe POSTs, write methods, WebSockets, ambiguous paths, and anonymous/public doors
  cannot select the platform credential;
- own usable, disabled, absent, out-of-scope, unusable, node-routed, binding override,
  and approval states preserve the precedence matrix;
- base plus quantity settles once in one row, including allowance/grant/wallet funding;
- deferred Twilio completion, transient retry, 24-hour timeout, credential failure, and
  concurrent reconcile replay converge without double charging;
- Lago operation-price sync round-trips the full charge array, rejects stale writes,
  retries failures, and removes obsolete charges durably;
- migration is idempotent and never overwrites a v2 credential;
- `/keys` renders one provider object in every source state, and admin forms enforce
  hard caps, safe methods, write-only credentials, and normalized presets.

The existing platform-operation error variants remain unchanged; the authoritative
HTTP and numeric mappings stay in `backend/src/errors/mod.rs` and `CLAUDE.md`.
