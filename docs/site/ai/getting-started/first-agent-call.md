---
title: Your first agent call
description: Ask an MCP client to list the NyxID connections it can see and check the returned service slug.
---

Start with a connected service and an authenticated MCP client. [Connect your agent](/docs/ai/getting-started/connect-your-agent) covers setup for Claude Code, Cursor and Codex.

## List visible connections

Send this prompt to your client:

> Call the NyxID tool `nyx__list_connected_services` with `{}`. Show the count and the name and slug of each visible connection.

Open the tool result in the client. Look for a `services` array containing your connection and a matching `count`. For example, a GitHub connection can appear as follows (other fields omitted):

```json
{"services":[{"name":"GitHub","slug":"api-github"}],"count":1}
```

This read verifies authentication and service discovery. To check GitHub itself, ask your client to use that connection to read your authenticated GitHub profile and report its `login`. The username in the tool result should match the account you connected. Review any approval request before continuing.

## If the connection is missing

An empty list is a valid response. Check the owner of the connection in Studio and the services granted to the client. After correcting the grant, reconnect the MCP server and repeat the same read. If the service appears but a provider call fails, check its credential in Studio. [Manage your connection](/docs/web/getting-started/first-connection) · [MCP tool discovery](/docs/ai/guides/mcp-proxy)
