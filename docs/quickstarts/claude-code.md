# Per-Agent Keys for Claude Code and Codex

Give each local coding agent its own NyxID Agent Key. Instead of reusing one shared API key across Claude Code, Codex, Cursor, or any other tool, every agent gets a scoped `nyx_…` key, its own service scope, and its own row in the audit log. The result is per-agent attribution, blast-radius isolation if a key leaks, and the ability to route each agent's traffic through a different upstream provider or account.

```
Claude Code  ── nyx_claude_… ──┐
                                │
                                ├── NyxID ──┬── Anthropic API
                                │           │     (scoped to claude-coding key)
Codex        ── nyx_codex_…  ──┘           │
                                            └── OpenAI API
                                                  (scoped to codex-work key)
```

This guide walks Claude Code through the Anthropic provider and Codex through the OpenAI provider — the typical native split. The same pattern works for any combination (two Codex sessions on different OpenAI accounts, three Claude Code projects on different Anthropic accounts, etc.); see [Two isolation patterns](#two-isolation-patterns) and [Production pattern](#production-pattern).

## Concepts

### What this solves

Without per-agent keys, multiple coding tools share a single upstream API key. That makes it hard to answer:

- Which agent made this request?
- Which key should be revoked if one tool leaks credentials?
- Was this request from a personal project or a work project?
- How do I separate quota or premium-model access across agents?

With one NyxID Agent Key per agent, every request is attributable, scopes are independent, and rotating one agent's access doesn't disturb the others.

### What gets stored where

| Item | Stored in | Purpose |
|---|---|---|
| NyxID Agent Key for Claude Code (`nyx_claude_…`) | Local Claude Code project env | Authenticates Claude Code to NyxID |
| NyxID Agent Key for Codex (`nyx_codex_…`) | Local Codex project env | Authenticates Codex to NyxID |
| Anthropic API key | NyxID | Injected when Claude Code calls Anthropic |
| OpenAI API key | NyxID | Injected when Codex calls OpenAI |
| Service scope (per-key allowed services) | NyxID Agent Key settings | Bounds what each agent can call |

The local agents only see NyxID Agent Keys. They never receive the upstream provider keys.

### Two isolation patterns

NyxID supports two patterns for per-agent isolation:

**Pattern 1 — separate service entries.** Each agent points at a distinct NyxID service slug (e.g. `claude-coding` → `llm-anthropic`, `codex-work` → `llm-openai`). This is what this guide uses; it is the simplest pattern and what `nyxid service add` creates in one step.

**Pattern 2 — same slug, per-agent credential overrides.** Multiple agents call the same logical service (e.g. `llm-openai`) but each binding overrides which credential gets injected. Used when two agents share a provider but need different upstream accounts. Implemented via `agent_service_bindings`; full data model in [docs/AGENT_ISOLATION.md](../AGENT_ISOLATION.md).

### Before and after

**Without per-agent keys**, every tool holds the same upstream key:

```
Claude Code ─┐
Codex       ─┼── shared API key
Cursor      ─┘
```

The audit log can't distinguish them; one leak compromises everything; you cannot route different tools to different accounts.

**With per-agent keys**, each tool authenticates with its own scoped Agent Key:

```
Claude Code  ── nyx_claude_… ── Anthropic
Codex        ── nyx_codex_…  ── OpenAI
Cursor       ── nyx_cursor_… ── OpenAI Team
```

Per-agent audit attribution, scoped revocation, separate quotas.

## Prerequisites

- A NyxID account and a logged-in `nyxid` CLI on your laptop. Follow [Step 0 of the n8n quickstart](n8n.md#step-0--get-nyxid-running-and-create-an-agent-key) if not already done.
- An Anthropic API key for Claude Code. Get one from [console.anthropic.com → API Keys](https://console.anthropic.com/settings/keys). Save to `~/.anthropic_key` with `chmod 600`.
- An OpenAI API key for Codex. Get one from [platform.openai.com → API keys](https://platform.openai.com/api-keys). Save to `~/.openai_key` with `chmod 600`.

## Quickstart: Claude Code with its own Agent Key (Anthropic)

This walks one agent (Claude Code) end-to-end. The next section ([Add Codex](#add-codex-with-its-own-agent-key-openai)) repeats the pattern for Codex.

### 1. Register Anthropic in NyxID

```bash
ANTHROPIC_KEY="$(cat ~/.anthropic_key)" \
  nyxid service add llm-anthropic \
  --credential-env ANTHROPIC_KEY \
  --label "Anthropic"
```

NyxID prints the assigned slug — typically `llm-anthropic` on a fresh account, suffixed (`-2`, `-3`, …) if you already have a service with that slug. Record it for the wire-up step.

### 2. Create the Claude Code Agent Key

```bash
nyxid api-key create \
  --name "claude-coding" \
  --platform claude-code \
  --scopes "proxy"
```

Save the printed `nyx_…` value — shown once.

| Flag | Purpose |
|---|---|
| `--name` | Human-readable name shown in the NyxID console and audit logs |
| `--platform` | Tool label (`claude-code`, `codex`, `cursor`, `openclaw`, `generic`) recorded with every proxied request |
| `--scopes "proxy"` | Allows the key to send proxied requests through NyxID |

> The key allows access to all of your services by default. Step 3 restricts it to the Anthropic service.

### 3. Restrict the Agent Key to the Anthropic service

A scoped key cannot call services outside its allowed list, so a leak only affects Anthropic.

**Web console (recommended):**

1. Open `AI Services` → `Agent Keys` → `claude-coding`.
2. In the `Service Scope` card, uncheck `Allow all services`.
3. Select `Anthropic`. Click `Save`.

> **Important — both `allowed_services` and `allow_all_services: false` are required.**
>
> Selecting services without unchecking `Allow all services` stores the list but does **not** scope the key — it can still hit every service. The web console handles both fields in one save; the CLI requires both flags explicitly.

**CLI alternative** (uses UUIDs):

```bash
nyxid service list
ANTHROPIC_SVC=11111111-aaaa-…   # llm-anthropic ID

nyxid api-key list
CLAUDE_KEY=44444444-eeee-…      # claude-coding ID

nyxid api-key update "$CLAUDE_KEY" \
  --allowed-services "$ANTHROPIC_SVC" \
  --allow-all-services false
```

### 4. Wire Claude Code

Claude Code uses the Anthropic API natively. Point it at NyxID's Anthropic provider proxy by setting the standard Anthropic SDK environment variables:

```bash
# Self-host
export ANTHROPIC_BASE_URL="http://localhost:3001/api/v1/llm/anthropic"
export ANTHROPIC_API_KEY="nyx_…"   # claude-coding

# Hosted
export ANTHROPIC_BASE_URL="https://<your-nyxid-host>/api/v1/llm/anthropic"
export ANTHROPIC_API_KEY="nyx_…"
```

Most local coding tools expect a provider API key environment variable (here `ANTHROPIC_API_KEY`). NyxID exploits this slot — the tool sends the NyxID Agent Key in the same place an Anthropic key would normally go. NyxID authenticates the Agent Key, swaps in the real Anthropic key, and forwards the request to `api.anthropic.com`.

For project-scoped variables, use `direnv` so the right key is active automatically when you `cd` into the project:

```bash
# .envrc (in the Claude Code project directory)
export ANTHROPIC_BASE_URL="http://localhost:3001/api/v1/llm/anthropic"
export ANTHROPIC_API_KEY="nyx_claude_…"
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
  -d '{"model":"claude-3-5-sonnet-20241022","max_tokens":1024,"messages":[{"role":"user","content":"ping"}]}'
```

Successful response includes:

- `HTTP/1.1 200 OK`
- `X-NyxID-Agent-Id: <claude-coding-key-uuid>` — confirms NyxID attributed the request to your Agent Key
- An Anthropic `messages` response body

Open `AI Services` → `Agent Keys` → `claude-coding` and look at the `Usage` card to see the request logged against this key.

## Add Codex with its own Agent Key (OpenAI)

Repeat the Quickstart pattern for Codex on the OpenAI provider. Each step is the same shape as the Claude Code steps above.

### 1. Register OpenAI in NyxID

```bash
OPENAI_KEY="$(cat ~/.openai_key)" \
  nyxid service add llm-openai \
  --credential-env OPENAI_KEY \
  --label "OpenAI"
```

### 2. Create the Codex Agent Key

```bash
nyxid api-key create \
  --name "codex-work" \
  --platform codex \
  --scopes "proxy"
```

Save the printed `nyx_…` value.

### 3. Restrict to the OpenAI service

Same procedure as Step 3 of the Quickstart, but pick `OpenAI` in the `Service Scope` card (or pass the OpenAI service UUID to `nyxid api-key update`).

### 4. Wire Codex

Codex uses the OpenAI API natively. Point it at NyxID's OpenAI-compatible gateway:

```bash
# Self-host
export OPENAI_BASE_URL="http://localhost:3001/api/v1/llm/gateway/v1"
export OPENAI_API_KEY="nyx_…"   # codex-work

# Hosted
export OPENAI_BASE_URL="https://<your-nyxid-host>/api/v1/llm/gateway/v1"
export OPENAI_API_KEY="nyx_…"
```

The OpenAI-compatible gateway routes by model name (`gpt-*` → OpenAI, `claude-*` → Anthropic, `gemini-*` → Google AI). One `OPENAI_BASE_URL` therefore covers multiple providers, but the corresponding service must exist in NyxID for each provider you want to call — see [docs/MCP_DELEGATION_FLOW.md#openai-compatible-gateway](../MCP_DELEGATION_FLOW.md#openai-compatible-gateway).

### 5. Verify

```bash
curl -i -X POST "$OPENAI_BASE_URL/chat/completions" \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"ping"}]}'
```

Successful response includes:

- `HTTP/1.1 200 OK`
- `X-NyxID-Agent-Id: <codex-work-key-uuid>` — different from the Claude Code UUID
- An OpenAI chat completions JSON body

Both keys now show up independently in the NyxID audit log under `Admin` → `Audit Log`, filterable by API key.

## Common mistakes

- **Reusing the same Agent Key for every tool.** Defeats per-agent attribution. Create one key per agent.
- **Forgetting to disable `Allow all services`.** Selecting services in the `Service Scope` card without unchecking `Allow all services` stores the list but does not scope the key. Confirm the card shows the service list, not "All services".
- **Routing Claude traffic without registering Anthropic.** If Claude Code sends `claude-*` requests through NyxID but no Anthropic service is registered, the request fails. Pattern 1 in this guide assumes one service per provider.
- **Committing `.envrc`.** Add it to `.gitignore` *before* writing Agent Keys into it.
- **Loading the wrong shell environment.** If `echo $ANTHROPIC_API_KEY` shows the wrong key, run `direnv reload` or restart the shell. The audit log will show whichever key was in scope at request time.
- **Mixing personal and work keys in one terminal.** Use project-scoped `.envrc` so each project automatically loads the right key when you `cd` in.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `403` from `/api/v1/proxy/...` or `/api/v1/llm/...` | Agent Key is missing the `proxy` scope | Open the key under `AI Services` → `Agent Keys` → `[key]`, edit `Scopes` in the `Key Details` card and add `proxy` |
| `403 forbidden` after scoping | Key's `allowed_service_ids` does not include the service the agent is calling | Add the service to the key's scope, or re-enable `Allow all services` if scoping is not required |
| Allowed-services list is stored but ignored | `allow_all_services` was left at the default `true` | On `nyxid api-key update`, pass both `--allowed-services <ids>` and `--allow-all-services false`. The web console handles both in one save |
| `X-NyxID-Agent-Id` header missing on the response | The request authenticated via session token (browser flow), not an Agent Key | Use `Authorization: Bearer nyx_…` (or `x-api-key: nyx_…` for Anthropic-native paths), not session-derived auth |
| Claude Code fails but Codex works | Anthropic service not registered, or the Agent Key's scope excludes it | Register an Anthropic service ([Step 1 of the Quickstart](#1-register-anthropic-in-nyxid)) and confirm the Agent Key's `Service Scope` includes it |
| Codex uses the wrong account | The terminal loaded the wrong `OPENAI_API_KEY` | Run `echo $OPENAI_API_KEY` and confirm the key matches the expected agent in the NyxID audit log |
| Requests show under the wrong Agent Key | The project loaded the wrong `.envrc` or the shell environment is stale | Run `direnv reload` (or restart the shell) and verify the exported key |
| Audit log doesn't separate the two agents | `--platform` was not set when the keys were created | The CLI's `update` command does not accept `--platform`. Either delete and re-create the key with `--platform claude-code` (or `codex`), or call `PUT /api/v1/api-keys/{id}` with `platform` in the body |
| Hosted deployment doesn't work with localhost URL | Base URL still points to `localhost:3001` | Replace it with `https://<your-nyxid-host>` in both `ANTHROPIC_BASE_URL` and `OPENAI_BASE_URL` |

## Production pattern

For production workflows, create one Agent Key per tool per environment and scope each one to only the services that tool needs. This keeps the audit log readable and limits the blast radius of any single key.

| Environment | Tool | Agent Key | Allowed services |
|---|---|---|---|
| Personal laptop | Claude Code | `claude-coding-personal` | `llm-anthropic-personal` |
| Work laptop | Claude Code | `claude-coding-work` | `llm-anthropic-work` |
| Work laptop | Codex | `codex-work` | `llm-openai-work` |
| Work laptop | Cursor | `cursor-work` | `llm-openai-work` |
| CI runner | Codex automation | `codex-ci` | `llm-openai-ci` |

To register the same provider twice with different credentials (e.g. personal vs work Anthropic accounts), pass `--slug <unique-name>` on the second `nyxid service add`:

```bash
ANTHROPIC_PERSONAL="$(cat ~/.anthropic_personal)" \
  nyxid service add llm-anthropic \
  --slug llm-anthropic-personal \
  --label "Anthropic Personal" \
  --credential-env ANTHROPIC_PERSONAL

ANTHROPIC_WORK="$(cat ~/.anthropic_work)" \
  nyxid service add llm-anthropic \
  --slug llm-anthropic-work \
  --label "Anthropic Work" \
  --credential-env ANTHROPIC_WORK
```

Each Agent Key is then scoped to whichever slug applies.

## Operational notes

- **Per-agent rate limits.** Configure them on the `Rate Limits` card of each key's detail page (or via `PUT /api/v1/api-keys/{id}` with `rate_limit_per_second` / `rate_limit_burst` in the body). Useful for throttling experimental agents, isolating CI traffic, or capping personal usage.
- **Same Agent Key, multiple clients.** Any HTTP client can use the Agent Key as `X-API-Key` or `Authorization: Bearer` (or `x-api-key` for Anthropic-native paths). Treat each Agent Key as scoped to one workflow context, not "the Codex key" or "the curl key".
- **Rotating one upstream credential.** Run `nyxid external-key rotate <id>` (or replace via the web console). Every Agent Key bound to that credential picks up the new value on the next request — no agent restart required.

## Reference

- **Per-agent data model and edge cases**: [docs/AGENT_ISOLATION.md](../AGENT_ISOLATION.md)
- **One credential, four APIs in n8n**: [n8n quickstart](n8n.md)
- **Reach localhost APIs from a cloud-hosted agent**: [Node Proxy quickstart](node-proxy.md)
- **Wrap any REST API as MCP tools**: [MCP wrapping quickstart](mcp-wrapping.md)
