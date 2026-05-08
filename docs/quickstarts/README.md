# Quickstarts

End-to-end recipes for real workflows. Each one is a complete narrative — start to working integration — covering setup, the proxy URL pattern, before/after, and gotchas.

For first-time NyxID setup (Docker stack, hosted signup, registering an account), see [docs/SETUP.md](../SETUP.md). For the mechanical "how do I get `HTTP/1.1 200`" reference per interface (Web UI / CLI / AI-driven / Direct API), see [docs/connecting-services/](../connecting-services/).

| Quickstart | What you build | NyxID feature it shows |
|---|---|---|
| **[n8n: 1 credential, 4 APIs](n8n.md)** | Daily AI digest with Gemini, TwitterAPI.io, Google Sheets (OAuth), Telegram all proxied through one Header Auth credential | Per-service credential injection across 4 auth styles |
| **[Claude Code & Codex per-agent keys](claude-code.md)** | Two coding agents on one machine, each scoped to its own credential, with per-agent attribution in the audit log | Agent isolation + scoped Agent Keys |
| **[Reach localhost from a cloud agent](node-proxy.md)** | Expose a home-server API to a remote agent without VPN, port forwarding, or Cloudflare Tunnel | Credential node / outbound-only NAT traversal |
| **[Wrap any REST API as MCP tools](mcp-wrapping.md)** | Drop an OpenAPI spec URL, get typed MCP tools in Claude Code / Cursor / Codex without writing a single tool definition | OpenAPI → MCP auto-wrap |

Each quickstart shares "Step 0 — Get NyxID running and create an Agent Key" — see the [n8n quickstart's Step 0](n8n.md#step-0--get-nyxid-running-and-create-an-agent-key) for the canonical version.
