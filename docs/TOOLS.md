# Tools

Tools are catalog services users call through a NyxID-held credential, their own API key, or without authentication. `/tools` groups them by supplier with topics, prices and published operations. `/admin/tools` provides Admin and Tools Manager workspaces. Tools Managers may edit their own marked offerings; provider administration remains under Catalog Manager/Admin authority. AI Services (`/keys`) retains a person's BYOK bindings to Tool rows while hiding their platform bindings.

## Model and execution boundary

`DownstreamService.offering_kind` defaults to `ai_service`; curated offerings use `tool`. Tools require `service_category = internal` and `requires_user_credential = false`, with either `auth_method = none`, a stored master credential, or platform-key configuration. A master credential and `provider_config_id` remain mutually exclusive. Authenticated runtime twins are provider-less and use the existing encrypted master credential and live public/restricted platform ACL.

`topics` allows up to 20 unique slugs from `GET /api/v1/tools/topics`. `supplier` identifies the API operator (128 characters maximum). `import_source` records kind (`monid`, `vendor_spec`, `manual`, `catalog_twin`), reference (512 characters), optional version (128 characters) and optional `imported_at` (RFC3339 in the API, BSON datetime in MongoDB). Twins set `imported_at` to their creation time. On update, omitted `supplier` or `import_source` remains unchanged, while JSON `null` clears it.

Endpoint classifications are optional `data_scope` (`public`, `account`, `owned_resource`), optional `cost_class` (`free`, `metered`, `resource_backed`) and default `execution = http_operation` (`job_start` and `job_poll` are metadata options). They neither override billing nor introduce a job engine. All additions are serde-defaulted for legacy records.

## Publication

New endpoints on tool rows start inactive and `draft`, whether created manually, discovered, or synchronized from a spec. Non-tool creation retains its existing behavior. Legacy endpoint rows default to `published`; flipping an existing service does not reset its live operations.

States are `draft`, `validated`, `published` and `paused`. Only published endpoints are active. Admins publish; Tools Managers prepare, validate and pause their own marked operations. Execution/contract edits atomically demote live operations and advance generation; no-op sync preserves state. Service contract changes and operation route-shape changes conservatively demote all live operations to avoid exposing a broader published route when a blocking route moves. Other operation contract edits demote only the edited row. Admin-only endpoint deletion uses the same transaction and chained audit. Bulk publication and its audit appends commit in one transaction; any failure rolls back the entire batch. Equally specific matching rows must all be published and active. Each publication request advances the positive `operation_generation` and appends `catalog_endpoint_publication_changed` through the audit service. HTTP and WebSocket execution require a published, active method/path-template match. Unmatched or unpublished operations return HTTP 404 / code 12700. Unconfigured public twins also receive this publication error before credential readiness checks. MCP omits unpublished operations and returns `isError` carrying 12700 when one is called.

MCP names remain `{service_slug}__{endpoint.name}`. A source connection and its twin have distinct slugs and tool sets. MCP discovery includes tools; human catalog browsing excludes them unless `offering_kind=tool` or `include_all=true` is requested.

## Enable an existing API

There are no tool seeds. Admins create and configure offerings at runtime through the API, CLI, or Tools workspace.

For an eligible existing internal, provider-less row such as Chrono LLM, flip in place:

```sh
nyxid service update --catalog-admin chrono-llm-public --offering-kind tool
```

Its UUID, slug, credential, inference metadata and published endpoints remain intact. Existing BYOK connections remain visible under AI Services.

For a provider-linked or connection service, create a tool twin:

```sh
nyxid service add --catalog-admin --twin-of api-firecrawl \
  --slug tools-firecrawl --offering-kind tool \
  --topic web-search --topic page-fetch --supplier Firecrawl
nyxid catalog endpoint list tools-firecrawl
nyxid catalog publish tools-firecrawl --operation search --operation scrape
```

`POST /api/v1/services` accepts `twin_of_service_id` as a catalog UUID or slug and requires the new slug. An Admin-created twin copies base URL, destination targets, auth method/key name, OpenAPI/AsyncAPI URLs, default headers, user agent, capabilities, description, documentation links, auth notes, limitations and required permissions. Request fields override copied values. It sets an internal, provider-less tool with no credential and defaults the platform audience to enabled/public unless overridden. Manager-created twins use the restricted allowlist and empty audience described below. Source credentials are never copied; a credential supplied at twin creation is rejected. Configure it afterward through Service settings or `service update --credential-env`.

All source endpoint rows receive new UUIDs, generation 1, draft publication and inactive state. Catalog creation and copied endpoints commit transactionally. The source is unchanged. Provenance records `catalog_twin`, the source slug and its updated-at RFC3339 version. Twins share the source's spec URL. An omitted offering kind is inferred as Tool for twin creation before manager preflight; other caller-provided fields still pass the full restricted classifier. Exact twin retries with the same skills_request_id bind caller intent and source version, excluding only the server-generated attempt clock, and replay the original row and draft operation IDs. Explicit Tool discovery is authoritative: omitted published operations become inactive drafts with an advanced generation and a chained withdrawal audit. Their rows remain as blocking routes. Unchanged contracts preserve decisions. Route-shape changes (name, method, path or destination target) and endpoint deletion demote all published operations; description/schema-only edits demote only the edited operation. New operations remain drafts. Changed operations require renewed Admin publication. Manager-created Tools use explicit authorized discovery; automatic startup/background spec synchronization leaves these rows alone.

Phase 1 targets are Firecrawl (`api-firecrawl`, bearer), X (`api-twitter`, bearer), ElevenLabs (`api-elevenlabs`, `xi-api-key` header), Twilio (`api-twilio`, `Authorization` header containing the Basic value), and Chrono LLM (flip in place). Chrono rows are admin-created rather than seeded; configure an eligible row first on a fresh deployment.

The existing `api-firecrawl` hosted overlay exposes `scrape` with markdown,
`waitFor`/`timeout`, `maxAge`, and `storeInCache`, plus `map_site` with
`sitemap` (`include`, `only`, `skip`), `ignoreQueryParameters`, `ignoreCache`,
`limit`, and `timeout`. The suggested 10-second render wait, 60-second timeout,
and 200-URL map limit are agent guidance, not NyxID request validation. These
are provider controls in the existing connection,
not a NyxID fetch service. The schema keeps provider extension options open.
Request `markdown` when the page body matters; a title-only result is incomplete.
`maxAge` is cache eligibility and `ignoreCache` bypasses the sitemap cache;
neither establishes a live scrape from the response alone. Report `sourceURL`,
final `url`, `statusCode`, and `provider` only when returned. For private or
authenticated pages, use an authorized machine browser instead of forwarding
cookies, headers, or page contents to a public extractor. The active NyxAgent
prompt gives NyxBot and specialists this workflow within their existing grants.

NyxBot's current priority is reliable, cost-effective execution of NyxID services,
connections, and tools. Native GPT image search is deferred for later discussion;
this integration does not enable it. Image generation belongs to a dedicated
NyxID-connected service and follows the existing authorization and usage paths.

For a new API, use Add tool → New service, or CLI `service add --catalog-admin --slug … --endpoint-url … --auth-method … --service-category internal --offering-kind tool --openapi-spec-url …`. Configure a master credential/platform grant and pricing in Service settings. Run `catalog discover` to populate draft operations from the spec, then review and publish selected operations.

## Manifest CLI and wizard

Run `nyxid admin tools add --template firecrawl` in a terminal, or use a typed
YAML/JSON manifest. The wizard asks for twin/new API, concise metadata, hidden
first credential and operation selection. Use `--save tools.json` to save its
final selection; saved documents contain only metadata and an optional credential
environment-variable name. `--skip-credentials` prepares a draft and reports
`pending_credential`. `--credential-env NAME` overrides one Tool's environment
name. A noninteractive add needs `--yes` and sufficient `--file`/`--template`
data. Apply and validate never prompt.

```yaml
version: 1
variables:
  vendor:
    default: local-example
    allowed: [local-example, staging-example]
tools:
  - slug: tools-${vendor}
    name: Example API
    base_url: https://api.example.test
    auth_method: bearer
    openapi_spec_url: https://api.example.test/openapi.json
    homepage_url: https://example.test
    auth_notes: Vendor bearer key required
    credential_env: EXAMPLE_TOOL_CREDENTIAL
    operations: [search]
    lanes: [platform, byok]
```

```sh
nyxid admin tools apply --file tools.yaml --set vendor=local-example --dry-run
nyxid admin tools apply --file tools.yaml --set vendor=local-example
nyxid admin tools validate --file tools.yaml --output json
nyxid admin tools add --template firecrawl --yes --credential-env TOOLS_FIRECRAWL_CREDENTIAL --save tools.json
nyxid admin tools icon tools-firecrawl --file firecrawl.svg
```

The Firecrawl template uses `twin_of: api-firecrawl`, selects `search` and
`scrape`, and names `TOOLS_FIRECRAWL_CREDENTIAL`. Replace `twin_of` with
`base_url` and supported `auth_method` for a new API. Header/query auth requires
`auth_key_name`; bearer/basic/none omit it. Unknown manifest fields are rejected
before writes. Raw credentials, authorization headers, audience, billing,
provider links and provenance cannot be authored in this document.
The optional CLI-only `lanes` field declares a unique, nonempty subset of
`platform` and `byok`; it defaults to `[platform]` and never reaches the service
API. A no-auth Tool cannot declare `byok`.
`name` is optional for twins: creation inherits the source name and reapply
preserves the existing name. New APIs require a nonempty `name`.

Supported descriptive fields are description, supplier, topics, homepage,
repository, issues/examples URLs, uploaded `icon_url`, safe capability flags,
auth notes, known limitations and required permissions. AsyncAPI and custom
user-agent settings require an explicit Admin update outside onboarding.
Variables use `--set` > interactive answer > default; values accept only letters,
digits, dot, hyphen and underscore. Undeclared/unresolved variables are rejected.
Variables never load credentials.

Apply reads inactive rows, checks shape/source/ownership, reconciles only changed
metadata, discovers only when no endpoints exist, and sets a credential only
when never configured. Spec drift requires explicit review rather than automatic
reconciliation. Complete reruns report `unchanged` with no mutations. Omitted
`operations` preserves all publication choices; `operations: []` pauses every
published operation. Missing selected names fail. Admin apply publishes the
selected set; Manager apply validates it and supplies Admin handoff commands.

Dry run and validate are read-only and do not access credential environment
values, refresh tokens, discover or publish. Validate emits version-1 structured
readiness with credential/audience/publication reasons, primary/component and
operation sync/effective prices, ordinary-user eligibility, a safe GET test
command when one is available, Admin commands and Lark fields. Restricted
membership is explicitly unevaluated; an Admin preview does not establish user
eligibility. Legacy Lago plan prices unavailable through metadata are reported
as unknown. Exit codes: `0` ready (or successful apply/dry-run), `2` pending setup,
`1` failed. Saved manifests and reports never contain credential values.

Validate reports readiness separately for every declared lane. The platform
lane checks the shared credential (or no-auth), live audience configuration,
publication and effective platform prices. The BYOK lane checks authenticated
injection, publication and effective BYOK prices, and prints the existing
`nyxid service add <tool-slug> --credential-env VENDOR_API_KEY` connection command. Overall
`ready` requires every declared lane. Missing BYOK or platform pricing is an
Admin advisory; an intentionally free lane remains allowed. The Lark row
records the declared lanes and its status means configuration readiness.

Every report retains `verification.status: not_verified`. For each declared
lane, `pending_gates` contains `outside_user_platform_call_admin_confirmed`
or `outside_user_byok_call_admin_confirmed`. An Admin must confirm that an
outside user called the published Tool against the real vendor and that usage
was billed to that person on the declared lane. For BYOK the user connects
their own vendor API key; for no-auth only the call needs confirmation. Record
the confirmation in the Lark row and a redacted verification-report line.
Local fake-vendor tests establish software behavior and leave these gates open.

The historical `scripts/tools/add-tool-twin.sh` remains a compatibility wrapper
that generates a secret-free manifest and invokes apply/validate. New automation
should use `admin tools` directly. Existing low-level commands remain supported.

## API and authority

The seeded `tools_manager` system role carries `nyxid:catalog:tools:write` and requires platform Operator membership. Admins retain full authority. `capabilities.manage_tools` controls the management page and navigation. Managers require current role, creator identity and the server-owned `tools_manager_created` marker on every write; no legacy takeover or backfill is allowed. Both-role users retain separate catalog and Tool authority. Writes require first-party human accounts; delegated metadata reads retain `account:read` parity. Managers cannot delete/reactivate rows, transfer ownership, change billing/audience/inference, identity/token forwarding, destination targets, headers, WS injection or anonymous access. The first nonempty credential is installed transactionally with destination and live-authority fences. Its durable freeze survives Admin clearing; replacement, clearing and subsequent destination changes require an Admin. Manager twins sanitize inherited configuration and start with a restricted, empty audience pending Admin approval. Tools metadata reads accept JWT/session or general API keys and allow delegated `account:read` parity. Static topic vocabulary is public.

| Method | Route | Behavior |
|---|---|---|
| GET | `/api/v1/tools` | Published ready tools under live platform ACL; admins can see missing-credential offerings |
| GET | `/api/v1/tools/{slug}` | One visible offering |
| GET | `/api/v1/tools/topics` | Controlled topic vocabulary |
| POST | `/api/v1/services` | Create a new row, optionally with `twin_of_service_id` |
| PUT | `/api/v1/services/{id}` | Authorized metadata/first credential; Admin-only flip/replacement/clear |
| POST | `/api/v1/services/{id}/icon` | Validate, store and attach a static Tool SVG with live authority recheck |
| POST | `/api/v1/services/{id}/endpoints/{endpoint_id}/publication` | One operation's state |
| POST | `/api/v1/services/{id}/publication` | 1–200 distinct explicit endpoint names |

The Tools API batches memberships, provider metadata and published operations, and checks credential presence without decrypting secrets. `limits` is `null` when `PLATFORM_SERVICE_RATE_LIMIT_PER_SECOND=0` (disabled), otherwise `{ rate_limit_per_second, burst }`; the UI displays “No per-user rate limit” for null. Ordinary users do not see authenticated offerings without credentials or offerings without published operations. `GET /keys?include_tool_bindings=true` includes platform Tool bindings in the same key response shape (`offering_kind: "tool"`, `credential_binding: "platform"`) without widening owner, API-key scope or delegated access. Default `/keys` hides only platform tool bindings; BYOK bindings and key details remain available. Agent grant selection includes hidden platform tool connections.

## CLI

Catalog targets accept UUIDs or slugs; endpoint targets accept IDs or names. Tables include publication state; `--output json` emits raw API responses. Catalog creation requires `--endpoint-url` or `--twin-of` and otherwise fails locally with `Catalog creation requires --endpoint-url or --twin-of`. Tools tables render pricing as `Free` or `<credits> credits / <metric>`, limits as `<n>/s, burst <b>` or `none`, and comma-joined topics. `tools show` prints an operation table with name, method, path, risk, data scope and cost class.

- `service add|update --catalog-admin`: offering kind, service category, repeated topics, supplier and spec URL; add accepts `--twin-of` and `--slug`; update accepts `--clear-topics` and `--clear-supplier` (mutually exclusive with `--supplier`).
- `catalog endpoint list <service> [--published-only]`; add/update support operation metadata, parameter/body JSON files and classifications; disable pauses an endpoint.
- `catalog discover <service>`; `catalog publish <service> --operation … [--state …]`; `catalog topics`.
- `tools list [--topic …]`; `tools show <slug>`.

## Billing and planned Phase 2

Tools reuse `ServiceBilling.platform_key_pricing` and optional `byok_pricing`. The platform lane uses NyxID's master credential and audience ACL; a user binds a BYOK key through AI Services (`/keys`). A restricted-empty platform audience does not deny that user's published BYOK operation. Both lanes require published operations and bill the acting person. OAuth user-account connections remain on provider-linked Catalog services; Tools remain provider-less. Free/absent lanes display Free; priced lanes display decimal credits per metric. Cost-class metadata does not override prices. A Requests lane can also price published operations individually by endpoint name (see [Operation prices](PLATFORM_KEYS_AND_INFERENCE.md#operation-prices)). The tools card shows the effective request price range and each operation's price in its operation list; pending prices show the base rate until they sync. Upgrade every replica before authoring operation prices: older binaries ignore them and charge the base rate. Existing platform ACLs, personal billing ownership, allowances, grants and metering remain authoritative.

Runtime curated overlay storage, an Imports tab and `catalog spec import|show|delete` are planned for Phase 2 and are absent from Phase 1. Existing embedded hosted specs remain supported.

## Local verification

`scripts/tools/test-tools-manager-e2e.sh` builds the CLI and runs the ignored
fake-vendor integration test against its own ephemeral loopback MongoDB replica
set. Requires local `mongod`, `mongosh`, Python 3 and Rust. It uses fake credentials
and fake Lago; no vendor or production service is contacted. Evidence and
limitations are recorded in [the implementation verification report](plans/2026-10-13-tools-manager-verification.md).

Pass `tools` explicitly for the Tool flow or `catalog` for the Catalog CLI and
fake OAuth flow. Both modes own a separate temporary replica set, require an
elected writable primary before dispatch, and retain bounded Mongo diagnostics
on failure. Local software-flow proof leaves every real-service confirmation
gate open.

## Commit-time authority and recovery helpers

Service, anonymous-rule, endpoint, publication, provider-link and ownership
mutations bind the observed kind/ownership and live authority in their MongoDB
transaction. Concurrent conversion or ownership drift returns Conflict; role
revocation denies the write. Provider links and provider requirements are not
supported on Tools, including legacy Tools, and are rejected in the transaction.
A Tool with a legacy provider requirement is ineligible for manager writes.
The API reports a secret-free tools_manager_eligible boolean; frontend and CLI
manager planning consume that server decision and fail closed if it is absent.

Identity resync uses the same parent fence, requiring a live Admin whenever the
caller acted as Admin (always for Tools, whose identity fields are Admin-only),
while journaling only the instances it rewrites and auditing the recovery
transaction. A stale authority/identity
snapshot returns Conflict and requires reload. OIDC credentials, redirects and
secret regeneration are incompatible with Tools. The OIDC mutation helpers
fence their non-Tool parent and actor before changing the client/service, so a
concurrent conversion cannot inherit creator authority. Normal identity
propagation after a committed edit and durable skill-receipt replay complete
already-authorized work; they do not authorize a new catalog mutation. Catalog
Editor/Curation accounts retain their existing non-Tool skill-management path.

The internal authority_fence counter serializes a mutation with revocation on
its parent, actor user, relevant permission-granting groups, and mutable roles.
Immutable system-role documents and unrelated groups are untouched. Semantic
no-op endpoint discovery/edit performs no authority-counter write. The source
change-stream inventory found machine_access_service watching its revocation
outbox and assistant_live filtering assistant/channel/machine collections;
neither consumes users, roles or identity groups or reacts to authority_fence.
