---
title: Connect your agent
description: Install the nyxid CLI, authenticate, and wire an MCP-capable AI client to your NyxID deployment in under five minutes.
---

This page gets an AI agent connected to NyxID. By the end you will have the `nyxid` CLI installed, your account authenticated, and at least one MCP-capable client (Claude Code, Cursor, or Codex) pointed at your NyxID MCP endpoint.

:::note
If this is your first time ever adding a service, consider completing the Web UI flow first so you have at least one working connection before wiring up MCP. See the web console at `https://nyx.chrono-ai.fun`.
:::

:::warning Two identities, never confuse them

There are two NyxID identities involved here, and only the human can move between them:

- **You** — the human running these steps. You authenticate **once** via `nyxid login` (browser or device-code). Your session is what authorizes everything else; the CLI saves it to `~/.nyxid/`.
- **The agent** — a separate, scoped identity (`nyxid_ag_…` API key) that **you mint for the agent** with `nyxid api-key create`. The agent reads it from `NYXID_API_KEY` in its environment and uses it for proxy requests.

**Agents must never obtain a human account session through ordinary `nyxid login`.** Use a pre-issued API key in `NYXID_API_KEY`, or have a human operator enroll a restricted profile with `nyxid login --agent-key` as described below. Both Agent Key enrollment and device-code login require a human to review and approve the request; an autonomous agent cannot approve its own request.
:::

## Install the CLI

The `nyxid` CLI is a Rust binary. Install the Rust toolchain if you don't have it:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Then install the CLI from the repo:

```bash
cargo install --git https://github.com/ChronoAIProject/NyxID nyxid-cli
nyxid --help    # verify
```

:::tip
End users install prebuilt release binaries from the releases page. `cargo install` is the development path; both produce the same binary.
:::

## Authenticate (you, the human)

Log in once. The `--base-url` is saved to `~/.nyxid/base_url`; all subsequent commands pick it up automatically. This is **your** session, not the agent's.

```bash
# Hosted deployment
nyxid login --base-url https://nyx-api.chrono-ai.fun

# Self-hosted
nyxid login --base-url http://localhost:3001
```

For headless environments (SSH, container, WSL with no `$DISPLAY`), `nyxid login` auto-falls back to the **device-code flow**. You can also force it explicitly:

```bash
nyxid login --device --base-url https://nyx-api.chrono-ai.fun
```

The CLI prints a one-time code and a URL; open the URL on any signed-in browser (phone, another machine), paste the code, review the requester IP and time, then approve or reject. Approval completes the CLI login; rejection stops it immediately.

For non-interactive CI use a pre-issued API key (`nyxid api-key create --platform <agent>`) instead of an interactive login — `nyxid login` short-circuits in CI environments with a hint to do exactly that.

Confirm the session is working:

```bash
nyxid status
```

### Authorize an agent profile without transferring a secret

As a human operator, you can run `nyxid login --agent-key --profile my-agent --base-url <BASE_URL>` on the agent machine. Enter the terminal's code in the web UI, then choose an existing eligible key or create a limited key and confirm its permissions. **Approve from your phone** displays a QR code for the NyxID mobile app; approval stays on your phone and leaves no account login in that computer's browser.

The profile stores a new restricted login credential bound to the key, not an account session. Selecting a shared key leaves its existing secret unchanged. `nyxid whoami --profile my-agent` reports Agent Key authentication; `nyxid logout --profile my-agent` revokes that credential. The key detail page can revoke one login credential or the whole key. Scopes, bindings, restrictions, rate limits, and expiry apply live, and rejected credentials never fall back to a human session. An autonomous agent cannot approve its own request; a human must review and confirm it. See [Agent Key login](/docs/cli/getting-started/authenticate#agent-key-login).

## Create an Agent Key (for the agent to use)

This is the second identity — a scoped NyxID API key the agent reads from `NYXID_API_KEY`. You mint it from your authenticated session above. It carries its own rate limit, service scope, and audit attribution so the agent's traffic is fully separated from yours.

```bash
nyxid api-key create --name "my-agent" --platform claude-code --scopes "proxy"
```

The key value (`nyx_...`) is shown once — save it immediately. Store it as an environment variable, not in source control:

```bash
export NYXID_API_KEY="nyx_..."
```

For how scope and credential bindings work, see [Isolate agents with scoped keys](/docs/ai/guides/agent-isolation).

## Wire your MCP client

NyxID's MCP endpoint lives at `<BASE_URL>/mcp`. Pick your client:

### Claude Code

```bash
claude mcp add --transport http --scope user nyxid https://nyx-api.chrono-ai.fun/mcp
```

`--scope user` stores the config at user scope so authentication is not directory-dependent. On the next `claude` launch, a browser tab opens for OAuth login. Authentication ties the MCP session to your NyxID account.

Alternatively, edit `~/.claude/settings.json` directly:

```json
{
  "mcpServers": {
    "nyxid": {
      "type": "http",
      "url": "https://nyx-api.chrono-ai.fun/mcp"
    }
  }
}
```

### Cursor

In the web console at `https://nyx.chrono-ai.fun`, go to **Settings → MCP** and click **Install to Cursor**.

Or create `.cursor/mcp.json` in your project root:

```json
{
  "mcpServers": {
    "nyxid": {
      "url": "https://nyx-api.chrono-ai.fun/mcp"
    }
  }
}
```

Restart Cursor. Authenticate via browser when prompted.

### Codex

```bash
codex mcp add nyxid --url https://nyx-api.chrono-ai.fun/mcp
```

Or edit `~/.codex/config.toml`:

```toml
[mcp_servers.nyxid]
url = "https://nyx-api.chrono-ai.fun/mcp"
```

### Generate the config automatically

The CLI can print the exact config snippet for any client:

```bash
nyxid mcp config --tool claude-code
nyxid mcp config --tool cursor
nyxid mcp config --tool codex
nyxid mcp config --tool generic   # raw MCP URL only
```

## What you see after wiring MCP

After the client authenticates, you should see NyxID's meta-tools:

- `nyx__discover_services` — browse the catalog
- `nyx__list_connected_services` — enumerate services already connected to your account
- `nyx__connect_service` — add a service from within the agent; accepts optional `scopes` for extra OAuth permissions
- `nyx__search_tools` — find tools across connected services
- `nyx__call_tool` — invoke any connected service endpoint

These are NyxID's own tools. Per-operation tools for connected services (for example `create_issue`, `chat_completions`) appear after you add a service.

:::tip
Paste this prompt verbatim into your agent to connect a service end-to-end without leaving the chat:

> Help me connect an AI Service in NyxID. Use `nyx__discover_services` to list what's available in the catalog and ask me which one I want. Once I pick, ask me for the credential, then call `nyx__connect_service`. After it returns success, call `nyx__search_tools` to confirm the new tools are exposed, then call `nyx__call_tool` on one of them to verify the proxy works end-to-end. Report back with the actual response.
:::

## Next steps

- [Your first agent call](/docs/ai/getting-started/first-agent-call) — make a proxied API request
- [Set up Claude Code, Cursor & Codex](/docs/ai/guides/claude-code-cursor-codex) — per-agent key isolation
- [Wrap a REST API as MCP tools](/docs/ai/guides/wrap-rest-api-as-mcp) — expose any OpenAPI spec as typed tools
