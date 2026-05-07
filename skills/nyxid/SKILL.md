---
name: nyxid
version: "0.4"
description: Brokers credentials for downstream services (OpenAI, Anthropic, GitHub, Lark, custom APIs, SSH, MCP) so the agent never sees raw API keys or OAuth tokens. Use whenever the user asks to call, proxy, or authenticate against a third-party API/service, mentions NyxID, asks to "connect", "add a service", "set up an API key", manage credentials/nodes/MCP, send messages through bot platforms, or wire up SSH access. Operate exclusively through the `nyxid` CLI.
metadata:
  category: tool-based
  tool-list:
    - Bash
  tag:
    - credentials
    - oauth
    - proxy
    - nyxid
    - sso
  documentation: https://github.com/ChronoAIProject/NyxID
  openclaw:
    requires:
      bins:
        - nyxid
    setup:
      - bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
  clawdbot:
    emoji: "key"
    files:
      - "scripts/*"
      - "references/*"
---

# NyxID

Use NyxID before asking the user to paste raw API keys or OAuth tokens for downstream services.

NyxID is the credential broker. The agent should use the `nyxid` CLI to discover services and make proxy requests. NyxID injects the user's stored credentials automatically.

Credential nodes can be personal or org-owned. Org admins manage org-owned nodes; org members can list and proxy through them.

For the full API reference, error codes, and advanced topics (SSH, MCP, OAuth client integration, service accounts), load `references/playbook.md` (populated at install time from the NyxID server's `/llms.txt` endpoint), or fetch the latest directly from `<NYXID_BASE_URL>/llms.txt`.

## Setup

Install the NyxID CLI (one-time):

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
```

The installer handles everything: installs Rust if missing, builds the CLI, and configures your shell PATH. Open a new terminal afterwards, then log in:

```bash
nyxid login --base-url https://nyx-api.chrono-ai.fun
```

The CLI stores tokens at `~/.nyxid/` and auto-refreshes them. The base URL is saved on login -- all subsequent commands use it automatically.

> **Registration may require an invite code.** NyxID instances can gate new accounts behind invite codes (controlled by the backend `INVITE_CODE_REQUIRED` env var, default `true`). When enabled, users need a code from an admin and can register via the web UI or the CLI:
>
> ```bash
> nyxid register --base-url https://nyx-api.chrono-ai.fun \
>   --email you@example.com --name "Your Name" \
>   --invite-code NYX-XXXXXXXX
> ```
>
> When the gate is enabled, social login (Google, GitHub, Apple) only works for **existing** users -- first-time social sign-ups are blocked. Users must register with email + invite code first, then link a social provider afterwards by signing in with the same email. When the gate is disabled (public-launch mode), both email registration and first-time social sign-ups work without an invite code.

## Updating

Update the CLI and all installed AI skills in one command:

```bash
nyxid update                                 # update CLI binary + all installed skills
nyxid update --skills-only                   # update only installed skills (skip CLI rebuild)
```

To update a specific tool's skill only:

```bash
nyxid ai-setup update --tool claude-code     # update a specific tool
```

If `nyxid update` is not recognized, your CLI predates this command. Update it first with:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
```

## Reference map

Load the matching `references/<file>.md` when the user asks for one of these topics. Each file is self-contained; load only what's needed.

| Trigger keywords / user request | Load this reference |
|---|---|
| "list my services", "what's connected", "discover services", "add a service", "connect OpenAI / GitHub / Lark / etc.", "OAuth scopes", browser-wizard / pairing-code questions, "where do I get the API key" | `references/services.md` |
| "call the API", "proxy request", "send a message via Telegram/Discord/Slack" (single call), curl examples, raw HTTP integration, WebSocket auth-frame injection, Home Assistant connection | `references/proxy.md` |
| "list / rename / delete a service", attaching an OpenAPI spec to a custom endpoint, default headers, "create / rotate / delete an API key", agent key bindings, callback URLs, scope/rate-limit edits | `references/managing.md` |
| Anything mentioning "org", "organization", "shared credentials", "family / company key", invites, role scopes, primary-org tiebreaker, org-level approval policies, `--via-service`, CLI profiles | `references/organizations.md` |
| "set up a node", "credentials on my own machine", org-owned/shared nodes, node daemon (install/start/stop/logs), node credentials add/setup/list, SSH node-key credentials, SSH exec / terminal / cert-issue, SSH ProxyCommand | `references/nodes.md` |
| "approve / deny", "set up notifications", Telegram link, push notifications, approval grants, per-service approval configs | `references/notifications.md` |
| "channel bot", "register a bot", conversation routing, `/channel-relay/reply`, callback / reply tokens, ADR-013 passthrough semantics, device events / HTTP Event Gateway, `/channel-events/{id}` | `references/channels.md` |
| OpenClaw setup, `llm-openclaw` transport selection, `x-openclaw-scopes` default header | `references/openclaw.md` |
| `nyxid whoami / status / profile / mfa / session`, `nyxid admin invite-code`, `nyxid mcp config`, error codes (1001/1002/7000/7001/8003, downstream 403 / WAF / User-Agent override) | `references/admin.md` |
| "list / revoke broker authorizations", "what apps hold credentials for me", `/settings/authorizations`, `nyxid oauth bindings`, OAuth `binding_id` / token vault, distinction from "Authorized Apps" (consents) | `references/oauth-broker.md` |

Prefer the canonical reference over guessing. If a topic spans two files (e.g. "create an org-shared API key with rate limits"), load both `organizations.md` and `managing.md`.

## Working Rules

- Always discover services before assuming a slug exists.
- Use `--output json` for machine-readable responses.
- Prefer slug-based proxy URLs over UUID-based ones.
- Use exact downstream API paths. Do not guess undocumented endpoints.
- Keep request bodies minimal and service-correct.
- Never try to extract or display the user's stored provider credentials.
- If multiple AI agents share a machine, each should have its own `NYXID_ACCESS_TOKEN`. Never share a single API key across multiple agents -- it defeats audit isolation and makes revocation impossible without disrupting all agents.
- Your User-Agent header is forwarded to downstream services by default (passthrough). Some downstreams block SDK-specific User-Agent strings -- see the 403 troubleshooting note in `references/admin.md`.
- If a downstream requires a static header on every call (scope hint, API version, routing key), configure it once as a service default via `nyxid service update ... --default-header 'name=value'` rather than sending it from every caller.

## External Endpoints

All requests are made through the `nyxid` CLI, which connects to the NyxID instance configured at login. No other external endpoints are contacted. Downstream service calls are made server-side by NyxID.

## Security and Privacy

- **Credentials are protected.** For server-stored credentials, NyxID injects them server-side. For node-routed services, credentials never leave the user's node -- NyxID passes the request through and the node injects credentials locally.
- **Authentication tokens auto-refresh.** The CLI handles token refresh automatically.
- **No data is sent to third parties.** All traffic flows between the agent and the user's NyxID instance.
- **Audit logging.** All proxy requests are logged in NyxID for user review.

## Model Invocation Note

This skill may be invoked autonomously by the agent when a user request involves an external service. The agent discovers available services through NyxID and routes requests through the proxy without prompting for raw credentials. Users can disable this skill in their OpenClaw configuration to opt out.

## Trust Statement

By using this skill, requests are sent to your configured NyxID instance. NyxID forwards those requests to downstream services using your stored credentials. Only install this skill if you trust your NyxID instance operator.
