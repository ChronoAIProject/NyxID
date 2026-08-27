# Platform services

Platform services expose a small set of NyxID-owned operations backed by vendor APIs. A caller selects an operation, never an arbitrary vendor method or URL. NyxID validates a typed request and constructs the upstream path, headers, and body.

The user-facing feature has three layers, evaluated in this order:

1. The `experimental:platform-services` feature flag controls who can discover or call the operations. It defaults to off.
2. Each `PlatformOperation` row has its own `enabled` switch.
3. Each enabled row has typed, operation-specific configuration.

The flag does not gate the admin page. Administrators can provision vendors, configure operations, and set prices before exposing the feature to users.

## Operation registry

The registry in `services/platform_operation_service.rs` owns the mappings. Database templates cannot add operations or weaken these contracts.

| Operation | HTTP route | MCP tool | Platform vendor row | User catalog row |
| --- | --- | --- | --- | --- |
| `x_search` | `POST /api/v1/platform-ops/x-search` | `nyx__x_search` | `platform-x` | `api-twitter` |
| `speak` | `POST /api/v1/platform-ops/speak` | `nyx__speak` | `platform-elevenlabs` | `api-elevenlabs` |
| `call_and_say` | `POST /api/v1/platform-ops/call-and-say` | `nyx__call_and_say` | `platform-twilio` | `api-twilio` |
| `flight_search` | `POST /api/v1/platform-ops/flight-search` | `nyx__flight_search` | `platform-duffel` | `duffel` |

`GET /api/v1/platform-ops` returns enabled operations for the caller. It is control-plane discovery and does not create a billing meter.

Session and access-token users may call the routes. API-key access requires a `nyxid_ag_` agent key. Delegated, relay, and service-account tokens are rejected by the route group.

## Credential precedence

Execution and discovery use the same credential-source resolver. Execution uses the normal proxy resolution cascade, including personal, service-pool, and organization fallback. X OAuth refresh follows the same path as `/proxy`.

| Connection state | Result | Billing | Discovery state |
| --- | --- | --- | --- |
| No active matching `UserService` | Platform credential | Credits path | No `own_connection` object |
| Matching row is disabled | Platform credential | Credits path | `usable: false`, `reason: "disabled"` |
| Active row resolves to a master credential or has no `api_key_id` | Platform credential | Credits path | Platform source |
| Active row has a user-supplied, server-held credential | Own connection | Skipped | `usable: true`, no reason |
| Active row has an expired, revoked, missing, or unreadable credential | Return the existing resolver error | Skipped | `usable: false`, `reason: "unusable"` |
| Active row is node-routed or has no server-held credential | HTTP 409, `platform_operation_own_connection_unsupported` | Skipped | `usable: false`, `reason: "node_routed"` |

An unusable active connection never falls back to platform credits. The user must repair it or disable it. This avoids charging a caller who expected NyxID to use their account.

Discovery runs in read-only mode. It does not decrypt credentials, refresh OAuth tokens, update last-used timestamps, or turn an unusable connection into an API error.

## Controls by credential source

| Applies to | Controls |
| --- | --- |
| Every request | `deny_unknown_fields`; character and result limits; hard caps; E.164 validation; XML escaping and control-character rejection; server-composed paths, queries, headers, and bodies; no caller-supplied URLs, callbacks, recording fields, or vendor operations |
| Platform credential only | `allowed_voice_ids`; `allowed_destination_prefixes`; `max_calls_per_user_per_day`; `max_searches_per_user_per_day`; the configured Twilio `account_sid` and `call_from` |
| Own connection only | `speak` accepts any voice identifier that passes the safe-identifier check; `call_and_say.from` is required and must be E.164; the Twilio account SID comes from the stored `AccountSID:AuthToken` credential |

`call_and_say.from` is rejected when the platform credential is selected. This keeps the platform caller identity under administrator control.

## Execution and billing

Every operation follows this order:

1. Feature flag.
2. Caller credential class.
3. Per-agent rate limit.
4. Enabled operation and valid typed configuration.
5. Credential-source resolution.
6. Platform-only allowlists and daily-quota reservation.
7. Billing `open` for a platform credential.
8. Platform credential decryption.
9. Billing `mark_forwarded` immediately before the vendor request.
10. Vendor request.
11. Deferred settlement on success, or billing failure plus daily-quota release on error.

Platform execution uses `BillingIngress::PlatformOperation`, `CredentialClass::NyxidManagedMaster`, and the vendor row's `ServiceBilling` configuration. The metered quantity is one `requests` unit. Price authoring remains on `DownstreamService.billing.platform_pricing`; there is no price field on `PlatformOperation`.

Billing opens before platform credential decryption or any network request. `AppError::InsufficientCredits` therefore leaves no daily-quota reservation, does not decrypt the vendor credential, and does not contact the vendor. A non-2xx response or an invalid bounded response fails the meter and releases both the wallet hold and daily quota. Successful settlement is persisted before an asynchronous worker completes the charge, so Lago latency does not delay the operation response.

Own-connection execution uses a disabled meter. It does not create a `usage_meter` row or touch a wallet.

Audit events contain operation names, caller attribution, sizes, durations, credential source, and normalized outcomes. They do not contain message text, audio, full phone numbers, credentials, vendor account identifiers, or upstream payloads.

## Response signaling

Every operation response includes:

```text
X-NyxID-Credential-Source: platform
```

or:

```text
X-NyxID-Credential-Source: own_connection
```

MCP results carry the same value in `structuredContent.credential_source` and `_meta.credential_source`. JSON text content also includes `credential_source`; audio stays audio content and uses the structured metadata fields. At `tools/list` time, each platform tool description states whether the caller's connection or the platform credential will be used, including the current per-call price when one is active.

The discovery response contains dedicated response objects, not database models:

```json
{
  "operations": [
    {
      "op": "speak",
      "display_name": "Speak",
      "description": "Synthesize speech from bounded text input.",
      "vendor": "elevenlabs",
      "catalog_service_slug": "api-elevenlabs",
      "credential_source": "platform",
      "own_connection": null,
      "pricing": {
        "billable": true,
        "credits_per_call": "0.25",
        "metric": "requests"
      },
      "mcp_tool": "nyx__speak"
    }
  ]
}
```

`pricing.billable` requires both the vendor's `platform_billable` setting and the billing rollout flag. `credits_per_call` is the exact stored price string, or `null` when the operator has not set a NyxID price.

## Bounded operations

`x_search` searches recent public posts. NyxID limits `query` to 512 characters and `max_results` to the configured cap, with a hard cap of 25. The platform base uses `/2/tweets/search/recent`; an own connection whose base URL already ends in `/2` uses `/tweets/search/recent` relative to that base.

`speak` sends bounded text and a safe voice identifier to ElevenLabs. NyxID constructs `{text, model_id}` and streams the successful audio response. The platform voice allowlist does not constrain a user's own ElevenLabs key.

`call_and_say` places one Twilio call with server-generated TwiML. NyxID accepts only `to`, `message`, and the source-dependent `from` field. It never accepts `Url`, `StatusCallback`, recording options, or arbitrary Twilio form fields.

`flight_search` sends `POST /air/offer_requests?return_offers=true` with `Duffel-Version: v2` and gzip support. It accepts IATA origin and destination codes, travel dates within 365 days, one to nine adults, a fixed cabin-class enum, and a bounded offer count. The configured default is 10 offers, with a hard cap of 50 and a default platform quota of 20 searches per user per UTC day.

A Duffel offer request creates a search resource. It does not move money or book travel. NyxID projects Duffel's response into bounded offer, slice, and segment fields. Orders, payments, cancellations, order reads, stays, cars, and Duffel's general API are not reachable through `flight_search`.

## Platform vendor isolation

Platform vendor rows hold shared credentials, but callers cannot use them through any generic path. Each canonical row carries an explicit empty `proxy_operation_policy`, which is a deny-all policy for actor-addressed requests.

Service creation and update force this policy for canonical vendor slugs. Operation binding rejects a row without the policy. Startup and bind-time backfills set the empty policy only on canonical vendor rows whose field is missing or null. The backfill never modifies another row or `credential_encrypted`.

The empty policy returns a not-found-shaped denial for both `/api/v1/proxy/{vendor-id}/...` and `/api/v1/proxy/s/{vendor-slug}/...` before credential decryption. The denial does not depend on `PLATFORM_REQUIRE_OPERATION_POLICY` or the platform-services feature flag. Generic MCP catalogs and dispatch, `/llm/*`, `/catalog`, and `list_catalog_all` also exclude the canonical rows. They never auto-provision into `UserService` rows or appear as platform-managed entries on `/keys`.

Only the code-owned platform operation path may select these rows. Its server-chosen authorization validates the vendor contract but deliberately ignores actor-addressed operation policies. `chrono-llm-public` keeps its existing auto-provision and access behavior.

## Operator activation

Turn on platform services in this order:

1. Grant `experimental:platform-services` to the intended users or organization. Keep the default off while staging.
2. Create the four canonical vendor rows from Admin Platform Operations and supply their master credentials.
3. Configure and enable each operation that should be available.
4. Set the per-request platform price on each vendor's normal service edit page. The operation card links to that page.

An enabled operation with a missing or invalid vendor row fails closed. A price that is absent remains absent in discovery; the UI reports "Price not set" instead of inventing a zero price.

The main implementation is in `models/platform_operation.rs`, `services/platform_operation_service.rs`, `handlers/platform_ops.rs`, `handlers/admin_platform_ops.rs`, and `services/mcp_service.rs`. Frontend schemas and hooks live in `frontend/src/schemas/platform-ops.ts` and `frontend/src/hooks/use-platform-ops.ts`.
