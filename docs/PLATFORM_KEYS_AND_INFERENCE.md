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
service/owner checks. Key listing shares that snapshot with org row traversal and,
for human/delegated callers, provisioning. API-key inventory reads never provision
or reconcile rows. Provider eligibility uses the already-loaded catalog/status provider
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

Public platform services auto-provision one personal row per active person when
no active or user-disabled connection to that catalog service already exists.
They never create org-owned auto-connected rows, even when the person is an active
Member/Admin with `can_proxy()`. Restricted services provision a personal row only
when the person is in `allowed_owner_ids`, and an org row only when that org is in
`allowed_owner_ids` and is reached through an active `can_proxy()` membership.
Human and delegated key listing, Agent Key login delivery (login options), and
device-code approval/onboarding for the acting person's own account invoke this
shared provisioning. The 0.19.0 guarantee remains: org-targeted device
approval/onboarding resolves existing org services and never provisions rows for
the target org as a side effect of targeting. Legacy no-auth provisioning remains
personal-only during the org walk. Org views badge eligible rows
`auto_connected=true`; removing an org grant removes the automatic org rows and
orphan endpoints on the next owner reconciliation. Stale automatic rows are
removed with orphan endpoint cleanup, allowing re-provisioning after a grant
returns. JWT/API-key authentication itself never provisions rows.

Active connections and inactive non-auto `UserService` rows whose endpoint still
exists block provisioning for both personal and org owners. These disabled rows
retain the user's choice and can be enabled on their original slug. Deleted
tombstones (inactive non-auto rows whose endpoint is gone) do not block a
replacement, and the partial active slug index lets the new auto-connected row
reuse the catalog slug instead of receiving a `-2` suffix. Inactive automatic rows
are reconciled away before provisioning.

The startup sweep `cleanup_public_org_auto_provisions` removes pre-fix public
platform rows owned by organizations, along with orphan endpoints. It deletes
unshared credentials first, unshared endpoints next, and the service row last,
without transactions, so it also works on standalone MongoDB. Interrupted runs
retain the service's resource references for retry; rows whose endpoint is already
gone are hidden by `/keys` and removed on the next sweep. Failures log deletion
counts without failing startup. The sweep is idempotent and leaves personal rows,
restricted org rows, and explicit (`source != auto_provision`) org platform
bindings untouched. Those explicit
public bindings continue to resolve under the existing execution ACL: public
audience permits any active authenticated owner. The personal-only automatic
provisioning rule does not narrow explicit public execution access. Authentication's
`allow_auto_connected_services` union includes active same-owner automatic rows
and explicit platform bindings; it does not require public org automatic rows.
As with listing reconciliation, saved API-key allowlists and agent bindings are
not rewritten: deleted service UUIDs cannot resolve or grant access to a replacement.
Automatic rows never create credentials; startup defensively deletes any orphan
credential attached to a malformed legacy row.

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

`ServiceBilling.byok_pricing` and `platform_key_pricing` are optional `LanePricing`
blocks. Existing `metric`, decimal `credits_per_unit`, `lago_metric_code`, `sync_status`
and `sync_error` fields remain the primary component. Optional/defaulted `components`
adds objects with those same five fields. Metrics must be unique across the primary
and additional components of each lane. Supported units are `tokens` (provider total),
`requests`, `bytes`, `input_tokens`, `output_tokens`, `cache_read_tokens`,
`cache_write_tokens`, and `images`. Legacy `platform_metric` and `resale_metric` still
accept only tokens/requests/bytes. Backend `BillingMetric` metadata and frontend
`schemas/billing-metrics.ts` / CLI `commands/billing_units.rs` centralize unit names and labels.

| Final credential class | Lane |
| --- | --- |
| UserOwned, NyxidPlatformOauthApp, AgentOverrideUserOwned, NodeManaged | BYOK |
| NyxidManagedMaster | Platform key |
| NoAuth | None (meter only) |

At least one configured lane selects lane mode. A missing matching lane is free,
even if legacy platform billing is enabled. While the selected lane's primary is
pending or failed, the whole lane uses the legacy configuration (or is free);
all additional components are ignored, even if synced. Once the primary is synced,
it and every synced additional component create separate platform usage rows using
their own metric/code. Unsynced additional components are free until they sync and
never add a legacy charge. For example, a synced input primary and pending output
component charges only input; after output syncs, both charge. Total `tokens` is
charged only when explicitly configured.
With no lanes, legacy behavior is unchanged. Resale remains independent. The
`platform_charge_nyxid_credentials_only` restriction still applies after lane selection.

Stable primary Lago codes remain `platform_svc_{slug}_byok` / `platform_svc_{slug}_pk`;
additional components use `platform_svc_{slug}_{byok|pk}_{metric}`. Each price has its
own rate-cache row and synchronization state. Removed components are recorded in
server-owned `component_cleanup_metric_codes`, including when their entire lane is
removed. The same pricing synchronizer, full Lago plan charge array with IDs, and
reconcile interval handle all charges. Price/metric/code fences prevent stale admin
sync completions from activating obsolete prices; stale writes after removal restore
cleanup intent. Clients cannot control metric codes, sync state, or cleanup markers.

Older admin payloads omitting lanes or nested `components` preserve them. A null lane
clears the entire lane; `components: null` or `[]` clears only additional components.
Lane-only edits preserve omitted legacy fallback and resale fields. Legacy-only edits
retain their historical full-block behavior while preserving omitted lanes.

Capture checks token-family and image metrics on **both** configured lanes, including
pending components and non-`llm-` slugs. JSON, accumulated SSE, realtime WS, node and
MCP paths feed normalized `PlatformUsage` classes. OpenAI cached prompt/input tokens
and Gemini `cachedContentTokenCount` are subsets of input and are subtracted from
priced `input_tokens`, clamped at zero. Anthropic cache-read/cache-creation counts
are outside input and are kept separate without subtraction. Output tokens are their
own class. `TokenBreakdown` preserves provider accounting for display; normalized
classes are priced. Successful OpenAI `/images/generations`, `/images/edits`, and
`/images/variations` responses count `data[]` entries, including paths with proxy or
version prefixes; image usage can also populate all token classes. Completed image
SSE events count once per image index; partial previews do not count. Capture uses
existing bounded bodies/buffers and never reads an additional response body. MCP
estimates tokens only for token-family services when reported usage is absent.
Without provider-reported usage, input/output/cache token classes are zero while
legacy `tokens` still uses the byte estimate, so per-class pricing requires
providers that report usage.

Each platform component has independent allowance -> grant -> wallet funding,
settlement, ledger reference and Lago event. The primary transaction identity remains
unchanged; additional rows append `:component:{metric_code}`. A durable primary-row
`pending_platform_usage` snapshot lets reconciliation finish partially materialized
component settlements. Repeated opens/settlements reuse the same identities. Estimates
use the request-byte token estimator for `tokens`, `input_tokens`, and
`output_tokens` (the best available output proxy); `cache_read_tokens` and
`cache_write_tokens` reserve one unit because the input estimate already covers
cache quantities. Images use the request's `n` (default 1); requests and bytes
reserve one unit. The request body is parsed for `n` only when an active platform
price uses images. Standalone legacy metrics and resale retain their existing
one-unit reservation gate. Zero final units release every hold
and emit no Lago event. Platform-key execution still bills the acting person.

An allowance may select any configured primary/component unit on either lane, plus
the legacy fallback while any lane primary is unsynced (or when no lanes exist).
An unsynced additional component never re-enables the fallback metric. Admin service
responses expose this computed, non-stored list as `allowance_metrics`; the allowance
dialog consumes it directly. Allowances fund only identical-metric usage rows.
Omitted allowance metric defaults to BYOK primary, then platform-key primary,
then legacy; `effective_platform_metric` remains this display default.
Existing allowances preserve their stored unit on unrelated edits. Periods, recurrence,
grant expiry, ledger canonical fields/order/hash/dedupe keys and verification are unchanged.

Unit prices support `PRICE_FRACTIONAL_DIGITS = 12` and at most 1,000,000 credits/unit.
The normalized exact decimal goes to Lago. Optional `credits_per_unit_pico` (10^-12
credits) is preferred in cache/funding/reservations; legacy `credits_per_unit_micros`
is still populated by truncation for rolling compatibility. Missing precise fields
use the old micro rate exactly. All money multiplication uses saturating integer i128
intermediates. Gross/funding display costs truncate **after** multiplying to micros;
grant movements remain micros; the exact remaining wallet cost rounds **up** to whole
credits per component, including sub-microcredit costs. Lago receives the wallet-funded quantity,
rounded up to its existing micro-unit precision, capped at actual units. No floating
point is used in rate or cost arithmetic. Ledger amount encoding remains unchanged.

**Rollout:** upgrade ALL replicas before authoring component prices, allowances
using new metrics, or prices beyond six fractional digits. Old binaries cannot deserialize the new enum variants or charge
additional components. Defaulted fields require no data migration; existing lanes and
prices of up to six fractional digits keep their prior accounting.

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
`--platform-key-price`, and `--platform-key-free`. Repeat `--byok-component <metric>=<price>` or
`--platform-key-component <metric>=<price>` to add/update individual components.
`--byok-clear-components` / `--platform-key-clear-components` removes additional prices
while retaining the primary. Prices accept up to 12 fractional digits. Catalog create uses `service add
<slug> --catalog-admin --endpoint-url <url> --label <name>` with the same controls.

### Upstream capability evidence

- xAI [Models REST API](https://docs.x.ai/developers/rest-api-reference/inference/models.md): `GET /v1/models`, OpenAI-style `data`/model objects.
- xAI [Voice agent guide](https://docs.x.ai/docs/guides/voice/agent): bearer-authenticated `wss://api.x.ai/v1/realtime`.
- OpenAI [Models](https://developers.openai.com/api/reference/resources/models/methods/list), [DeepSeek models](https://api-docs.deepseek.com/api/list-models), [Mistral models](https://docs.mistral.ai/api/endpoint/models), [Anthropic models](https://docs.anthropic.com/en/api/models-list), and [OpenRouter models](https://openrouter.ai/api/v1/models) establish model-list capability. Transport construction tests cover OpenAI and xAI realtime; no paid upstream session is required for the local test suite.

## Service-instance history

Explicit platform/user credential-binding transitions and platform-instance settings are recorded in the service journal. Automatic provisioning and removal use verified system attribution; retained UUID history remains available after physical cleanup under current owner/admin scope. Re-provisioning starts a new instance history even when the slug is reused. Platform credential material is never included. See [SERVICE_HISTORY.md](SERVICE_HISTORY.md) for capture, safe details, archive discovery, and MongoDB transaction prerequisites.
