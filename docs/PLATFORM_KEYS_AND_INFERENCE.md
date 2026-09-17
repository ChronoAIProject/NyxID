# Platform keys, inference discovery, and billing lanes

Admin service creation, provider linking, and legacy vendor retirement are documented in [SERVICE_CONFIGURATION.md](SERVICE_CONFIGURATION.md).

NyxID 0.20.0 adds catalog inference metadata and an authenticated, owner-authorized
platform credential binding. A platform key is the catalog row's existing encrypted
master credential. It is never a new credential store and never appears in a client
response. Catalog availability describes an option; the connection's binding chooses
the credential used for execution.

## Stored data and response contracts

- `DownstreamService.inference` is optional. Its `wire_protocol` enum is
  `anthropic_messages`, `openai_responses`, or `openai_completions`; `model_list` and
  `realtime` are defaulted booleans. Unknown protocol values fail validation.
  The defaulted `inference_admin_modified` marker records explicit edits, including
  null clears, so startup never restores an admin-cleared block.
- `DownstreamService.platform_key` is optional, with defaulted `enabled`, an
  `audience` enum (`public` or `restricted`), and `allowed_owner_ids`. Owners are UUID
  strings identifying people or org users. The existing encrypted master credential
  holds the secret. Admin catalog PUT accepts write-only `credential` for replacement. Admin responses contain configuration,
  never secret bytes or lengths. Audit records contain metadata only: identifiers,
  changed field names and configuration state, without credentials or owner lists.
- `UserService.credential_binding` is an optional string: `platform` or `user`.
  Absent means the historical interpretation: no API key plus `auto_provision`
  source uses the platform path; other rows use their own credential. Truly no-auth
  rows still resolve without a credential and meter as `NoAuth`.
- Catalog list, full list, single entry, and MCP discovery carry optional inference,
  `platform_key: { available, pricing }`, and `byok_pricing`. Price views expose the
  metric, exact decimal credits per unit, and synchronization status, without Lago
  identifiers or diagnostic details. Keys expose the binding, availability, and the
  same price views, including immediate BYOK creation responses and assistant-reserved
  connection IDs.
- Inference `binding` and `status_slug` are computed, never stored. A caller who can
  use an authorized platform key receives `binding=platform`, without `status_slug`.
  Otherwise the binding is `user`; provider-linked rows use `ProviderConfig.slug`
  for `status_slug`, and other rows use the catalog slug. Clients look up the former
  in `/llm/status` by `provider_slug` (`ready`, `expired`, `not_connected`), and the
  latter in `/keys` by `catalog_service_slug`, checking `is_active` separately from
  credential status. A configured inference block does not itself grant access.

## Authorization, routing, and precedence

Platform availability requires an active HTTP service, enabled configuration, and
nonempty encrypted master credential. Providers requiring a user gateway URL cannot
use platform keys: enabling is rejected and runtime availability is false, preventing
placeholder catalog destinations from receiving the platform credential. Public audience allows authenticated owners;
restricted audience requires an explicit owner grant or an active org membership
whose role permits proxying. Org-owned connections retain the existing owner access,
member scope, and `admin_only` gates. A personal grant never grants another owner's
connection. Admin role alone does not bypass the platform-key execution ACL. Restricted checks
fetch active memberships once and batch-check person/org activity, then intersect
owner IDs in memory; query count is independent of allowlist size. Catalog and key
listings, auto-provisioning and reconciliation, MCP discovery and callable-service
loading, and LLM status/gateway checks share request-scoped `OwnerGrants` across all
service/owner checks. Key listing shares that snapshot with its provisioning and org
row traversal. Provider eligibility uses the already-loaded catalog/status provider
batch or one provider batch shared across the other listing/provisioning paths.
`available_with_grants` performs no database calls. Owner validation uses a single
`$in` query. No membership or provider eligibility data is cached across requests.

An absent platform configuration preserves the legacy public/internal/master-key
predicate, including its provider exclusion. Explicit disabled or restricted config
takes precedence over that implicit legacy grant. Existing private consent-backed
master credentials and no-auth auto-connections retain their existing behavior.

All credential resolvers authorize against the live catalog and owner before
decryption: streamlined and legacy proxy, HTTP and WebSocket, LLM gateway, MCP,
and delegated execution. Existing actor-addressed operation policies and exact
execution-authority approval checks remain in force. Server-selected surfaces retain
the legacy public/internal master predicate and also
accept explicit enabled/public configuration on that same shape. Explicit restricted
or disabled configs are denied without an actor. Anonymous/public execution retains
its existing authorization rules and gains no platform-key access.
Revocation yields the existing not-found-shaped unavailable-service error, including
for previously provisioned connections. Platform usage retains the per-user
`PLATFORM_SERVICE_RATE_LIMIT_*` gate and `NyxidManagedMaster` classification.

Personal connections keep precedence over legacy personal connections and org
fallback. A platform binding uses the live catalog destination and effective catalog
auth injection (including `ServiceProviderRequirement` for provider-linked seeds).

### Node transport hardening

A platform binding cannot accept a user destination, auth override, or node route that would send
the platform credential elsewhere. Node selection with a platform binding is a
validation error. Legacy internal master credentials also always use server transport;
existing owner-node bindings are ignored for those credentials. Owner nodes inject
only their own credentials. This is intentional hardening at both HTTP/WS and MCP
routing boundaries, including legacy rows without platform configuration. Agent credential overrides retain their existing behavior and
final credential classification; they do not bypass a revoked connection grant.

### Personal and org provisioning

Public platform services auto-provision through the existing idempotent lifecycle.
Restricted services provision only eligible personal owners and granted org owners.
Key listing, Agent Key login delivery (login options), and device-code
approval/onboarding for the acting person's own account invoke shared provisioning,
which may idempotently create org-owned auto-connected rows only through that
person's own active Member/Admin memberships with `can_proxy()` and explicit
platform-key grants. The 0.19.0 guarantee remains: org-targeted device
approval/onboarding resolves existing org services and never provisions rows for
the target org as a side effect of targeting. This limited reconciliation side
effect requires no org-admin action.
Org views badge these rows `auto_connected=true`. Removing the org grant removes
the automatic org rows and orphan endpoints on the next owner reconciliation; live
execution is refused immediately, before that cleanup. This new org
walk provisions explicit platform configurations only; legacy no-auth provisioning
remains personal unless an existing caller explicitly provisions an org owner. Stale automatic
rows are removed with orphan endpoint cleanup, allowing re-provisioning after a
grant returns. User-selected bindings retain their connection and any inactive
personal credential when access is revoked, so management can switch back to BYOK.
JWT/API-key authentication itself never provisions rows.
Authentication's `allow_auto_connected_services` union includes active same-owner
platform-bound rows as well as historical automatic rows.

## Connection and administration surfaces

### Credential replacement and audit

`PUT /services/{catalog-id}` accepts a write-only master `credential` through the same
envelope encryption used at catalog creation; the user `/connections/{id}/credential`
route remains a distinct connection operation. Credential and inference edits, and
platform configuration creation/update, emit metadata-only audit-chain events.
No secret value, ciphertext, length, or owner allowlist appears in those events.

### Admin catalog editing

Catalog responses expose `legacy_public_master` so admin editors and
`nyxid service show <catalog-id-or-slug> --catalog-admin` display
“enabled, public (implicit)” for eligible absent configurations. Explicit enable on
such a row defaults to public. Catalog CLI prices use the shared free/unit/pending
wording; missing discovery fields display “not configured”. Catalog update 404s
explain that a connection ID cannot identify the catalog row. CLI slug lookup uses
the admin service listing, whose responses carry catalog IDs; discovery entries do
not carry IDs. Use a catalog ID for rows absent from that listing, such as disabled
services.

### User platform connection editing

`POST /keys` accepts `use_platform_key` (default false for existing callers). It is
exclusive with credential/OAuth inputs, custom destination/auth, and node routing.
The unified key service owns provisioning for normal, hosted, and assistant callers.
Assistant schema revision v9 opts into `catalogService.use_platform_key`; default
and revisions v4-v8 remain byte-compatible with deployed Aevatar pins.

Key updates accept `use_platform_key`: true selects an available platform key; false
requires a fresh credential or the existing OAuth setup path, including the existing
raw/copy custom-app credential inputs. Explicit platform-bound rows may update
`label`, `admin_only`, `recommended_skills`, `custom_user_agent`, and
`default_request_headers`, plus Disable/Enable. Endpoint, auth, node, OpenAPI, identity,
forward-token and delegation fields stay locked with a field-naming “Switch to your
own key to change …” error. Automatic rows remain managed by reconciliation. Switching to platform
retains the personal `UserApiKey`; it does not silently delete credentials or their
other consumers. Actual user credential replacement retains the pipeline-based
credential-epoch bump. Automatic rows may be adopted into a user-managed connection
when choosing a binding; their endpoint becomes editable only for BYOK.

Hosted connect links persist the creator's optional choice, offer the same available
choice to the authenticated human, and complete through the existing serialized,
single-use unified provisioning path. Public previews expose no owner authorization
decision or secret. MCP/assistant connection parameters accept the same choice.

The CLI offers `service add --platform-key`, interactive choice, binding in service
list/show (`nyxid keys` also lists connections), and `service update --use-platform-key|--use-own-key`. Admin update/add
accept inference protocol/model-list/realtime controls, platform enable/audience/
owner grants, the existing write-only credential environment input, and lane metric/
price controls. Owner resolution reuses UUID/org slug/display-name resolution.

The frontend uses the existing compact form primitives and design tokens. Eligible
connections default to “Use NyxID's key” and show both lane prices before submission.
Key details show and switch binding. Admin service editing includes Inference,
Platform key (write-only credential, audience and owner selection), and two billing
lane cards. Legacy billing is labeled superseded while lanes are configured.

## Billing lanes and durable accounting

Platform-key usage is billed to the requesting person regardless of the granting audience.
An organization grant authorizes its members to use NyxID's key; it never charges that
organization's wallet for the master credential. `BillingOwnerResolver::resolve_for_execution`
uses the final credential class: `NyxidManagedMaster` selects the person's wallet and billing
rollout flag, while org BYOK and agent override credentials retain org-wallet billing.
Resource authorization, approval ownership, and rate limiting are unchanged.

`ServiceBilling` gains optional `byok_pricing` and `platform_key_pricing`, each a
`LanePricing { metric, credits_per_unit, lago_metric_code, sync_status, sync_error }`.
Decimals use the existing exact normalization. Server-owned codes are stable:
`platform_svc_{slug}_byok` and `platform_svc_{slug}_pk`; the legacy code is unchanged.
Each lane has its own durable cleanup marker. Client input cannot author sync state,
metric codes, cleanup markers, or upstream error details.

| Final credential class | Lane |
| --- | --- |
| UserOwned, AgentOverrideUserOwned, NodeManaged | BYOK |
| NyxidManagedMaster | Platform key |
| NoAuth | None (meter only) |

At least one configured lane selects lane mode. A missing matching lane is free,
even if legacy platform billing is enabled. A synced matching lane supplies the
platform charge's metric/code. A pending or failed matching lane temporarily uses
the prior legacy platform configuration (or free), matching existing price-sync
rollout behavior. With no lanes, legacy behavior is unchanged. Clearing the last
lane returns the service to legacy mode. Resale is an independent layer, with its
existing flag and credential-class gates unchanged.

Synchronization reuses the Lago standard-charge implementation and round-trips the
entire plan charge array with IDs. Both lanes retry pending/failed writes and durable
charge cleanup on the existing reconcile interval. Concurrent admin changes fence
sync completion against the current price and metric so stale syncs cannot activate
an obsolete configuration.

Lane selection happens in the shared billing route context after final credential
classification and before wallet gating/reservation. Existing metering, actual-unit
allowances, expiring grants, wallet funding, settlement, Lago outbox, and dashboard
queries consume the selected metric. Provider-reported token usage remains the
authoritative token input. Capture recognizes token pricing on either configured lane,
including pending lanes and non-`llm-` slugs, for JSON and SSE. MCP estimates tokens
only for token-metered services; other services report zero tokens unless the body
actually carries provider usage.

When lanes use different units, each request's final `ctx.platform_metric` controls
reservation and settlement allowance matching. Admin allowance create/update accepts
optional `metric`: it must match a configured lane (or a legacy metric still used
while a lane is pending/failed). Omitting it chooses BYOK's metric first, otherwise
the platform-key metric, otherwise the legacy service default. The existing
`effective_platform_metric` display field uses that same deterministic default;
it is not a claim that every credential lane has that unit. The allowance UI offers
a unit selector for mixed lanes. Existing allowances keep their stored unit on
unrelated updates, including older clients repeating the same service reference, and only
fund requests with that matching unit. Ledger canonical fields, order, hash derivation, dedupe
keys, and verification are unchanged; lane charges use the existing platform layer
and reference usage rows that distinguish lanes by metric code and credential class.


Lane-only admin updates preserve omitted legacy platform and resale fields, including
the pending-sync fallback. Legacy billing-only updates retain their existing full-block
semantics; omitted new lanes are always preserved, and explicit null clears a lane.

## Inference defaults and transports

Startup fills only absent/null inference blocks whose `inference_admin_modified`
marker is absent/false, including admin-created Chrono rows. It never replaces an
admin-authored block or an explicit null clear.

| Catalog slug | Protocol | Model list | Realtime |
| --- | --- | --- | --- |
| llm-openai | openai_responses | true | true |
| llm-anthropic | anthropic_messages | true | false |
| llm-deepseek, llm-mistral, llm-openrouter | openai_completions | true | false |
| chrono-llm, chrono-llm-public | openai_completions | true | false |
| llm-xai | openai_completions | true | true |

OpenAI, Anthropic, DeepSeek, Mistral and OpenRouter document `GET /models`.
Anthropic's list uses the familiar `data` model array with its own pagination fields;
clients must not require OpenAI-only envelope fields. The Chrono contract is recorded
in `docs/chat/direct-chronollm-spec.md`. Chrono public resolves its legacy platform
binding; Chrono BYOK uses `status_slug=chrono-llm`. Google AI and Cohere have no block.
Codex is deliberately omitted: its seed points to
`https://chatgpt.com/backend-api/codex`, with a dedicated translator and device OAuth;
it is not a generic OpenAI chat-completions/model-list upstream.

xAI is an API-key provider at `https://api.x.ai/v1`, documented at
`https://docs.x.ai`, with a bearer-injected `llm-xai` service. No catalog overlay is
invented. The existing WS URL join maps the xAI/OpenAI `/realtime` proxy paths to
`wss://api.x.ai/v1/realtime` and `wss://api.openai.com/v1/realtime`; upgrade construction
injects the bearer credential server-side. Tests prove URL and credential construction;
this capability describes transport, not upstream account/model entitlement.

## Migration and verification

All stored additions are optional/defaulted. Existing IDs, routes, credential stores,
response fields, metric codes, and ledger encodings remain stable. Omitted connection
choice keeps deployed clients' BYOK creation behavior. New choices and lane pricing
must execute on 0.20-capable replicas; old binaries cannot enforce fields they do not
understand. Upgrade execution replicas before enabling new platform configurations or
lane prices. Existing rows continue working throughout the rolling upgrade.

Coverage includes live Mongo ACL/revocation and binding lifecycle, all resolver entry
points, serialized connect completion, seed/backfill guards, inference response shapes,
WS construction, lane selection and Lago sync/removal, funded settlement and ledger
verification, CLI parsing/rendering, and frontend schemas/components. Required full
Rust tests/clippy/format and frontend lint/test/build run before final commits; wizard
assets are rebuilt when their inputs change. Release versions and lockfiles move
together to 0.20.0.

### Operator examples

```sh
nyxid service update <catalog-id> --catalog-admin --platform-key-enabled true \
  --platform-key-audience restricted --platform-key-allow <owner-uuid> \
  --credential-env UPSTREAM_KEY --platform-key-metric tokens --platform-key-price 0.00001 --byok-free
nyxid service add llm-xai --platform-key
nyxid service update <connection-id> --use-own-key --credential-env MY_XAI_KEY
nyxid service update <connection-id> --use-platform-key
```

Admin catalog flags select catalog administration; `--catalog-admin` also permits
credential-only or name/URL edits. Without catalog flags, add/update retain their
existing user-connection routes. Owner arguments accept person/org UUIDs and the
existing org slug/display-name resolver; the admin UI reuses the people/org picker.
Admin inference flags are `--inference-protocol`, `--inference-model-list`, and
`--inference-realtime`; `--inference-protocol none` clears metadata. Lane flags are
`--byok-metric`, `--byok-price`, `--byok-free`, `--platform-key-metric`,
`--platform-key-price`, and `--platform-key-free`. Catalog create uses `service add
<slug> --catalog-admin --endpoint-url <url> --label <name>` with the same controls.

### Upstream capability evidence

- xAI [Models REST API](https://docs.x.ai/developers/rest-api-reference/inference/models.md): `GET /v1/models`, OpenAI-style `data`/model objects.
- xAI [Voice agent guide](https://docs.x.ai/docs/guides/voice/agent): bearer-authenticated `wss://api.x.ai/v1/realtime`.
- OpenAI [Models](https://developers.openai.com/api/reference/resources/models/methods/list), [DeepSeek models](https://api-docs.deepseek.com/api/list-models), [Mistral models](https://docs.mistral.ai/api/endpoint/models), [Anthropic models](https://docs.anthropic.com/en/api/models-list), and [OpenRouter models](https://openrouter.ai/api/v1/models) establish model-list capability. Transport construction tests cover OpenAI and xAI realtime; no paid upstream session is required for the local test suite.

## Service-instance history

Explicit platform/user credential-binding transitions and platform-instance settings are recorded in the service journal. Automatic provisioning and removal use verified system attribution; retained UUID history remains available after physical cleanup under current owner/admin scope. Re-provisioning starts a new instance history even when the slug is reused. Platform credential material is never included. See [SERVICE_HISTORY.md](SERVICE_HISTORY.md) for capture, safe details, archive discovery, and MongoDB transaction prerequisites.
