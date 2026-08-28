# Platform services continuation

This note records the target design and the state of draft PR #1506 on 2026-08-28. It is a handoff for the next implementation session. The feature remains behind `experimental:platform-services` and is not ready to merge.

## Product outcome

NyxID should expose platform-funded access as another credential source for eligible catalog services.

- Admins see platform operations in a compact table. Each row shows the provider, operation, target, availability, price, daily usage, health, and billing sync state.
- Admins can promote an eligible catalog provider, configure its write-only platform credential, and enable an explicit method and canonical-path allowlist.
- Each operation can charge a base fee per call plus one variable metric. The supported metrics are requests, seconds, tokens, characters, and bytes.
- NyxID uses a usable owner credential first. It can fall back to the platform credential when no usable owner credential exists and the owner permits platform use.
- A caller can request `platform_only` execution even when the owner has a usable credential. This intent requires explicit authority because it spends owner credits.

## Decisions and reasons

### Reuse the catalog service

Platform capability belongs to the existing `DownstreamService`. NyxID must not create a second `platform-*` service for the same provider. One provider identity keeps catalog discovery, BYOK setup, MCP operations, billing, and admin status aligned.

Promotion applies only to catalog providers that pass an explicit eligibility check. The first release excludes OAuth-backed, path-auth, query-auth, token-exchange, node-only, and master-credential services. Their credential and routing behavior cannot share the same method-and-path authorization rule safely.

### Keep the allowlist empty by default

A configured platform credential grants no operation authority. An enabled `PlatformOperationRow` grants one constrained operation or one canonical method-and-path operation. This rule makes credential storage and request authorization independent.

The general identity is `operation_id`. The existing `PlatformOperationName` enum remains only for the three code-owned constrained operations. CRUD, quota accounting, API-key scope, audit, and discovery must use `operation_id` so arbitrary endpoint rows do not disappear into the old three-value enum.

### Store owner intent without a connection

Platform-use preference cannot live on `UserService`. The owner may need to opt out when no connection row exists.

Use a separate owner-scoped collection keyed by the polymorphic owner ID and the catalog service ID. Resolve an explicit request intent with the stored preference into one of these values:

```text
CredentialIntent = auto | own_only | platform_only
```

`auto` prefers a usable owner credential and falls back only before downstream dispatch. `own_only` forbids platform spending. `platform_only` selects the platform path even when an owner credential exists.

Never retry through the platform credential after provider dispatch starts. That retry could duplicate an external effect.

### Keep pricing within the current ledger model

The current usage meter records one metric quantity and an optional base fee. The first release therefore supports `base_fee_per_call + one variable metric`.

True multi-variable prices, such as call fee plus input tokens plus output tokens, need a separate billing design. Adding them here would change reservations, grant precedence, Lago drift comparison, ledger encoding, and settlement idempotency at once.

### Apply authority before spending or decryption

Platform execution must follow this order:

1. Check the rollout flag and caller class.
2. Check the API-key rate limit, service scope, and platform-operation scope.
3. Resolve the owner connection and `CredentialIntent`.
4. Construct the canonical operation.
5. Authorize the enabled operation row.
6. Evaluate the approval policy.
7. Apply limits and reserve the daily quota.
8. Reserve credits from the operation price snapshot.
9. Decrypt the platform credential through the authorized wrapper.
10. Mark the meter forwarded immediately before provider dispatch.
11. Settle the known quantity or persist deferred settlement state.

Platform-funded execution must not broaden beyond the current curated providers until approval checks and platform-operation API-key scopes are enforced on that path.

## Current draft state

The committed branch contains the catalog-linked credential and operation models, the round-two platform execution work, and `docs/PLATFORM_SERVICES.md`. The existing document describes the intended v2 system. Some sections still describe planned behavior in the present tense.

The current uncommitted checkpoint adds the next billing unit:

- `characters` and `seconds` billing metrics;
- per-operation price sync and cleanup state;
- a base fee combined with one variable quantity;
- allowance, grant, and wallet funding for the combined reservation;
- idempotent deferred Twilio base-fee and duration settlement;
- reconciliation hooks, billing-ledger fields, and focused tests.

`cargo fmt --all -- --check` passes. `cargo check -p nyxid --bin nyxid-server` passes with warnings that expose unfinished wiring. `set_credential`, `credential_status`, `authorize_endpoint`, and several new operation fields have no production caller yet.

The draft is not functionally complete. The main gaps are:

- The admin frontend still uses three operation cards and its strict Zod pricing schema does not match the backend response.
- Admin routes cannot set a platform credential or perform operation-ID CRUD.
- Endpoint authorization is not called by the REST proxy or MCP execution paths.
- Listing and daily quota logic still depend on the three-value constrained-operation enum.
- Platform-funded execution does not yet enforce approval parity or a platform-operation API-key scope.
- Owner preferences and `CredentialIntent` do not exist.
- The explicit platform-only REST and MCP paths do not exist.
- `/keys` does not yet provide the unified provider projection described in `docs/PLATFORM_SERVICES.md`.

## Recommended implementation order

Each step should end with a focused check before the next step starts.

1. Add a frontend regression fixture that parses the real backend pricing response. Fix the strict Zod schemas and the user and admin pricing projections.
2. Add provider eligibility and write-only platform credential lifecycle endpoints. Keep the currently curated providers as the initial eligible set.
3. Add operation-ID admin CRUD. Query stored rows instead of looping over `PlatformOperationName`, and key daily quota rows by operation ID.
4. Replace the admin cards with a grouped table. Put provider credential and promotion controls at provider level. Put method, path, limits, allowlists, and billing details in an edit drawer.
5. Wire exact endpoint authorization into REST and MCP behind the rollout flag. Prove parity across UUID, slug, typed MCP, and generic MCP entry points.
6. Add the owner preference model and `CredentialIntent`. Resolve the preference in the same context load that resolves visible owner connections.
7. Add platform-operation scopes to agent API keys. Check the scope before quota reservation, billing, credential decryption, or provider effects.
8. Resolve approval policy by owner and catalog service when platform execution has no `UserService` ID. Apply the same operation descriptor at observe and redeem time.
9. Add explicit `platform_only` REST and MCP calls and audit the resolved intent and operation ID.
10. Add the unified `/keys` provider projection and remove the separate platform-service cards.
11. Rewrite `docs/PLATFORM_SERVICES.md` so implemented behavior and planned behavior are visibly separate.
12. Run the backend suite, frontend build and tests, browser checks for the admin table and `/keys`, and an independent adversarial review.

## Admin table shape

Use one operation per row and group rows by provider.

| Column | Meaning |
| --- | --- |
| Provider | Catalog name and slug |
| Operation | Constrained operation or endpoint name |
| Target | `METHOD /canonical/path` or `constrained` |
| Availability | Enabled state and credential readiness |
| Price | Base fee and metered unit in one readable value |
| Usage today | Owner usage against the daily cap |
| Credits spent | Settled platform-funded credits |
| Health | Recent execution health |
| Billing sync | NyxID and Lago synchronization state |
| Edit | Opens the operation editor |

Provider-level controls own promotion, eligibility explanation, and credential replacement. The operation editor owns the canonical method and path, limits, allowlists, vendor configuration, billing fields, and Lago diagnostics.

## Verification target

The work is complete only when the following statement is true:

> For every eligible catalog service, an admin can configure a write-only platform credential and an empty-by-default operation allowlist; an authorized owner call resolves `auto`, `own_only`, or `platform_only` consistently across REST and MCP; approval, API-key scope, quota, billing, decryption, and dispatch occur in the documented order; and the admin table and `/keys` show the same source, price, availability, and health state.

The Opus 5 adversarial review that shaped this continuation is Heca agent `01a04663-86fe-7681-a1cd-a2bfb5ae6677`.
