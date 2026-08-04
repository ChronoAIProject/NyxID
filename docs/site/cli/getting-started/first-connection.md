---
title: Your first connection
description: Connect an AI service from the terminal and verify it with a proxied request that returns HTTP 200.
---

Four steps, ending in an `HTTP/1.1 200` from your first proxied call. This assumes the [CLI is installed](/docs/cli/getting-started/install) and you are [logged in](/docs/cli/getting-started/authenticate).

Throughout, substitute your real OpenAI / Anthropic / GitHub key for `sk-...` — that is your **external service credential**, not a NyxID key. NyxID stores it [encrypted](/docs/shared/concepts/encryption) and never returns it.

## 1. Set the provider credential

```bash
export OPENAI_API_KEY=sk-...
```

## 2. Add the service from the catalog

```bash
nyxid service add llm-openai --credential-env OPENAI_API_KEY
```

This provisions the endpoint, stores the credential, and creates the routing config in one step. (For a private API the catalog doesn't know, pass `--custom --endpoint-url <url> --auth-method bearer`.)

## 3. Copy the returned slug

The CLI prints a `Slug:` line. If `llm-openai` already existed on your account, the new entry may be suffixed (e.g. `llm-openai-2`). Use **that exact slug** in the next step — it is the handle that addresses your specific service instance.

## 4. Verify with a proxied request

```bash
nyxid proxy request <RETURNED_SERVICE_SLUG> models
```

Success is an `HTTP/1.1 200` carrying a real provider JSON body — for OpenAI's `models` endpoint, `{"object":"list","data":[{"id":"gpt-...","object":"model",...}]}`. NyxID injected your stored credential; the request never carried it from your terminal.

:::warning
**Windows:** run every command above from WSL Ubuntu or Git Bash, not the raw command prompt.
:::

### If it errors

- **`401` from the provider** — the key from step 1 is wrong or revoked. Rotate it: `nyxid service rotate-credential <id> --credential-env <NEW_VAR>` (find `<id>` with `nyxid service list`).
- **`403` from NyxID** — your session or key lacks the `proxy` scope.

## Next

- [Wire your AI agent to NyxID](/docs/ai/getting-started/first-agent-call) — let Claude Code, Cursor, or Codex call this service through MCP.
- [The proxy](/docs/shared/concepts/the-proxy) — how a proxied request flows and where the credential is injected.
- [Browse the catalog](/docs/cli/reference/catalog): `nyxid catalog list`, `nyxid catalog show <slug>`, `nyxid catalog endpoints <slug>`.
