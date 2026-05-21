---
title: MCP proxy & tool discovery
description: How NyxID's MCP endpoint aggregates connected services into typed tools, injects credentials, and issues short-lived delegation tokens to downstream services.
---

NyxID exposes a single MCP endpoint at `<BASE_URL>/mcp`. When an MCP client (Claude Code, Cursor, Codex) connects and authenticates, NyxID aggregates every endpoint from every connected service into MCP tools. When a tool is invoked, NyxID injects the user's stored credential and forwards the request to the downstream service.

For the conceptual background, see [MCP proxy](/docs/shared/concepts/mcp-proxy).

## How tool names are formed

Each service endpoint becomes one MCP tool. Tool names follow the pattern `{service_slug}__{operation_id}`, for example:

- `llm_openai__chat_completions`
- `api_github__create_issue`
- `my_internal_api__list_users`

The `operationId` comes from the service's OpenAPI spec. If no `operationId` is set, NyxID generates a name from the HTTP method and path (e.g. `post_v1_chat_completions`). Adding `operationId` to spec entries produces cleaner tool names.

In addition to per-service tools, NyxID always surfaces four meta-tools:

| Tool | Purpose |
|---|---|
| `nyx__discover_services` | Browse the catalog and list connected services |
| `nyx__connect_service` | Add a service from within the agent |
| `nyx__search_tools` | Fuzzy-search across all available tools |
| `nyx__call_tool` | Invoke any tool by name with arbitrary arguments |

## Connecting a service via the agent

From inside the MCP client, paste this prompt to add a service without leaving the chat:

> Help me connect an AI Service in NyxID. Use `nyx__discover_services` to list available catalog entries and ask me which one I want. Once I pick, ask me for the credential, then call `nyx__connect_service`. After success, call `nyx__search_tools` to confirm the new tools are exposed, then call `nyx__call_tool` on one to verify the proxy works end-to-end. Report back with the actual response.

## Manual wiring (CLI)

```bash
nyxid mcp config --tool claude-code   # prints the claude mcp add command
nyxid mcp config --tool cursor        # prints .cursor/mcp.json snippet
nyxid mcp config --tool codex         # prints codex mcp add command
```

Run the printed command. On the first launch after wiring, the client opens a browser for OAuth authentication. The session is tied to your NyxID user account; subsequent tool calls authenticate via the MCP session token.

## Credential injection at tool call time

When a tool is invoked, NyxID runs the full proxy pipeline:

```
MCP client ──(tool call)──► NyxID /mcp
                                 │
                                 ├─ 1. Resolve target: service slug + endpoint path
                                 ├─ 2. Decrypt stored credential (AES-256-GCM)
                                 ├─ 3. Build identity headers (if enabled)
                                 ├─ 4. Generate delegation token (if enabled)
                                 ├─ 5. Forward request to downstream with credential injected
                                 └─ 6. Return response to MCP client
```

The MCP client never sees the upstream API key. The response it receives is the raw downstream API response.

## Delegation tokens

For downstream services that need to call NyxID's LLM gateway or proxy on behalf of the user, NyxID can inject a short-lived delegation token into every proxied request. This is the MCP delegation flow.

### How delegation tokens work

The downstream service receives the `X-NyxID-Delegation-Token` header on every proxied request. The token is a 5-minute RS256 JWT with these key claims:

```json
{
  "sub": "<user_id>",
  "act": { "sub": "<service_slug>" },
  "scope": "llm:proxy",
  "delegated": true
}
```

The service uses the token as a Bearer token when calling NyxID APIs (LLM gateway, proxy). NyxID validates the token, resolves the user's provider credential, and proxies the call — the downstream service never sees the underlying API key.

To enable delegation token injection on a service, an admin sets `inject_delegation_token: true` and `delegation_token_scope` on the service's catalog entry.

### Using the delegation token

```http
POST /api/v1/llm/gateway/v1/chat/completions HTTP/1.1
Authorization: Bearer <delegation_token>
Content-Type: application/json

{
  "model": "claude-sonnet-4-5-20250929",
  "messages": [{"role": "user", "content": "Summarize this document."}]
}
```

NyxID resolves the user's Anthropic key and forwards to Anthropic. The downstream service sees only the delegation token; Anthropic sees only the real key.

### Refreshing a delegation token

Delegation tokens have a 5-minute TTL. For multi-step workflows that span longer than that, refresh before expiry:

```http
POST /api/v1/delegation/refresh HTTP/1.1
Authorization: Bearer <delegation_token>
```

Response:

```json
{
  "access_token": "<new_token>",
  "token_type": "Bearer",
  "expires_in": 300,
  "scope": "llm:proxy"
}
```

:::note
Only delegation tokens can be refreshed at this endpoint. If the token has already expired, the downstream service must wait for the next MCP tool invocation to receive a fresh one.
:::

### Token exchange (RFC 8693)

For downstream services that use NyxID as their OIDC provider, delegation tokens can also be obtained via the OAuth 2.0 Token Exchange grant. The service exchanges the user's NyxID access token for a scoped delegation token:

```http
POST /oauth/token HTTP/1.1
Content-Type: application/x-www-form-urlencoded

grant_type=urn:ietf:params:oauth:grant-type:token-exchange
&client_id=<oauth_client_id>
&client_secret=<oauth_client_secret>
&subject_token=<user_access_token>
&subject_token_type=urn:ietf:params:oauth:token-type:access_token
&scope=llm:proxy
```

The resulting token has the same format and capabilities as the MCP-injected token.

## LLM gateway routing

The OpenAI-compatible gateway at `/api/v1/llm/gateway/v1` routes by model name prefix:

| Model prefix | Provider |
|---|---|
| `gpt-`, `o1-`, `o3-`, `o4-`, `chatgpt-` | OpenAI |
| `claude-` | Anthropic |
| `gemini-` | Google AI |
| `mistral-`, `codestral-` | Mistral |
| `command-` | Cohere |

The corresponding service must be registered in NyxID for each provider. Check which providers are ready for a given delegation token:

```http
GET /api/v1/llm/status HTTP/1.1
Authorization: Bearer <delegation_token>
```

## Security properties

| Property | Detail |
|---|---|
| Short-lived tokens | 5-minute TTL limits the blast radius of a leaked delegation token |
| Scoped access | Delegation tokens are constrained to the configured scope (e.g. `llm:proxy` only) |
| No credential exposure | Upstream API keys are injected server-side; downstream services never see them |
| No chained delegation | A delegation token cannot be exchanged for another delegation token |
| Active user check | Every request re-verifies the user is active |
| Audit trail | Token generation, usage, and refresh are all logged |

Delegation tokens can only reach a subset of NyxID endpoints: the LLM gateway, proxy, and delegation refresh. They cannot reach auth, user profile, admin, MCP config, or session endpoints.
