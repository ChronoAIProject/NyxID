---
title: Connect your agent
description: Connect Claude Code, Cursor or Codex to NyxID through MCP and check which services the client can see.
---

You need a NyxID account, one connected service and an MCP client with remote HTTP support. If this is a new account, first [connect a service in Studio](/docs/web/getting-started/first-connection#2-add-an-ai-service).

In the examples below the client opens NyxID's OAuth sign-in. Review the requested permissions and use the account that owns or can use the service. The resulting credential lets the client use NyxID within the approved access. The provider credential stays with the service connection in Studio.

## Wire your MCP client

Pick your client below. Use the hosted MCP URL shown in its instructions, or substitute your instance's API origin for a self-hosted deployment.

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

### Generate the config automatically

To generate the configuration with the NyxID CLI, first sign in to your CLI profile with `nyxid login`. Run `nyxid mcp config --tool codex`, substituting `claude-code` or `cursor` for your client. Review the generated configuration before adding it. The client setup commands above can also be used directly in a fresh profile.

## What you see after wiring MCP

Open the client's MCP tools and look for `nyx__list_connected_services`. Use it to list the connections available to your signed-in account. If sign-in fails, check that the server URL ends in `/mcp` on the API host and retry. [Make your first read-only call](/docs/ai/getting-started/first-agent-call)

## Create an Agent Key (for the agent to use)

For a client that uses a dedicated key, create an Agent Key with the `proxy` scope and access to the intended connection. Store the full key in the client's private credential settings and use it in the remote MCP server's `X-API-Key` header. The provider credential stays with the connection. [Create a scoped key in Studio](/docs/web/getting-started/first-connection#3-create-a-nyxid-agent-key)

## Install the CLI

The CLI is optional for an interactive MCP client that signs in through the browser. Install it if you want it to generate a configuration or run an agent profile. [Install and verify the released CLI](/docs/cli/getting-started/install)

## Authenticate (you, the human)

Run `nyxid login` on your own computer and review the device request in your browser. Check the signed-in account with `nyxid whoami`. [CLI login options](/docs/cli/getting-started/authenticate#log-in)

### Authorize an agent profile without transferring a secret

For a CLI profile on an agent machine, a human can approve enrollment of a restricted Agent Key. The resulting profile uses that key's live permissions. [Agent Key login](/docs/cli/getting-started/authenticate#agent-key-login)

## Next steps

Use the read-only example to see the provider response. For per-agent keys, choose the client-specific setup. [Your first agent call](/docs/ai/getting-started/first-agent-call) · [Set up Claude Code, Cursor and Codex](/docs/ai/guides/claude-code-cursor-codex)
