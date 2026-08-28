# Platform services

Platform services let an authenticated NyxID owner use a NyxID-managed vendor
credential for a specific catalog operation. The platform credential is an alternate
credential source for an existing `DownstreamService`; it is not a second catalog
service or a synthetic user connection.

Caller-facing discovery and execution are behind the
`experimental:platform-services` feature flag. Admin configuration and billing
reconciliation remain available while the flag is off.

This document separates behavior that is implemented in the current code from work
that still needs storage or migration support.

## Implemented behavior

### Data ownership

The subsystem uses the existing catalog provider identity plus dedicated control,
credential, operation, preference, and reservation records.

| Collection                     | Implemented purpose                                                                            |
| ------------------------------ | ---------------------------------------------------------------------------------------------- |
| `downstream_services`          | The provider identity used by catalog, BYOK, MCP, and platform execution.                      |
| `platform_provider_promotions` | Promotion state and the administrator audit fields for vendor-terms acceptance.                |
| `platform_credentials`         | One encrypted, NyxID-managed credential per promoted catalog provider.                         |
| `platform_operations`          | The exact enabled endpoint or constrained operations and their prices.                         |
| `platform_service_preferences` | Owner consent, provider ceilings, and per-operation overrides.                                 |
| `platform_op_usage`            | Daily operation reservation counts. This is not durable usage history.                         |
| `usage_meter`                  | Billing reservation and settlement records. These rows do not contain `platform_operation_id`. |

IDs are UUID strings. Provider references use `DownstreamService._id`, so a catalog
provider has one identity across own credentials and platform-managed access.

### Provider promotion and credentials

The code-owned `REGISTERED_PLATFORM_PROVIDERS` registry currently admits these catalog
slugs:

- `api-elevenlabs`
- `api-twilio`
- `duffel`
- `api-twitter`

Registry membership is an eligibility gate, not operation authority. A provider must
also be active, promoted, have a configured platform credential, and have an enabled
operation that matches the request.

Promotion requires `vendor_terms_accepted: true`. The resulting
`PlatformProviderPromotion` stores:

```text
vendor_terms_accepted_by
vendor_terms_accepted_at
promoted_by
promoted_at
updated_by
updated_at
```

This makes vendor-terms acceptance an enforced go/no-go gate with an administrator and
timestamp, rather than an operational note.

The platform credential is write-only through the admin API. It is encrypted with
`EncryptionKeys`, key material is held in `Zeroizing` wrappers when materialized, and
credential `Debug` output is redacted. API responses expose configured state and
timestamps but never return plaintext or ciphertext.

The provider lifecycle routes are:

```text
GET    /api/v1/admin/platform-providers
GET    /api/v1/admin/platform-providers/{catalog_service_id}
PUT    /api/v1/admin/platform-providers/{catalog_service_id}
DELETE /api/v1/admin/platform-providers/{catalog_service_id}
PUT    /api/v1/admin/platform-providers/{catalog_service_id}/credential
DELETE /api/v1/admin/platform-providers/{catalog_service_id}/credential
```

Demotion removes live promotion state after disabling its operations. Catalog-provider
deletion deactivates the catalog row first, disables every provider operation, removes
active and historical Lago operation charges and local rate-cache rows, then deletes
the provider's operation, encrypted credential, promotion, and owner-preference rows.
The existing `service_deleted` audit event records the cascade counts. The promotion
decision remains represented by the earlier `admin_platform_provider_promoted` audit
event even after its live row is removed.

### Operation model

`platform_operations` contains one row for each catalog provider and operation. The
relevant stored shape is:

```rust
PlatformOperationRow {
    _id: String,
    catalog_service_id: String,
    kind_key: String,
    enabled: bool,
    kind: PlatformOperationKind,
    limits: OperationLimits,
    billing: OperationBilling,
    billing_cleanup_metric_codes: Vec<String>,
    created_by: String,
    updated_by: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

OperationBilling {
    metric: BillingMetric,
    price_per_unit: String,
    secondary: Option<OperationBillingComponent>,
    base_fee_per_call: Option<String>,
    lago_metric_code: String,
    sync_status: PricingSyncStatus,
    sync_error: Option<String>,
}
```

The optional secondary component has its own metric, price, and Lago metric code.
`billing_cleanup_metric_codes` is a vector because one edit can obsolete both primary
and secondary metrics. Cleanup markers remain until Lago charge removal and local
rate-cache cleanup succeed.

`kind_key` is derived by the service and is not accepted from an admin request:

```text
endpoint:{METHOD} {normalized_path_template}
constrained:{snake_case_op}
```

The unique `(catalog_service_id, kind_key)` index prevents duplicate operation rows.
Creating a provider credential grants no operation access. The platform credential can
be selected only for an enabled row that passes the current safety checks.

The admin CRUD routes are:

```text
GET    /api/v1/admin/platform-ops
POST   /api/v1/admin/platform-ops
PUT    /api/v1/admin/platform-ops/{operation_id}
DELETE /api/v1/admin/platform-ops/{operation_id}
```

### Endpoint and constrained operations

Endpoint operations match the normalized HTTP method and canonical path template. The
path matcher is root-anchored, segment-based, query-independent, and shared by REST and
MCP entry points. Ambiguous encodings, encoded separators, dot segments, regular
expressions, globs, partial placeholders, empty segments, and WebSocket upgrades do not
authorize platform credentials.

The safe-method registry permits `GET` and `HEAD` for registered providers. The only
registered `POST` pair is Duffel `POST /air/offer_requests`. Unsafe methods and
unregistered provider/template pairs are rejected on write and checked again when an
operation is authorized.

Three operations remain constrained because NyxID validates and constructs their
provider requests:

| Operation       | Catalog service  | Server-owned constraint                                                               |
| --------------- | ---------------- | ------------------------------------------------------------------------------------- |
| `speak`         | `api-elevenlabs` | Text size, voice allowlist, model, and bounded audio response.                        |
| `call_and_say`  | `api-twilio`     | Message size, destination prefixes, caller identity, TwiML, and duration ceiling.     |
| `flight_search` | `duffel`         | Bounded search input and projected result count; booking and payment are unreachable. |

X recent search is an endpoint operation:

```text
GET /2/tweets/search/recent
```

An enabled constrained row authorizes only that constrained handler. It does not grant
the equivalent generic vendor endpoint.

### Authorization and materialization boundary

Operation authorization and credential decryption are separate service calls:

```rust
authorize_endpoint(...) -> AppResult<AuthorizedPlatformOperation>
authorize_constrained(...) -> AppResult<AuthorizedPlatformOperation>
list_enabled_authorized_operations(...)
    -> AppResult<Vec<AuthorizedPlatformOperation>>
materialize_authorized(...) -> AppResult<AuthorizedPlatformCredential>
materialize_platform_vendor_target(...)
```

`AuthorizedPlatformOperation` contains the validated catalog service and operation but
no decrypted secret. `authorize_endpoint` requires exactly one enabled method/template
match. `authorize_constrained` requires the expected enabled constrained row and its
code-owned provider binding. Invalid provider associations and malformed enabled rows
fail before decryption.

Only materialization reads and decrypts `platform_credentials`. Execution reaches that
step after authority, consent, approval, quota, spend-ceiling, and billing-reservation
checks. `/keys` and platform-operation discovery use
`list_enabled_authorized_operations`; they do not materialize or decrypt the shared
credential.

### Owner consent and credential intent

Platform spending requires a `PlatformServicePreference` for the resolved personal or
organization owner. The preference stores provider-level opt-in and maximum credits per
call and per day, with optional operation-specific enablement and ceilings. A stored
preference is necessary even when a caller explicitly requests the platform
credential.

Every supported execution door resolves one of these intents:

```text
auto
own_only
platform_only
```

Generic REST proxy requests use the `x-nyxid-credential-intent` header. Typed platform
REST request bodies accept `credential_intent`. Typed and generic MCP tool schemas
expose `credential_intent` as a NyxID transport control. MCP removes it before
forwarding provider arguments.

`platform_only` selects the platform credential in preference to a usable own
credential, but it does not bypass owner consent, service or agent-key scope, approval,
the operation allowlist, quotas, rate limits, spend ceilings, or billing.
`own_only` never selects a platform credential.

Credential resolution follows this matrix:

| Own-connection state and intent                                                            | Result                                                                |
| ------------------------------------------------------------------------------------------ | --------------------------------------------------------------------- |
| Active usable own credential with `auto`                                                   | Use `own_connection`; platform billing is skipped.                    |
| No own row and stored consent                                                              | Use the platform credential.                                          |
| No own row and no stored consent                                                           | Fail with `owner_opt_in_required`.                                    |
| Disabled or deleted own row                                                                | Fail with `own_connection_disabled`; never fall back.                 |
| Active own credential with an allowlisted credential-resolution failure and stored consent | Use platform fallback.                                                |
| Node-routed own connection                                                                 | Fail closed for platform selection.                                   |
| Approval-required own connection during discovery                                          | Fail closed for platform selection.                                   |
| Out-of-scope own connection                                                                | Fail closed for platform selection.                                   |
| Explicit `platform_only` and stored consent                                                | Use the platform credential even when a usable own credential exists. |
| `own_only`                                                                                 | Use the own path or fail; never use platform.                         |

Automatic fallback from an active own row is deliberately narrow. The code-owned
`own_error_allows_platform_fallback` predicate admits the recognized credential failure
classes. Other resolver, authority, node, approval, and scope failures cannot turn into
paid platform execution. A provider failure after dispatch never retries through a
different credential.

Anonymous proxy calls and public MCP never inspect, authorize, or decrypt a platform
credential.

### Execution order

A platform-funded request runs these checks and side effects in order:

1. Check the rollout flag and caller class.
2. Check `platform:spend`, agent-key rate limits, and service scope.
3. Resolve the personal or organization owner, agent binding, own connection, and credential intent.
4. Construct the canonical operation.
5. Authorize the enabled endpoint or constrained row without decrypting a credential.
6. Evaluate the owner's approval policy.
7. Enforce request limits, owner and API-key spend ceilings, and reserve the daily quota slot.
8. Reserve credits using the operation price snapshot.
9. Materialize and decrypt the platform credential.
10. Mark the meter forwarded immediately before provider dispatch.
11. Settle measured usage or persist the deferred Twilio descriptor.

Failures before dispatch release the billing and daily quota reservations. This order
ensures that authorization and approval failures do not decrypt the shared credential.

### Billing and limits

Each operation has one primary variable price, an optional secondary variable price,
and an optional base fee. Decimal credit prices remain strings through validation and
persistence. The primary and secondary components use distinct Lago metric codes and
metric-specific allowance funding.

Stable operation metric codes always include a digest of the normalized operation
identity. The digest is appended for short and long inputs, which makes structurally
different operation identities distinct even when their readable slugs collide.

Supported measurements include requests, response bytes, provider-reported token
counts, characters, and seconds. For generic endpoint execution, `bytes` means response
body bytes only. Request bytes are not billed. Constrained operations use their
validated quantities, such as input characters for speech and completed-call seconds
for Twilio. Invalid metric and operation combinations are rejected by admin writes.

The operation price is synchronized to Lago. Pending and failed syncs are retried by
billing reconciliation, stale completions cannot overwrite newer edits, and obsolete
charges retain durable cleanup markers until both Lago and local cleanup finish.

Every operation may have a per-owner daily call cap. The conditional reservation uses
`platform_op_usage`, keyed by `(operation_id, user_id, yyyymmdd)`. A reservation is
removed when a pre-dispatch failure or explicit provider 4xx makes the attempt
non-billable. Consequently, this collection is a quota counter, not an analytics or
usage-history source.

Platform execution can also enforce a process-local rate limit per
`(catalog_service_id, owner_id)`, configured by
`PLATFORM_SERVICE_RATE_LIMIT_PER_SECOND` and
`PLATFORM_SERVICE_RATE_LIMIT_BURST`. It is off when the configured rate is zero. There
is no provider-wide aggregate throughput ceiling across owners.

An explicit provider 4xx releases the billing, spend, and daily quota reservations. A
provider 5xx or malformed successful response may follow billable vendor work, so
NyxID settles the bounded pre-dispatch estimate and retains the quota reservation. It
does not infer a partial refund without authoritative provider evidence.

Own-credential execution bypasses platform quota and billing.

### Deferred Twilio settlement

`call_and_say` reserves the configured maximum duration before dispatch. After Twilio
accepts the create-call request, NyxID validates the returned call SID, stores a tagged
deferred descriptor on the same `UsageMeterRow`, and settles the snapshotted base fee.

Billing reconciliation polls Twilio for completed duration and finalizes that same row
with the measured seconds. Transient and non-terminal results are retried. An unresolved
call reaches the existing 24-hour base-only terminal rule, which clears the hold and
emits a metadata-only audit event. Claim filters and idempotent settlement prevent a
replayed reconciliation worker from charging twice.

### Discovery and `/keys`

`GET /api/v1/platform-ops` returns the caller's authorized platform-operation
projections without decrypting credentials, refreshing OAuth state, creating
approvals, updating last-used timestamps, or opening usage meters.

`GET /api/v1/keys` is the unified user-facing provider inventory. `KeyInfo` has these
additive platform fields:

```text
platform_managed: bool
platform: Option<PlatformKeySummary>

PlatformKeySummary {
    operations: [{ name, kind, price_label }]
    credential_source:
        platform
        | own_connection
        | platform_fallback
        | unusable
    reason: optional string
}
```

For a provider with enabled authorized operations and no own row, `/keys` emits one
synthetic response row with this deterministic ID:

```text
platform-provider:{catalog_service_id}
```

Synthetic IDs are response-only. Detail and mutation routes do not accept them as
`UserService` IDs. When an own row exists, the row keeps its real ID and receives the
platform summary instead of producing a second provider entry. A disabled or deleted
own row is reported as `unusable` with reason `own_connection_disabled`; it is never
reported as platform fallback.

The frontend renders operation count, credential source, reason, and the
backend-formatted price labels in card and table views. Synthetic rows are not
navigable and provide **Connect your own**, which opens the normal connection form
with the provider preselected.

### Admin interface

The implemented admin page is `/admin/platform-ops`. It groups operation rows by
provider and uses this desktop column set:

| Provider | Operation | Kind | Enabled | Metric | Price | Limits |
| -------- | --------- | ---- | ------- | ------ | ----- | ------ |

Mobile uses a responsive row layout with the same data. The page includes an operation
edit drawer and a provider drawer. Admins can promote or demote a registered provider,
accept vendor terms, set, replace, or delete its write-only credential, and create,
update, or delete operations. Forms use `useAppForm`, Zod validation, and TanStack
Query invalidation.

The page does not show `Usage today`, `Credits spent`, or `Health`. The storage needed
to calculate those columns does not exist yet.

### Audit and response attribution

Platform operation audit metadata includes the operation and catalog service IDs,
resolved intent, credential source, fallback reason, actor and API-key attribution,
outcome, and the billing request ID for a platform-funded call. Provider request and
response bodies, credentials, speech text, phone numbers, account SIDs, and call SIDs
are not included.

REST responses identify the selected credential source. MCP results carry equivalent
credential-source metadata in their supported result shapes. Billing settlement
remains authoritative for the final amount, especially for deferred calls, so audit
events do not claim an amount before settlement completes.

## Planned work

The following items are not implemented. They must not be inferred from the admin UI,
quota counter, or billing rows.

| Item                                       | Missing prerequisite                                                                                                                                     |
| ------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Usage today` admin column                 | Durable per-operation usage history with a reporting index. `platform_op_usage` is a reservation counter and failed reservations are deleted.            |
| `Credits spent` admin column               | A durable `usage_meter.platform_operation_id` field and an index. Joining through mutable, unindexed `lago_metric_code` is not an acceptable substitute. |
| `Health` admin column                      | Durable per-operation success and failure counters. No health-counter model exists.                                                                      |
| Provider-wide aggregate throughput ceiling | The current limiter is per owner and provider, not a shared ceiling across all owners.                                                                   |
| Legacy `platform-*` migration              | No startup migration currently converts legacy provider rows into catalog-linked credentials and operations.                                             |

Any implementation of these items must add the durable storage and indexes first, then
define retention, reconciliation, and failure semantics before exposing the data in an
admin view.
