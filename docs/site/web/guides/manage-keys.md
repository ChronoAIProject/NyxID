---
title: Manage keys & credentials
description: Understand and manage external service credentials, NyxID Agent Keys, scopes, rotation, and per-agent credential bindings from the web console.
---

NyxID separates two distinct key types that are used together but serve different purposes. This guide explains both, how to manage them from the web console, and how to scope and rotate them safely.

For a conceptual overview, see [Endpoints, keys, and services](/docs/shared/concepts/endpoints-keys-services).

## Two key types

| Type | What it is | Created in | Used by |
|---|---|---|---|
| **External service credential** (`UserApiKey`) | A real third-party API key, OAuth token, or bearer credential. NyxID stores it encrypted at rest (AES-256 envelope encryption). | **AI Services → External Services → Add Service** | NyxID proxy — never returned to agents |
| **NyxID Agent Key** (`ApiKey`, `nyx_...`) | A scoped key that identifies your agent or tool to NyxID. NyxID injects the underlying external credential server-side when this key is presented. | **AI Services → Agent Keys → Create API Key** | Your AI tools: `X-API-Key: nyx_...` |

The external credential is pasted once and never seen again. The Agent Key is what you copy into your tool's configuration.

## External service credentials

### Add a credential

From **AI Services → External Services**, click **Add Service**. Pick a catalog entry (OpenAI, Anthropic, GitHub, etc.) or enter a custom endpoint URL. Credentials are accepted as:

- **API key / bearer token** — pasted directly in the dialog
- **OAuth 2.0** — NyxID redirects you through the provider's consent flow and stores the resulting token + refresh token
- **Device code** — NyxID opens a device-code flow (used by providers like Anthropic and OpenAI for delegated access)
- **Node-managed** — the credential never reaches NyxID; it is stored locally on a [credential node](/docs/shared/concepts/credential-nodes)

### Update a credential

Open the service's detail page from **AI Services → External Services**, click the service card, then click **Edit Credential**. The new value replaces the previous one immediately; any in-flight proxy requests that started before the update use the old credential.

### Delete a service

From the service detail page, click **Delete Service**. This removes the `UserService`, `UserEndpoint`, and `UserApiKey` records. Active Agent Keys that were scoped to this service will get 403 on future proxy calls (the service no longer exists); update or revoke them accordingly.

## NyxID Agent Keys

### Create a key

From **AI Services → Agent Keys**, click **Create API Key**. Fill in:

| Field | Purpose |
|---|---|
| **Name** | Human-readable label (e.g. `claude-code`, `n8n-prod`). Appears in audit logs. |
| **Platform** | Optional tag — `claude-code`, `codex`, `cursor`, `generic`, etc. Informational only. |
| **Scopes** | Controls what the key can do. Add `proxy` for `/api/v1/proxy/...` requests. |
| **Service scope** | Optionally restrict the key to specific services. Leave blank for full access. |
| **Rate limits** | Optional per-key rate limit override (requests per second + burst capacity). |

The key (`nyx_...`) is shown **once** at creation. Copy it immediately.

### Scopes

| Scope | Allows |
|---|---|
| `proxy` | Call `/api/v1/proxy/...` and `/api/v1/proxy/s/...` |
| `read` | Read-only access to user data |
| `write` | Write access to user data |

Most agent use cases only need `proxy`. Add `read` or `write` if your agent manages NyxID resources programmatically.

### Service scope (allow-list)

By default a key can proxy any service you own. To lock a key to specific services:

1. Open the key's detail page from **Agent Keys**.
2. Under **Service Scope**, disable **Allow all services**.
3. Add specific service IDs from the picker.

A key that attempts to proxy a service outside its allow-list gets HTTP 403 with `error_code: 4003`.

### Rotate a key

From the key's detail page, click **Rotate Key**. NyxID generates a new `nyx_...` value and immediately invalidates the old one. Copy the new value and update it in all tools that use this key.

:::warning
Rotation is immediate. Any tool still using the old key will fail with 401 until it is updated.
:::

### Delete a key

From the key's detail page, click **Delete Key**. This revokes all access immediately.

## Per-agent credential bindings

Agent isolation lets different Agent Keys use different external credentials for the same service. This is useful when:

- Multiple developers share an org but each wants their own OpenAI billing key
- A staging agent should use a sandbox API key while production agents use the live key

### Set up a binding

1. Open a key's detail page from **Agent Keys**.
2. Scroll to **Credential Bindings**.
3. Click **Add Binding**, choose the target service, and pick the external credential to use.

When this Agent Key makes a proxy request to that service, NyxID uses the bound credential instead of the service's default.

For conceptual details, see [Agent isolation](/docs/shared/concepts/agent-isolation).

## The proxy URL

Every service is accessible at two proxy URL shapes:

```
# By service UUID
POST https://nyx.chrono-ai.fun/api/v1/proxy/{service_id}/{path}

# By slug (preferred)
POST https://nyx.chrono-ai.fun/api/v1/proxy/s/{slug}/{path}
```

The slug is shown on the service detail page. Use the slug form for readability; use the UUID form in automation where stability matters more than readability.

Authenticate proxy requests with the Agent Key:

```bash
curl https://nyx.chrono-ai.fun/api/v1/proxy/s/llm-openai/v1/chat/completions \
  -H "X-API-Key: nyx_YOUR_AGENT_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4o-mini", "messages": [{"role": "user", "content": "hello"}]}'
```

The response is the unmodified response from the upstream service — NyxID injects the credential and forwards everything else transparently.
