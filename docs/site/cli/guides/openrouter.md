---
title: Use OpenRouter through NyxID
description: Connect one OpenRouter key from the CLI and route hundreds of models through NyxID — agents and tools only ever hold scoped nyx_ keys.
---

[OpenRouter](https://openrouter.ai) exposes hundreds of models from OpenAI, Anthropic, Google, Meta, Mistral, and more behind a single OpenAI-compatible API. Connect the key once with `nyxid service add`; every tool then reaches all of those models through its own scoped Agent Key.

Working from the dashboard instead? See the [web guide](/docs/web/guides/openrouter). Setting up a coding agent? The [AI-assisted guide](/docs/ai/guides/openrouter) covers pointing tools at the proxy.

## 1. Connect OpenRouter

Create a key at [openrouter.ai/keys](https://openrouter.ai/keys), then pass it via an environment variable so the secret never lands in your shell history:

```bash
export OPENROUTER_KEY=sk-or-v1-...
nyxid service add llm-openrouter --credential-env OPENROUTER_KEY --label "OpenRouter"
```

The CLI prints a `Slug:` line — `llm-openrouter` on a fresh account, suffixed (`llm-openrouter-2`) on repeat connects. That slug is the handle you proxy through.

## 2. Create a scoped Agent Key

```bash
nyxid api-key create \
  --name "openrouter-agent" \
  --platform generic \
  --scopes "proxy"
```

Save the printed `nyx_...` value — it is shown once. To limit the blast radius of a leak, scope it to the OpenRouter service only:

```bash
KEY_ID="<uuid-of-openrouter-agent-key>"
SVC_ID="<uuid-of-llm-openrouter-service>"    # from `nyxid service list`

nyxid api-key update "$KEY_ID" \
  --allowed-services "$SVC_ID" \
  --allow-all-services false
```

Both flags are required — a service list without `--allow-all-services false` is stored but not enforced. See [Create scoped agent keys](/docs/cli/guides/scoped-agent-keys).

## 3. Call models through the proxy

OpenRouter speaks the OpenAI Chat Completions API, and model IDs use the `vendor/model` form — `anthropic/claude-sonnet-4`, `openai/gpt-4o`, `meta-llama/llama-3.3-70b-instruct`:

```bash
curl -X POST "https://nyx-api.chrono-ai.fun/api/v1/proxy/s/llm-openrouter/chat/completions" \
  -w "\nHTTP=%{http_code}\n" \
  -H "Authorization: Bearer nyx_..." \
  -H "Content-Type: application/json" \
  -d '{"model":"openai/gpt-4o-mini","messages":[{"role":"user","content":"ping"}]}'
```

A successful response carries `X-NyxID-Agent-Id: <uuid>` — the request was attributed to your Agent Key in the audit log. For OpenAI-SDK tools, the equivalent base URL is `https://nyx-api.chrono-ai.fun/api/v1/llm/openrouter/v1`.

## Maintain the connection

```bash
nyxid catalog show llm-openrouter    # base URL, auth method, capabilities
nyxid service list                   # ids, slugs + status of your connections
nyxid service rotate-credential <id> --credential-env OPENROUTER_KEY   # rotate the key
```

## App attribution

OpenRouter [attributes API traffic to apps](https://openrouter.ai/docs/app-attribution) via the `HTTP-Referer` and `X-OpenRouter-Title` request headers. NyxID attaches its own attribution on proxied requests **only when your client does not send these headers itself** — a tool that already identifies itself to OpenRouter keeps its own attribution.
