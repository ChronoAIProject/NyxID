---
title: Wrap a REST API as MCP tools
description: Add an OpenAPI spec URL to a NyxID service and each operation becomes a typed MCP tool — no MCP server to write, no credentials to manage in the client.
---

Any REST API that publishes an OpenAPI 3.0 or 3.1 spec can be exposed to MCP clients as a set of typed tools — one tool per operation — without writing a custom MCP server. NyxID fetches the spec, surfaces each operation through `/mcp`, and injects the upstream credential at proxy time.

```
OpenAPI spec ──► NyxID ──(/mcp)──► Claude Code sees:
                                     create_issue(repo, title, body)
                                     list_pull_requests(repo, state)
                                     search_code(query, language)
                                     … one tool per OpenAPI operation
```

The MCP client never holds the upstream API key.

## Prerequisites

- `nyxid` CLI installed and authenticated. Follow [Connect your agent](/docs/ai/getting-started/connect-your-agent) if not.
- An OpenAPI 3.0 or 3.1 spec accessible at a URL. Swagger 2.0 specs must be converted first; [`swagger2openapi`](https://github.com/Mermade/oas-kit) handles that.
- An MCP-capable client (Claude Code, Cursor, Codex).

## Step 1: Add the service with an OpenAPI spec URL

### From the catalog (most common)

Catalog entries for services like `api-github` already include a spec URL. `nyxid service add` resolves it automatically:

```bash
GH_TOKEN="ghp_..." \
  nyxid service add api-github \
  --credential-env GH_TOKEN \
  --label "GitHub"
```

### Custom service not in the catalog

```bash
INTERNAL_KEY="secret-token" \
  nyxid service add --custom \
  --slug my-internal-api \
  --label "Internal API" \
  --endpoint-url "https://api.internal.example.com" \
  --openapi-spec-url "https://api.internal.example.com/openapi.json" \
  --auth-method bearer \
  --auth-key-name "Authorization" \
  --credential-env INTERNAL_KEY
```

NyxID fetches the spec on first use: DNS-pinned, capped at 5 MB, cached for 60 seconds. Updates to the spec are picked up after the cache expires.

### Adding a spec URL to an existing service

```bash
nyxid service update <SERVICE_ID> \
  --openapi-spec-url "https://api.internal.example.com/openapi.json"
```

## Step 2: Verify the spec parsed

List the operations NyxID extracted:

```bash
nyxid catalog endpoints my-internal-api
```

This returns a table of `METHOD PATH` rows with one-line descriptions from the spec's `summary` / `description` fields. An empty table means the spec failed to parse — see [Troubleshooting](#troubleshooting).

You can also query the endpoint list via API:

```bash
curl https://nyx-api.chrono-ai.fun/api/v1/catalog/my-internal-api/endpoints \
  -H "Authorization: Bearer $NYXID_API_KEY"
```

## Step 3: Wire your MCP client

```bash
nyxid mcp config --tool claude-code   # prints the claude mcp add command
```

Run the printed command. On the next `claude` launch, authenticate via browser. After that, typed per-operation tools appear automatically in the tool list.

For other clients:

| Client | Command |
|---|---|
| Cursor | `nyxid mcp config --tool cursor` |
| Codex | `nyxid mcp config --tool codex` |
| Generic | `nyxid mcp config --tool generic` |

## Step 4: Use the tools from the agent

In Claude Code, run `/mcp` to confirm `nyxid` is connected. The tool list now includes one entry per OpenAPI operation.

A typical interaction:

> "Open a GitHub issue on `myorg/myrepo` titled 'Investigate flaky test in CI'."

The agent finds `create_issue`, fills the parameters, and invokes it:

```
Claude Code ──(MCP tool call)──► NyxID /mcp
                                     │
                                     ├─ Resolves tool: POST /repos/{owner}/{repo}/issues
                                     ├─ Injects Authorization: Bearer ghp_…
                                     └─ Forwards to api.github.com
```

The issue is created. Claude Code never sees the GitHub token.

## What spec parsing provides

OpenAPI specs encode everything NyxID needs to build typed MCP tools:

| Spec field | MCP output |
|---|---|
| `operationId` | Tool name (`api_github__create_issue`) |
| `summary` / `description` | Tool description shown to the model |
| `parameters` | Per-parameter schemas and descriptions |
| `requestBody.content.schema` | Request body schema |
| `responses` | Error mapping |

For services without a published spec, credential injection still applies — the agent receives a generic `call_proxy(slug, method, path, body)` tool instead of typed per-operation tools.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `nyxid catalog endpoints` returns empty | Spec URL returns 404, returns HTML, or exceeds 5 MB | Confirm the spec URL serves valid OpenAPI: `curl -fsS <spec-url>`. If too large, host a trimmed spec |
| Only `nyx__…` meta-tools appear | Service has no `openapi_spec_url`, or spec failed to parse | Set via `nyxid service update <ID> --openapi-spec-url <URL>`, then rerun Step 2 |
| Tool names like `post_v1_chat_completions` | Spec operations lack `operationId` | Add `operationId` to each operation for cleaner names |
| Spec changes not picked up | 60-second parse cache | Wait 60 s or append a query string to the spec URL to bust the cache |
| Tool call returns `401` from upstream | Spec URL set but no credential stored | Add a credential via `nyxid service rotate-credential <ID>` or through the web console |
| Swagger 2.0 spec fails to parse | NyxID only supports OpenAPI 3.0 / 3.1 | Convert with [`swagger2openapi`](https://github.com/Mermade/oas-kit) |

## Operational notes

- **Private localhost APIs.** Combine this guide with a credential node — the spec parses identically whether the upstream is public or behind a node. See [Credential nodes](/docs/shared/concepts/credential-nodes).
- **Per-agent MCP sessions.** The default `nyxid mcp config` flow ties the MCP session to a NyxID user via OAuth. For per-agent isolation across multiple MCP sessions, use scoped Agent Keys — see [Set up Claude Code, Cursor & Codex](/docs/ai/guides/claude-code-cursor-codex).
- **Endpoint discovery and credential injection are independent.** A service with a spec URL but no credential surfaces tools that fail with `401` at call time. A service with a credential but no spec URL works through the generic proxy tool. Both are usually wanted together.
