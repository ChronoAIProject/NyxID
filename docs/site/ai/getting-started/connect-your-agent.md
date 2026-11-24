---
title: Connect your agent
description: Connect Claude Code, Cursor or Codex to NyxID through MCP and check which services the client can see.
---

You need a NyxID account, one connected service and an MCP client with remote HTTP support. If this is a new account, first [connect a service in Studio](/docs/web/getting-started/first-connection#connect-a-service).

In the examples below the client opens NyxID's OAuth sign-in. Review the requested permissions and use the account that owns or can use the service. The resulting credential lets the client use NyxID within the approved access. The provider credential stays with the service connection in Studio.

For a client using a dedicated key, [create a scoped Agent Key](/docs/web/getting-started/first-connection#create-a-scoped-agent-key) and configure its full value as the remote MCP server's `X-API-Key` header using your client's private credential settings. It needs `proxy` scope and access to the intended service. Keep credentials out of shared project configuration.

Pick your client: [Claude Code](#claude-code), [Cursor](#cursor) or [Codex](#codex). For a self-hosted deployment, substitute your API origin in the MCP URL.

### Claude Code

```bash
claude mcp add --transport http --scope user nyxid https://nyx-api.chrono-ai.fun/mcp
```

Open Claude Code, run `/mcp`, select NyxID and complete authentication.

### Cursor

In **Tools & MCP**, add a custom server to your MCP configuration:

```json
{"mcpServers":{"nyxid":{"url":"https://nyx-api.chrono-ai.fun/mcp"}}}
```

Open the server entry and follow the sign-in prompt.

### Codex

```bash
codex mcp add nyxid --url https://nyx-api.chrono-ai.fun/mcp
codex mcp login nyxid
```

Finish sign-in in the browser, then open a Codex session with the server enabled.

## Make a read-only call

Follow [Your first agent call](/docs/ai/getting-started/first-agent-call) to list visible connections and check the response. If the client reports an authentication error, confirm that its URL ends in `/mcp` on the API host and reconnect using the intended account. [MCP proxy and tool discovery](/docs/ai/guides/mcp-proxy)
