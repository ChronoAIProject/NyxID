# Platform services continuation

This note records the target design and the state of draft PR #1506. It is the plan of record for the
remaining implementation. The feature remains behind `experimental:platform-services`, default off, and
is not ready to merge.

Revised 2026-08-28 after a second adversarial review found that the ordering in the previous revision
was unsafe: it treated approval parity, spend scope, and audit attribution as follow-up steps, when the
credit-spending code path they protect already exists. The feature is currently safe because it is
inert, not because it is gated.

## Product outcome

NyxID exposes platform-funded access as another credential source for eligible catalog services.

- Admins see platform operations in a compact table grouped by provider.
- Admins can promote an eligible catalog provider, configure its write-only platform credential, and
  enable an explicit method and canonical-path allowlist. The allowlist starts empty.
- Each operation charges a base fee per call plus up to two variable components.
- An owner's own credential is used first and costs nothing. Platform capacity is used only where the
  owner has opted in.
- A caller may request platform execution explicitly, with authority, even when an own credential
  exists.

## Decisions and reasons

### Reuse the catalog service

Platform capability belongs to the existing `DownstreamService`. NyxID must not create a second
`platform-*` service for the same provider. One provider identity keeps catalog discovery, BYOK setup,
MCP operations, billing, and admin status aligned.

Promotion applies only to catalog providers that pass an explicit eligibility check. The first release
excludes OAuth-backed, path-auth, query-auth, token-exchange, node-only, and master-credential services.

The eligibility predicate initially resolves only the four hand-reviewed providers in
`REGISTERED_PLATFORM_PROVIDERS`. Do not mistake this for the safety control. It gates credential
provisioning *shape* only. The real control is the per-operation enabled row plus the safe-method
registry, and until that is wired the feature's safety comes from being unreachable.

### Keep the allowlist empty by default

A configured platform credential grants no operation authority. An enabled `PlatformOperationRow` grants
one constrained operation or one canonical method-and-path operation.

The general identity is `operation_id`. `PlatformOperationName` remains only for the three code-owned
constrained operations. CRUD, quota accounting, API-key scope, audit, and discovery must use
`operation_id`.

### Own credential first; platform is opt-in

Superseded decision: an earlier revision made platform the default whenever no *active* own row matched,
and explicitly permitted fallback for a disabled row.

That is wrong. Per the services architecture, both **Disable** and **Delete** leave `is_active: false`.
Under the superseded rule, a user turning their own service off silently moved onto a metered path. A
user's off-switch must never select a payment method.

The rule is now:

| Own-credential state | Behaviour |
| --- | --- |
| Active and usable | Use it. No credits, no platform resolution. |
| Explicitly disabled or deleted | **Never** fall back. Fail closed. |
| Lapsed, expired, or unreadable | Fall back only if the owner opted in for this provider. |
| Absent entirely | Fall back only if the owner opted in for this provider. |
| Node-routed | Fail closed by default. |
| Out of an agent key's service scope | **Denied.** Never fall back. |

That last row inverts current behaviour. Today a connection outside a key's scope resolves to the
platform credential, so narrowing a key's scope *increases* credit exposure: deny-by-scope becomes
pay-by-scope. Scope restriction must never widen spend.

Never retry through the platform credential after provider dispatch starts. That retry could duplicate
an external effect.

### Store owner intent without a connection

Platform-use preference cannot live on `UserService`; the owner may need to opt out when no connection
row exists.

Use an owner-scoped collection keyed by the polymorphic owner ID and the catalog service ID, with an
optional per-operation override. It must also carry a **price ceiling**: `max_credits_per_call` and
`max_credits_per_day`. Without one, an admin raising `price_per_unit` silently re-uses standing consent
at the new price, which is not consent.

An org admin's setting binds org members; a member cannot opt an org into spending.

Resolve an explicit request intent with the stored preference into:

```text
CredentialIntent = auto | own_only | platform_only
```

### Pricing: base fee plus up to two components

Superseded decision: an earlier revision claimed one variable metric was an architectural constraint.
It is not. The measurement side already exists.

- `PlatformUsage` already carries requests, bytes, tokens, characters, and seconds simultaneously.
- `TokenBreakdown` is already persisted on `UsageMeterRow`, marked "observability, not priced
  separately".
- `BillingReservation.layers` is already a `Vec`, and settlement is already per row.
- `transaction_id(billing_request_id, layer, flush_seq)` already carries a spare discriminator.

Two components deliver the case that matters: input tokens and output tokens at different rates.

Quantity allowances are component-specific. An `input_tokens` allowance covers only the input-token
component, an `output_tokens` allowance covers only the output-token component, and a `tokens`
allowance covers a deliberately combined-token component. One allowance is never independently
applied to both split components. `funding::reserve_allowances` therefore receives the metric stored
on each reservation component, not the route's primary metric.

For endpoint operations, `bytes` means response-body bytes only. Request bytes are excluded because
they are caller-controlled; charging their size would let a caller amplify its own spend before the
provider returned any value. Token quantities use the provider-reported response usage, characters
count the decoded response body, and seconds are elapsed provider-request time rounded up to a whole
second. Step 10 applies these definitions at the common REST/MCP settlement boundary.

### Apply authority before spending or decryption

1. Rollout flag and caller class.
2. API-key rate limit, service scope, and platform-operation spend scope.
3. Owner connection and `CredentialIntent`.
4. Canonical operation.
5. Authorize the enabled operation row.
6. Evaluate the approval policy.
7. Limits, per-owner spend cap, and daily quota reservation.
8. Reserve credits from the operation price snapshot.
9. Decrypt the platform credential.
10. Mark the meter forwarded immediately before dispatch.
11. Settle the known quantity or persist deferred settlement state.

Steps 5 and 9 are currently the same call, and the previous revision of this document misreported them
as separate. Splitting them matters: today a request failing row authorization has already burned a
quota slot and a wallet reservation.

## Vendor terms and attribution

Reselling vendor access on a shared credential is a per-provider go/no-go gate that precedes enabling
any provider in production. It is a business decision, not an engineering one, and no provider may be
enabled until it is answered.

Provider failure settlement is conservative after dispatch. An explicit 4xx rejection releases the
billing reservation, owner spend reservation, and daily quota slot. A provider 5xx may arrive after
Twilio or ElevenLabs performed and billed the work, so NyxID settles the full pre-dispatch estimate
and retains both quota reservations. A malformed 2xx success payload follows the same rule. NyxID
does not retry through another credential after dispatch and does not issue an automatic partial
refund because it has no authoritative performed-quantity evidence; later provider credits are
handled by operational reconciliation.

| Provider | Exposure |
| --- | --- |
| Twilio | Every user's calls originate from one NyxID number. A2P 10DLC registration, caller-ID reputation, carrier spam scoring, and TCPA liability attach to NyxID. One abusive tenant can get the shared number blocked for every user. Strongest candidate for BYOK-only. |
| ElevenLabs | Voice-cloning consent and per-account licensing terms. |
| Duffel | Agency-of-record and IATA rules. |
| X | Per-account API tier and rate allocation. |

## Current state

Committed and verified:

- Round 1 and 2 platform execution, verified green by an independent reviewer.
- v2 catalog-linked credential and operation models.
- Per-operation pricing, combined base-fee reservation, deferred Twilio settlement.
- The pricing response contract, pinned across Rust and TypeScript by a generated fixture.

Known defects in the committed branch, from the second adversarial review:

| Defect | Location |
| --- | --- |
| Approval policy is never evaluated for platform-funded execution | `handlers/platform_ops.rs`, approval block opens on `ExecutionTarget::OwnConnection` |
| `AuthMethod::AccessToken` is admitted with no scope check | `ensure_platform_operation_caller` |
| No API-key scope gates spend; `/platform-ops` is exempt from management write-scope | `services/key_service.rs`, `mw/auth.rs` |
| Out-of-scope own connection resolves to the platform credential | `platform_operation_service.rs` |
| Org members are charged from their personal wallet | `platform_ops.rs`, `resolve_for_resource(actor, actor)` |
| No per-owner or per-key spend cap; `speak` has no daily cap at all | `reservation.rs`, `platform_operation_service.rs` |
| Audit records no `operation_id`, credits, or `billing_request_id` | `platform_operation_audit_metadata` |
| Vendor-rejected requests are free and quota-neutral; no global limiter on the shared credential | `fail_platform_attempt`, `mw/rate_limit.rs` |
| `metric_code_for_operation` short-path collisions can charge one operation's price for another | `services/billing/pricing.rs` |
| Deleting a catalog provider leaves `platform_operations` enabled and the credential stored | `handlers/services.rs` |
| Endpoint rows cannot be priced per token, character, or second | `operation_supports_metric` |

Not built at all: admin credential and promotion routes, operation-ID CRUD, endpoint authorization
callers, owner preferences, `CredentialIntent`, explicit platform routes, the admin table, the `/keys`
projection.

## Implementation order

Security first, because the path it protects already exists.

1. **Done.** Pin the pricing contract across Rust and TypeScript; fix the strict Zod schemas.
2. Approval parity and caller authority: evaluate approval for the platform target; reject
   `AuthMethod::AccessToken` without an explicit scope; add a `platform:spend` API-key scope checked
   before quota, billing, decryption, or any provider effect.
3. Fix the scope inversion and org payer: out-of-scope means denied; resolve the org owner as payer.
4. Audit attribution: `operation_id`, resolved intent, credential source, fallback reason, actor and
   API-key identity, `billing_request_id`, and settled credits.
5. Spend bounds: per-owner and per-key credit caps, a daily cap for every operation, and a global
   limiter on the shared credential so rejected requests are not free.
6. Owner preference model and `CredentialIntent`, with the opt-in default and the disable/lapse
   inversion above.
7. Two-component pricing, after settling the allowance question.
8. Provider eligibility and write-only platform credential lifecycle endpoints.
9. Operation-ID admin CRUD; re-key daily quotas by operation ID; fix the metric-code collision; allow
   tokens, characters, and seconds on endpoint rows.
10. Wire exact endpoint authorization into REST and MCP, splitting authorization from decryption. Prove
    parity across UUID, slug, typed MCP, and generic MCP entry points.
11. Replace the admin cards with a grouped table and an edit drawer.
12. Explicit `platform_only` REST and MCP calls.
13. Unified `/keys` provider projection.
14. Catalog-provider deletion cascade to operations and credentials.
15. Rewrite `docs/PLATFORM_SERVICES.md` so implemented and planned behaviour are visibly separate.
16. Full backend suite, frontend build and tests, browser checks, and an independent adversarial review.

## Admin table shape

One operation per row, grouped by provider.

Columns backed by data that exists today: Provider, Operation, Kind, Enabled, Metric, Price, Limits,
Billing sync. Billing sync is already in the response and the schema; the card UI simply never rendered
it.

Columns that need new storage before they can be shown honestly:

| Column | What it needs first |
| --- | --- |
| Usage today | A date-leading index on `platform_op_usage`, and a usage row for `speak`, which writes none. It is a reservation counter that is deleted on failure, so it is not a usage history. |
| Credits spent | A `platform_operation_id` field and index on `usage_meter`. The only join today is `lago_metric_code`, which is unindexed and rewritten on every price edit. |
| Health | A `PlatformOperationMetrics` embedded document mirroring `NodeMetrics`, written on success and failure. No counters exist. |

Do not ship a column that cannot be populated. `speak` would show a permanent, false zero.

Provider-level controls own promotion, eligibility explanation, and credential replacement. The
operation editor owns the canonical method and path, limits, allowlists, vendor configuration, billing
components, and Lago diagnostics.

## Verification target

The work is complete only when this is true:

> For every eligible catalog service, an admin can configure a write-only platform credential and an
> empty-by-default operation allowlist; an authorized owner call resolves `auto`, `own_only`, or
> `platform_only` consistently across REST and MCP; a user's own credential is never bypassed without
> their opt-in and never charged past their ceiling; approval, spend scope, quota, billing, decryption,
> and dispatch occur in the documented order; every charge is traceable from an audit row; and the
> admin table shows only state the system actually stores.
