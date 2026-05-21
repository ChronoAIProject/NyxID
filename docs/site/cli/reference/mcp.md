---
title: nyxid mcp
description: Reference for nyxid mcp — generate MCP client configuration so AI tools reach your NyxID services as tools.
---

`nyxid mcp` generates the Model Context Protocol configuration that points an AI tool at NyxID's MCP proxy, so the agent discovers and calls your connected services as tools — without ever seeing a raw credential. For the agent-side walkthrough see [MCP proxy & tool discovery](/docs/ai/guides/mcp-proxy); for the model see [The MCP proxy](/docs/shared/concepts/mcp-proxy).

:::note
The subcommand accepts the common flags `--base-url`, `--access-token` / `--access-token-env`, `--profile`, and `--output`. See [Authenticate](/docs/cli/getting-started/authenticate).
:::

## mcp config

```bash
nyxid mcp config [--tool cursor|claude-code|vscode|generic]
```

Print MCP server configuration tailored to the target tool (default `generic`). Paste the output into the tool's MCP config file (or pipe it where the tool expects it). The config wires the tool to NyxID's `/mcp` endpoint authenticated as you, so its tool calls proxy through NyxID with your stored credentials injected server-side.

:::tip
For per-agent isolation, generate the config against a [scoped agent key](/docs/cli/guides/scoped-agent-keys) (via `--access-token-env`) rather than your personal session, so each tool gets its own scoped identity and audit trail.
:::
