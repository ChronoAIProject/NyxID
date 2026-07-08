---
title: Use OpenRouter through NyxID
description: Route hundreds of AI models through one OpenRouter key that NyxID stores and injects — your agents and tools only ever hold scoped NyxID keys.
---

[OpenRouter](https://openrouter.ai) exposes hundreds of models from OpenAI, Anthropic, Google, Meta, Mistral, and more behind a single OpenAI-compatible API. Register your OpenRouter key with NyxID once, and every agent or tool reaches all of those models through its own scoped NyxID key — the real OpenRouter key never leaves NyxID.

For the underlying data model, see [Agent isolation](/docs/shared/concepts/agent-isolation). Prefer clicking through the dashboard? See the [web guide](/docs/web/guides/openrouter). Working purely from the terminal? See the [CLI guide](/docs/cli/guides/openrouter).

## Prerequisites

- `nyxid` CLI installed and authenticated. Follow [Connect your agent](/docs/ai/getting-started/connect-your-agent) if not.
- An OpenRouter API key. Create one at [openrouter.ai/keys](https://openrouter.ai/keys).

## 1. Register OpenRouter in NyxID

```bash
OPENROUTER_KEY="sk-or-v1-..." \
  nyxid service add llm-openrouter \
  --credential-env OPENROUTER_KEY \
  --label "OpenRouter"
```

NyxID prints the assigned slug. On a fresh account it is `llm-openrouter`; subsequent registrations of the same catalog entry are suffixed (`llm-openrouter-2`, and so on).

## 2. Create a scoped Agent Key

```bash
nyxid api-key create \
  --name "openrouter-agent" \
  --platform generic \
  --scopes "proxy"
```

Save the printed `nyx_...` value — it is shown once. To limit the blast radius of a leak, scope the key to the OpenRouter service only: open **AI Services → Agent Keys → openrouter-agent**, uncheck **Allow all services** in the **Service Scope** card, and select `OpenRouter`. The [Claude Code, Cursor & Codex guide](/docs/ai/guides/claude-code-cursor-codex) shows the equivalent CLI flags.

## 3. Point your tool at NyxID

OpenRouter speaks the OpenAI Chat Completions API, so any OpenAI-compatible tool works. Set the standard SDK environment variables to NyxID's OpenRouter proxy:

```bash
export OPENAI_BASE_URL="https://nyx-api.chrono-ai.fun/api/v1/llm/openrouter/v1"
export OPENAI_API_KEY="nyx_..."   # the openrouter-agent key
```

NyxID authenticates the Agent Key, swaps in your stored OpenRouter key, and forwards to `openrouter.ai`. Model IDs use OpenRouter's `vendor/model` form — for example `anthropic/claude-sonnet-4`, `openai/gpt-4o`, or `meta-llama/llama-3.3-70b-instruct`.

## 4. Verify

```bash
curl -i -X POST "$OPENAI_BASE_URL/chat/completions" \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"openai/gpt-4o-mini","max_tokens":16,"messages":[{"role":"user","content":"ping"}]}'
```

A successful response includes `HTTP/1.1 200 OK`. The same request also works through the general proxy route form, `POST /api/v1/proxy/s/llm-openrouter/chat/completions`, whose responses additionally carry `X-NyxID-Agent-Id: <uuid>` — confirming NyxID attributed the request to your Agent Key.

## App attribution

OpenRouter [attributes API traffic to apps](https://openrouter.ai/docs/app-attribution) via the `HTTP-Referer` and `X-OpenRouter-Title` request headers. NyxID attaches its own attribution on proxied requests **only when your client does not send these headers itself** — a tool that already identifies itself to OpenRouter keeps its own attribution.
