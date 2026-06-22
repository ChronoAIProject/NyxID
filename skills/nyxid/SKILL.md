---
name: nyxid
version: "0.5"
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

Install the NyxID CLI (one-time). This is the default "install NyxID" path; do not run the Docker backend setup unless the user explicitly asks to self-host:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
```

The installer downloads an attested prebuilt release binary (verified against the GitHub release workflow's Sigstore attestation), installs it into a versioned layout under `~/.local/share/nyxid/versions/`, links `~/.local/bin/nyxid` to the active version, and configures your shell PATH. No Rust toolchain is required on published targets: macOS x64/arm64 and Linux x64/arm64. Linux arm64 binaries target the Ubuntu 20.04 / `glibc 2.31` baseline. The installer falls back to a Cargo source build only on platforms with no compatible published binary; on Linux arm64 source fallback it uses `CC=clang` when available and otherwise tells the user to install `clang` if it detects the `aws-lc-sys` GCC compiler guard. Open a new terminal afterwards, then log in:

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
nyxid update                                 # download + verify + install the latest prebuilt CLI, then update skills
nyxid update --skills-only                   # update only installed skills (skip CLI download)
nyxid update --check                         # report installed vs latest without installing anything
nyxid update --version 0.5.0                 # pin to a specific release (rollback or test a prerelease)
nyxid update --rollback                      # retarget the active symlink to the previous installed version
nyxid update --list-versions                 # list versions installed under ~/.local/share/nyxid/versions
nyxid update --from-source                   # force the cargo install fallback (useful on unsupported targets)
```

`nyxid update` verifies the downloaded binary against the GitHub release workflow's Sigstore attestation before swapping the active symlink. Verification failures fail closed; pass `--insecure-skip-verify` only as an explicit opt-out.

To update a specific tool's skill only:

```bash
nyxid ai-setup update --tool claude-code     # update a specific tool
```

When running any nyxid subcommand interactively, the CLI also prints a one-line "newer version available" notice once per 24h (telemetry-free; only hits the GitHub releases API). Set `NYXID_NO_UPDATE_CHECK=1` to disable, or run in CI (`CI=true` is auto-detected).

If `nyxid update` is not recognized, your CLI predates this command. Update it first with:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
```

The wrapper installer detects an existing legacy single-file install at `~/.local/bin/nyxid` and migrates it into the versioned layout transparently.

## Diagnosing install or auth issues

When the user reports "nyxid is broken", "I can't log in", "is my install OK", or similar, run `nyxid doctor` first before debugging individual commands. It prints a structured health check covering:

- **Installation**: binary path, active symlink target, whether the install dir is in `$PATH`
- **GitHub Releases**: API reachability, latest release tag vs installed, rate limit + reset time
- **Authentication**: stored base URL, login state (token expiry shown; the token itself is never printed)
- **Telemetry**: consent state
- **Update check**: last-check timestamp, whether auto-check is enabled

```bash
nyxid doctor                                 # human-readable report
nyxid doctor --json                          # structured output for scripts
```

Doctor exits non-zero if any check fails (warnings do not fail). Use it as the first triage step, then drill into the failing area with the specific reference page (`references/admin.md` for auth/error codes, `references/services.md` for service issues, etc.).

## Reference map

Load the matching `references/<file>.md` when the user asks for one of these topics. Each file is self-contained; load only what's needed.

| Trigger keywords / user request | Load this reference |
|---|---|
| "list my services", "what's connected", "discover services", "add a service", "connect OpenAI / GitHub / Lark / etc.", "OAuth scopes", browser-wizard / pairing-code questions, "where do I get the API key" | `references/services.md` |
| "call the API", "proxy request", "send a message via Telegram/Discord/Slack" (single call), curl examples, raw HTTP integration, WebSocket auth-frame injection, Home Assistant connection | `references/proxy.md` |
| "list / rename / delete a service", attaching an OpenAPI spec to a custom endpoint, default headers, "create / rotate / delete an API key", agent key bindings, callback URLs, scope/rate-limit edits | `references/managing.md` |
| Anything mentioning "org", "organization", "shared credentials", "family / company key", invites, role scopes, primary-org tiebreaker, org-level approval policies, `--via-service`, CLI profiles | `references/organizations.md` |
| "set up a node", "credentials on my own machine", org-owned/shared nodes, node daemon (install/start/stop/logs), node credentials add/setup/list, remote credential injection / `node-credential inject` / "push a secret to a node from my laptop or browser without SSH" / fingerprint verification / browser accept page, SSH node-key credentials, SSH exec / terminal / cert-issue, SSH ProxyCommand | `references/nodes.md` |
| "provision a headless device / 无头设备", "approve a device", "ESP32", "factory key", "nyxprov QR", "device-code grant", `nyxid device approve/onboard/factory-key`, `/devices/code/*`, `/devices/onboard` | `references/devices.md` |
| "approve / deny", "set up notifications", Telegram link, push notifications, approval grants, per-service approval configs, granular approval rules (method/path/verb), allow-list or deny specific endpoints, `default_effect`, scoped grants | `references/notifications.md` |
| "channel bot", "register a bot", conversation routing, `/channel-relay/reply`, callback / reply tokens, ADR-013 passthrough semantics, device events / HTTP Event Gateway, `/channel-events/{id}` | `references/channels.md` |
| OpenClaw setup, `llm-openclaw` transport selection, `x-openclaw-scopes` default header | `references/openclaw.md` |
| `nyxid whoami / status / profile / mfa / session`, `nyxid admin user list/show/set-role`, platform roles (admin / operator / user), `nyxid admin invite-code`, `nyxid mcp config`, error codes (1001/1002/7000/7001/8003, downstream 403 / WAF / User-Agent override) | `references/admin.md` |
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
- **Remote credential injection is end-to-end encrypted.** When an admin provisions a node credential from a browser or laptop (`nyxid node-credential inject` or the web "Accept credential" page), the secret is encrypted client-side and NyxID relays only the ciphertext -- the server never sees the plaintext, never decrypts, and never derives the shared key. Only the target node decrypts it.
- **Authentication tokens auto-refresh.** The CLI handles token refresh automatically.
- **No data is sent to third parties.** All traffic flows between the agent and the user's NyxID instance.
- **Audit logging.** All proxy requests are logged in NyxID for user review.

## Model Invocation Note

This skill may be invoked autonomously by the agent when a user request involves an external service. The agent discovers available services through NyxID and routes requests through the proxy without prompting for raw credentials. Users can disable this skill in their OpenClaw configuration to opt out.

## Trust Statement

By using this skill, requests are sent to your configured NyxID instance. NyxID forwards those requests to downstream services using your stored credentials. Only install this skill if you trust your NyxID instance operator.
