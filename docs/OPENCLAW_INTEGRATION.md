# OpenClaw + NyxID Integration Guide

This repository ships an OpenClaw integration at [`integrations/openclaw`](../integrations/openclaw).

## Included Assets

- `openclaw-plugin-nyxid`: TypeScript auth plugin for OpenClaw (optional, for OAuth flows)
- `skills/nyxid`: Skill bundle that uses the `nyxid` CLI (works standalone or via ClawHub)
- `openclaw.plugin.json`: OpenClaw auth-plugin manifest with bundled skill reference

## Default Hosted Instance

The hosted NyxID base URL is:

```
https://nyx-api.chrono-ai.fun
```

The hosted NyxID dashboard is:

```
https://nyx.chrono-ai.fun
```

## Installation

### Local installation (no ClawHub required)

**Option A -- Copy to OpenClaw managed skills (recommended):**

```bash
mkdir -p ~/.openclaw/skills
cp -r skills/nyxid ~/.openclaw/skills/nyxid
```

This makes the skill available to all OpenClaw agents on your machine.

**Option B -- Copy to a workspace (project-scoped):**

```bash
cp -r skills/nyxid /path/to/workspace/skills/nyxid
```

Workspace skills take highest precedence in OpenClaw's skill loading order.

**Option C -- Install from Git URL:**

Paste the repository URL into OpenClaw chat:

```
https://github.com/ChronoAIProject/NyxID
```

OpenClaw will clone and install the skill automatically.

### ClawHub installation (when published)

```bash
clawhub install nyxid
```

### Plugin installation (optional, for OAuth flows)

The plugin provides OAuth login, token refresh, and RFC 8693 delegation. Install it if you need interactive authentication rather than API key access:

```bash
cd integrations/openclaw
npm install
npm run build
```

Then reference it in your OpenClaw plugin config (see Configuration below).

## Configuration

### Quickstart: Using nyxid CLI (recommended)

```bash
# 0. Install Rust if needed (macOS / Linux)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# 1. Install the NyxID CLI
cargo install --git https://github.com/ChronoAIProject/NyxID nyxid-cli

# 2. Log in via browser SSO (saves URL; only needed once)
nyxid login --base-url https://nyx-api.chrono-ai.fun

# 3. Create an API key for OpenClaw
nyxid api-key create --name "openclaw-agent" --scopes "proxy read"

# 4. Add OpenClaw as an AI service (direct mode -- credential on NyxID)
nyxid openclaw setup --url http://localhost:18789 --credential-env OPENCLAW_TOKEN

# Or via node agent (credential stays local):
nyxid service add llm-openclaw --via-node my-node
# Then on the node: nyxid node openclaw connect --url http://localhost:18789

# 5. Verify (no --base-url needed after login)
nyxid service list --output json
nyxid proxy discover --output json
```

### OpenClaw skill configuration

The skill requires the `nyxid` CLI on PATH. No environment variables are needed -- the CLI manages auth and base URL internally after `nyxid login`.

Verify the skill is eligible:

```bash
openclaw skills check
```

You should see `NyxID` marked as ready. If it shows as blocked, ensure `nyxid` is on PATH.

Reload OpenClaw after installing the skill:
- **Start a new chat session** -- simplest option
- **Optional:** install the gateway as a background service so it stays running:
  ```bash
  openclaw gateway install
  openclaw gateway start
  openclaw gateway status
  ```

Ask: "What services do I have connected in NyxID?"

### Full plugin config (OAuth mode)

For interactive OAuth login with token refresh and delegation:

1. Create a developer app in NyxID (Dashboard > Developer Apps > Create)
2. Note the `client_id` and `client_secret`
3. Add to `~/.openclaw/openclaw.json`:

```json
{
  "plugins": {
    "nyxid": {
      "enabled": true,
      "baseUrl": "https://nyx-api.chrono-ai.fun",
      "clientId": "your-client-id",
      "clientSecret": "your-client-secret",
      "defaultScopes": "openid profile email",
      "delegationScopes": "proxy:*"
    }
  }
}
```

## Auth Modes

### CLI mode (recommended)

Run `nyxid login --base-url <URL>` once. The CLI stores tokens at `~/.nyxid/` and auto-refreshes them. The base URL is saved on login. No environment variables needed.

### OAuth plugin mode (optional)

Provide `clientId` with `baseUrl` in the plugin config. Add `clientSecret` for RFC 8693 token exchange. Requires registering a developer app in NyxID. See "Full plugin config" above.

## Using with AI Assistants

The `nyxid` CLI is the recommended way for AI agents to interact with NyxID. Install it and the agent can manage everything.

### Claude Code / Codex / any terminal-based agent

```bash
# Install CLI and log in (one-time; saves URL for all future commands)
cargo install --git https://github.com/ChronoAIProject/NyxID nyxid-cli
nyxid login --base-url https://nyx-api.chrono-ai.fun

# Add services non-interactively (credential from env var)
nyxid service add llm-anthropic --credential-env ANTHROPIC_KEY --output json

# Then the agent can:
nyxid service list --output json                        # list services (includes IDs)
nyxid proxy discover --output json                      # list available services
nyxid proxy request openai /chat/completions -m POST \
  -d '{"model":"gpt-4","messages":[{"role":"user","content":"Hello"}]}'
nyxid catalog list --output json                        # browse available services
nyxid node show my-server --output json                 # node commands accept names
```

### What the agent needs to know

- `nyxid service list --output json` returns machine-readable service list with IDs
- `nyxid proxy request <slug> <path> -m <METHOD> -d <BODY>` calls any service
- `--credential-env <VAR>` reads secrets from env vars (fully non-interactive)
- Node commands accept names (e.g., `nyxid node show test-server`)
- Service `add` auto-fetches label from catalog -- no `--label` needed for catalog services
- NyxID injects credentials server-side -- the agent never handles raw tokens
- If a response has `error_code: 7000`, the user needs to approve the request. The error includes an `action_description` (e.g., "POST /v1/chat/completions (model: gpt-4, 3 messages)") and a `request_id`. Default mode is per-request -- every call needs fresh approval.
- Use `nyxid approval list` to check pending approvals

## Flow Summary

### CLI flow (skill)

1. User runs `nyxid login` (browser SSO, one-time)
2. `nyxid service list --output json` lists available services
3. `nyxid proxy request <slug> <path>` calls any service
4. NyxID injects the user's credentials and forwards the request
5. If approval required (per-request by default), each call returns 7000 with an `action_description`; approve via `nyxid approval approve <ID>`

### OAuth flow (plugin, optional)

1. OpenClaw authenticates via OAuth 2.0 Authorization Code + PKCE
2. Plugin stores tokens in the OpenClaw auth profile
3. Proxy calls use the access token or delegated token via RFC 8693

## Constraints

- RFC 8693 token exchange requires a confidential NyxID OAuth client
- Delegated tokens cannot call `GET /api/v1/proxy/services`; use the CLI or base token for discovery
- Approval-gated proxy calls block until approved or timeout. Default mode is per-request (every call needs fresh approval). Use `approval_mode: "grant"` for time-based grants if per-request approval is too granular for your workflow.

## NyxID Backend Integration (NyxID-to-OpenClaw)

In addition to the OpenClaw skill/plugin (OpenClaw-to-NyxID), NyxID natively supports OpenClaw as a provider and proxy target.

### Connecting OpenClaw as an AI Service

OpenClaw is pre-seeded in the NyxID catalog (`llm-openclaw`). Four ways to connect:

**Option A -- Direct via nyxid CLI (credential on NyxID):**
```bash
nyxid openclaw setup --url http://localhost:18789 --credential-env OPENCLAW_TOKEN
# Reads bearer token from env var, creates the service automatically
```

**Option B -- Via node agent (credential stays local, recommended for privacy):**
```bash
# In NyxID (creates the routed AI service only; no OpenClaw credential is uploaded):
nyxid service add llm-openclaw --via-node my-node

# On the node machine:
nyxid node openclaw connect --url http://localhost:18789
```

**Option C -- Node agent auto-setup (generic, works for any service):**
```bash
nyxid node credentials setup --service llm-openclaw
# Auto-detects the catalog requirements and stores the gateway URL + bearer token locally
```

**Option D -- Node agent OpenClaw-specific helper:**
```bash
nyxid node openclaw connect --url http://localhost:18789
nyxid node openclaw status
nyxid node openclaw disconnect
```

### Proxy passthrough

Once connected, proxy requests through the `nyxid` CLI or API:

```bash
# Via CLI
nyxid proxy request llm-openclaw /v1/chat/completions -m POST \
  -d '{"model":"gpt-4","messages":[{"role":"user","content":"Hello"}]}'

# Via API
POST /api/v1/proxy/s/llm-openclaw/v1/chat/completions
POST /api/v1/proxy/s/llm-openclaw/v1/responses
POST /api/v1/proxy/s/llm-openclaw/tools/invoke
```

Each user's requests route to their own OpenClaw instance.

### WebSocket passthrough (OpenClaw CLI / TUI)

The OpenClaw CLI's native chat flow is a WebSocket upgrade against the
gateway root. NyxID's proxy supports WebSocket passthrough on the same
slug/UUID routes as HTTP, so point the CLI at the proxy URL with the
`wss://` scheme and an `Authorization: Bearer <nyxid_access_token>`
header:

```text
wss://<nyxid-host>/api/v1/proxy/s/llm-openclaw
wss://<nyxid-host>/api/v1/proxy/<service-uuid>
```

The `GET /api/v1/proxy/services` discovery response now advertises this
via `"websocket_supported": true` and `"streaming_supported": true` for
`llm-openclaw`. Clients that see `websocket_supported: false` on other
services should stay on the HTTP path.

**Node-routed OpenClaw requires an up-to-date node agent.** Older agents
do not respond to the server's `open_ws_proxy` frame, which surfaces as
a `NodeProxyTimeout` on the first WS upgrade. If the CLI hangs on
connect, upgrade the node agent and restart the daemon:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/tools/install.sh)"
nyxid node daemon restart
```

### Channel integration

Map OpenClaw channel users (WhatsApp, Telegram, etc.) to NyxID identities:

```bash
# Create mapping (returns per-user webhook_secret -- configure in OpenClaw plugin)
POST /api/v1/integrations/openclaw/mappings
{"channel": "whatsapp", "channel_user_id": "+1234567890"}

# OpenClaw sends webhooks to:
POST /api/v1/integrations/openclaw/channel
# Headers: X-OpenClaw-Signature (HMAC), X-OpenClaw-Webhook-Secret
```

Each mapping has its own webhook secret (generated at creation, only shown once). No server-level env var needed.

### Node agent support

**Setting up a node for OpenClaw (or any service):**

```bash
# Install the nyxid CLI (includes node agent subcommand, --keychain recommended)
cargo install --git https://github.com/ChronoAIProject/NyxID nyxid-cli
nyxid node register \
  --token "nyx_nreg_..." \
  --url "wss://<server>/api/v1/nodes/ws" \
  --keychain

# Auto-setup credentials (detects requirements from catalog)
nyxid node credentials setup --service llm-openclaw

# Start the agent
nyxid node start
```

**OpenClaw-specific convenience setup:**

```bash
nyxid node openclaw connect --url http://localhost:18789 [--access-token <JWT>]
nyxid node openclaw status
nyxid node openclaw disconnect
```

`connect` stores the bearer token locally on the node. If the command also has a NyxID access token available, it will create or confirm the routed AI service on NyxID, but it does not upload the OpenClaw credential to NyxID.

`credentials setup` is recommended for new setups -- it auto-detects the service type and guides through the right flow.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `1001 unauthorized` | Token/key invalid or expired | Run `nyxid login` or create a new API key with `nyxid api-key create` |
| `1002 forbidden` | Missing scope or service not connected | Ensure key has `proxy` scope; add service with `nyxid service add` |
| `7000 approval_required` | Approval gating enabled (per-request by default) | Check `nyxid approval list`; approve via mobile app or Telegram. Each request includes an `action_description`. Use `--approval-mode grant` for time-based grants instead. |
| `8003 node_proxy_error` | Node-backed proxy failed | Check node agent with `nyxid node list`; ensure `nyxid node start` is running |
| Empty services list | No services configured | Browse catalog: `nyxid catalog list`; add: `nyxid service add <slug>` |
| Skill not loading in OpenClaw | Skill not in a recognized directory | Copy to `~/.openclaw/skills/nyxid` or add `extraDirs` |
| Can't reach OpenClaw | Wrong gateway URL or node offline | Verify with `nyxid node openclaw status`; check URL with `nyxid service show <id>` |
