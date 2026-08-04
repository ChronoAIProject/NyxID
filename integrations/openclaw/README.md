# NyxID OpenClaw Integration

`openclaw-plugin-nyxid` lets OpenClaw agents discover and call user-connected services through NyxID's credential brokering proxy.

## Quickstart

### Step 1: Install Rust and the NyxID CLI

```bash
# Install Rust (macOS / Linux)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Install NyxID CLI
cargo install --git https://github.com/ChronoAIProject/NyxID nyxid-cli

# Log in (opens browser, saves URL for all future commands)
nyxid login --base-url https://nyx-api.chrono-ai.fun
```

### Step 2: Optional API key for plugin / env-based auth

```bash
nyxid api-key create --name "openclaw-agent" --scopes "proxy read"
```

Copy the generated key (starts with `nyxid_`) only if you plan to use the OAuth/plugin path or environment-based auth.

> The bundled OpenClaw skill uses the `nyxid` CLI directly after `nyxid login`. It does not need `NYXID_API_KEY` for normal CLI-based operation.

### Step 3: Add some AI services

```bash
nyxid catalog list                                                # browse available services
nyxid service add llm-openai --credential-env OPENAI_API_KEY      # add OpenAI (non-interactive)
nyxid service add api-github --oauth                              # add GitHub (OAuth flow)
nyxid service list --output json                                  # verify (includes IDs)
```

> Use `--credential-env <VAR>` to read secrets from environment variables for fully non-interactive setup.

### Step 4: Install the skill

**Option A -- Copy to OpenClaw managed skills (recommended):**

```bash
mkdir -p ~/.openclaw/skills
cp -r skills/nyxid ~/.openclaw/skills/nyxid
```

**Option B -- Copy to a workspace:**

```bash
cp -r skills/nyxid /path/to/your/workspace/skills/nyxid
```

**Option C -- Install from ClawHub (when published):**

```bash
clawhub install nyxid
```

### Step 5: Verify the CLI is available to OpenClaw

```bash
openclaw skills check
```

You should see `NyxID` marked as ready. If it is blocked, ensure `nyxid` is on `PATH` for the OpenClaw process.

### Step 6: Test it

Start a new OpenClaw session and ask: "What services do I have connected in NyxID?"

The agent will run `nyxid service list` and show your configured services. Then try "Post a tweet saying hello" or "List my GitHub repos" -- the agent proxies through NyxID automatically.

## Using with other AI assistants

The `nyxid` CLI works with any terminal-based AI assistant.

### Claude Code / Codex / any agent

```bash
# Install and log in (one-time; saves URL for all future commands)
cargo install --git https://github.com/ChronoAIProject/NyxID nyxid-cli
nyxid login --base-url https://nyx-api.chrono-ai.fun

# Add services non-interactively (credential from env var)
nyxid service add llm-openai --credential-env OPENAI_API_KEY --output json

# The agent can then:
nyxid service list --output json                      # list services (includes IDs)
nyxid proxy request llm-openai /chat/completions \
  -m POST -d '{"model":"gpt-4","messages":[...]}'    # call a service
nyxid proxy request api-twitter /2/tweets \
  -m POST -d '{"text":"Hello"}'                      # call another service
nyxid node show my-server --output json               # node commands accept names

# SSH remote access (accepts service ID, slug, or name)
nyxid ssh exec kw-office --principal ubuntu -- uptime
nyxid ssh terminal kw-office                           # auto-resolves principal
```

The agent only needs to know:
- `nyxid service list --output json` shows configured services with IDs
- `nyxid proxy request <slug> <path> -m <METHOD> -d <BODY>` calls any service
- `nyxid ssh exec <service> --principal <user> -- <command>` runs remote commands
- `nyxid ssh terminal <service>` opens an interactive SSH terminal
- `--credential-env <VAR>` reads secrets from env vars (fully non-interactive)
- NyxID handles credential injection -- the agent never sees raw tokens
- The CLI auto-refreshes tokens -- no manual re-authentication needed
- Node and SSH commands accept names or slugs (not just UUIDs)

### CLI requirement

The skill drives all of NyxID through the `nyxid` CLI, which must be on `PATH` for the OpenClaw process. For non-CLI auth flows, use the plugin or environment-backed API key flow instead:

```bash
export NYXID_API_KEY="nyxid_your_key_here"
export NYXID_BASE_URL="https://nyx-api.chrono-ai.fun"
```

## Full plugin setup (OAuth mode -- advanced)

For interactive OAuth login instead of API key auth, configure the plugin:

1. Create a developer app: `nyxid` dashboard > Developer Apps > Create
2. Configure in `~/.openclaw/openclaw.json`:

```json
{
  "plugins": {
    "nyxid": {
      "enabled": true,
      "baseUrl": "https://nyx-api.chrono-ai.fun",
      "clientId": "your-client-id",
      "clientSecret": "your-client-secret",
      "delegationScopes": "proxy:*"
    }
  }
}
```

This enables OAuth 2.0 + PKCE login, automatic token refresh, and RFC 8693 delegated token exchange.

## Auth modes

| Mode | Setup | Best for |
|------|-------|----------|
| **CLI login** | `nyxid login` | Interactive use, auto token refresh |
| **API key** | Set `NYXID_API_KEY` | Plugin / env-based auth, headless agents |
| **OAuth plugin** | Configure `clientId` | Multi-user, delegated access |
| **Bearer token** | Set `NYXID_ACCESS_TOKEN` | Existing JWT from another source |

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `1001 unauthorized` | Run `nyxid login` or create a new API key with `nyxid api-key create` |
| `1002 forbidden` | Ensure API key has `proxy` scope; add service with `nyxid service add` |
| `7000 approval_required` | Check `nyxid approval list`; approve via mobile app or Telegram |
| `8003 node_proxy_error` | Check `nyxid node list`; ensure node agent is running |
| Empty service list | Add services: `nyxid catalog list` then `nyxid service add <slug>` |
| Skill not loading | Copy to `~/.openclaw/skills/nyxid` or add `extraDirs` to OpenClaw config |
