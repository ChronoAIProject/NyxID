# MCP Delegation Flow

This document explains how downstream services receive delegation tokens through NyxID's MCP proxy, how those tokens are used to call the LLM gateway or other NyxID-protected resources, and how to handle token expiry and refresh. It also covers alternative flows when the downstream service uses NyxID as its OIDC provider.

---

## Table of Contents

- [Overview](#overview)
- [Flow A: MCP Token Injection (No OIDC)](#flow-a-mcp-token-injection-no-oidc)
  - [Prerequisites](#prerequisites)
  - [Step-by-Step Flow](#step-by-step-flow)
  - [Headers Received by Downstream Service](#headers-received-by-downstream-service)
  - [Using the Delegation Token](#using-the-delegation-token)
  - [Token Expiry and Refresh](#token-expiry-and-refresh)
- [Flow B: OIDC-Connected Downstream Service](#flow-b-oidc-connected-downstream-service)
  - [How It Differs](#how-it-differs)
  - [Token Exchange (RFC 8693)](#token-exchange-rfc-8693)
  - [Using the Exchanged Token](#using-the-exchanged-token)
  - [Token Refresh](#token-refresh)
- [LLM Gateway Usage](#llm-gateway-usage)
  - [Provider-Specific Proxy](#provider-specific-proxy)
  - [OpenAI-Compatible Gateway](#openai-compatible-gateway)
  - [Checking Provider Status](#checking-provider-status)
- [Security Properties](#security-properties)
- [Configuration Reference](#configuration-reference)

---

## Overview

NyxID acts as an MCP server. When users connect to NyxID through MCP clients (Cursor, Claude Code, etc.), their configured downstream services are exposed as MCP tools. When a tool is invoked, NyxID proxies the request to the downstream service and can inject identity information and delegation tokens into the outgoing request.

There are two primary flows depending on whether the downstream service has its own OIDC integration with NyxID:

| Flow | When to Use | Token Source |
|------|-------------|--------------|
| **A: MCP Token Injection** | Downstream service does NOT use NyxID as OIDC provider | NyxID generates and injects a delegation token per tool call |
| **B: Token Exchange (RFC 8693)** | Downstream service uses NyxID as OIDC provider and has its own user sessions | Service exchanges a user access token for a scoped delegation token |

Both flows produce a delegation token that can be used to call NyxID APIs (LLM gateway, proxy, etc.) on behalf of the user.

---

## Flow A: MCP Token Injection (No OIDC)

This is the primary flow for downstream services that do not use NyxID as their identity provider. The downstream service receives delegation tokens automatically through headers on every proxied request.

### Prerequisites

An admin configures the downstream service in NyxID with:

- **Delegation Token Injection** enabled (`inject_delegation_token: true`)
- **Delegation Token Scope** set (e.g., `llm:proxy`)
- **Identity Propagation** set to `headers` (or `jwt` or `both`)
- **Endpoints** defined as MCP tools (method, path, parameters, descriptions)

### Step-by-Step Flow

```
User (in Cursor/Claude Code)
  │
  ├─ 1. Authenticates with NyxID (OAuth/login)
  │     MCP client receives NyxID access token
  │
  ├─ 2. MCP client fetches tool list
  │     GET /api/v1/mcp/config
  │     → Returns available services and their endpoints as MCP tools
  │
  ├─ 3. User invokes an MCP tool (e.g., "search_documents")
  │     MCP client sends tool call to NyxID MCP server
  │
  └─ NyxID MCP Server
       │
       ├─ 4. Resolves target service and endpoint
       │
       ├─ 5. Builds identity headers (user ID, email, name)
       │
       ├─ 6. Generates delegation token
       │     JWT with: sub=user_id, act.sub=service_slug,
       │     delegated=true, scope="llm:proxy", TTL=5 min
       │
       ├─ 7. Resolves any provider credentials the service needs
       │     (e.g., user's OpenAI API key for injection)
       │
       └─ 8. Forwards request to downstream service
             with all headers injected
             │
             Downstream Service
               │
               ├─ 9. Reads X-NyxID-Delegation-Token header
               │
               ├─ 10. Calls NyxID LLM gateway (or other API)
               │      Authorization: Bearer <delegation_token>
               │
               └─ 11. Returns response → NyxID → MCP client → User
```

### Headers Received by Downstream Service

When identity propagation is set to `headers` and delegation token injection is enabled, the downstream service receives these headers on every proxied request:

#### Identity Headers

| Header | Value | Description |
|--------|-------|-------------|
| `X-NyxID-User-Id` | UUID string | Authenticated user's NyxID ID |
| `X-NyxID-User-Email` | Email string | User's email address |
| `X-NyxID-User-Name` | Name string | User's display name |

Which identity fields are included depends on the service configuration (`identity_include_user_id`, `identity_include_email`, `identity_include_name`).

#### Delegation Token Header

| Header | Value | Description |
|--------|-------|-------------|
| `X-NyxID-Delegation-Token` | JWT string | Delegation access token (5 min TTL) |

#### Identity Assertion JWT (if propagation mode is `jwt` or `both`)

| Header | Value | Description |
|--------|-------|-------------|
| `X-NyxID-Identity-Token` | JWT string | Short-lived identity assertion (60 sec TTL) |

Contains user claims (`sub`, `email`, `name`) signed by NyxID. Useful for downstream services that want to cryptographically verify user identity without calling NyxID.

#### Provider Credentials (if service has provider requirements)

Provider credentials are injected based on the configured injection method:

| Method | Header/Param | Example |
|--------|-------------|---------|
| `bearer` | `Authorization: Bearer <token>` | OpenAI API key |
| `header` | Custom header (e.g., `x-api-key: <token>`) | Anthropic API key |
| `query` | Query parameter (e.g., `?key=<token>`) | Google API key |

### Using the Delegation Token

The downstream service extracts the delegation token from the `X-NyxID-Delegation-Token` header and uses it as a Bearer token when calling NyxID APIs.

#### Delegation Token JWT Claims

```json
{
  "sub": "<user_id>",
  "iss": "nyxid",
  "aud": "https://your-nyxid-instance.com",
  "exp": 1700000300,
  "iat": 1700000000,
  "jti": "<unique_token_id>",
  "scope": "llm:proxy",
  "token_type": "access",
  "act": {
    "sub": "<service_slug>"
  },
  "delegated": true
}
```

Key fields:

- `sub` -- the user the token acts on behalf of
- `act.sub` -- the service slug (identifies which downstream service is acting)
- `delegated: true` -- distinguishes delegation tokens from direct user tokens
- `scope` -- constrained to only the configured delegation scope

#### Example: Calling the LLM Gateway

```http
POST /api/v1/llm/gateway/v1/chat/completions HTTP/1.1
Host: your-nyxid-instance.com
Authorization: Bearer <delegation_token>
Content-Type: application/json

{
  "model": "claude-sonnet-4-5-20250929",
  "messages": [
    {"role": "user", "content": "Summarize this document..."}
  ]
}
```

NyxID receives this request, validates the delegation token, resolves the user's Anthropic API key from their connected providers, and proxies the request to Anthropic's API. The downstream service never sees the user's API key.

### Token Expiry and Refresh

**TTL:** Delegation tokens injected via MCP have a 5-minute TTL.

**For short-lived operations (most MCP tool calls):** The token is generated fresh for each tool invocation. The downstream service uses it for the duration of that single request. No refresh is needed.

**For long-running operations:** If the downstream service needs to make multiple calls to NyxID over a longer period (e.g., a multi-step workflow triggered by a single tool call), it can refresh the delegation token before it expires:

```http
POST /api/v1/delegation/refresh HTTP/1.1
Host: your-nyxid-instance.com
Authorization: Bearer <delegation_token>
```

Response:

```json
{
  "access_token": "<new_delegation_token>",
  "token_type": "Bearer",
  "expires_in": 300,
  "scope": "llm:proxy"
}
```

**Refresh rules:**

- Only delegation tokens can be refreshed at this endpoint (must have `act.sub` claim)
- NyxID re-verifies the user is still active
- NyxID re-verifies the user still has consent for the acting service (if applicable)
- A new 5-minute token is issued with the same scope and `act.sub`
- The old token remains valid until its original expiry

**Handling expiry errors:** If the token has already expired, the downstream service receives a `401 Unauthorized` response. At that point, the service cannot refresh the token -- it must wait for the next MCP tool invocation to receive a fresh token.

**Best practice:** Check the `exp` claim and refresh proactively when less than 60 seconds remain.

---

## Flow B: OIDC-Connected Downstream Service

When the downstream service uses NyxID as its OIDC provider, users have accounts on both NyxID and the downstream service. The downstream service has its own user sessions and holds the user's NyxID access token (obtained during OIDC login).

### How It Differs

| Aspect | Flow A (MCP Injection) | Flow B (OIDC + Token Exchange) |
|--------|----------------------|-------------------------------|
| User account | Only on NyxID | On both NyxID and downstream service |
| Token source | Injected by NyxID per tool call | Service exchanges user's access token |
| When tokens are obtained | Per MCP tool invocation | Any time the service needs to call NyxID |
| Token holder | Downstream service (per request) | Downstream service (stored in its session) |
| Use case | Stateless tool execution | Service-initiated calls on behalf of user |

### Token Exchange (RFC 8693)

The downstream service uses the OAuth 2.0 Token Exchange grant to obtain a delegation token from the user's access token.

**Prerequisites:**

- Downstream service is registered as an OAuth client in NyxID
- OAuth client has `delegation_scopes` configured (e.g., `["llm:proxy"]`)
- User has consented to the OAuth client (via OIDC login flow)
- Service holds the user's NyxID access token from the OIDC login

**Request:**

```http
POST /oauth/token HTTP/1.1
Host: your-nyxid-instance.com
Content-Type: application/x-www-form-urlencoded

grant_type=urn:ietf:params:oauth:grant-type:token-exchange
&client_id=<oauth_client_id>
&client_secret=<oauth_client_secret>
&subject_token=<user_access_token>
&subject_token_type=urn:ietf:params:oauth:token-type:access_token
&scope=llm:proxy
```

**Response:**

```json
{
  "access_token": "<delegation_token>",
  "token_type": "Bearer",
  "expires_in": 300,
  "issued_token_type": "urn:ietf:params:oauth:token-type:access_token",
  "scope": "llm:proxy"
}
```

**Validation steps NyxID performs:**

1. Authenticate the OAuth client via `client_id` + `client_secret`
2. Validate the `subject_token` is a valid, non-expired NyxID access token
3. Reject chained delegation (subject token cannot itself be a delegation token)
4. Verify user has active consent for this OAuth client
5. Validate requested scope is within the client's `delegation_scopes`
6. Issue delegation token (5-minute TTL, `act.sub` = client_id)

### Using the Exchanged Token

The exchanged delegation token has the same format and capabilities as the MCP-injected token. Use it identically:

```http
POST /api/v1/llm/gateway/v1/chat/completions HTTP/1.1
Host: your-nyxid-instance.com
Authorization: Bearer <delegation_token>
Content-Type: application/json

{
  "model": "gpt-4o",
  "messages": [...]
}
```

### Token Refresh

Same as Flow A -- use `POST /api/v1/delegation/refresh` with the delegation token:

```http
POST /api/v1/delegation/refresh HTTP/1.1
Host: your-nyxid-instance.com
Authorization: Bearer <delegation_token>
```

NyxID re-verifies user consent for the OAuth client before issuing a new token.

**If the user's original access token expires:** The downstream service must use its OIDC refresh token to obtain a new user access token, then perform a new token exchange.

### Combined Flow: OIDC Service as MCP Tool

When a downstream service that uses NyxID as OIDC is also exposed as an MCP tool, both flows can coexist:

- **Via MCP tool call:** NyxID injects delegation token automatically (Flow A). The downstream service can use this injected token OR exchange the user's stored access token (Flow B).
- **Via the service's own UI/API:** The service uses token exchange (Flow B) since there is no MCP proxy involved.

The service can identify the source by checking headers:
- If `X-NyxID-Delegation-Token` header is present → request came through MCP proxy
- If not → request came through the service's own channels

---

## LLM Gateway Usage

The LLM gateway is the primary use case for delegation tokens. It provides two proxy modes and a status endpoint.

### Provider-Specific Proxy

Route requests directly to a known provider:

```
ANY /api/v1/llm/{provider_slug}/v1/{path}
```

Example (OpenAI):

```http
POST /api/v1/llm/openai/v1/chat/completions HTTP/1.1
Authorization: Bearer <delegation_token>
Content-Type: application/json

{
  "model": "gpt-4o",
  "messages": [{"role": "user", "content": "Hello"}]
}
```

NyxID resolves the user's OpenAI API key and forwards to `https://api.openai.com/v1/chat/completions`.

Supported provider slugs: `openai`, `anthropic`, `google-ai`, `mistral`, `cohere`.

### OpenAI-Compatible Gateway

Send requests in OpenAI format to any supported provider -- NyxID routes based on model name and translates request/response formats:

```
ANY /api/v1/llm/gateway/v1/{path}
```

```http
POST /api/v1/llm/gateway/v1/chat/completions HTTP/1.1
Authorization: Bearer <delegation_token>
Content-Type: application/json

{
  "model": "claude-sonnet-4-5-20250929",
  "messages": [{"role": "user", "content": "Hello"}]
}
```

Model-to-provider routing:

| Model Prefix | Provider |
|-------------|----------|
| `gpt-`, `o1-`, `o3-`, `o4-`, `chatgpt-` | OpenAI |
| `claude-` | Anthropic |
| `gemini-` | Google AI |
| `mistral-`, `codestral-`, `pixtral-`, `ministral-` | Mistral |
| `command-`, `embed-`, `rerank-` | Cohere |

### Checking Provider Status

Before making LLM calls, check which providers the user has connected:

```http
GET /api/v1/llm/status HTTP/1.1
Authorization: Bearer <delegation_token>
```

Response:

```json
{
  "providers": [
    {
      "provider_slug": "openai",
      "provider_name": "OpenAI",
      "status": "ready",
      "proxy_url": "https://your-nyxid-instance.com/api/v1/llm/openai/v1"
    },
    {
      "provider_slug": "anthropic",
      "provider_name": "Anthropic",
      "status": "not_connected",
      "proxy_url": "https://your-nyxid-instance.com/api/v1/llm/anthropic/v1"
    }
  ],
  "gateway_url": "https://your-nyxid-instance.com/api/v1/llm/gateway/v1",
  "supported_models": ["gpt-", "o1-", "claude-", "gemini-", ...]
}
```

Status values:
- `ready` -- user has a valid, non-expired token for this provider
- `not_connected` -- user has not connected this provider
- `expired` -- user's token for this provider has expired

---

## Security Properties

### Trust Chain

```
User authenticates to NyxID via MCP client
  → MCP tool invoked for downstream service
    → NyxID generates delegation token (5 min, scoped, with act.sub)
      → Injected as X-NyxID-Delegation-Token header
        → Downstream service uses token to call NyxID LLM gateway
          → NyxID validates token, resolves user's provider credentials
            → Request forwarded to LLM provider with credentials injected
```

### Key Guarantees

- **Short-lived tokens:** 5-minute TTL limits blast radius of token leakage
- **Scoped access:** Delegation tokens are constrained to configured scopes (e.g., `llm:proxy` only)
- **No credential exposure:** User's LLM provider API keys are never sent to the downstream service; NyxID injects them server-side
- **No chained delegation:** A delegation token cannot be exchanged for another delegation token
- **Active user check:** Every request with a delegation token re-verifies the user is active
- **Consent verification:** Token exchange and refresh both verify user consent is still active
- **Audit trail:** Token generation, usage, and refresh are all logged

### Route Access Control

Delegation tokens can only access a subset of NyxID endpoints:

| Accessible | Not Accessible |
|-----------|----------------|
| LLM gateway (`/llm/*`) | Auth flows (`/auth/*`) |
| Proxy (`/proxy/*`) | User profile (`/users/*`) |
| Delegation refresh (`/delegation/*`) | Admin panel (`/admin/*`) |
| | MCP config (`/mcp/*`) |
| | Session management (`/sessions/*`) |

---

## Configuration Reference

### Downstream Service Settings (Admin)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `inject_delegation_token` | bool | `false` | Enable delegation token injection |
| `delegation_token_scope` | string | `"llm:proxy"` | Scopes included in delegation token |
| `identity_propagation_mode` | string | `"none"` | `none`, `headers`, `jwt`, or `both` |
| `identity_include_user_id` | bool | `true` | Include `X-NyxID-User-Id` header |
| `identity_include_email` | bool | `true` | Include `X-NyxID-User-Email` header |
| `identity_include_name` | bool | `true` | Include `X-NyxID-User-Name` header |
| `identity_jwt_audience` | string | (base_url) | `aud` claim in identity assertion JWT |

### OAuth Client Settings (for Token Exchange)

| Field | Type | Description |
|-------|------|-------------|
| `client_id` | string | OAuth client identifier |
| `client_secret` | string | OAuth client secret |
| `delegation_scopes` | string[] | Allowed scopes for token exchange |

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SA_TOKEN_TTL_SECS` | `3600` | Service account token TTL |
| `JWT_ACCESS_TTL_SECS` | `900` | User access token TTL (15 min) |
| `BASE_URL` | `http://localhost:3001` | Used as `aud` in tokens |
| `JWT_ISSUER` | `nyxid` | Used as `iss` in tokens |

Delegation token TTLs are hardcoded:
- MCP injection: 300 seconds (5 minutes)
- Token exchange: 300 seconds (5 minutes)
