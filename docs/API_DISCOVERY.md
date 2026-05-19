# API Discovery and Catalog

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
      "request_body_required": true
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

## Recommended Operator Flow

1. Register the downstream service with its `base_url`.
2. Check `GET /api/v1/proxy/services` to confirm docs discovery and streaming flags.
3. If discovery missed the real spec location, update `openapi_spec_url` and `asyncapi_spec_url`.
4. Enrich the service with metadata: `homepage_url`, `repository_url`, `capabilities`, `auth_notes`, `known_limitations`, `required_permissions` so AI agents can discover the service fully.
5. Share `GET /api/v1/proxy/services/{service_id}/docs` with internal consumers so they test through NyxID instead of bypassing it.

