# Quickstarts

Step-by-step procedures that take a single use case from prerequisites to a working integration. Each quickstart covers a distinct NyxID capability and is independent — complete them in any order.

For first-time NyxID setup (Docker stack, hosted signup, account registration), see [docs/SETUP.md](../SETUP.md). For the per-interface reference walkthrough (Web UI · CLI · AI-driven · Direct API), see [docs/connecting-services/](../connecting-services/).

| Quickstart | Outcome | NyxID capability |
|---|---|---|
| [**n8n: Daily AI News Digest with One NyxID Credential**](n8n.md) | An n8n workflow pulls an RSS feed, summarizes each article with Gemini, and posts to Telegram — using one `Header Auth` credential in n8n while NyxID stores the upstream Gemini and Telegram secrets. | Per-service credential injection |
| [**Per-Agent Keys for Claude Code and Codex**](claude-code.md) | Two coding agents on one machine, each scoped to a distinct service and credential, attributed independently in the audit log. | Agent isolation, scoped Agent Keys |
| [**Reach a Localhost API from a Cloud-Hosted Agent**](node-proxy.md) | A private-host API is reachable from a cloud agent without VPN, port forwarding, or a tunneling service. | Credential Node, outbound-only NAT traversal |
| [**Wrap a REST API as MCP Tools**](mcp-wrapping.md) | An OpenAPI spec is exposed as typed MCP tools to Claude Code / Cursor / VS Code / Codex with no MCP server code. | OpenAPI → MCP auto-wrap |

Every quickstart begins with the same prerequisite step — **Step 0: Get NyxID running and create an Agent Key**. The canonical version lives in the [n8n quickstart](n8n.md#step-0--get-nyxid-running-and-create-an-agent-key); the other three reference it.
