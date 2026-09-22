# API Discovery and Catalog

Admin service creation, provider linking, and legacy vendor retirement are documented in [SERVICE_CONFIGURATION.md](SERVICE_CONFIGURATION.md).

NyxID now documents both its own API surface and the downstream APIs it proxies. This guide shows where those documents live, how downstream specs are discovered, and how to test everything through NyxID instead of talking to services directly.

---

## NyxID's Own Docs

All docs endpoints require normal NyxID authentication: a session cookie, a bearer token, or another supported authenticated caller.

| Endpoint | Purpose |
|----------|---------|
| `GET /api/v1/docs` | Scalar UI for NyxID's OpenAPI 3.1 document |
| `GET /api/v1/docs/openapi.json` | Raw OpenAPI 3.1 JSON for NyxID |
| `GET /api/v1/docs/asyncapi.json` | Raw AsyncAPI 3.0 JSON for NyxID's streaming protocols |
| `GET /api/v1/docs/catalog` | Unified catalog page for downstream service docs |

The AsyncAPI document covers NyxID's current streaming transports:
- Node agent WebSocket control plane
- SSH-over-WebSocket tunnel
- MCP streamable HTTP
- Direct proxy SSE passthrough
- LLM gateway SSE streaming

---

## Downstream Spec Discovery

When you create or update a downstream service, NyxID tries to discover documentation automatically from the service's `base_url`.

### OpenAPI probe order

- `/openapi.json`
- `/swagger.json`
- `/docs/openapi.json`
- `/.well-known/openapi`

### AsyncAPI probe order

- `/asyncapi.json`
- `/.well-known/asyncapi`

If the downstream service already exposes specs somewhere else, set them explicitly on the service:

```bash
curl -X PUT http://localhost:3001/api/v1/services/<service_id> \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "openapi_spec_url": "https://api.example.com/openapi.json",
    "asyncapi_spec_url": "https://api.example.com/asyncapi.json"
  }'
```

Notes:
- `openapi_spec_url` accepts the legacy alias `api_spec_url`
- sending an empty string clears a stored spec URL
- both URLs are validated with the same SSRF checks used for `base_url`

---

## Catalog and Proxied Specs

After discovery succeeds, NyxID exposes downstream docs through authenticated proxy-aware endpoints:

| Endpoint | Purpose |
|----------|---------|
| `GET /api/v1/proxy/services` | Service discovery JSON, including docs and streaming metadata |
| `GET /api/v1/proxy/services/{service_id}/docs` | Scalar UI for a downstream service |
| `GET /api/v1/proxy/services/{service_id}/openapi.json` | Proxied OpenAPI document |
| `GET /api/v1/proxy/services/{service_id}/asyncapi.json` | Proxied AsyncAPI document |

`GET /api/v1/proxy/services` now includes:
- `docs_url`
- `openapi_url`
- `asyncapi_url`
- `streaming_supported`
- `has_node_binding`

This makes the discovery endpoint useful as both a routing index and a developer-facing service catalog.

---

## Proxy-Aware Rewriting

When NyxID serves a downstream OpenAPI document, it rewrites `servers[].url` to the authenticated NyxID proxy route:

```text
{NYXID_BASE_URL}/api/v1/proxy/{service_id}/
```

That means:
- Scalar "Try it" calls stay inside NyxID
- auth, audit logging, approval checks, node routing, and delegation still apply
- consumers do not need direct network access to the downstream service

NyxID also annotates proxied specs with `x-nyxid-*` metadata such as the service ID and slug.

---

## Streaming Detection

NyxID marks a service as `streaming_supported` when either of these is true:
- the discovered or configured OpenAPI document exposes a response with `text/event-stream`
- an AsyncAPI document is available for the service

For direct proxy requests, NyxID now passes SSE through without buffering when:
- the client sends `Accept: text/event-stream`, or
- the upstream responds with `Content-Type: text/event-stream`

This behavior is reflected in:
- `GET /api/v1/proxy/services`
- `GET /api/v1/docs/catalog`
- `GET /api/v1/docs/asyncapi.json`

---

## Catalog Endpoint Discovery

`GET /api/v1/catalog`, `/catalog/{slug}`, and `/catalog/{slug}/endpoints` under `/api/v1` accept general and scoped agent API keys via `X-API-Key` or Bearer without proxy scope. Scheduled-invocation keys, service accounts, and relay tokens are rejected; delegated access retains the exact `account:read` GET exception. Catalog metadata uses template values and live platform grants, without instance connection flags or overrides. Private instance-backed access and mounted-spec fallback obey the effective key allowlist and Member/Admin org scopes. Discovery performs no auto-provisioning or pending-OAuth reconciliation. MCP `nyx__discover_services` gives unrestricted API keys the same discovery result as the owner's session; credential-free auto-connected templates remain suppressed for all callers. For restricted keys, instance-based suppression uses only visible instances. MCP discovery applies the same visibility rule as `GET /api/v1/catalog` for every caller: public and legacy rows, rows the actor created, and platform-key-enabled rows subject to live availability.

The catalog API exposes parsed OpenAPI endpoint metadata for any service that has an `openapi_spec_url`:

| Endpoint | Purpose |
|----------|---------|
| `GET /api/v1/catalog` | List catalog entries (add `?include_all=true` for system services) |
| `GET /api/v1/catalog/{slug}` | Full catalog entry with rich metadata |
| `GET /api/v1/catalog/{slug}/endpoints` | Parsed API endpoints from the service's OpenAPI spec |

The `/endpoints` response includes structured endpoint data:

```json
{
  "slug": "llm-openai",
  "openapi_spec_url": "https://api.openai.com/v1/openapi.json",
  "endpoints": [
    {
      "name": "create_chat_completion",
      "description": "Creates a model response for the given chat conversation.",
      "method": "POST",
      "path": "/chat/completions",
      "parameters": null,
      "request_body_schema": { ... },
      "request_content_type": "application/json",
      "request_body_required": true,
      "response": {
        "content_types": ["application/json"],
        "binary_artifact": false
      }
    }
  ]
}
```

The spec is fetched through a hardened path with DNS pinning, 5MB response size limit, redirect policy, and 60-second caching.

### Rich catalog metadata

Catalog entries can include metadata to help AI agents understand what a service is and how it works:

- `homepage_url`, `repository_url`, `issues_url` -- links to docs, source code, and issue tracker
- `openapi_spec_url`, `asyncapi_spec_url` -- spec URLs for API discovery
- `capabilities` -- structured flags: `supports_proxy_read`, `supports_proxy_write`, `supports_proxy_binary_upload`, `supports_direct_downstream_auth`, `supports_authoring_via_nyx`, `supports_websocket`, `supports_streaming`
- `auth_notes` -- freeform notes on downstream auth expectations
- `known_limitations` -- important caveats for agents and CLI users
- `required_permissions` -- downstream permissions required for key actions

CLI access:

```bash
nyxid catalog list --all                # include system services
nyxid catalog show <slug>               # full metadata display
nyxid catalog endpoints <slug>          # parsed OpenAPI endpoints
```

---

## Hosted Catalog Overlay Specs

Most official upstream APIs either publish no OpenAPI document, publish one in YAML (unsupported), or publish one far above the 5MB fetch limit. For those, NyxID ships small hand-curated OpenAPI 3.1 overlays in-tree under `backend/specs/catalog/` and serves them publicly at:

```text
GET /api/v1/catalog-specs/{spec_key}/openapi.json
```

- The registry (`backend/src/services/catalog_spec_registry.rs`) maps catalog slugs to spec keys; several slugs can share one overlay (`api-github` / `api-github-pat`, the Lark / Feishu pairs).
- The hosted route accepts either the registered overlay key (`firecrawl`) or its catalog service slug (`api-firecrawl`). It only resolves the static in-tree registry; it never treats a user-service slug as a lookup into `/keys`.
- Each operation carries an `x-aevatar-tool` annotation (`name`, `readOnly`, `destructive`, `requiresApproval`) so agent runtimes can admit operations without guessing.
- Overlay schemas are restricted to the schema-keyword subset aevatar's workflow admission accepts (`type, enum, properties, required, items, additionalProperties, title, description, default, example, examples, deprecated`; no `$ref`, no union `type` arrays) -- anything outside it makes the operation inadmissible for workflow binding. Enforced by a registry unit test; upstream APIs still enforce real limits (lengths, ranges) at runtime.
- Seeded services get their `openapi_spec_url` pointed at the hosted overlay automatically (new rows at seed time; existing rows via a null-guarded backfill that never overwrites an admin-set URL).
- At startup, `catalog_spec_sync` parses every overlay and additively upserts `ServiceEndpoint` rows for the seeded services (matched by endpoint name; admin-added endpoints under other names are never touched or soft-deleted). This is what makes `/api/v1/mcp/config` publish concrete `service_id` + `endpoint_id` operations for catalog-backed user services -- required by Aevatar v4 workflow admission (issue #1290).

A valid instance-mounted `openapi_spec_url` (set via `nyxid service update --openapi-spec-url`) **overrides** the catalog template's registered endpoint rows for that instance -- the per-instance spec is a deliberate user decision. A broken or empty instance spec falls back to the template rows when they exist (so a bad override never takes a working catalog service offline); with no rows either, it degrades to the generic proxy tool like a custom endpoint.

Admin-created catalog services get the same treatment: any active HTTP service with an `openapi_spec_url` and **zero** endpoint rows has discovery run automatically -- once at startup (background sweep) and whenever an admin creates or updates the service with a spec URL. The fetch uses the hardened SSRF-checked path, so spec URLs on private/internal hosts still require the manual `POST /services/{id}/discover-endpoints` route. Services with any existing endpoint rows are never touched automatically; re-run manual discovery to refresh them.

A weekly `catalog-spec-drift` workflow (`scripts/check-catalog-spec-drift.py`) verifies every overlay operation still exists in the official upstream spec for providers that publish one (OpenAI, X, Discord, ElevenLabs, Telnyx, Twilio). A red run means a provider moved or removed an operation; update the overlay by hand -- overlays are never auto-updated.

ElevenLabs and Twilio both publish JSON specifications that currently fit under the 5MB fetch limit (roughly 2.0MB and 1.9MB respectively), but NyxID deliberately uses hosted overlays for them. The upstream documents expose much broader, frequently changing surfaces than the default MCP catalog needs. The overlays keep discovery focused on ElevenLabs speech/voice/ConvAI operations and Twilio Calls/Messages/Recordings operations, while preserving Aevatar risk annotations. ElevenLabs realtime WebSocket paths are transport capabilities rather than OpenAPI operations; clients use the normal proxy WebSocket route with the vendor frame protocol.

To extend coverage: add a JSON overlay under `backend/specs/catalog/`, register it in `HOSTED_SPEC_SOURCES` and `SLUG_TO_SPEC_KEY`, and the seed, backfill, sync, and serving paths all pick it up.

The Telnyx overlay covers core AI inference, assistants, conversations, speech,
messaging, and calling. Its full upstream spec exceeds the fetch limit. See
[Telnyx integration](TELNYX_INTEGRATION.md) for connection setup and platform
credential provisioning.

---

## Recommended Operator Flow

1. Register the downstream service with its `base_url`.
2. Check `GET /api/v1/proxy/services` to confirm docs discovery and streaming flags.
3. If discovery missed the real spec location, update `openapi_spec_url` and `asyncapi_spec_url`.
4. Enrich the service with metadata: `homepage_url`, `repository_url`, `capabilities`, `auth_notes`, `known_limitations`, `required_permissions` so AI agents can discover the service fully.
5. Share `GET /api/v1/proxy/services/{service_id}/docs` with internal consumers so they test through NyxID instead of bypassing it.


## Catalog recommendation curation

Catalog responses add optional `recommended_skill_refs`, `skills_revision`, and a separate versioned `skills_manifest_digest`. MCP keeps the existing name-based `catalog_digest` construction; exact-ref changes are discoverable through the new manifest digest. An instance's `recommended_skills` override suppresses inherited refs, even for an empty override.

A dedicated protected Curation service account uses `/api/v1/catalog-curation/services` for grant-scoped discovery, `/services/{id}/openapi.json` for a bounded operation-contract read, and `/services/{id}/skills`, `/skills/history`, and `/skills/restore` for conditional recommendation management. The contract route returns the source document with upstream servers intact for authoring, without granting execution access or rewriting proxy URLs. Generated skills must use the consumer's authorized NyxID service connection. It supports hosted overlays and anonymously readable custom documents; authenticated custom documents require a separate credentialed capability and are not fetched through the curation bearer. It cannot use unrestricted catalog, `/keys`, or service-management routes. Human service editing shares the same revision/history transaction and must send the observed skill revision; omitted legacy revision means zero. See [Service accounts: catalog skill curation](SERVICE_ACCOUNTS.md#catalog-skill-curation) for grant administration, request examples, no-op/replay semantics, rollout ordering, and the Ornn package-content boundary.
## Inference and platform-key discovery (0.20)

Catalog list, `?include_all=true`, single-entry lookup and MCP
`nyx__discover_services` expose an optional `inference` block. No block means no
advertised model-call protocol. `wire_protocol` is `anthropic_messages`,
`openai_responses` or `openai_completions`; `model_list=true` advertises
`GET /api/v1/proxy/s/{slug}/models` with a `data` model array. Anthropic keeps its
own pagination fields. Optional `realtime=true` advertises WebSocket
`/api/v1/proxy/s/{slug}/realtime`. xAI and OpenAI preserve the bearer-injected
`wss://api.x.ai/v1/realtime` and `wss://api.openai.com/v1/realtime` transports.

`binding` is computed for the caller. `platform` means an authorized server-held
key is available and omits `status_slug`; it does not change the binding of an
existing personal connection. `user` means the client needs a personal connection:

1. For provider-linked services, `status_slug` is `ProviderConfig.slug`. Find that
   `provider_slug` in `GET /api/v1/llm/status`; `ready` is usable, `expired` needs
   reauthorization, `not_connected` needs connection.
2. Otherwise `status_slug` is the catalog slug. In `GET /api/v1/keys`, find rows
   whose `catalog_service_slug` matches, check `is_active=true`, then inspect
   credential health (`status` / `connection_status`). Do not match the user
   connection's potentially customized `slug`, and do not treat a healthy
   credential on a disabled service as a usable connection.
3. Execute through the selected connection's returned proxy URL/slug. A user may
   have several connections and may choose BYOK even when catalog binding is
   platform. Restricted availability is revalidated at execution time.

Every catalog entry also returns `platform_key: { available, pricing }` and
`byok_pricing`. A price view contains `metric`, exact decimal `credits_per_unit`
and `sync_status`; null means no configured lane price. In lane mode the missing
lane is free. Services without lanes retain legacy `billing` behavior; resale can
also apply. No credentials, secret lengths or allowed-owner lists are exposed.

Startup fills only null/absent inference values for OpenAI, Anthropic, DeepSeek,
Mistral, OpenRouter, xAI, `chrono-llm` and `chrono-llm-public`. Chrono uses chat
completions and models as recorded in [its upstream contract](chat/direct-chronollm-spec.md).
Chrono public preserves platform binding; Chrono BYOK uses `status_slug=chrono-llm`.
Codex, Google AI and Cohere do not advertise a generic protocol block.

Connect through `POST /keys` or `nyx__connect_service` with
`use_platform_key: true`, or omit it for existing BYOK behavior. Hosted connect-link
creation/completion accepts the same choice. Assistant clients can request the
additive boolean schema with `GET /api/v1/assistant/actions?revision=nyxid-assistant-actions.v9`;
the default and revisions v4-v8 keep their deployed pinned schemas. The v9
`catalogService.use_platform_key` field is optional and the human can review the
choice in the connection dialog.

Admin-cleared inference stays absent after restart: `inference_admin_modified` is a
stored, defaulted tombstone and is not a client inference capability. Admin catalog
responses additionally expose `legacy_public_master`; editors render such absent
platform configurations as enabled/public (implicit). Gateway-URL providers never
advertise an available platform key.

## Aurinko email operations

The `api-aurinko` catalog entry uses the `aurinko` hosted overlay and seeds fifteen concrete operations from documented Aurinko account/email/draft/sync contracts. The base is `https://api.aurinko.io`; paths include `/v1`. Authentication is the owner's account Bearer token. Writes carry approval/risk annotations and do not claim upstream idempotency. Aurinko publishes a machine-readable OpenAPI specification and is included in the existing drift map. See [Aurinko integration](./AURINKO_INTEGRATION.md) for connection, permissions, and channel setup.

### Duplicate operation identities

Dynamic (instance-mounted) specs derive each MCP endpoint identity from the
producer's `operationId`. Producers do publish repeated `operationId`s
(api.jina.ai did in September 2026). Operations that share an `operationId`
fall back to their method/path identity, repeated tool names get a numeric
suffix (`name_2`, `name_3`), and an operation whose identity still collides is
dropped. Separately, the operation catalog omits any single service whose
service or endpoint identities are missing or repeated and counts it in
`invalid_contract_services`, instead of failing `tools/list`, `nyx__call_tool`
and `/api/v1/mcp/config` for the whole user.

### Tool search semantics

`nyx__search_tools` splits the query on non-alphanumeric characters and matches
each word as a case-insensitive substring of the qualified tool name
(`<slug>__<operation>`), the service name and the description. Tools containing
every word rank first, then partial matches in catalog order, capped at 25. Word
order is irrelevant, so "skill search" and "search skills" both find
`ornn-api__searchskills`, and concatenated operation names such as
`getentitystate` match "entity state". An empty query lists the first 25 tools.
