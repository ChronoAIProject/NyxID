---
title: The MCP proxy
description: How AI tools such as Claude Code and Cursor discover and call NyxID-connected services through the Model Context Protocol, including how delegation tokens flow.
---

NyxID implements an MCP (Model Context Protocol) server at `/mcp`. When a user connects an MCP client — Claude Code, Cursor, Codex, or any MCP-compatible runtime — to that endpoint, their configured services are exposed as MCP tools. The agent can invoke those tools without managing credentials, knowing URLs, or writing any MCP server code.

## What MCP unlocks

Without MCP, an agent that wants to call multiple services needs either hardcoded credentials for each or a separate configuration step per service. With NyxID's MCP proxy, the agent discovers all of the user's connected services as a unified tool list and calls them using the same authentication it already has with NyxID.

## How the MCP client connects

MCP clients that support OAuth use NyxID's standard discovery flow:

1. The client fetches `GET /.well-known/oauth-protected-resource` to find NyxID's authorization server URL.
2. The client fetches `GET /.well-known/oauth-authorization-server` to discover the authorization and registration endpoints.
3. The client self-registers via `POST /oauth/register` (RFC 7591 dynamic client registration) as a public client.
4. The user authenticates via the Authorization Code + PKCE flow.
5. The client uses the resulting access token as a Bearer token when connecting to `/mcp`.

This is fully automatic — the user does not need to register the MCP client manually or copy client IDs. The only user action is logging in to NyxID when prompted.

## Tool discovery

Once connected, the MCP client requests the tool list. NyxID returns one set of tools per connected service, based on the endpoints defined in the service catalog. Tool names and descriptions are derived from the catalog metadata, so the agent understands what each tool does without additional context.

NyxID also exposes a small number of built-in tools for navigation:

| Tool | Purpose |
|------|---------|
| `nyx__discover_services` | Browse all available services |
| `nyx__search_tools` | Search and activate tools by keyword |
| `nyx__connect_service` | Connect to a specific service and activate its tools |

## How a tool call flows

When the agent invokes a tool, NyxID:

1. Resolves the tool to a target service and endpoint.
2. Builds identity headers if the service has identity propagation enabled (`X-NyxID-User-Id`, `X-NyxID-User-Email`, etc.).
3. Generates a short-lived delegation token (5-minute TTL, RS256 JWT) and injects it as `X-NyxID-Delegation-Token`.
4. Resolves any provider credentials the service requires and injects them.
5. Forwards the tool call as an HTTP request to the downstream service.
6. Returns the response to the agent.

The downstream service receives the request and can use the delegation token to make further calls to NyxID (for example, to call the LLM gateway on behalf of the user) without ever seeing the user's upstream API keys.

## Delegation tokens

A delegation token is a scoped, short-lived JWT that represents "the downstream service acting on behalf of this user." Key properties:

- `sub` claim: the user's NyxID ID (who is being acted for)
- `act.sub` claim: the service slug (who is acting)
- `delegated: true` flag: distinguishes it from a regular user token
- 5-minute TTL: limits blast radius if a token is intercepted
- Scope-constrained: only the scopes configured on the service (e.g. `llm:proxy`) are included

The downstream service presents the delegation token to NyxID's LLM gateway or other protected endpoints. NyxID validates the token, resolves the user's provider credentials server-side, and forwards the request. The service never sees the user's OpenAI or Anthropic API key.

:::note
Delegation tokens cannot be exchanged for other delegation tokens. Chained delegation is blocked.
:::

## Token refresh for long-running operations

Most MCP tool calls complete within the 5-minute token TTL. For long-running workflows that make multiple downstream calls, the service can refresh a delegation token before it expires:

```http
POST /api/v1/delegation/refresh
Authorization: Bearer <delegation_token>
```

NyxID re-verifies that the user is still active and issues a new 5-minute token with the same scope and `act.sub`. The old token remains valid until its original expiry.

## Token exchange for OIDC-connected services

If the downstream service uses NyxID as its own OIDC provider, it may already hold the user's NyxID access token from the login flow. In this case, the service can exchange that token for a delegation token using RFC 8693 Token Exchange:

```http
POST /oauth/token
Content-Type: application/x-www-form-urlencoded

grant_type=urn:ietf:params:oauth:grant-type:token-exchange
&client_id=<oauth_client_id>
&client_secret=<oauth_client_secret>
&subject_token=<user_access_token>
&subject_token_type=urn:ietf:params:oauth:token-type:access_token
&scope=llm:proxy
```

NyxID validates the user token, checks that the OAuth client has consent and the requested scope within its `delegation_scopes`, and returns a 5-minute delegation token. This path is for service-to-NyxID calls that happen outside an active MCP tool invocation.

## LLM gateway

The most common use of delegation tokens is calling NyxID's LLM gateway. The gateway accepts an OpenAI-compatible request format and routes to the correct provider based on the model name:

```http
POST /api/v1/llm/gateway/v1/chat/completions
Authorization: Bearer <delegation_token>
Content-Type: application/json

{"model": "claude-sonnet-4-6", "messages": [...]}
```

NyxID matches the `claude-` prefix, resolves the user's Anthropic API key, and proxies to Anthropic's API. The downstream service only needed to know the NyxID gateway URL and have a valid delegation token.

## Security properties

- **No credential exposure.** User API keys are injected server-side and never sent to the downstream service.
- **No chained delegation.** A delegation token cannot be exchanged for another delegation token.
- **Active user check.** Every request with a delegation token re-verifies the user is active.
- **Route access control.** Delegation tokens can access only `llm/*`, `proxy/*`, and `delegation/*`. They cannot access auth flows, user profiles, admin panels, or session management.
- **Consent verification.** Token exchange and refresh both verify user consent before issuing a new token.

## Related guides

- [MCP proxy and tool discovery](/docs/ai/guides/mcp-proxy)
- [Wrap a REST API as an MCP tool](/docs/ai/guides/wrap-rest-api-as-mcp)
- [The proxy](/docs/shared/concepts/the-proxy)
- [OAuth & OIDC identity](/docs/shared/concepts/oauth-oidc)
