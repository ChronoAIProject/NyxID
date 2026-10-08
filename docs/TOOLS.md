# Tools

Tools are catalog services provided through NyxID-held credentials or without authentication. `/tools` groups them by supplier; `/admin/tools` manages providers, operation publication and runtime OpenAPI imports. AI Services (`/keys`) continues to show a person's own connections, including BYOK connections to tool services.

## Model and execution boundary

`DownstreamService.offering_kind` is `ai_service` by default and `tool` for curated offerings. Tools use `service_category = internal` and `requires_user_credential = false`. Authenticated tools use an encrypted master credential with an enabled platform-key configuration. No-auth tools use `auth_method = none`. A master credential and `provider_config_id` remain mutually exclusive. A BYOK binding may coexist on the same catalog row.

`topics` contains up to 20 unique entries from `GET /api/v1/tools/topics`. `supplier` identifies the API operator (128 characters maximum). `import_source` records `kind` (`monid`, `vendor_spec`, `manual`), `reference` (512 characters), optional `version` (128 characters) and BSON `imported_at`. These fields are defaulted for legacy records.

Operations optionally classify `data_scope` (`public`, `account`, `owned_resource`) and `cost_class` (`free`, `metered`, `resource_backed`). `execution` defaults to `http_operation`, with `job_start` and `job_poll` available. These classifications do not change billing or create a job execution engine.

## Publication

New operations on tool services start `draft` and inactive, whether created manually, discovered from a URL, or imported from an overlay. Legacy operations default to `published`; other service kinds retain their existing activation behavior.

Publication states are `draft`, `validated`, `published` and `paused`. Only `published` is active. Every publication request advances the positive `operation_generation` and appends `catalog_endpoint_publication_changed` through the audit service. HTTP and WebSocket proxy execution matches the method and path template against catalog operations and requires a published, active match. Unmatched, draft, validated and paused operations return HTTP 404 / error code 12600 (`ToolOperationNotPublished`). MCP omits unpublished operations and returns an `isError` result carrying 12600 when one is called.

MCP names remain `{service_slug}__{endpoint.name}`. MCP discovery still includes tool services; human catalog browsing excludes them unless `offering_kind=tool` or `include_all=true` is requested.

## Runtime overlays

Each service may have one live `catalog_spec_overlays` row, enforced by a unique `service_id` index. Uploads accept an OpenAPI 3.x JSON object with `paths`, at most 1 MiB. The server computes SHA-256, advances the revision, retains the immediately previous document and upserts operations by name. Rows absent from a replacement document and manually added rows remain intact. Existing publication choices are preserved.

A stored overlay overrides an embedded overlay for that service. Uploading to a service without a spec URL installs its hosted URL, `/api/v1/catalog-specs/service/{service_id}/openapi.json`. Deleting an overlay preserves endpoints; embedded specifications remain available where registered. Imports return operation sync, added and changed counts.

## Authority

Platform admins can manage every field. Human roles grant `nyxid:catalog:services:read` and `nyxid:catalog:services:write`; protected catalog-editor service accounts use token scopes `catalog:services:read` and `catalog:services:write`. Token issuance, live role checks and route confinement follow the catalog skill-editor pattern. A read token cannot publish or import.

Editors can create tool rows and edit `name`, `description`, `visibility`, spec/documentation URLs, `auth_notes`, `known_limitations`, `required_permissions`, `capabilities`, `topics`, `supplier`, `import_source` and `offering_kind`. They can manage endpoints, discover operations, import/delete overlays and publish. A submitted admin-only field is rejected even when its value is null. Transport, credentials, platform grants, billing, provider links, identity/delegation, operation policy, anonymous routes, developer apps, injected headers and deletion require a platform admin. Converting a tool to `ai_service` also requires an admin: otherwise an editor who created the row could cycle through the legacy creator-authority path and change protected transport fields.

An editor's metadata-only create uses an internal no-auth transport placeholder (`https://example.invalid`), because editors cannot submit `base_url`, `auth_method` or `service_category`. An admin must configure the actual transport in Service settings before it can reach a provider. CLI editors omit `--endpoint-url`; admin creates supply it.

## API

| Method | Route | Behavior |
|---|---|---|
| GET | `/api/v1/tools` | Ready published offerings under the caller's live platform ACL; admins can see missing-credential offerings |
| GET | `/api/v1/tools/{slug}` | One visible offering |
| GET | `/api/v1/tools/topics` | Controlled topic vocabulary |
| GET | `/api/v1/tools/editor-authority` | Current human catalog read/write authority for navigation |
| POST | `/api/v1/services/{id}/endpoints/{endpoint_id}/publication` | Set one operation's state |
| POST | `/api/v1/services/{id}/publication` | Set 1–200 explicitly named operations |
| PUT | `/api/v1/services/{id}/spec-overlay` | Upload `{document, source?}` and sync |
| GET | `/api/v1/services/{id}/spec-overlay` | Current document, previous document and provenance |
| DELETE | `/api/v1/services/{id}/spec-overlay` | Remove overlay, retain operations |
| GET | `/api/v1/catalog-specs/service/{id}/openapi.json` | Public hosted spec, stored overlay first |

Tool listing batches membership, provider and operation reads. Missing credentials and offerings without published operations are hidden from ordinary callers. `/keys` omits platform bindings of tool services and retains BYOK bindings. Key details and agent grant selection remain available. Metadata-only Tools reads permit delegated `account:read` access; service accounts use the catalog editing routes.

## Add a tool

Configure the CLI's base URL and access token as usual. With an admin token:

```sh
nyxid service add --catalog-admin --slug tools-crossref \
  --label 'Crossref search' --endpoint-url https://api.crossref.org \
  --auth-method none --service-category internal --offering-kind tool \
  --topic research-papers --supplier Crossref
nyxid catalog spec import tools-crossref --file crossref.openapi.json \
  --source-kind manual --source-ref 'Crossref works API'
nyxid catalog endpoint list tools-crossref
nyxid catalog publish tools-crossref --operation search_works
nyxid tools show tools-crossref
```

Alternatively, supply `--openapi-spec-url https://…/openapi.json` on add, then run `nyxid catalog discover tools-crossref`. Review imported parameters and operation classifications before publication. For authenticated offerings, configure the master credential, enable the desired public or restricted platform audience, and configure pricing in the existing service editor. Never pass credentials in overlay documents.

## CLI reference

Catalog targets accept catalog UUIDs or slugs; endpoint targets accept IDs or names.

- `service add|update --catalog-admin`: `--service-category`, `--offering-kind`, repeatable `--topic`, `--clear-topics` (update), `--supplier`, `--openapi-spec-url`.
- `catalog endpoint list <service> [--all] [--published-only]`: all states by default, publication column.
- `catalog endpoint add <service> --name … --method … --path …`: optional description, parameters/body-schema JSON files, data scope, cost class and execution.
- `catalog endpoint update <service> <id|name>`: sparse changes using the same flags.
- `catalog endpoint disable <service> <id|name>`: pause.
- `catalog discover <service>`; `catalog publish <service> --operation … [--state published|paused|draft|validated]`.
- `catalog spec import <service> --file … [--source-kind … --source-ref … --source-version …]`; `catalog spec show|delete <service>`.
- `catalog topics`; `tools list [--topic …] [--all]`; `tools show <slug>`.

Tables use the CLI output helper; `--output json` prints the raw API response. `--all` cannot override server ACL or readiness checks for ordinary users.

## Seeds and billing

`tools-x` shares the existing Twitter spec without renaming operations. Initial insertion publishes `search_recent_tweets`, `get_user_by_username` and `get_user_tweets` as public, metered HTTP operations; the other six remain draft. TinyFish Search and Fetch use hand-curated embedded overlays from the audited examples, with read-only annotations and public/free initial operations. All three rows start without credentials and are hidden from ordinary users until configured. Later startups preserve publication decisions.

Tools reuse `ServiceBilling.platform_key_pricing` and optional `byok_pricing`. Missing/free platform lanes display Free; priced lanes display their exact decimal credits per metric. Cost-class metadata does not override prices. Existing platform ACL, benefit funding, metering and billing-owner rules continue to apply.
