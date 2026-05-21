---
title: Your first agent call
description: Add a service, make a proxied API call, and invoke it as an MCP tool — a complete end-to-end walkthrough for a first NyxID agent request.
---

This walkthrough connects an OpenAI service, sends a proxied HTTP request through NyxID, and then invokes the same endpoint as an MCP tool from Claude Code. By the end you will have seen credential injection, proxy routing, and MCP tool discovery working together.

## Prerequisites

- `nyxid` CLI installed and authenticated. If not, follow [Connect your agent](/docs/ai/getting-started/connect-your-agent).
- An OpenAI API key. Any key from [platform.openai.com/api-keys](https://platform.openai.com/api-keys) works.
- Claude Code (or another MCP client) wired to your NyxID MCP endpoint.

## Step 1: Add the service

Store the OpenAI key in an environment variable. The CLI reads it at add time so the value never appears in your shell history.

```bash
export OPENAI_KEY="sk-proj-..."
nyxid service add llm-openai --credential-env OPENAI_KEY --label "OpenAI"
```

NyxID provisions three records in a single operation:

- **UserEndpoint** — target URL `https://api.openai.com/v1`
- **UserApiKey** — encrypted credential
- **UserService** — proxy routing config with slug `llm-openai`

You can confirm the service is active:

```bash
nyxid service list
```

## Step 2: Make a proxied HTTP request

Use your NyxID API key (or a session token from `nyxid login`) as the `Authorization` header. NyxID authenticates the request, looks up your stored OpenAI credential, and injects it before forwarding to OpenAI.

```bash
curl -s https://nyx-api.chrono-ai.fun/api/v1/proxy/s/llm-openai/v1/chat/completions \
  -H "Authorization: Bearer $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Say hello in one sentence."}]
  }' | jq .
```

A successful response returns a standard OpenAI chat completion. The `Authorization: Bearer sk-proj-...` header you see going out to OpenAI is injected by NyxID — your `NYXID_API_KEY` is what actually left your machine.

:::tip
You can also use the CLI proxy command directly:

```bash
nyxid proxy request llm-openai v1/chat/completions \
  -m POST \
  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"Say hello."}]}'
```
:::

## Step 3: Verify the proxy flow

The proxy URL pattern is:

```
https://nyx-api.chrono-ai.fun/api/v1/proxy/s/{slug}/{path}
```

Where `{slug}` is your service slug (`llm-openai`) and `{path}` is passed through to the upstream base URL. NyxID prepends the stored base URL (`https://api.openai.com/v1`) so the full upstream request goes to `https://api.openai.com/v1/chat/completions`.

For a full diagram of what happens inside the proxy call, see [The proxy](/docs/shared/concepts/the-proxy).

## Step 4: Invoke as an MCP tool

With Claude Code wired to NyxID (see [Connect your agent](/docs/ai/getting-started/connect-your-agent)), open a Claude Code session and run `/mcp` to confirm `nyxid` is connected.

For MCP tools to appear, the catalog entry for `llm-openai` must have service endpoints registered (your admin configures this, or the catalog entry comes pre-configured). If endpoints are registered, you should see tools like `llm_openai__chat_completions` in the tool list.

To discover what tools are available, ask the agent:

> Use `nyx__search_tools` to find any tools related to chat completions.

Then invoke one directly:

> Use the `llm_openai__chat_completions` tool to send the message "Hello from MCP" with model `gpt-4o`.

The flow is:

```
Claude Code ──(MCP tool call)──► NyxID /mcp
                                     │
                                     ├─ Resolves tool: POST /v1/chat/completions
                                     ├─ Injects your stored OpenAI key
                                     └─ Forwards to api.openai.com
                                     ◄─ Returns response to Claude Code
```

Claude Code never sees your OpenAI key. The response it receives is the raw API response from OpenAI.

## Step 5: Check the audit log

Every proxied request is logged. To see the request you just made:

```bash
# Web console
# Navigate to: https://nyx.chrono-ai.fun → Admin → Audit Log

# CLI (if you have admin access)
nyxid admin audit-log --limit 5
```

If you created an Agent Key with `--platform claude-code`, each request carries that attribution. See [Isolate agents with scoped keys](/docs/ai/guides/agent-isolation) for how to separate traffic across multiple agents in the audit log.

## What's next

You have a working proxy and MCP integration. From here:

- **Use a different provider** — repeat Step 1 with `llm-anthropic` and your Anthropic key
- **Expose typed per-operation tools** — add an OpenAPI spec URL to surface named tools instead of a generic proxy; see [Wrap a REST API as MCP tools](/docs/ai/guides/wrap-rest-api-as-mcp)
- **Isolate agents** — create per-agent keys for Claude Code and Codex; see [Set up Claude Code, Cursor & Codex](/docs/ai/guides/claude-code-cursor-codex)
- **Add approvals** — require human approval before sensitive calls are forwarded; see [Approvals for agents](/docs/ai/guides/approvals-for-agents)
- **Understand MCP delegation** — how NyxID issues short-lived delegation tokens to downstream services; see [MCP proxy and tool discovery](/docs/ai/guides/mcp-proxy)
