# Wrap a REST API as MCP Tools

Expose a REST API to MCP clients (Claude Code, Cursor, VS Code, Codex) as a set of typed tools — one tool per OpenAPI operation — without writing an MCP server. NyxID parses the OpenAPI spec, surfaces each operation through its `/mcp` endpoint, and injects the upstream credential at proxy time.

```
OpenAPI spec ──► NyxID ──(/mcp)──► Claude Code sees:
                                     • create_issue(repo, title, body)
                                     • list_pull_requests(repo, state)
                                     • search_code(query, language)
                                     ... one tool per OpenAPI operation
```

The MCP client never holds the upstream API key. Adding a new API to an agent's toolbox is one CLI command.

## Prerequisites

- A NyxID account and a logged-in `nyxid` CLI on your laptop. Follow [Step 0 of the n8n quickstart](n8n.md#step-0--get-nyxid-running-and-create-an-agent-key) if not already done.
- An OpenAPI 3.0 or 3.1 specification (URL- or file-hosted). Swagger 2.0 specs need to be converted first; [`swagger2openapi`](https://github.com/Mermade/oas-kit) handles the conversion.
- An MCP-capable client (Claude Code, Cursor, VS Code with an MCP plugin, or Codex).

## Procedure

### 1. Add the service to NyxID with its OpenAPI spec URL

For an API in NyxID's catalog (for example `api-github`, `llm-openai`), the catalog entry already declares the spec URL — `nyxid service add` resolves it automatically:

```bash
GH_TOKEN="$(cat ~/.gh_token)" \
  nyxid service add api-github \
  --credential-env GH_TOKEN \
  --label "GitHub"
```

For an API not in the catalog, register a custom service and pass `--openapi-spec-url`:

```bash
INTERNAL_API_KEY="$(cat ~/.internal_api_key)" \
  nyxid service add --custom \
  --slug my-internal-api \
  --label "Internal API" \
  --endpoint-url "https://api.internal.example.com" \
  --openapi-spec-url "https://api.internal.example.com/openapi.json" \
  --auth-method bearer \
  --auth-key-name "Authorization" \
  --credential-env INTERNAL_API_KEY
```

NyxID fetches the spec the first time it is needed: DNS-pinned, capped at 5 MB, cached for 60 seconds. Updates to the spec are picked up on the next request after the cache expires.

### 2. Verify the spec parses

List the operations NyxID extracted from the spec:

```bash
nyxid catalog endpoints my-internal-api
```

The output is a table of `METHOD PATH` rows with one-line descriptions pulled from the spec's `summary` / `description` fields. An empty table indicates the spec failed to parse — see [Troubleshooting](#troubleshooting).

### 3. Wire your MCP client to NyxID

Generate the MCP configuration snippet for your client:

```bash
nyxid mcp config --tool claude-code
```

The CLI prints the exact `claude mcp add` command pre-filled with your NyxID base URL, for example:

```bash
claude mcp add --transport http --scope user nyxid http://localhost:3001/mcp
```

Run it. On the next `claude` launch, the client opens a browser tab to authenticate against NyxID via OAuth. Authentication ties the MCP session to your NyxID user account; subsequent tool calls authenticate via the MCP session token, so you do not paste keys.

For other clients:

| Client | Command | Output |
|---|---|---|
| Claude Code | `nyxid mcp config --tool claude-code` | `claude mcp add` command + `.claude/settings.json` snippet |
| Cursor | `nyxid mcp config --tool cursor` | `.cursor/mcp.json` snippet |
| VS Code | `nyxid mcp config --tool vscode` | `.vscode/mcp.json` snippet |
| Codex | `nyxid mcp config --tool codex` | `codex mcp add` command + `~/.codex/config.toml` snippet |
| Other (raw URL) | `nyxid mcp config --tool generic` | NyxID MCP URL only |

### 4. Use the tools from the agent

In Claude Code, run `/mcp` to confirm `nyxid` is connected. The tool list now includes one entry per OpenAPI operation, named after the `operationId`.

A typical interaction:

> "Open a GitHub issue on `myorg/myrepo` titled 'Investigate flaky test in CI'."

The agent finds `create_issue` in the tool list, fills the parameters, and invokes it. The call flows:

```
Claude Code ──(MCP tool call)──►  NyxID /mcp
                                      │
                                      ├─ Resolves tool to POST /repos/{owner}/{repo}/issues
                                      ├─ Injects Authorization: Bearer ghp_…
                                      └─ Forwards to api.github.com
```

The issue is created. Claude Code never sees the GitHub token.

## Verification

```bash
nyxid catalog endpoints <slug>
```

confirms the spec parsed (Step 2). To confirm credential injection works end-to-end, invoke any tool from the MCP client that requires authentication. A successful response (e.g. an issue created on GitHub) verifies that NyxID parsed the spec, exposed the operation as a tool, accepted the MCP call, and injected the upstream credential.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `nyxid catalog endpoints` returns an empty table | Spec URL returns 404, returns HTML instead of JSON/YAML, or exceeds NyxID's 5 MB cap | Confirm the spec URL serves machine-readable OpenAPI (test with `curl -fsS <spec-url>`); if too large, host a trimmed spec covering only the operations you need |
| Tool list in Claude Code only shows `nyx__…` meta-tools, no operation tools | The connected service has no `openapi_spec_url`, or the spec failed to parse | Set the spec URL via `nyxid service update <ID> --openapi-spec-url <URL>`; rerun Step 2 to confirm parsing succeeds |
| Tool names look like `post_repos__owner___repo__issues` | The spec lacks `operationId` on those operations; NyxID auto-generates names from method + path | Add `operationId` to each operation in the spec for cleaner tool names |
| Spec changes are not picked up | Parsed-spec cache TTL is 60 s | Wait 60 s, change the spec URL query string, or restart NyxID |
| Tool call returns `401` from the upstream | Service has a spec URL but no credential | Add a credential via `nyxid service rotate-credential <ID>` (or set one through the web console) |
| Swagger 2.0 spec fails to parse | NyxID supports OpenAPI 3.0 / 3.1 only | Convert with [`swagger2openapi`](https://github.com/Mermade/oas-kit) and reference the converted file |

## Operational notes

- **MCP authentication.** The default `nyxid mcp config` flow ties the MCP session to a NyxID **user** via OAuth. For per-agent isolation across multiple MCP sessions on the same machine, use scoped Agent Keys — see the [Claude Code & Codex per-agent quickstart](claude-code.md) for the underlying pattern. The MCP transport supports custom headers via the client's config file.
- **Endpoint discovery and credential injection are independent.** A service with a spec URL but no credential surfaces tools that fail at call time with `401`. A service with a credential but no spec URL works through the generic proxy tool but does not surface typed per-operation tools. Both are usually wanted.
- **Wrapping a private localhost API as MCP tools.** Combine this guide with the [Node Proxy quickstart](node-proxy.md) — the OpenAPI spec parses identically whether the upstream is public or behind a node.
- **What spec parsing replaces.** OpenAPI specs encode tool names (`operationId`), descriptions (`summary` / `description`), parameter schemas (`parameters`, `requestBody.content.schema`), and error mappings. NyxID extracts these directly, so a hand-written MCP server with `@tool`-style decorators is unnecessary for any API that publishes a spec. For services without a published spec, the credential-injection benefit still applies — the agent receives a generic `call_proxy(slug, method, path, body)` tool instead of typed per-operation tools.

## Reference

- **MCP delegation, identity headers, token exchange**: [docs/MCP_DELEGATION_FLOW.md](../MCP_DELEGATION_FLOW.md)
- **Per-agent isolation**: [Claude Code & Codex per-agent quickstart](claude-code.md)
- **Other quickstarts**: [n8n](n8n.md) · [Node Proxy](node-proxy.md)
