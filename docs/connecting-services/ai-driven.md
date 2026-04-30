# AI-Driven — Let Your Agent Connect a Service for You

Use Claude Code, Codex, or Cursor to call NyxID's MCP meta-tools (`nyx__discover_services`, `nyx__connect_service`, `nyx__call_tool`) and connect a service end-to-end.

This path involves more moving parts than the [Web UI](web-ui.md) or [CLI](cli.md) — MCP setup, agent prompt interpretation, model behavior. If this is your **first** service ever, prefer the Web UI. If you already have one working service, this path is faster than clicking through the dashboard for service two and beyond.

## Step 1 — Wire MCP

Pick your client. `<BASE_URL>` is `https://nyx.chrono-ai.fun` for hosted, `http://localhost:3001` for self-host.

### Claude Code

```bash
claude mcp add --transport http nyxid <BASE_URL>/mcp
```

### Codex

```bash
codex mcp add nyxid --url <BASE_URL>/mcp
```

### Cursor

Open the web console, go to **Settings → MCP**, and click **Install to Cursor**.

The first run opens your browser to authenticate (OAuth) and stores a session.

### What you should see after this step

Your agent now sees NyxID's meta-tools: `nyx__discover_services`, `nyx__connect_service`, `nyx__search_tools`, `nyx__call_tool`. These are NyxID itself — not yet a connected downstream service.

## Step 2 — Paste this prompt into your agent

This is the canonical NyxID connection prompt. Paste it verbatim:

> Help me connect an AI Service in NyxID. Use `nyx__discover_services` to list what's available in the catalog and ask me which one I want (e.g. OpenAI, Anthropic, GitHub). Once I pick, ask me for the credential I want to use, then call `nyx__connect_service` with the `service_id` from discover results and my credential. After it returns success, call `nyx__search_tools` to confirm the new service's tools are now exposed, then call `nyx__call_tool` on one of them (e.g. list models, list repos) to verify the proxy works end-to-end. Report back with the actual response so I know it's working — not just "looks good." If anything errors, tell me whether it's a credential problem or a service config problem.

The agent walks you through everything: discover → ask → connect → search → call. The final `nyx__call_tool` is the verification — if it returns a real downstream response, the chain is working end-to-end.

## If the agent only calls `nyx__discover_services` and stops

It doesn't have a tool problem, it has an instruction problem. Re-paste the prompt and tell it explicitly to keep going through all five steps.

## If your MCP client only shows `nyx__...` tools after this

Real downstream tools (`chat_completions`, `list_models`, `get_repo`) appear **after** a service is connected. If you only see `nyx__...`, the connection step failed silently. Common causes:

- Wrong credential value (re-run with the correct one).
- Catalog slug mismatch (the agent can verify via `nyx__discover_services`).
- You connected to a different account than the one your MCP client is authenticated as.
- Your client cached the old tool list — restart it.

If you're stuck here, fall back to the [Web UI](web-ui.md) path to confirm the service connected at all.

## Adding more services

Re-paste the same prompt. The agent handles service N the same way it handles service 1.

## Next

- **Without MCP / without an agent:** [Web UI](web-ui.md) or [CLI](cli.md).
- **MCP delegation under the hood:** [docs/MCP_DELEGATION_FLOW.md](../MCP_DELEGATION_FLOW.md).
