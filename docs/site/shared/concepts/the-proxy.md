---
title: The proxy
description: How a request travels through NyxID's proxy layer, where credentials are injected, and what policy checks happen along the way.
---

Every API call that NyxID brokers passes through a single shared execution pipeline called `execute_proxy`. Whether the request arrives at a UUID-based URL (`/proxy/{service_id}/…`) or a slug-based URL (`/proxy/s/{slug}/…`), it follows the same path. Understanding that path — what is checked, what is injected, and what is forwarded — is useful for debugging slow requests, unexpected 403 responses, or questions about what NyxID can see.

## The execution pipeline

At a high level, `execute_proxy` has five stages:

### 1. Authentication

The incoming request must carry a valid identity: a session cookie, a Bearer JWT, or an API key in the `x-api-key` header. Authentication is enforced by middleware before the proxy handler is reached. Unauthenticated requests never enter the pipeline.

### 2. Service resolution

NyxID looks up which downstream service to call. For slug-based requests, the slug is resolved to a `UserService` record owned by the authenticated user. The resolution order is:

1. Personal `UserService` (the new streamlined model)
2. Legacy `UserServiceConnection` (pre-migration records)
3. Org-inherited `UserService` (if the user is a member of an org that owns the service)

If no service record is found for the caller, the request fails with 404.

### 3. Node routing check

Before touching any credentials, NyxID checks whether the resolved service is bound to a credential node. If an active, online node is bound, the request is dispatched to that node over WebSocket. The node injects credentials locally and forwards the request to the target; the credential never passes through NyxID. Node routing is transparent to the caller — the response looks identical to a direct-routed response.

See [Credential nodes](/docs/shared/concepts/credential-nodes) for how node routing and failover work.

### 4. Approval check

If approval is enabled for the service and the caller is using a programmatic authentication method (API key, delegated token, or service account), NyxID checks for an existing approval grant. If no valid grant exists, the proxy creates an approval request, notifies the user (via Telegram, mobile push, or web), and holds the HTTP connection open polling for a decision. The request proceeds only if the user approves. Requests that time out or are rejected receive a `403`.

Direct browser sessions (session-cookie auth) bypass approval — approvals are for programmatic callers, not interactive users.

See [Approvals (human-in-the-loop)](/docs/shared/concepts/approvals) for the per-request vs. grant distinction.

### 5. Credential injection and forwarding

With service and routing resolved and approval confirmed, NyxID:

- Decrypts the `UserApiKey` credential (AES-256-GCM, held in memory only for this request).
- Optionally adds identity propagation headers (`X-NyxID-User-Id`, `X-NyxID-User-Email`, etc.) if the service is configured for identity propagation.
- Optionally injects a short-lived delegation JWT (`X-NyxID-Delegation-Token`) for services that need to call NyxID APIs on behalf of the user.
- Builds the outbound request: target URL from `UserEndpoint`, allowed headers forwarded from the inbound request, credential injected per the configured `auth_method`.
- Forwards the request using a shared `reqwest` connection pool.
- Returns the downstream response to the caller, forwarding allowed response headers.

The credential is not stored anywhere after this point. It exists in process memory only for the duration of the outbound request.

## What NyxID does and does not forward

NyxID maintains explicit allowlists for which request headers to forward and which response headers to pass back. Headers outside the allowlist are stripped. This prevents callers from injecting internal headers and prevents downstream headers from leaking NyxID's internal context.

Request bodies are forwarded as-is, up to the configured size limit (100 MB for proxy routes by default). If approval is enabled for the service, the body is buffered briefly to build the action description shown to the approver; it is not persisted.

## Streaming responses

SSE (`text/event-stream`) responses, media types (`video/*`, `audio/*`), and large responses are streamed chunk-by-chunk rather than buffered in memory. Small responses (JSON API responses) are buffered, which allows error bodies to be logged for diagnostics. The streaming/buffering decision is made by inspecting the response `Content-Type` and `Content-Length`.

For node-routed responses, NyxID and the node agent use a WebSocket binary frame protocol for streaming chunks — data chunks travel as raw bytes prefixed with a request ID, avoiding the 33% overhead of base64 encoding.

## Two proxy URL formats

NyxID exposes two URL shapes for the proxy. Both route through the same `execute_proxy` pipeline:

| Format | Example | Notes |
|--------|---------|-------|
| Slug-based | `/api/v1/proxy/s/llm-openai/v1/chat/completions` | Preferred; uses the `UserService.slug` |
| UUID-based | `/api/v1/proxy/{service_id}/v1/chat/completions` | Supported for backward compatibility |

The slug format is preferred because the slug is human-readable, stable across credential rotations, and is what the CLI and MCP config use.

## Org-owned service resolution

When the authenticated user has no personal `UserService` for the requested slug, NyxID falls back to checking org memberships. If the user is a member of an org that owns a matching service, and the user's role and scope permit it, the org-owned service is used. The fallback query has a 500 ms wall-clock timeout to bound latency if the database is degraded. Users with a personal service never hit this path.

## Audit trail

Every proxied request is written to the audit log asynchronously (fire-and-forget, never blocking the response). Audit records include the user ID, service, method, path, response status, and — when relevant — the agent API key ID, node ID, and org routing context.

## Related guides

- [Your first agent call](/docs/ai/getting-started/first-agent-call)
- [The broker model](/docs/shared/concepts/broker-model)
- [Credential nodes](/docs/shared/concepts/credential-nodes)
- [Approvals (human-in-the-loop)](/docs/shared/concepts/approvals)
- [MCP proxy](/docs/shared/concepts/mcp-proxy)
