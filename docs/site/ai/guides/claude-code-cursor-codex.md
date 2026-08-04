---
title: Set up Claude Code, Cursor & Codex
description: Give each local AI coding tool its own scoped NyxID Agent Key so requests are attributed, blast radius is limited, and credentials stay separate.
---

Without per-agent keys, Claude Code, Cursor, and Codex all share the same upstream API credential. One leaked key compromises everything; the audit log cannot distinguish which tool made which request; you cannot route different tools through different provider accounts.

This guide walks both Claude Code (Anthropic) and Codex (OpenAI) to the point where each has its own Agent Key, its own service scope, and its own row in the audit log. Cursor follows the same pattern.

For the underlying data model, see [Agent isolation](/docs/shared/concepts/agent-isolation).

## Prerequisites

- `nyxid` CLI installed and authenticated. Follow [Connect your agent](/docs/ai/getting-started/connect-your-agent) if not.
- An Anthropic API key for Claude Code. Get one from [console.anthropic.com](https://console.anthropic.com/settings/keys).
- An OpenAI API key for Codex. Get one from [platform.openai.com](https://platform.openai.com/api-keys).

## Part 1: Claude Code with Anthropic

### 1. Register Anthropic in NyxID

```bash
ANTHROPIC_KEY="sk-ant-api03-..." \
  nyxid service add llm-anthropic \
  --credential-env ANTHROPIC_KEY \
  --label "Anthropic"
```

NyxID prints the assigned slug. On a fresh account it is `llm-anthropic`; subsequent registrations of the same catalog entry are suffixed (`llm-anthropic-2`, and so on). Note the slug.

### 2. Create a Claude Code Agent Key

```bash
nyxid api-key create \
  --name "claude-coding" \
  --platform claude-code \
  --scopes "proxy"
```

Save the printed `nyx_...` value. It is shown once.

| Flag | Purpose |
|---|---|
| `--name` | Label in the console and audit log |
| `--platform` | Tool label recorded with every proxied request |
| `--scopes "proxy"` | Allows the key to send proxied requests |

### 3. Scope the key to Anthropic only

A scoped key cannot reach services outside its allowed list. If it leaks, only Anthropic traffic is at risk.

**Web console (recommended):**

1. Open **AI Services → Agent Keys → claude-coding**.
2. In the **Service Scope** card, uncheck **Allow all services**.
3. Select `Anthropic`. Click **Save**.

:::warning
Selecting services without unchecking **Allow all services** stores the list but does not enforce it — the key can still call any service. The console handles both fields in one save. If using the CLI, both flags are required:

```bash
CLAUDE_KEY="<uuid-of-claude-coding-key>"
ANTHROPIC_SVC="<uuid-of-llm-anthropic-service>"

nyxid api-key update "$CLAUDE_KEY" \
  --allowed-services "$ANTHROPIC_SVC" \
  --allow-all-services false
```
:::

### 4. Point Claude Code at NyxID

Claude Code uses the Anthropic SDK natively. Set the standard SDK environment variables to redirect it through NyxID's Anthropic proxy:

```bash
export ANTHROPIC_BASE_URL="https://nyx-api.chrono-ai.fun/api/v1/llm/anthropic"
export ANTHROPIC_API_KEY="nyx_..."   # the claude-coding key
```

NyxID receives the request, authenticates the Agent Key, swaps in your stored Anthropic key, and forwards to `api.anthropic.com`. Claude Code never sees the real Anthropic key.

For project-scoped variables (recommended), use `direnv`:

```bash
# .envrc in the project directory
export ANTHROPIC_BASE_URL="https://nyx-api.chrono-ai.fun/api/v1/llm/anthropic"
export ANTHROPIC_API_KEY="nyx_claude_..."
```

```bash
echo ".envrc" >> .gitignore   # add BEFORE writing the key
direnv allow
```

### 5. Verify

```bash
curl -i -X POST "$ANTHROPIC_BASE_URL/v1/messages" \
  -H "x-api-key: $ANTHROPIC_API_KEY" \
  -H "anthropic-version: 2023-06-01" \
  -H "Content-Type: application/json" \
  -d '{"model":"claude-3-5-sonnet-20241022","max_tokens":16,"messages":[{"role":"user","content":"ping"}]}'
```

A successful response includes `HTTP/1.1 200 OK` and the header `X-NyxID-Agent-Id: <uuid>` — confirming NyxID attributed the request to your Claude Code key.

## Part 2: Codex with OpenAI

Repeat the same pattern for Codex.

### 1. Register OpenAI

```bash
OPENAI_KEY="sk-proj-..." \
  nyxid service add llm-openai \
  --credential-env OPENAI_KEY \
  --label "OpenAI"
```

### 2. Create a Codex Agent Key

```bash
nyxid api-key create \
  --name "codex-work" \
  --platform codex \
  --scopes "proxy"
```

### 3. Scope to OpenAI only

Same procedure as Part 1, Step 3 — pick `OpenAI` in the **Service Scope** card, or pass the OpenAI service UUID with `--allowed-services` and `--allow-all-services false`.

### 4. Point Codex at NyxID

Codex uses the OpenAI API natively. Point it at NyxID's OpenAI-compatible gateway:

```bash
export OPENAI_BASE_URL="https://nyx-api.chrono-ai.fun/api/v1/llm/gateway/v1"
export OPENAI_API_KEY="nyx_..."   # the codex-work key
```

The gateway routes by model name: `gpt-*` and `o*` → OpenAI, `claude-*` → Anthropic, `gemini-*` → Google AI. One `OPENAI_BASE_URL` covers all providers, but the corresponding service must be registered in NyxID for each provider you want to call.

### 5. Verify

```bash
curl -i -X POST "$OPENAI_BASE_URL/chat/completions" \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"ping"}]}'
```

Response includes `X-NyxID-Agent-Id: <uuid>` — a different UUID from the Claude Code key. Both now appear independently in **Admin → Audit Log**.

## Adding Cursor

Follow Part 1 with `--platform cursor` and scope it to whichever services Cursor needs. The `OPENAI_BASE_URL` / `ANTHROPIC_BASE_URL` override pattern is the same.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `403` on proxy calls | Key is missing the `proxy` scope | Edit **AI Services → Agent Keys → [key] → Scopes**, add `proxy` |
| Scope list stored but ignored | `allow_all_services` left as `true` | Pass both `--allowed-services` and `--allow-all-services false` |
| `X-NyxID-Agent-Id` missing | Request used session auth, not an Agent Key | Use `Authorization: Bearer nyx_…` (or `x-api-key: nyx_…` for Anthropic paths) |
| Claude traffic fails | Anthropic service not registered | Run `nyxid service add llm-anthropic` |
| Wrong agent shows in audit log | Terminal loaded the wrong `.envrc` | Run `direnv reload` and re-check `echo $ANTHROPIC_API_KEY` |

## Production key layout

For production, create one key per tool per environment and scope each to only the services that tool needs:

| Environment | Tool | Agent Key | Allowed services |
|---|---|---|---|
| Laptop | Claude Code | `claude-coding-personal` | `llm-anthropic-personal` |
| Work laptop | Claude Code | `claude-coding-work` | `llm-anthropic-work` |
| Work laptop | Codex | `codex-work` | `llm-openai-work` |
| CI | Codex | `codex-ci` | `llm-openai-ci` |

To register the same provider twice with different credentials, pass `--slug` on the second add:

```bash
ANTHROPIC_WORK="sk-ant-work-..." \
  nyxid service add llm-anthropic \
  --slug llm-anthropic-work \
  --label "Anthropic Work" \
  --credential-env ANTHROPIC_WORK
```

## Operational notes

- **Per-agent rate limits.** Set them on the **Rate Limits** card of the key's detail page or via `PUT /api/v1/api-keys/{id}` with `rate_limit_per_second` and `rate_limit_burst` in the body.
- **Rotating one upstream credential.** Run `nyxid external-key rotate <id>` or replace via the web console. Every Agent Key bound to that credential picks up the new value on the next request — no agent restart needed.
- **Credential overrides.** Two agents can call the same service but inject different credentials. Configure this via **Agent Keys → [key] → Bindings**. Full details in [Agent isolation](/docs/shared/concepts/agent-isolation).
