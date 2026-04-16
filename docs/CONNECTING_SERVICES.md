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

NyxID stores your credentials encrypted, then proxies your AI agent's requests to the real downstream API and injects the credential server-side. Connecting a service has three substeps that need to happen at some point:

1. Pick a service from the catalog (or define a custom one)
2. Provide its credential
3. **Verify the proxy actually works** — call a real downstream tool, get a real response back

The trap from issue [#298](https://github.com/ChronoAIProject/NyxID/issues/298) is wiring MCP **without** doing the three substeps. Your AI agent then sees only NyxID's `nyx__...` meta-tools and there's nothing real to call.

Below you have four paths that complete the substeps. The **AI-driven path** uses an MCP-connected agent to do them itself, so MCP must be wired first. The **manual paths** (CLI, web UI, curl) don't depend on MCP at all — you can wire MCP whenever, or never. Either way, all that matters is that the three substeps actually happen.

---

## Step 1 — Get authenticated

You need NyxID auth before any of the steps below will work. Pick the route that matches what you'll use next.

**Using Claude Code, Codex, or Cursor (most users):** skip ahead to Step 2. The `claude mcp add` / `codex mcp add` commands open a browser the first time and authenticate you interactively via OAuth — there's no separate login step.

**Using the `nyxid` CLI:** install it if you don't have it, then run `nyxid login`:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/tools/install.sh)"
source ~/.cargo/env
nyxid login --base-url <BASE_URL>
```

The login command opens your browser and stores a session locally. The CLI and any subsequent `claude mcp add` / `codex mcp add` will reuse it.

**Using a headless HTTP client (curl, n8n, Zapier, custom code, CI/CD):** open the web console (`https://nyx.chrono-ai.fun` for hosted, `http://localhost:3000` for self-host), sign in, go to **AI Services → API Keys → Create**, and copy the raw key. You'll pass it as an `x-api-key` header on every request. There's no `claude mcp add` for this route — your tool talks to NyxID directly.

---

## Step 2 — Wire your AI agent to NyxID's MCP endpoint

So your AI agent can see NyxID at all. Pick whichever AI tool you use:

```bash
# Claude Code
claude mcp add --transport http nyxid <BASE_URL>/mcp

# Codex
codex mcp add nyxid --url <BASE_URL>/mcp

# Cursor — open the web console (https://nyx.chrono-ai.fun for hosted, http://localhost:3000
# for self-host), go to Settings → MCP, click "Install to Cursor"
```

The first run of `claude mcp add` / `codex mcp add` opens a browser to authenticate you (OAuth) and stores a session. If you already ran `nyxid login` from Step 1, the session is reused and there's no second prompt.

After this, your AI agent will see NyxID's `nyx__discover_services`, `nyx__connect_service`, `nyx__search_tools`, and `nyx__call_tool` meta-tools — but **not yet** any real downstream tools, because you haven't connected a real service yet. That's Step 3. **Don't stop here**, or you'll hit issue [#298](https://github.com/ChronoAIProject/NyxID/issues/298).

> **Headless HTTP client?** There is no `mcp add` for n8n / Zapier / curl. Skip directly to Step 3 Path D and pass `x-api-key: <YOUR_KEY>` (from Step 1) on every request to `<BASE_URL>/api/v1/...` and `<BASE_URL>/mcp`.

---

## Step 3 — Connect your first service

This is the headline. Four paths, in order of how friction-free they are. Pick whichever you like — they all complete the three substeps from the mental model above and avoid the issue #298 trap (where MCP is wired but no real service is connected, so the agent only sees `nyx__...` meta-tools).

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

1. Open the **web console** in your browser and sign in. The console URL is **not the same as `<BASE_URL>`** — it's the dashboard, a separate port from the API:
   - **Hosted:** `https://nyx.chrono-ai.fun`
   - **Self-host:** `http://localhost:3000` (port 3000, while the API runs on 3001)
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

You can also rotate credentials on existing services from the same surfaces — `nyxid service rotate-credential <id> --credential-env <NEW_ENV_VAR>` (use `nyxid service list` to find the service ID), **AI Services → \[service\] → Rotate Credential**, or `PUT /api/v1/keys/<id>`.

---

## Connecting custom (non-catalog) services

Got a private API NyxID's catalog doesn't know about? You can still connect it. The slug is positional and the URL flag is `--endpoint-url`:

```bash
nyxid service add my-internal-api \
  --custom \
  --endpoint-url https://internal.example.com \
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
