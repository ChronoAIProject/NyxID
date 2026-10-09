# Tools: implementation plan

Status: approved for implementation on 2026-10-08. Branch: `enrich-ai-search-tools`. Companion design notes live in `docs/plans/2026-10-07-public-data-tools-approach.md` and `docs/plans/monid-api-audit/` (`TOOLS_REGISTRY_PLAN.md`, `PORT_COVERAGE.md`, `NAMING_REUSE_CLI_ROLES.md`). This file is the executable contract; where it disagrees with the notes, this file wins.

**Phase 1 scope (revised 2026-10-08):** the UI, the data model, publication gating, and the ability to offer **existing managed APIs** as tools: Firecrawl, X/Twitter, ElevenLabs, Twilio, and Chrono LLM. No catalog seeding of new vendors. Everything must be addable later programmatically through the admin API and CLI, not by code changes. Sections marked **Phase 2** are out of scope for the worker now; they stay in the file as the agreed direction.

## 1. Product decisions (fixed)

- **Name**: user entry **Tools** at `/tools`; admin entry **Tools** at `/admin/tools`. Items are "tools", grouped by provider and category. Keep **AI Services** (`/keys`) for the user's own connections.
- **Record**: every tool is one catalog `DownstreamService` row (Admin > Services). No new "provider" concept; `ProviderConfig` stays for user login flows only.
- **Boundary**: a tool is answered by a NyxID-held credential (`service_category = "internal"`, `requires_user_credential = false`, master credential, `platform_key.enabled = true`) or by no credential (`auth_method = "none"`). The same row may also accept a user's own key (BYOK lane). A user's BYOK binding is shown under AI Services; the auto-provisioned platform binding is hidden there.
- **Identity the AI sees is unchanged**: MCP tool name stays `{service_slug}__{endpoint.name}` (`backend/src/services/mcp_service.rs:2582`). No renaming of existing tools, slugs, or overlay keys.
- **Billing**: no new billing machinery. Tools price through the existing `ServiceBilling.platform_key_pricing` lane (metric `requests` for API tools) and optional `byok_pricing`. A tool is "Free" only when its platform lane is absent or free.
- **Publication gates execution**: a draft or paused operation on a tool row must be uncallable through MCP *and* the HTTP proxy, not merely hidden from cards.

## 2. Backend

All new fields are additive, serde-defaulted, and must not change behavior for existing rows. Follow CLAUDE.md Critical Rules 1-3 (model conventions, layer separation, `AppError`). Never put business logic in handlers.

### 2.1 `DownstreamService` additions (`backend/src/models/downstream_service.rs`)

```rust
#[serde(default)] pub offering_kind: OfferingKind,      // enum: AiService (default) | Tool
#[serde(default)] pub topics: Vec<String>,               // controlled vocabulary, see 2.7
#[serde(default)] pub supplier: Option<String>,          // verified API operator, free text <= 128
#[serde(default)] pub import_source: Option<CatalogImportSource>,
```

`CatalogImportSource { kind: "monid" | "vendor_spec" | "manual", reference: String (<= 512), version: Option<String> (<= 128), imported_at: Option<DateTime<Utc>> (bson_datetime::optional) }`.

Validation in `handlers/services.rs` create/update DTOs: `offering_kind = tool` requires `service_category = "internal"` and (`auth_method = "none"` or a stored master credential present or `platform_key` present). Reject `offering_kind = tool` with `service_category = "connection"` (`AppError::ValidationError`). Topics: 0-20 entries, each lowercase kebab, 2-40 chars, deduplicated, must be in the vocabulary (2.7). Expose all four fields on the admin service response, the catalog detail response, and the user-facing tools responses (2.5).

### 2.2 `ServiceEndpoint` additions (`backend/src/models/service_endpoint.rs`)

```rust
#[serde(default)] pub data_scope: Option<DataScope>,     // Public | Account | OwnedResource
#[serde(default)] pub cost_class: Option<CostClass>,     // Free | Metered | ResourceBacked
#[serde(default)] pub execution: ExecutionKind,          // HttpOperation (default) | JobStart | JobPoll
#[serde(default = "default_publication")] pub publication: PublicationState, // Draft | Validated | Published | Paused; default Published (legacy rows stay callable)
```

Rule for new endpoint rows: when the parent service has `offering_kind = tool`, every endpoint created by auto-discovery, overlay sync, or manual create defaults to `publication = draft` and `is_active = false`. On non-tool services nothing changes (`published`, active). Implement this in one shared helper in `services/` used by `api_docs_service` discovery, `catalog_spec_sync`, the overlay sync (2.4), and the endpoint create handler.

### 2.3 Publication gating

- New admin route `POST /api/v1/services/{service_id}/endpoints/{endpoint_id}/publication` body `{ "state": "draft" | "validated" | "published" | "paused" }`. `published` sets `is_active = true`; any other state sets `is_active = false`. Bump `operation_generation` on every transition (existing generation semantics). Audit event `catalog_endpoint_publication_changed` with service id, endpoint id, from, to, actor.
- Bulk variant `POST /api/v1/services/{service_id}/publication` body `{ "state": ..., "endpoint_names": [..] }` (names required, max 200).
- **Proxy gate**: in `proxy_service::execute_proxy` (and the WS path), when the resolved catalog row has `offering_kind = tool`, resolve the request against the service's endpoint rows (method + path template match, same matcher the operation policy / MCP dispatch uses). If no row matches, or the matched row is not `published`/active, return a new `AppError::ToolOperationNotPublished` (HTTP 404, numeric code **12700**, add it to the CLAUDE.md reserved-code list). Non-tool rows are untouched.
- **MCP gate**: `/api/v1/mcp/config`, `tools/list`, `nyx__search_tools`, `nyx__discover_services` already use `is_active` on endpoint rows; add a test proving a draft endpoint on a tool row is absent from all four and that `nyx__call_tool` on it returns an `isError` result carrying code 12700.

### 2.4 Overlay store (runtime curated specs) — **Phase 2, do not implement now**

- New collection `catalog_spec_overlays` (`models/catalog_spec_overlay.rs`, `COLLECTION_NAME`): `_id` UUID v4, `service_id`, `document: serde_json::Value` (OpenAPI 3.x JSON, <= 1 MiB, validated as an object with `openapi` and `paths`), `sha256`, `revision: i64`, `source: Option<CatalogImportSource>`, `created_by`, `created_at`, `updated_at`. One live overlay per service (unique index on `service_id`); keep the previous document in `previous_document` on replace (one level of history is enough).
- Routes: `PUT /api/v1/services/{service_id}/spec-overlay` (upload + sync), `GET .../spec-overlay` (metadata + document), `DELETE .../spec-overlay` (removes the overlay; endpoint rows are **not** deleted, mirroring the never-soft-delete rule).
- Sync: reuse the exact upsert-by-name logic of `catalog_spec_sync` (factor it into a shared function taking a parsed document instead of an embedded one). Admin-added rows are never deleted; rows absent from the new revision are left untouched.
- Serving: `GET /api/v1/catalog-specs/service/{service_id}/openapi.json` (public, like the embedded overlays) serves the stored document. On upload, if the service has no `openapi_spec_url`, set it to that hosted URL. Embedded overlays in `backend/specs/catalog/` keep precedence for the seeded slugs they already cover; a stored overlay on such a service wins over the embedded one for that service only.

### 2.5 User-facing Tools API

- `GET /api/v1/tools` (JWT/session or general API key; delegated `account:read` parity allowed, it is metadata-only): published tool offerings visible to the caller under the existing platform-key ACL (`available_with_grants`, no extra DB calls per row). Response per tool service: `id, slug, name, description, supplier, topics, homepage_url, provider_label (ProviderConfig name when linked for the BYOK twin, else supplier/name), access: { platform: bool, byok: bool }, pricing: { platform: {metric, credits_per_unit} | "free", byok: ... | null }, limits: { rate_limit_per_second, burst } from PLATFORM_SERVICE_RATE_LIMIT_*, operations: [{ name, description, method, path, data_scope, cost_class, execution, risk }] (published only), credential_configured: bool`. Hide rows where `credential_configured = false` and `auth_method != "none"` from non-admin callers (not ready). Group key is the service; the client groups by `supplier`.
- `GET /api/v1/tools/{slug}` same shape for one tool.
- `GET /api/v1/keys`: rows whose catalog service has `offering_kind = tool` **and** `credential_binding = platform` (auto-provisioned) are omitted from the listing; BYOK rows on tool services remain. Add `offering_kind` to the key response so the frontend never has to guess.
- `GET /api/v1/catalog` (human browse for "add a service"): exclude `offering_kind = tool` rows unless `?offering_kind=tool` or `include_all=true`. MCP discovery (`nyx__discover_services`, `/mcp/config`, `nyx__search_tools`) is **unchanged** and still includes tools.

### 2.6 Roles — **Phase 2, do not implement now**. Phase 1 keeps every admin route on `require_admin`.

Mirror `services/catalog_editor_service.rs` exactly.

- Role scopes `nyxid:catalog:services:read` and `nyxid:catalog:services:write` (assignable through existing role management); service-account token scopes `catalog:services:read` and `catalog:services:write` (add to `scope_catalog.rs`, same validation style as `catalog:skills:*`).
- **Editor authority** (platform admin OR role/token write scope): create a service with `offering_kind = tool`; update these fields only: `name, description, visibility, openapi_spec_url, asyncapi_spec_url, homepage_url, repository_url, issues_url, examples_url, auth_notes, known_limitations, required_permissions, capabilities, topics, supplier, import_source, offering_kind`; endpoint CRUD; discover; overlay PUT/GET/DELETE; publication routes. A write by an editor that carries any other field returns `AppError::Forbidden` naming the field.
- **Admin-only** (unchanged): `credential`, `platform_key`, `billing`, `base_url`, `destination_targets`, `auth_method`, `auth_key_name`, `service_category`, `provider_config_id`, identity propagation and delegation fields, `token_exchange_config`, `anonymous_endpoints`, `proxy_operation_policy`, `developer_app_ids`, `default_request_headers`, `ws_frame_injections`, delete.
- Read scope lets the holder call the admin GET service/endpoint/overlay routes for tool rows.

### 2.7 Topic vocabulary

Single source of truth `services/tool_topics.rs` (`pub const TOOL_TOPICS: &[(&str, &str)]` slug + label), served at `GET /api/v1/tools/topics` for the frontend and CLI. Initial list (Monid categories plus NyxID groupings): `web-search, page-fetch, news, company-data, people-data, contact-enrichment, seo, social, jobs, real-estate, automotive, finance, crypto, weather, air-quality, reviews, software-directory, research-papers, government-data, economic-data, llm-benchmarks, generation-image, generation-video, generation-audio, email, phone, browser, machine, storage`.

### 2.8 Enabling existing managed APIs as tools (no seeds)

Nothing is seeded. Tools are created by an admin at runtime through two primitives, both of which must work from the API, the CLI, and the admin UI:

**A. Flip in place.** `PUT /services/{id}` with `offering_kind = tool` on a row that already satisfies the tool boundary (internal, provider-less, master credential or `auth_method = none`, platform key). Chrono LLM (`chrono-llm`, `chrono-llm-public`) is this case. Existing endpoints on a flipped row keep `publication = published` (they were already live); the admin may pause any of them.

**B. Tool twin of an existing catalog row.** `POST /services` with a new field `twin_of_service_id` (UUID or slug). The server copies from the source row: `base_url`, `destination_targets`, `auth_method`, `auth_key_name`, `openapi_spec_url`, `asyncapi_spec_url`, `default_request_headers`, `custom_user_agent`, `capabilities`, `homepage_url`, `repository_url`, `issues_url`, `examples_url`, `auth_notes`, `known_limitations`, `required_permissions`, `description`, and **all endpoint rows** (new UUIDs, `publication = draft`, `is_active = false`, generation 1). It sets `offering_kind = tool`, `service_category = internal`, `requires_user_credential = false`, `provider_config_id = None`, `platform_key = { enabled: true, audience: public }` unless the body overrides, and stores no credential. Request fields override copied values. The twin's `slug` is required (convention `tools-<vendor>`); the twin records `import_source = { kind: "catalog_twin", reference: <source slug>, version: <source updated_at RFC3339> }`. This is how Firecrawl (`api-firecrawl`, bearer), X (`api-twitter`, bearer, provider-linked so a twin is required), ElevenLabs (`api-elevenlabs`, header `xi-api-key`), and Twilio (`api-twilio`, header `Authorization` with the Basic value) become tools. Because the twin shares the source's `openapi_spec_url` (the hosted overlay), `catalog_spec_sync` must treat the twin like any spec-backed service: name-based upsert, never deleting admin rows, and respecting the draft default of 2.2.

After A or B the admin stores the credential and prices through the existing service editor, publishes operations (2.3), and the tool appears on `/tools`. Add `kind: "catalog_twin"` to `CatalogImportSource.kind`.

Twins must not break the source: no shared mutable state, no change to the source row, and MCP names for the twin are `tools-<vendor>__<op>` by construction, so a user who has both the source connection and the tool sees two distinct tool sets.

### 2.9 Programmatic POC (one tool, through the API only)

Check in `scripts/tools/add-tool-twin.sh`: a small bash script that uses only the `nyxid` CLI (no direct Mongo, no code seeds) to create one tool from an existing catalog row and publish a chosen operation set. Parameters: `--source <slug>`, `--slug <tools-slug>`, `--supplier`, `--topic` (repeatable), `--publish <op>` (repeatable), optional `--credential-env`. It is idempotent: if the slug exists it updates topics/supplier and the publication set and never re-creates. Ship it with a documented invocation for `tools-x` (twin of `api-twitter`, publish `search_recent_tweets`, `get_user_by_username`, `get_user_tweets`) and run that invocation in the manual proof as step (h). This is the proof that future seeding is a scripted admin action, not a code change; code-level seeding is deferred until the structure is confirmed.

### 2.10 Documentation

- New `docs/TOOLS.md`: model, boundary, publication rule, roles, API, CLI, seeding, how to add a tool (flip in place, tool twin, new row with spec URL), billing lanes, Phase 2 items (overlay store, editor roles) listed as planned.
- `docs/API.md`: Tools section. CLAUDE.md: one short rule under section 8 pointing at `docs/TOOLS.md`, plus the 12700 code line.

## 3. CLI (`cli/`) — Phase 1 subset

- `nyxid service add|update --catalog-admin`: add `--openapi-spec-url` on **add** (update already sends it), `--service-category`, `--offering-kind <ai_service|tool>`, `--topic <slug>` (repeatable; on update replaces the list; `--clear-topics`), `--supplier`, and on add `--twin-of <catalog id|slug>` (2.8 B).
- New `nyxid catalog` admin subcommands (targets by catalog UUID or slug, reuse `catalog_admin::fetch_catalog_service`):
  - `catalog endpoint list <service> [--published-only]` with a `publication` column
  - `catalog endpoint add <service> --name --method --path [--description] [--parameters-file] [--body-schema-file] [--data-scope] [--cost-class] [--execution]`
  - `catalog endpoint update <service> <endpoint-id|name> [same flags]`
  - `catalog endpoint disable <service> <endpoint-id|name>`
  - `catalog discover <service>`
  - `catalog publish <service> --operation <name>... [--state published|paused|draft|validated]` (bulk route)
  - `catalog topics`
- New user command `nyxid tools list [--topic <slug>]` and `nyxid tools show <slug>` (table + `--output json`).
- **Phase 2**: `catalog spec import|show|delete` (needs 2.4).
- Table output through `output.rs`; JSON is the raw response. Unit tests for arg parsing and body construction in the style of existing `catalog_admin` tests.

## 4. Frontend (`frontend/`)

Read `DESIGN.md` first. Use `useAppForm`, Zod schemas in `schemas/`, hooks in `hooks/` (`use-tools.ts`, `use-catalog-admin.ts` additions), no `console.log`.

- **Sidebar**: user entry **Tools** (`/tools`) directly under AI Services; admin entry **Tools** (`/admin/tools`) next to Services. Add the route titles in `dashboard-layout.tsx` and routes in `router.tsx`.
- **`/tools` (user)**: cards grouped by supplier, topic filter chips, search box. Card: name, description, supplier, topics, "Provided by NyxID" badge, price line ("Free" only when platform lane is free/absent, otherwise `<credits> credits / request`), limits, operation count with an expandable list (name, description, method, path, read-only/destructive chips). Actions: "Use with my AIs" opens the existing agent grants flow (link to the agent's Grants with the service preselected); "Use my own key" is shown only when `access.byok` and routes to the existing add-service flow with the slug preselected. Empty state copy explains the difference from AI Services.
- **`/admin/tools` (admin)**: two tabs in Phase 1 (Providers, Tools). *Providers*: one row per supplier with tool counts, credential configured, published/draft counts, next blocker ("Store credential", "Publish operations", "No spec"). *Tools*: table of tool services with inline publication state per operation (toggle published/paused, draft stays until first publish), topic editor, supplier, import source; "Add tool" opens a dialog that creates an internal tool row (name, slug, base URL, auth method, key name, topics, supplier, spec URL or overlay upload) and links to the existing service editor for credential, platform key, and pricing. *Imports*: **Phase 2** (needs 2.4); do not build the tab now. The "Add tool" dialog offers two paths: "From an existing catalog service" (select a source row, enter the `tools-<vendor>` slug, topics, supplier; calls `POST /services` with `twin_of_service_id`) and "New service" (name, slug, base URL, auth method, key name, topics, supplier, spec URL). Both link to the existing service editor for credential, platform key, and pricing, and show the draft operation count with a link to publish.
- **Service editor** (`service-edit.tsx`): add Offering kind, Topics (multi-select from `/tools/topics`), Supplier, Import source (read-only when set). Endpoint editor (`endpoint-list.tsx`): add publication state badge + toggle, data scope, cost class, execution selects.
- **`/keys`**: rely on the server filter; additionally render the `offering_kind` badge "Tool · your key" on BYOK rows of tool services.
- Tests: vitest for the tools page (grouping, free vs priced rendering, byok button visibility), admin tools tabs (publication toggle calls the route, editor-vs-admin gating), and schema specs.

## 5. Order of work and commits (Phase 1)

Commit after each step with conventional commits on the current branch. Never commit to main. Do not bump versions.

1. `feat(catalog): additive tool offering fields and topic vocabulary` (2.1, 2.2, 2.7)
2. `feat(catalog): endpoint publication state with proxy and MCP gating` (2.3)
3. `feat(catalog): tool twins of existing catalog services` (2.8)
4. `feat(tools): user tools listing and keys/catalog filtering` (2.5)
5. `feat(cli): catalog endpoint/publish commands, tools commands, tool flags` (3)
6. `feat(frontend): Tools page, admin Tools workspace, editor fields` (4)
7. `feat(tools): programmatic tool twin script` (2.9)
8. `docs(tools): TOOLS.md, API.md, CLAUDE.md` (2.10)

## 6. Verification (must all pass before reporting done)

Environment facts for this Mac: Node is at `/opt/homebrew/bin` (`export PATH=/opt/homebrew/bin:$PATH`). A standalone `mongod` listens on `127.0.0.1:27017`; tests that need a replica set can use `NYXID_TEST_DATABASE_URL="mongodb://127.0.0.1:27019/?replicaSet=nyxidrs"` after starting one (see `docs/plans/2026-10-08-tools-implementation-plan.md` companion memory: `mongod --replSet nyxidrs --port 27019 --dbpath /tmp/nyxid-rs-data --bind_ip 127.0.0.1 --fork --logpath /tmp/nyxid-rs.log` then `mongosh --port 27019 --eval 'rs.initiate({_id:"nyxidrs",members:[{_id:0,host:"127.0.0.1:27019"}]})'`). Cold cargo builds take 10-20 minutes; run them detached with logs and batch edits before compiling. About 29 pre-existing backend tests fail on MongoDB 7.0 for environmental reasons (replica set / `$getField`); list any such failures explicitly and do not count new failures among them.

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p nyxid` (unfiltered) and `cargo test -p nyxid-cli`
- `cd frontend && npm run lint && npm test && npm run build`
- Manual proof (record the commands and outputs in the final report): with the backend running against local Mongo, (a) `nyxid service add --catalog-admin --twin-of api-firecrawl --slug tools-firecrawl --offering-kind tool --topic web-search --topic page-fetch --supplier Firecrawl`, (b) `nyxid catalog endpoint list tools-firecrawl` shows the copied operations as drafts, (c) a proxy call and `nyx__call_tool` on `tools-firecrawl__search` return 12700, (d) `nyxid catalog publish tools-firecrawl --operation search --operation scrape`, (e) `nyxid tools list` shows it (as admin; as a normal user it is hidden until a credential is stored, prove both), (f) store a dummy credential through the editor route and confirm the normal user now sees it and `GET /keys` does not show its platform binding, (g) flip `chrono-llm-public` in place with `nyxid service update --catalog-admin chrono-llm-public --offering-kind tool` and confirm it lists on `/tools` with its existing operations published and `/keys` still shows a user's BYOK rows unchanged. (h) run `scripts/tools/add-tool-twin.sh` for `tools-x` twice and show the second run is a no-op. Live vendor calls are not required; a 401 from Firecrawl with the dummy key is an acceptable proof that the published path reaches the vendor.

## 7. Definition of done

Every section above is implemented, tested, documented, and verified. The final report lists: files changed per commit, the verification outputs, the manual-proof transcript, and any deviation from this plan with its reason. No TODOs, no commented-out code, no skipped tests, no "follow-up" items. If a part of the plan is impossible as written, implement the closest correct alternative and explain it in the report rather than leaving it out.
