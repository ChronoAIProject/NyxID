# Wrap any REST API as MCP tools

**TL;DR** — Drop a REST API's OpenAPI spec URL into NyxID, point Claude Code (or Cursor or any MCP client) at NyxID's `/mcp` endpoint, and every endpoint in that spec becomes a typed MCP tool — with parameter schemas, descriptions, and credential injection wired in.

```
GitHub OpenAPI spec ──► NyxID ──(/mcp)──► Claude Code sees:
                                            • create_issue(repo, title, body)
                                            • list_pull_requests(repo, state)
                                            • search_code(query, language)
                                            ... (every operation in the spec)
```

You don't write a single MCP server. You don't define a tool schema. You don't paste an API key into Claude Code. Adding a new API to your agent's toolbox is one CLI call.

> This walkthrough assumes you've done [Step 0 of the n8n quickstart](n8n.md#step-0--get-nyxid-running-and-create-an-agent-key) — i.e., NyxID is reachable and your `nyxid` CLI is logged in. If not, see [docs/SETUP.md](../SETUP.md) first.

---

## The problem

I want my AI agent to use my project management API. It has 60-some endpoints. Three options I didn't take:

- **Hand-write an MCP server.** ~60 tool definitions, parameter schemas, error mapping, plus the auth code. Hours of glue, nothing about it is interesting.
- **Stuff API docs into the agent's system prompt.** It hallucinates endpoints, misformats payloads, leaks the API key into the chat transcript when it gets stuck.
- **Use a code-execution tool and let the agent write `curl`.** Same key-leak problem, plus you've handed the agent shell.

What I wanted: take the OpenAPI spec the API team already publishes, and let *NyxID* turn it into MCP tools. Credential lives in NyxID; agent never sees it.

---

## Setup

### 1. Add the service to NyxID with its OpenAPI spec URL

If the API is in NyxID's catalog (e.g., `llm-openai`, `api-github`), the spec URL is already set:

```bash
GH_TOKEN="$(cat ~/.gh_token)" \
  nyxid service add api-github --credential-env GH_TOKEN --label "GitHub"
```

If not, add a custom service and pass `--openapi-spec-url`. With `--custom` the slug comes from `--slug`, not the positional argument:

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

NyxID fetches the spec (DNS-pinned, 5MB cap, 60s cache) and parses operations. If you update the spec later, NyxID re-fetches on the next call past the cache TTL.

### 2. Verify the spec parses

```bash
nyxid catalog endpoints my-internal-api
```

Output is a table of `METHOD PATH` with a one-line description per operation. If you see an empty table, the spec didn't parse — common causes are 404 on the spec URL, the URL serving HTML (a docs page) instead of JSON/YAML, or a spec that's so large NyxID's 5MB ceiling refused it.

### 3. Wire Claude Code to NyxID's MCP endpoint

```bash
nyxid mcp config --tool claude-code
```

The output prints the exact `claude mcp add` command for your NyxID base URL, e.g.:

```bash
claude mcp add --transport http --scope user nyxid http://localhost:3001/mcp
```

Run it. The first time you launch `claude`, it opens a browser tab to authenticate against NyxID (OAuth). After that, Claude Code holds an MCP session token; you don't paste keys.

For other clients:

| Client | Command |
|---|---|
| Claude Code | `nyxid mcp config --tool claude-code` |
| Cursor | `nyxid mcp config --tool cursor` (writes `.cursor/mcp.json`) |
| VS Code | `nyxid mcp config --tool vscode` |
| Codex | `nyxid mcp config --tool codex` |
| Anything else (raw URL) | `nyxid mcp config --tool generic` |

### 4. Use it from the agent

In Claude Code, type `/mcp` to confirm `nyxid` is connected and listed. Then ask plainly:

> "Open a GitHub issue on `myorg/myrepo` titled 'Investigate flaky test in CI' and assign me."

Claude Code finds `create_issue` in the NyxID-provided tool list, fills in the parameters, and calls it. Behind the scenes, the call goes:

```
Claude Code ──(MCP tool call)──► NyxID /mcp
                                     │
                                     ├─ Maps tool → POST /repos/{owner}/{repo}/issues
                                     ├─ Injects Authorization: Bearer ghp_...
                                     └─ Forwards to api.github.com
```

Issue gets opened. Claude Code never saw the GitHub PAT.

---

## Why parsing the OpenAPI spec is the differentiator

Most of what an MCP server does is paperwork: tool name, description, parameter schema, response shape. OpenAPI specs already encode all of that. NyxID skips the paperwork by reading the spec your API team already published.

| | Hand-written MCP server | NyxID + OpenAPI spec |
|---|---|---|
| Tools added | One per `@tool`-decorated function you write | Every operation in the spec, automatically |
| Tool descriptions | You write them | Pulled from the spec's `summary`/`description` |
| Parameter schemas | You define `pydantic`/`zod` models | Pulled from `parameters` and `requestBody.content.schema` |
| Error mapping | You handle non-2xx and shape errors | NyxID's proxy returns the status + body unchanged |
| Auth | You paste the key into the MCP server's env | Injected by NyxID at proxy time |
| Adding a new API | A new MCP server | One `nyxid service add` |

For services where the API team **doesn't** publish a spec, the catalog still gives you the credential-injection benefit — the agent just sees a lower-level proxy tool (`call_proxy(slug, method, path, body)`) instead of typed per-operation tools.

---

## Gotchas

**Specs can be huge.** NyxID caps at 5 MB. AWS-sized specs (Stripe, AWS) won't fit. Workarounds: host a trimmed spec covering just the operations you want, and point `--openapi-spec-url` at it.

**OpenAPI 3.0 / 3.1 supported; Swagger 2.0 sometimes works.** If parsing fails, run the spec through [`swagger2openapi`](https://github.com/Mermade/oas-kit) first.

**Tool names come from `operationId`.** Specs without `operationId` get auto-generated names from method + path; they work but read as `post_repos__owner___repo__issues`. If you control the spec, give every operation a clean `operationId` — your agent's tool list reads like a Python module.

**Cache TTL is 60 seconds for the parsed spec.** During API development, change the spec URL query string or restart NyxID to force a re-fetch.

**MCP authentication is OAuth/browser by default.** That ties the MCP session to a NyxID user. If you want per-agent isolation (different scoped Agent Keys for different MCP sessions), see the [Claude Code per-agent quickstart](claude-code.md) for the underlying pattern; the MCP transport supports custom headers via the client's config file.

**Endpoint discovery and credential injection are independent.** A service with an OpenAPI spec URL but no credential will surface tools that fail at call time with a 401. A service with a credential but no spec URL will work via the lower-level proxy tools but won't show typed per-operation tools. You usually want both.

---

## Next

- **Per-agent isolation when multiple agents share the MCP endpoint:** [Claude Code per-agent keys](claude-code.md)
- **Wrap a private localhost API as MCP tools:** combine this with the [Node Proxy quickstart](node-proxy.md) — the OpenAPI spec works the same whether the upstream is public or behind a node
- **Reference for MCP delegation, identity headers, and token exchange:** [docs/MCP_DELEGATION_FLOW.md](../MCP_DELEGATION_FLOW.md)
