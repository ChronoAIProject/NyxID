# Tools

Tools are catalog services offered through NyxID-held credentials or without authentication. `/tools` groups them by supplier with topics, prices and published operations. `/admin/tools` has Providers and Tools tabs for platform admins. AI Services (`/keys`) retains a person's BYOK bindings to tool rows while hiding their platform bindings.

## Model and execution boundary

`DownstreamService.offering_kind` defaults to `ai_service`; curated offerings use `tool`. Tools require `service_category = internal` and `requires_user_credential = false`, with either `auth_method = none`, a stored master credential, or platform-key configuration. A master credential and `provider_config_id` remain mutually exclusive. Authenticated runtime twins are provider-less and use the existing encrypted master credential and live public/restricted platform ACL.

`topics` allows up to 20 unique slugs from `GET /api/v1/tools/topics`. `supplier` identifies the API operator (128 characters maximum). `import_source` records kind (`monid`, `vendor_spec`, `manual`, `catalog_twin`), reference (512 characters), optional version (128 characters) and optional `imported_at` (RFC3339 in the API, BSON datetime in MongoDB). Twins set `imported_at` to their creation time. On update, omitted `supplier` or `import_source` remains unchanged, while JSON `null` clears it.

Endpoint classifications are optional `data_scope` (`public`, `account`, `owned_resource`), optional `cost_class` (`free`, `metered`, `resource_backed`) and default `execution = http_operation` (`job_start` and `job_poll` are metadata options). They neither override billing nor introduce a job engine. All additions are serde-defaulted for legacy records.

## Publication

New endpoints on tool rows start inactive and `draft`, whether created manually, discovered, or synchronized from a spec. Non-tool creation retains its existing behavior. Legacy endpoint rows default to `published`; flipping an existing service does not reset its live operations.

States are `draft`, `validated`, `published` and `paused`. Only published endpoints are active. Bulk publication and its audit appends commit in one transaction; any failure rolls back the entire batch. Equally specific matching rows must all be published and active. Each publication request advances the positive `operation_generation` and appends `catalog_endpoint_publication_changed` through the audit service. HTTP and WebSocket execution require a published, active method/path-template match. Unmatched or unpublished operations return HTTP 404 / code 12700. Unconfigured public twins also receive this publication error before credential readiness checks. MCP omits unpublished operations and returns `isError` carrying 12700 when one is called.

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

`POST /api/v1/services` accepts `twin_of_service_id` as a catalog UUID or slug and requires the new slug. It copies base URL, destination targets, auth method/key name, OpenAPI/AsyncAPI URLs, default headers, user agent, capabilities, description, documentation links, auth notes, limitations and required permissions. Request fields override copied values. It sets an internal, provider-less tool with no credential and defaults the platform audience to enabled/public unless overridden. Source credentials are never copied; a credential supplied at twin creation is rejected. Configure it afterward through Service settings or `service update --credential-env`.

All source endpoint rows receive new UUIDs, generation 1, draft publication and inactive state. Catalog creation and copied endpoints commit transactionally. The source is unchanged. Provenance records `catalog_twin`, the source slug and its updated-at RFC3339 version. Twins share the source's spec URL; existing additive, name-based spec discovery never deletes admin rows and preserves publication decisions.

Phase 1 targets are Firecrawl (`api-firecrawl`, bearer), X (`api-twitter`, bearer), ElevenLabs (`api-elevenlabs`, `xi-api-key` header), Twilio (`api-twilio`, `Authorization` header containing the Basic value), and Chrono LLM (flip in place). Chrono rows are admin-created rather than seeded; configure an eligible row first on a fresh deployment.

For a new API, use Add tool → New service, or CLI `service add --catalog-admin --slug … --endpoint-url … --auth-method … --service-category internal --offering-kind tool --openapi-spec-url …`. Configure a master credential/platform grant and pricing in Service settings. Run `catalog discover` to populate draft operations from the spec, then review and publish selected operations.

## Programmatic POC

The checked-in script uses the `nyxid` CLI for every server read and write. Python 3 only compares JSON locally. Configure the CLI's URL and admin login, or set its usual environment variables. `NYXID_BIN` optionally selects a local CLI binary.

```sh
scripts/tools/add-tool-twin.sh \
  --source api-twitter --slug tools-x --supplier X --topic social \
  --publish search_recent_tweets \
  --publish get_user_by_username \
  --publish get_user_tweets
```

Run the same command again: it prints `tools-x unchanged (no-op)` and performs no writes. An existing slug must already be a tool twin of the requested source. The script updates changed topics/supplier, publishes selected operations only when necessary, and pauses other published operations. It never re-creates an existing twin or changes the source. Optional `--credential-env NAME` configures an absent credential; it does not rotate an already configured credential. Use an explicit CLI update for rotation. No direct database or HTTP calls occur in the script.

## API and authority

Phase 1 catalog administration and publication require platform admin authority; no new role or token scopes are added. Tools metadata reads accept JWT/session or general API keys and allow delegated `account:read` parity. Static topic vocabulary is public.

| Method | Route | Behavior |
|---|---|---|
| GET | `/api/v1/tools` | Published ready tools under live platform ACL; admins can see missing-credential offerings |
| GET | `/api/v1/tools/{slug}` | One visible offering |
| GET | `/api/v1/tools/topics` | Controlled topic vocabulary |
| POST | `/api/v1/services` | Create a new row, optionally with `twin_of_service_id` |
| PUT | `/api/v1/services/{id}` | Metadata update or eligible flip in place |
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

Tools reuse `ServiceBilling.platform_key_pricing` and optional `byok_pricing`. Free/absent platform lanes display Free; priced lanes display decimal credits per metric. Cost-class metadata does not override prices. Existing platform ACLs, personal billing ownership, allowances, grants and metering remain authoritative.

Runtime curated overlay storage, editor roles/scopes, an Imports tab and `catalog spec import|show|delete` are planned for Phase 2 and are absent from Phase 1. Existing embedded hosted specs remain supported.
