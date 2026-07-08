---
title: Use OpenRouter through NyxID
description: Connect one OpenRouter key from the web console and route hundreds of models through NyxID — with a built-in probe that verifies the credential end to end.
---

[OpenRouter](https://openrouter.ai) exposes hundreds of models from OpenAI, Anthropic, Google, Meta, Mistral, and more behind a single OpenAI-compatible API. Paste the key once in the console; every agent or tool then reaches all of those models through its own scoped Agent Key — the real OpenRouter key is stored encrypted and never shown again.

Prefer the terminal? See the [CLI guide](/docs/cli/guides/openrouter). Setting up a coding agent? The [AI-assisted guide](/docs/ai/guides/openrouter) covers pointing tools at the proxy.

## 1. Connect OpenRouter

Create a key at [openrouter.ai/keys](https://openrouter.ai/keys). Then, from **AI Services → External Services**, click **Connect Service** and search for *OpenRouter* in the **Add AI Service** dialog. Pick the **OpenRouter API** tile and paste your `sk-or-v1-...` key.

## 2. Create and test an Agent Key

After the service connects, the dialog offers to create an Agent Key for it — a scoped `nyx_...` key your tools present instead of the real credential. Copy it when shown; it appears once.

Click **Test Agent Key**: NyxID makes a real proxied call to OpenRouter's key endpoint, which confirms both the Agent Key and your stored OpenRouter credential work (an invalid OpenRouter key is rejected here, not on your first real request). On success, a **Shell env** snippet appears with ready-to-paste `OPENAI_BASE_URL` / `OPENAI_API_KEY` exports for OpenAI-compatible tools.

To limit the blast radius of a leak, scope the key: open **AI Services → Agent Keys → your key**, uncheck **Allow all services** in the **Service Scope** card, select `OpenRouter`, and save.

## 3. Copy usage examples

Open the service from **AI Services → External Services** to get its detail page. The **API Usage** card shows the proxy URL and copy-ready curl examples — note the model field uses OpenRouter's `vendor/model` form (`openai/gpt-4o`, `anthropic/claude-sonnet-4`, `meta-llama/llama-3.3-70b-instruct`); bare model names are rejected by OpenRouter.

Rotation and credential edits work like any other service — see [Manage keys & credentials](/docs/web/guides/manage-keys).

## App attribution

OpenRouter [attributes API traffic to apps](https://openrouter.ai/docs/app-attribution) via the `HTTP-Referer` and `X-OpenRouter-Title` request headers. NyxID attaches its own attribution on proxied requests **only when your client does not send these headers itself** — a tool that already identifies itself to OpenRouter keeps its own attribution.
