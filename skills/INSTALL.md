# Nyx Skills — Install Manifest (for AI agents)

You (the AI agent — Claude Code, Cursor, Codex, OpenClaw, etc.) have been asked to install **Nyx skills** on the user's machine. Follow the steps below, then hand control back to the user for the final login.

This file is the canonical install entry point for any Nyx skill. It is intentionally short — the bulk of each skill's behavior lives in its own `SKILL.md` and `references/`.

---

## Skills available

| Skill | Purpose | Source |
|---|---|---|
| `nyxid` | Credential broker for downstream APIs (OpenAI, Anthropic, GitHub, Slack, internal APIs, SSH, MCP tools). The skill is a thin wrapper over the `nyxid` CLI. | [`skills/nyxid/`](nyxid/) |

There is one skill today. This manifest will list more as they ship.

---

## Install procedure

Run the steps below in order. They are idempotent — safe to re-run.

### Step 1 — Install the `nyxid` CLI

Every Nyx skill calls the `nyxid` CLI under the hood, so install it first.

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
```

This downloads a Sigstore-attested prebuilt binary, installs it under `~/.local/share/nyxid/versions/<version>/`, links `~/.local/bin/nyxid` to it, and ensures `~/.local/bin` is on the user's shell `PATH`. It falls back to a Cargo source build only on platforms with no published binary. After install, verify with `nyxid doctor`.

### Step 2 — Place the skill files in your agent's skill directory

A Nyx skill is a folder of markdown + helper scripts:

```
skills/nyxid/
├── SKILL.md            # entry-point manifest with frontmatter
├── references/         # domain references loaded on demand
└── scripts/            # helper shell scripts the skill calls
```

Copy the entire `skills/nyxid/` directory into wherever your runtime loads skills from. Common locations:

- **Claude Code:** `~/.claude/skills/nyxid/`
- **OpenClaw / clawdbot:** managed through the platform's skill registry — the `metadata.openclaw` / `metadata.clawdbot` block in `SKILL.md` is consumed automatically on registration
- **Cursor / Codex / other runtimes:** consult your runtime's skills or instructions documentation

Sparse-checkout fetch (preferred — pulls only what you need):

```bash
git clone --filter=blob:none --sparse https://github.com/ChronoAIProject/NyxID /tmp/nyx-skills
git -C /tmp/nyx-skills sparse-checkout set skills/nyxid
# then copy /tmp/nyx-skills/skills/nyxid/ into your skill directory
```

Or fetch each file with `curl` from `https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/<path>` if your runtime prefers that.

After copying, reload or re-index your agent if it caches its skill list.

### Step 3 — Hand off to the user for login

The agent **must not** run `nyxid login` on the user's behalf — the user chooses the NyxID instance.

Print a message to the user similar to this:

> The Nyx skill is installed. To finish setup, log in to your NyxID instance:
>
> ```
> nyxid login --base-url <URL>
> ```
>
> - Hosted instance: `https://nyx-api.chrono-ai.fun`
> - Self-hosted: typically `http://localhost:3001` for a local Docker stack
>
> If you don't have an account yet, register at <https://nyx.chrono-ai.fun/register> (an invite code may be required during early access).

---

## What the skill does once loaded

The skill itself describes the full surface — load `SKILL.md` for the canonical reference. Briefly, with `nyxid` you can:

- Browse the catalog of broker-able services
- Add and configure a service (`nyxid service add ...`)
- Proxy requests through NyxID with automatic credential injection
- Manage credential nodes for localhost / private-network reach
- Wrap REST APIs as MCP tools for use across agents
- Issue scoped per-agent API keys with isolation, rate limiting, and audit attribution

---

## Updating

The CLI and any installed skills update from one command:

```bash
nyxid update              # update CLI and skills
nyxid update --check      # report installed vs latest, install nothing
nyxid update --skills-only
```

---

## Reporting issues

- GitHub: <https://github.com/ChronoAIProject/NyxID/issues>
- Discord: <https://discord.gg/QMvcs8UQBW>
