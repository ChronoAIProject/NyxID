# Claude Code & Codex: per-agent keys, per-agent credentials

**TL;DR** — One scoped NyxID key per agent. One credential override per agent per service. The result: blast-radius-bound keys, per-agent usage attribution in the audit log, and the freedom to run my "personal" Claude Code session against a cheap OpenAI account while my "work" Codex session uses the team's premium one — same NyxID, same machine, no env-var dance.

```
Claude Code (personal)  ── nyx_personal_... ──┐
                                              │
                                              ├── NyxID ──┬── OpenAI cheap ($50/mo)
                                              │           │     (used by personal)
Codex (work)            ── nyx_work_...     ──┘           │
                                                          └── OpenAI premium ($500/mo)
                                                                (used by work)
```

The mechanism is **agent-service bindings**: a `(api_key, service) → credential_override` row that says "when *this* agent calls *this* service, inject *this* credential instead of the default." See [docs/AGENT_ISOLATION.md](../AGENT_ISOLATION.md) for the full data model.

> This walkthrough assumes you've already done [Step 0 of the n8n quickstart](n8n.md#step-0--get-nyxid-running-and-create-an-agent-key) — i.e., you have NyxID running and a logged-in `nyxid` CLI. If not, see [docs/SETUP.md](../SETUP.md) first.

---

## The problem

I run multiple coding agents. They share my NyxID, and historically they shared a single OpenAI key. That created three problems:

1. **No usage attribution.** The OpenAI bill said `$420 this month`. Was that Claude Code refactoring my Rust monorepo, or Codex grinding through some side project? No idea.
2. **No blast-radius bound.** A leaked key meant *all* my agents and *all* my services were exposed.
3. **Couldn't route different agents to different downstream accounts.** I wanted my personal projects on the cheap OpenAI tier and my work projects on the team's premium account. With one key, you get one tier.

---

## Setup

### 1. Add OpenAI to NyxID twice — one per credential tier

Each service entry stores one default credential. To route different agents to different OpenAI accounts, register OpenAI twice with custom slugs:

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

Two `UserService` rows, both backed by the same upstream (OpenAI), each with its own credential.

### 2. Create one Agent Key per agent

```bash
nyxid api-key create --name "claude-coding-personal" --platform claude-code --scopes "proxy"
nyxid api-key create --name "codex-work"             --platform codex       --scopes "proxy"
```

Save the `nyx_...` value each call prints — shown once. The `--platform` tag flows through to the audit log so every request is attributable to the right agent (Codex vs Claude Code).

### 3. Restrict each key to its service (recommended)

By default a fresh Agent Key can call any of your services. To bound the blast radius — so a leaked `claude-coding-personal` key can't drain your premium OpenAI credit — scope each key to exactly the service it should reach.

`nyxid api-key create` doesn't take a scope-down flag (it always creates an unscoped key); you scope it after the fact. The fastest path is the web console:

1. **AI Services → Agent Keys → `claude-coding-personal` → Service Scope**
2. Uncheck **Allow all services**.
3. Pick **OpenAI Personal** from the list. Save.
4. Repeat for `codex-work`, picking **OpenAI Premium**.

If you'd rather stay in the terminal, the equivalent CLI dance needs both the service IDs and the API key IDs (the update endpoint addresses keys by UUID, not name):

```bash
# 1. Grab the two service IDs.
nyxid service list
# Copy the IDs of llm-openai-personal and llm-openai-premium.
PERSONAL_SVC=11111111-aaaa-...
PREMIUM_SVC=22222222-bbbb-...

# 2. Grab the two API key IDs.
nyxid api-key list
# Copy the IDs of claude-coding-personal and codex-work.
PERSONAL_KEY=44444444-eeee-...
WORK_KEY=55555555-ffff-...

# 3. Apply the scoping. Both flags are required — see the gotcha below.
nyxid api-key update "$PERSONAL_KEY" --allowed-services "$PERSONAL_SVC" --allow-all-services false
nyxid api-key update "$WORK_KEY"     --allowed-services "$PREMIUM_SVC"  --allow-all-services false
```

What you got:

| | `claude-coding-personal` | `codex-work` |
|---|---|---|
| Scope | `proxy` only — no `write`, no `admin` | `proxy` only |
| Allowed services | `llm-openai-personal` only | `llm-openai-premium` only |
| Platform tag | `claude-code` (for the audit log) | `codex` |

Per-key rate limits and burst caps are set in the same key detail page (**Rate Limits** card), or via `PUT /api/v1/api-keys/{id}` with `rate_limit_per_second` / `rate_limit_burst` in the body.

### 4. Wire each agent to its own key

**Claude Code** (uses the Anthropic API; route through NyxID's Anthropic provider proxy):

```bash
# In your personal project's terminal, before launching `claude`:
export ANTHROPIC_BASE_URL="http://localhost:3001/api/v1/llm/anthropic"
export ANTHROPIC_API_KEY="nyx_..."   # the claude-coding-personal key
```

For OpenAI-compatible tools used inside Claude Code (e.g., a sub-agent that calls OpenAI), set:

```bash
export OPENAI_BASE_URL="http://localhost:3001/api/v1/llm/gateway/v1"
export OPENAI_API_KEY="nyx_..."      # same claude-coding-personal key
```

**Codex** (OpenAI's CLI, uses OpenAI API natively):

```bash
# In your work project's terminal, before launching `codex`:
export OPENAI_BASE_URL="http://localhost:3001/api/v1/llm/gateway/v1"
export OPENAI_API_KEY="nyx_..."      # the codex-work key
```

Codex never sees the real `sk-...` premium key. NyxID does the swap on every request.

> Set the request path on the model name. The OpenAI-compatible gateway routes by model: `gpt-*` → OpenAI Personal/Premium (depending on which Agent Key you used), `claude-*` → Anthropic, `gemini-*` → Google AI. So a single `OPENAI_BASE_URL` works across providers — see [docs/MCP_DELEGATION_FLOW.md#openai-compatible-gateway](../MCP_DELEGATION_FLOW.md#openai-compatible-gateway).

> **Project-level keys via `.envrc` / direnv:** put the `export` lines in each project's `.envrc` so the right key is active automatically when you `cd` in. Don't commit them — `.gitignore` the file.

---

## Verify per-agent attribution

Make a request from each terminal, then open the web console: **AI Services → Agent Keys → \[your key\] → Usage**. Each key has its own request log and can be filtered to a single service. Same view in the admin audit log: **Admin → Audit Log**, filter by API key.

Every proxy and LLM-gateway response also returns an `X-NyxID-Agent-Id` header carrying the API key id. If you instrument spans with it, your observability stack gets per-agent breakdowns for free.

```
$ curl -i -X POST "$OPENAI_BASE_URL/chat/completions" \
    -H "Authorization: Bearer $OPENAI_API_KEY" \
    -H "Content-Type: application/json" \
    -d '{"model":"gpt-4o","messages":[{"role":"user","content":"ping"}]}'
HTTP/1.1 200 OK
X-NyxID-Agent-Id: 1f4a8c8e-...
...
```

Same service, two different agents, two different injected credentials, two distinct attribution rows. The bill stops being a mystery.

---

## Before vs After

| | Before | After |
|---|---|---|
| OpenAI keys held by agents | 1 (raw `sk-...` shared by every CLI) | 0 — agents only see `nyx_...` |
| Per-agent attribution | None — one bill, no breakdown | Audit log + `X-NyxID-Agent-Id` header |
| Per-agent rate limits | One global bucket | Per-key token bucket (configurable in the web console) |
| Different credentials per agent | Impossible without env-var juggling | Different service slug per credential, scoped to the right Agent Key |
| Leaked key blast radius | Full OpenAI account, all services | One service slug, proxy-scope only |
| Rotating one credential | Update every CLI/script | `nyxid external-key rotate <id>` — done |

---

## Gotchas

**The `proxy` scope is required for `/api/v1/proxy/...` and `/api/v1/llm/...`** Without it the proxy returns 403. Don't add `write` or `admin` unless your agent needs to manage NyxID resources too — it almost never does.

**Both `--allowed-services` and `--allow-all-services false` are required to scope a key.** If you set the allowed-list but leave `allow_all_services` at its default (`true`), the list is stored but ignored — the key still hits everything. Always pass both, on `nyxid api-key update` or in the web console.

**The agent's `--platform` tag is cosmetic but useful.** It shows up in the audit log and the API key list, and downstream observability can split by it. Use the convention `claude-code | codex | cursor | openclaw | generic`.

**`X-NyxID-Agent-Id` is only set on API-key auth.** Session-token auth (browser flows) doesn't populate it. The whole per-agent attribution story depends on agents using API keys, not user sessions.

**Two services, one credential pool — vs. credential overrides.** This guide uses two `UserService` rows (one per slug) because the CLI lets you create them in one step. If you want two agents hitting the *same* slug with *different* credentials, the underlying mechanism is `agent_service_bindings` (one slug, one default credential, plus an override row per agent). Manage external credentials in **AI Services → External Services** and bindings in **AI Services → Agent Keys → \[key\] → Bindings** in the web console. Full data model: [docs/AGENT_ISOLATION.md](../AGENT_ISOLATION.md).

---

## Next

- **One credential, four APIs in n8n:** [n8n quickstart](n8n.md)
- **Reach localhost APIs from a cloud-hosted agent:** [Node Proxy quickstart](node-proxy.md)
- **Wrap any REST API as MCP tools your agent can use:** [MCP wrapping quickstart](mcp-wrapping.md)
- **Full data model and edge cases:** [docs/AGENT_ISOLATION.md](../AGENT_ISOLATION.md)
