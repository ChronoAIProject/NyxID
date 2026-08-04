---
title: What is AI-assisted access
description: An overview of how NyxID acts as a credential broker between AI agents and downstream APIs, replacing shared keys with scoped, auditable access.
---

NyxID is an auth, SSO, and credential-brokering platform. When an AI agent needs to call an external API — OpenAI, Anthropic, GitHub, an internal service — it sends the request through NyxID instead of holding the upstream API key itself. NyxID authenticates the agent, injects the real credential server-side, and forwards the request. The agent never sees the underlying secret.

## The broker model

The core idea is that credentials live in one place (NyxID) and are injected at proxy time. Every agent, human, or automated pipeline that needs to call a downstream API goes through the same choke point.

```
AI Agent (Claude Code, Codex, n8n, …)
    │
    │  NyxID Agent Key (nyx_…)
    ▼
NyxID Proxy ──────────────────────────────────────┐
    │  authenticate agent key                      │
    │  look up user's stored credential            │
    │  inject Authorization: Bearer sk-proj-…      │
    ▼                                              │
Downstream API (OpenAI, Anthropic, GitHub, …) ◄───┘
```

The upstream API key (e.g. `sk-proj-...`) never leaves NyxID. The agent key (`nyx_...`) is what the agent holds. If a key leaks, it can be revoked without rotating the underlying provider credential or disturbing other agents.

For the full technical design, see [The proxy](/docs/shared/concepts/the-proxy) and [The broker model](/docs/shared/concepts/broker-model).

## What NyxID provides for AI workflows

**Credential storage and injection.** External API keys, OAuth tokens, and bearer tokens are encrypted at rest (AES-256-GCM) and injected per request. Supported injection modes: `bearer`, `header`, `query`, `path`, `basic`.

**MCP proxy.** NyxID exposes connected service endpoints as MCP tools at `/mcp`. Cursor, Claude Code, Codex, and any MCP-capable client can call any connected service as a tool without managing credentials or writing an MCP server. See [MCP proxy and tool discovery](/docs/ai/guides/mcp-proxy).

**Agent isolation.** Each AI agent gets its own scoped API key (`nyxid_ag_...`), independent rate limits, and its own row in the audit log. Multiple agents on the same account cannot access each other's traffic. See [Isolate agents with scoped keys](/docs/ai/guides/agent-isolation).

**On-premise credential nodes.** For credentials that must not leave a specific host, a lightweight node agent runs on that machine. NyxID routes proxy requests through the node via WebSocket; the credential is injected locally and never transits the NyxID server. See [Credential nodes](/docs/shared/concepts/credential-nodes).

**Approvals.** Services can be configured to require human approval before an agent's proxy call is forwarded. Approval requests are delivered via Telegram, mobile push, or the web console. See [Approvals for agents](/docs/ai/guides/approvals-for-agents).

**`/llms.txt` and `/llms-full.txt`.** NyxID serves machine-readable context files at these paths so AI agents can self-orient on a live deployment: what services are connected, how to call them, and where to find docs. See [The llms.txt playbook](/docs/ai/guides/llms-txt-playbook).

## Key concepts at a glance

| Concept | What it is |
|---|---|
| **Catalog** | Read-only list of pre-configured service templates (OpenAI, Anthropic, GitHub, …). Admin-managed. |
| **UserService** | A user's proxy routing config: binds an endpoint URL + stored credential + auth method. Defines the slug used in `/proxy/s/{slug}/*`. |
| **MCP proxy** | Exposes connected service endpoints as MCP tools at `/mcp`. |
| **Agent Key** | A scoped NyxID API key for an AI agent (`nyxid_ag_…`). Carries its own rate limit and service scope. |
| **Node** | An on-premise agent that holds credentials locally and proxies requests through the NyxID server without exposing secrets upstream. |

## How users add services

Adding a service auto-provisions three records in a single operation: a `UserEndpoint` (target URL), a `UserApiKey` (the encrypted credential), and a `UserService` (the proxy routing config and slug). From the CLI:

```bash
nyxid service add llm-openai --credential-env OPENAI_KEY
```

From the web console: **AI Services** → **+ Add Key** → pick a catalog entry, enter the credential.

After this, `POST https://nyx-api.chrono-ai.fun/api/v1/proxy/s/llm-openai/v1/chat/completions` (with an NyxID key in the `Authorization` header) proxies through to OpenAI with the stored key injected automatically.

## Next steps

- [Connect your agent](/docs/ai/getting-started/connect-your-agent) — install the CLI and wire up an MCP client
- [Your first agent call](/docs/ai/getting-started/first-agent-call) — make a proxied request end to end
- [Set up Claude Code, Cursor & Codex](/docs/ai/guides/claude-code-cursor-codex) — per-agent isolation for local coding tools
