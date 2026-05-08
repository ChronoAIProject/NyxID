# Per-Agent Keys for Claude Code and Codex

Configure two coding agents on one machine — for example, a personal Claude Code session and a work Codex session — so each agent has its own NyxID Agent Key, its own scope, and (optionally) routes to a distinct upstream credential. The result is per-agent attribution in the audit log, blast-radius isolation if a key leaks, and the ability to send each agent's traffic to a different downstream account.

```
Claude Code (personal) ── nyx_… ──┐
                                  │
                                  ├── NyxID ──┬── OpenAI Personal account
                                  │           │     (used by claude-coding-personal)
Codex (work)           ── nyx_… ──┘           │
                                              └── OpenAI Premium account
                                                  (used by codex-work)
```

## Prerequisites

- A NyxID account and a logged-in `nyxid` CLI on your laptop. Follow [Step 0 of the n8n quickstart](n8n.md#step-0--get-nyxid-running-and-create-an-agent-key) if not already done.
- Two OpenAI API keys you want to keep separate (e.g. a personal account and a team account). Save each in a local file (`~/.openai_personal`, `~/.openai_premium`) with `chmod 600`.

## Procedure

### 1. Register OpenAI twice — one entry per credential tier

Each NyxID service stores a single default credential. To route different agents to different OpenAI accounts, register OpenAI twice with distinct slugs:

```bash
OPENAI_PERSONAL="$(cat ~/.openai_personal)" \
  nyxid service add llm-openai \
  --slug llm-openai-personal \
  --label "OpenAI Personal" \
  --credential-env OPENAI_PERSONAL

OPENAI_PREMIUM="$(cat ~/.openai_premium)" \
  nyxid service add llm-openai \
  --slug llm-openai-premium \
  --label "OpenAI Premium" \
  --credential-env OPENAI_PREMIUM
```

This creates two `UserService` rows — both pointing at OpenAI's API, each with its own credential.

### 2. Create one Agent Key per agent

```bash
nyxid api-key create --name "claude-coding-personal" --platform claude-code --scopes "proxy"
nyxid api-key create --name "codex-work"             --platform codex       --scopes "proxy"
```

Save the `nyx_…` value each command prints — shown once. The `--platform` tag is recorded with every proxied request so the audit log distinguishes the agents.

> By default both keys allow access to all of your services. Step 3 restricts each key to a single service.

### 3. Restrict each key to one service

`nyxid api-key create` does not accept a scope-down flag. Apply scoping after creation, either through the web console (recommended) or via `nyxid api-key update`.

**Web console:**

1. Open **AI Services → Agent Keys → `claude-coding-personal`**.
2. In the **Service Scope** card, uncheck **Allow all services**.
3. Select **OpenAI Personal**. Save.
4. Repeat for `codex-work`, selecting **OpenAI Premium**.

**CLI (UUID-based):** the `update` endpoint addresses both services and keys by UUID, so look up both first.

```bash
# 1. Service IDs
nyxid service list
PERSONAL_SVC=11111111-aaaa-…   # llm-openai-personal
PREMIUM_SVC=22222222-bbbb-…    # llm-openai-premium

# 2. Agent Key IDs
nyxid api-key list
PERSONAL_KEY=44444444-eeee-…   # claude-coding-personal
WORK_KEY=55555555-ffff-…       # codex-work

# 3. Apply scoping. Both flags are required.
nyxid api-key update "$PERSONAL_KEY" --allowed-services "$PERSONAL_SVC" --allow-all-services false
nyxid api-key update "$WORK_KEY"     --allowed-services "$PREMIUM_SVC"  --allow-all-services false
```

After scoping, each key can only call its bound service. Per-key rate limits and burst caps are configured on the same key detail page (**Rate Limits** card) or via `PUT /api/v1/api-keys/{id}` with `rate_limit_per_second` and `rate_limit_burst` in the body.

### 4. Wire each agent to its key

**Claude Code** uses the Anthropic API; route it through NyxID's Anthropic provider proxy:

```bash
# In the personal project directory, before launching `claude`:
export ANTHROPIC_BASE_URL="http://localhost:3001/api/v1/llm/anthropic"
export ANTHROPIC_API_KEY="nyx_…"   # claude-coding-personal
```

For OpenAI-compatible tools invoked from inside Claude Code, also export:

```bash
export OPENAI_BASE_URL="http://localhost:3001/api/v1/llm/gateway/v1"
export OPENAI_API_KEY="nyx_…"      # claude-coding-personal (same key)
```

**Codex** (OpenAI's CLI, OpenAI API natively):

```bash
# In the work project directory, before launching `codex`:
export OPENAI_BASE_URL="http://localhost:3001/api/v1/llm/gateway/v1"
export OPENAI_API_KEY="nyx_…"      # codex-work
```

Replace `localhost:3001` with your NyxID host for hosted deployments.

The OpenAI-compatible gateway routes by model name (`gpt-*` → OpenAI, `claude-*` → Anthropic, `gemini-*` → Google AI). One `OPENAI_BASE_URL` therefore covers multiple providers; see [docs/MCP_DELEGATION_FLOW.md#openai-compatible-gateway](../MCP_DELEGATION_FLOW.md#openai-compatible-gateway).

For project-scoped environment variables, use `direnv` and put the `export` lines in each project's `.envrc`. Add `.envrc` to `.gitignore` so the keys are not committed.

## Verification

Send a request from each terminal:

```bash
curl -i -X POST "$OPENAI_BASE_URL/chat/completions" \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"ping"}]}'
```

A successful response includes:

- `HTTP/1.1 200 OK`
- `X-NyxID-Agent-Id: <key-uuid>` — the Agent Key that authenticated the request
- An OpenAI chat completions JSON body

Open **AI Services → Agent Keys → \[your key\] → Usage** in the web console: each key shows its own request log, scoped to its allowed services. The admin audit log under **Admin → Audit Log** offers the same data with filtering by API key.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `403` from `/api/v1/proxy/...` or `/api/v1/llm/...` | Agent Key is missing the `proxy` scope | Edit the key in **Agent Keys → \[key\] → Scopes** and add `proxy` |
| `403 forbidden` after scoping | Key's `allowed_service_ids` does not include the service the agent is calling | Add the service to the key's scope, or set **Allow all services** if scoping is not required |
| Allowed-services list is stored but ignored (key still hits any service) | `allow_all_services` was left at the default `true` | On `nyxid api-key update`, pass both `--allowed-services <ids>` and `--allow-all-services false`. The web console handles this in one save |
| `X-NyxID-Agent-Id` header missing on the response | The request authenticated via session token (browser flow), not an Agent Key | Use `Authorization: Bearer nyx_…` or `X-API-Key: nyx_…`, not session-derived auth |
| Audit log doesn't separate the two agents | `--platform` was not set on the keys | `nyxid api-key update <key-id> --platform claude-code` (or `--platform codex`); supported labels: `claude-code`, `codex`, `cursor`, `openclaw`, `generic` |

## Operational notes

- **Two services vs. credential overrides.** This guide uses two `UserService` rows (one slug per credential) because the CLI creates them in one step. To bind two agents to the **same** slug with **different** credentials, use `agent_service_bindings` instead — the override mechanism documented in [docs/AGENT_ISOLATION.md](../AGENT_ISOLATION.md). External credentials are managed in **AI Services → External Services**; per-agent bindings live under **AI Services → Agent Keys → \[key\] → Bindings**.
- **Rotating one credential.** Run `nyxid external-key rotate <id>` (or replace via the web console) — every agent bound to that credential picks up the new value on the next request, no agent restart required.

## Reference

- **Per-agent data model and edge cases**: [docs/AGENT_ISOLATION.md](../AGENT_ISOLATION.md)
- **One credential, four APIs in n8n**: [n8n quickstart](n8n.md)
- **Reach localhost APIs from a cloud-hosted agent**: [Node Proxy quickstart](node-proxy.md)
- **Wrap any REST API as MCP tools**: [MCP wrapping quickstart](mcp-wrapping.md)
