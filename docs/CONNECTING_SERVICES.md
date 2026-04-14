# Connecting AI Services to NyxID

How to connect a downstream API (OpenAI, GitHub, Anthropic, your private API, anything) to NyxID so your AI agents can call it through the proxy without ever seeing the raw credential.

This guide works for both deployment modes — **hosted** (`https://nyx.chrono-ai.fun`) and **self-host** (`http://localhost:3001`). It also works for your **first service** and your **tenth**.

> **If your MCP client only shows `nyx__...` tools and nothing else, you have not connected a real AI Service yet.** That's exactly what this guide fixes. Skip to [Step 3](#step-3--connect-your-first-service).

---

## Pick your base URL

Substitute `<BASE_URL>` everywhere in this guide with whichever applies to you:

| Deployment | `<BASE_URL>` |
|---|---|
| Hosted (closed beta) | `https://nyx.chrono-ai.fun` |
| Self-host (default) | `http://localhost:3001` |

If you're on hosted and don't have an account yet, sign up at [nyx.chrono-ai.fun](https://nyx.chrono-ai.fun) (currently invite-only — [join the waitlist](https://nyx.chrono-ai.fun/#waitlist)). If you're self-hosting and don't have NyxID running yet, see [docs/QUICKSTART.md](QUICKSTART.md) first.

---

## The 5-second mental model

NyxID stores your credentials encrypted, then proxies your AI agent's requests to the real downstream API and injects the credential server-side. Connecting a service is always:

1. Pick a service from the catalog (or define a custom one)
2. Provide its credential
3. **Verify the proxy actually works** before you wire MCP
4. Then your AI agent can use it

Step 3 is the gate everything hinges on. Skip it and you'll spend 20 minutes wondering why MCP only shows `nyx__...` tools.

---

## Step 1 — Get authenticated (one-time, ~30 seconds)

Before anything else you need a NyxID auth credential. Two ways:

**CLI (recommended)** — installs the `nyxid` CLI if you don't have it, then opens your browser to log in:

```bash
nyxid login --base-url <BASE_URL>
```

Don't have the CLI yet? One-line install:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/tools/install.sh)"
source ~/.cargo/env
```

**Or** — if you'd rather not install anything: open `<BASE_URL>` in your browser, sign in, go to **Settings → API Keys → Create**, and copy the key. You'll paste it into your AI agent's MCP config in Step 2.

> Already logged in from a previous session? Skip to Step 2.

---

## Step 2 — Wire your AI agent to NyxID's MCP endpoint

So your agent can see NyxID at all. Pick whichever AI tool you use:

```bash
# Claude Code
claude mcp add --transport http nyxid <BASE_URL>/mcp

# Codex
codex mcp add nyxid --url <BASE_URL>/mcp

# Cursor — open <BASE_URL> in browser, go to Settings → MCP, click "Install to Cursor"
```

The first run uses your credentials from Step 1. After this, your AI agent will see NyxID's `nyx__discover_services`, `nyx__connect_service`, `nyx__search_tools`, and `nyx__call_tool` meta-tools — but **not yet** any real downstream tools, because you haven't connected a real service yet. That's Step 3.

> Already wired up from a previous session? Skip to Step 3.

---

## Step 3 — Connect your first service

This is the headline. Four paths, in order of how friction-free they are. Pick whichever you like — they all do the same thing and any of them satisfies issue #298's "verify before MCP" gate.

### Path A — AI-driven (recommended)

Paste this prompt into your AI agent (now MCP-connected from Step 2):

> Help me connect an AI Service in NyxID. Use `nyx__discover_services` to list what's available in the catalog and ask me which one I want (e.g. OpenAI, Anthropic, GitHub). Once I pick, ask me for the credential I want to use (API key, token, etc.), then call `nyx__connect_service` with the `service_id` from discover results and my credential. After it returns success, call `nyx__search_tools` to confirm the new service's tools are now exposed, then call `nyx__call_tool` on one of them (e.g. list models, list repos) to verify the proxy works end-to-end. Report back with the actual response so I know it's working — not just "looks good." If anything errors, tell me whether it's a credential problem or a service config problem.

That's it. The agent walks you through everything: discover → ask → connect → search → call. The final `nyx__call_tool` is your verify-gate — if it returns a real downstream response (a list of OpenAI models, a list of GitHub repos, etc.), the chain is working end-to-end.

If the agent only manages to call `nyx__discover_services` and stops there, it doesn't have a tool problem — it has an instruction problem. Re-paste the prompt and tell it explicitly to keep going through all five steps.

### Path B — CLI

If you'd rather drive it yourself, three commands:

```bash
# 1. Connect a service from the catalog (e.g. OpenAI). Set OPENAI_API_KEY in your shell first.
nyxid service add llm-openai --credential-env OPENAI_API_KEY

# 2. Verify the proxy works end-to-end. You should see a real JSON list of models.
nyxid proxy request llm-openai models

# 3. (Optional) See what the catalog has if you want a different service.
nyxid catalog list
```

If `proxy request` returns a real response, your service is connected and the credential is good. Done.

### Path C — Web UI

If you'd rather click through:

1. Open `<BASE_URL>:3000` in your browser (or just `<BASE_URL>` for hosted) and sign in.
2. Click **AI Services** in the sidebar → **Add Service**.
3. Pick a service from the catalog (OpenAI, Anthropic, GitHub, etc.).
4. Paste the credential it asks for.
5. On the new service's detail page, click **Test request** (or use the "Try it" panel) to verify the proxy works. You should see a real downstream response, not an error.

### Path D — Direct API (for automation)

For scripting, CI/CD, or integrating with a config-management tool, hit the REST endpoints directly:

```bash
# Replace <TOKEN> with your bearer token (from `nyxid login`) or use x-api-key.

# Connect a service from the catalog
curl -X POST <BASE_URL>/api/v1/keys \
  -H "Authorization: Bearer <TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{
    "catalog_slug": "llm-openai",
    "credential": "sk-...",
    "label": "production-openai"
  }'

# Verify the proxy works — should return a real OpenAI models response
curl -X GET <BASE_URL>/api/v1/proxy/s/llm-openai/models \
  -H "Authorization: Bearer <TOKEN>"
```

Same as the CLI path under the hood — these are the exact endpoints `nyxid service add` and `nyxid proxy request` call. Use this when you don't want a CLI dependency in your automation environment.

---

## Did it work?

After connecting a service via any of the four paths above, reconnect your AI agent to MCP (some clients pick up new tools automatically; others need a restart). You should now see real downstream tools — `chat_completions`, `list_models`, `get_repo`, etc. — **alongside** the `nyx__...` meta-tools.

If you only see `nyx__...` tools after reconnecting, the service didn't actually get connected. Common causes:

- The credential was wrong (re-run with the correct value)
- The catalog slug doesn't match (run `nyxid catalog list` to find the exact slug)
- You connected the service to a different account than the one your MCP client is authenticated as
- Your MCP client cached the old tool list — restart it

Use `nyx__search_tools` from your AI agent (or `nyxid service list` from the CLI) to confirm what tools NyxID *thinks* it has exposed for you. If `nyx__search_tools` returns nothing, the service isn't connected on the NyxID side — the bug is upstream of MCP.

---

## Adding more services later

Same flow, skip the steps you've already done:

- **Already authenticated and MCP-wired?** Jump straight to [Step 3](#step-3--connect-your-first-service) and pick your favorite path. The AI prompt in Path A handles the Nth service the same way it handles the first.
- **CLI users:** `nyxid service add <slug> --credential-env <ENV_VAR>` and you're done. `nyxid catalog list` to browse what's available.
- **Web UI users:** **AI Services → Add Service** any time.
- **Bulk setup:** the API path scales — loop `POST /api/v1/keys` over your credentials with a small script.

You can also rotate credentials on existing services from the same surfaces — `nyxid service rotate <slug>`, **AI Services → \[service\] → Rotate Credential**, or `PUT /api/v1/keys/<id>`.

---

## Connecting custom (non-catalog) services

Got a private API NyxID's catalog doesn't know about? You can still connect it:

```bash
nyxid service add --custom \
  --slug my-internal-api \
  --endpoint https://internal.example.com \
  --credential-env MY_API_KEY \
  --auth-method bearer
```

For services behind a firewall (localhost, internal-only), see [docs/NODE_PROXY.md](NODE_PROXY.md) for the credential node setup that punches through NAT.

---

## Related docs

- [docs/QUICKSTART.md](QUICKSTART.md) — self-host setup (Docker, account creation)
- [docs/MCP_DELEGATION_FLOW.md](MCP_DELEGATION_FLOW.md) — how MCP auth + delegation work under the hood
- [docs/AI_AGENT_PLAYBOOK.md](AI_AGENT_PLAYBOOK.md) — patterns for using NyxID from agent code
- [docs/NODE_PROXY.md](NODE_PROXY.md) — connecting localhost / private-network services via credential nodes
- [docs/API.md](API.md) — full REST endpoint reference
