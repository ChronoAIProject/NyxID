---
title: Your first agent call
description: Ask an MCP client to list the NyxID connections it can see and check the returned service slug.
---

## Prerequisites

You need a NyxID account, a GitHub connection and an authenticated MCP client. [Connect your agent](/docs/ai/getting-started/connect-your-agent) covers setup for Claude Code, Cursor and Codex.

## Step 1: Add the service

If GitHub is new to this account, connect it in Studio and complete its authorization first. Copy the slug from **External Services**. [Connect GitHub](/docs/web/getting-started/first-connection#2-add-an-ai-service)

## Step 2: Make a proxied HTTP request

To check the service from a terminal first, use the CLI profile read. It sends `GET /user` through your connection and returns the authenticated GitHub username. You can also perform the same read from the MCP client below. [Run the CLI example](/docs/cli/getting-started/first-connection#4-verify-with-a-proxied-request)

## Step 3: Verify the proxy flow

In the authenticated MCP client, list the connections it can see.

Send this prompt to your client:

> Call the NyxID tool `nyx__list_connected_services` with `{}`. Show the count and the name and slug of each visible connection.

Open the tool result in the client. Look for a `services` array containing your connection and a matching `count`. For example, a GitHub connection can appear as follows (other fields omitted):

```json
{"services":[{"name":"GitHub","slug":"api-github"}],"count":1}
```

## Step 4: Invoke as an MCP tool

Ask your client “Use the GitHub connection you just listed to read my authenticated GitHub profile. Report its `login`.” Open the tool result and compare the username with the account you authorized. If a request needs approval, check that the requested connection and profile read match your task. [Review approvals](/docs/web/guides/approvals)

## Step 5: Check the audit log

If you administer the instance, open **Admin → Audit Log** to inspect the request. As a first-time user, you can verify the read directly in the client's tool result.

## If the connection is missing

An empty list is a valid response. Check the owner of the connection in Studio and the services granted to the client. After correcting the grant, reconnect the MCP server and repeat the same read. If the service appears but a provider call fails, check its credential in Studio. [Manage your connection](/docs/web/getting-started/first-connection) · [MCP tool discovery](/docs/ai/guides/mcp-proxy)

## What's next

Explore the operations available on another connection and discover how service permissions affect a client. [MCP proxy and tool discovery](/docs/ai/guides/mcp-proxy) · [Isolate agents with scoped keys](/docs/ai/guides/agent-isolation)
