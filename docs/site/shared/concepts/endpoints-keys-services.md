---
title: Endpoints, keys & services
description: The three user-owned records that together define a proxy route in NyxID: where to send requests, what credential to inject, and how to inject it.
---

When a user connects an external API through NyxID, three records are created automatically in a single operation. Understanding what each record represents — and why they are separate — makes it easier to manage connections, debug routing failures, and extend the model for advanced use cases.

## The three records

### UserEndpoint

A `UserEndpoint` is a target URL. It answers the question: where should this request be sent?

```
url:   https://api.openai.com
label: "OpenAI"
```

Endpoints come from two sources. When a user picks a service from the catalog, the catalog default URL is copied into a `UserEndpoint`. When the user sets up a custom service or routes through a credential node, the URL may be empty on the NyxID side — the node holds and resolves it locally.

### UserApiKey

A `UserApiKey` is an external credential. It answers the question: what secret should be injected?

```
credential_type: api_key | oauth2 | bearer | node_managed | ssh_certificate
status:          active | expired | revoked | pending_auth
```

The credential itself is encrypted with AES-256-GCM before storage. `node_managed` credentials live exclusively on the node agent and never transit NyxID at all. OAuth2 credentials carry access and refresh tokens and are refreshed automatically before expiry.

### UserService

A `UserService` is the routing configuration. It binds a `UserEndpoint` and a `UserApiKey` together and defines how the credential is injected.

```
slug:             llm-openai        (used in /proxy/s/{slug}/*)
endpoint_id:      → UserEndpoint
api_key_id:       → UserApiKey
auth_method:      bearer | header | query | basic
auth_key_name:    Authorization (for header injection)
node_id:          optional – route via node agent instead of direct
catalog_service_id: optional – populated when created from catalog
```

The `slug` is the public handle. Proxy requests to `/api/v1/proxy/s/llm-openai/v1/chat/completions` are resolved by finding the `UserService` with `slug = "llm-openai"` owned by the authenticated user.

## How they fit together

```
UserService
  ├── slug: "llm-openai"
  ├── endpoint_id ──────────── UserEndpoint { url: "https://api.openai.com" }
  ├── api_key_id ───────────── UserApiKey   { credential_encrypted: … }
  ├── auth_method: bearer
  └── node_id: null            (direct routing; no node)
```

At proxy time, the handler:

1. Finds `UserService` by `(slug, user_id)`
2. Loads and decrypts the `UserApiKey`
3. Loads the `UserEndpoint` URL
4. Builds the outbound request: forwards to the endpoint URL, injects `Authorization: Bearer <decrypted_key>`

## Why three records instead of one

Separating these concerns lets users reuse credentials across services without duplicating them, share endpoints across different credential configurations, and update a target URL or rotate a credential without touching the routing config.

The same `UserApiKey` — an OpenAI key — can back multiple `UserService` records (perhaps one scoped for the MCP proxy and another for direct HTTP calls with a different injection method). The same `UserEndpoint` can be shared across services that call the same host.

## Auto-provisioning

The single command `nyxid service add llm-openai` (or the equivalent `POST /api/v1/keys` API call) creates all three records in one transaction via `unified_key_service`. Catalog defaults supply the URL and injection method; the user only needs to provide the credential. The resulting `UserService` slug is derived from the catalog entry name.

```bash
nyxid service add llm-openai --credential-env OPENAI_KEY
# Creates:
#   UserEndpoint { url: "https://api.openai.com" }
#   UserApiKey   { credential_encrypted: AES-GCM(env:OPENAI_KEY) }
#   UserService  { slug: "llm-openai", auth_method: bearer, ... }
```

## Catalog relationship

The service catalog (read via `GET /api/v1/catalog`) contains admin-managed service templates. These are `DownstreamService` records that serve as blueprints — they define the default base URL, injection method, and auth notes. When a user adds a service from the catalog, NyxID copies the relevant catalog defaults into the user's own three records. The user's records are independent after creation; changes to the catalog template do not automatically propagate to existing user records.

## Node-managed services

When `UserService.node_id` is set, the proxy routes through a credential node. In this mode:

- The `UserEndpoint.url` may be blank (the node resolves the URL locally).
- The `UserApiKey.credential_type` is `node_managed` (the credential never leaves the node).
- NyxID sends only request metadata (method, path, headers) to the node over WebSocket; the node injects the credential and forwards to the target.

See [Credential nodes](/docs/shared/concepts/credential-nodes) for the full node routing model.

## Per-agent credential overrides

An agent can be bound to a specific `UserApiKey` for a given `UserService`, overriding the service default. This is how two agents that both call OpenAI can inject different API keys (e.g., a standard quota key vs. a higher-tier key). The binding is stored in `AgentServiceBinding` and is resolved at proxy time before falling back to the service default.

See [Agent isolation](/docs/shared/concepts/agent-isolation) for the full override model.

## Related guides

- [Connect your agent](/docs/ai/getting-started/connect-your-agent)
- [Manage your keys](/docs/web/guides/manage-keys)
- [The broker model](/docs/shared/concepts/broker-model)
- [Credential nodes](/docs/shared/concepts/credential-nodes)
