---
title: Create scoped agent keys
description: Give each AI agent its own NyxID API key from the terminal — scoped to specific services, optionally with its own upstream credential and audit identity.
---

In NyxID there is no separate "agent" object: **an API key is the agent identity.** Giving each tool (Claude Code, Codex, Cursor, a CI job) its own key means one leaked key reaches only the services you scoped it to, you can revoke or rotate one agent without touching the others, and every proxied request is attributed to that key in the audit log. This guide is the CLI procedure; for the full model see [Agent isolation](/docs/shared/concepts/agent-isolation), and for the agent-side setup see the [AI isolation guide](/docs/ai/guides/agent-isolation).

This assumes you are [logged in](/docs/cli/getting-started/authenticate) and have at least one [connected service](/docs/cli/guides/connect-a-service).

## Create a key

```bash
nyxid api-key create \
  --name "claude-coding" \
  --platform claude-code \
  --scopes "proxy"
```

`--platform` records the tool type for audit (`claude-code`, `codex`, `cursor`, `openclaw`, `generic`). `--scopes` is a space-separated subset of `read write proxy`. The key value (`nyx_...`) is shown **once** at creation — copy it immediately. Add `--terminal` to print it straight to the terminal instead of the browser wizard, `--expires-in-days <n>` for an auto-expiring key, and `--org <id|slug|name>` to create a key that authenticates as an organization.

## Scope it to specific services

A fresh key can reach all of your services (`allow_all_services: true`). To restrict it, list the service UUIDs **and** turn off the allow-all flag:

```bash
nyxid service list                       # copy the service UUIDs you want

nyxid api-key update <KEY_ID> \
  --allowed-services "<svc-uuid-1>,<svc-uuid-2>" \
  --allow-all-services false
```

:::warning
Both flags are required. Passing `--allowed-services` *without* `--allow-all-services false` stores the list but does not enforce it — the key still reaches everything. Restrict node access the same way with `--allowed-nodes` + `--allow-all-nodes false`.
:::

A key with `--allow-all-services false` and no allowed services has no proxy access at all — useful for a management-only or callback-only key.

## Bind a per-agent credential (optional)

Two agents can hit the same service with different upstream credentials — for example a cheap key for experiments and a premium key for production:

```bash
nyxid api-key bind claude-coding \
  --service llm-openai \
  --credential "openai-standard"   # label of the stored credential to inject
```

Without a binding, the proxy injects the service's default credential. Omit `--credential` to let NyxID auto-resolve it from the service.

## Use the key

Hand the `nyx_...` key value to the agent as an environment variable — never commit it. It authenticates proxy calls directly:

```bash
export NYXID_API_KEY=nyx_...
curl https://nyx-api.chrono-ai.fun/api/v1/proxy/s/llm-openai/models \
  -H "Authorization: Bearer $NYXID_API_KEY"
```

Responses carry an `X-NyxID-Agent-Id` header identifying the key, and each request records `api_key_id` + `api_key_name` in the audit log.

## Maintain keys

```bash
nyxid api-key list                 # all keys + platform + bindings count
nyxid api-key show <id>            # scope, rate limits, bindings
nyxid api-key rotate <id>          # new secret, same scope (old one stops working)
nyxid api-key delete <id> --yes    # revoke
```

## Next

- [`api-key` command reference](/docs/cli/reference/api-key) — every subcommand and flag, including per-key rate limits.
- [Set up Claude Code, Cursor & Codex](/docs/ai/guides/claude-code-cursor-codex) — wire a scoped key into a coding agent.
- [Agent isolation](/docs/shared/concepts/agent-isolation) — what isolation guarantees and why.
