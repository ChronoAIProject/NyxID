# Nyx Skills — Install Manifest (for AI agents)

You (the AI agent — Claude Code, Cursor, Codex, OpenClaw, etc.) have been asked to install **Nyx skills** on the user's machine. Follow the steps below, then hand control back to the user for the final login.

This file is the canonical install entry point for any Nyx skill. It is intentionally short — the bulk of each skill's behavior lives in its own `SKILL.md` and `references/`.

---

## Agent safety requirements

Before running anything, observe these constraints:

- **Do not run `nyxid login`.** The user picks the NyxID instance and enters credentials themselves.
- **Do not ask the user for NyxID credentials, API keys, tokens, or invite codes.** The CLI handles credential entry interactively when the user runs it.
- **Do not print secrets, environment variables, or credential files** at any point during install.
- **Do not use `sudo`** unless the user explicitly asks for a system-wide install. The default install is fully user-scoped under `~/.local/`.
- In security-sensitive environments, show the user the install command before executing it, or pin to a release tag instead of `main`.

---

## Prerequisites

The commands below assume a Unix-like shell with:

- `bash`
- `curl`
- `git` — only if you use the sparse-checkout fetch in Step 2
- `cargo` — only if the installer falls back to a source build (rare; most platforms have a prebuilt release)
- `clang` — recommended for Linux arm64 source builds; it avoids the known `aws-lc-sys` GCC compiler guard on affected GCC versions

Windows is supported via WSL only.

---

## Skills available

| Skill | Purpose | Source |
|---|---|---|
| `nyxid` | Credential broker for downstream APIs (OpenAI, Anthropic, GitHub, Slack, internal APIs, SSH, MCP tools). The skill is a thin wrapper over the `nyxid` CLI. | [`nyxid/`](nyxid/) |

There is one skill today. This manifest will list more as they ship.

---

## Install procedure

Run the steps below in order. They are idempotent — safe to re-run.

### Step 1 — Install the `nyxid` CLI

Every Nyx skill calls the `nyxid` CLI under the hood, so install it first.

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
```

This downloads a prebuilt binary and verifies its Sigstore attestation before installing it under `~/.local/share/nyxid/versions/<version>/`, then links `~/.local/bin/nyxid` to the active version. Published targets are macOS x64/arm64 and Linux x64/arm64; Linux arm64 binaries target the Ubuntu 20.04 / `glibc 2.31` baseline. The installer falls back to a Cargo source build only on platforms with no compatible published binary. On Linux arm64 source fallback, the installer uses `CC=clang` when `clang` is available and otherwise tells the user to install `clang` if it detects the `aws-lc-sys` GCC compiler guard.

The installer adds `~/.local/bin` to the user's shell `PATH` by editing their shell rc file, but that change only takes effect on next shell load. If `nyxid` is not found in the current session, export it:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

After install, verify:

```bash
nyxid doctor
```

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

Sparse-checkout fetch (preferred — pulls only the skill files):

```bash
git clone --filter=blob:none --sparse https://github.com/ChronoAIProject/NyxID /tmp/nyx-skills
git -C /tmp/nyx-skills sparse-checkout set skills/nyxid
```

Then copy into the runtime's skill directory. For **Claude Code**:

```bash
mkdir -p ~/.claude/skills
cp -R /tmp/nyx-skills/skills/nyxid ~/.claude/skills/
```

Clean up the temporary checkout:

```bash
rm -rf /tmp/nyx-skills
```

If your runtime prefers per-file fetches, pull each file from `https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/<path>` instead.

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

The CLI and any Nyx-managed skills update from one command:

```bash
nyxid update              # update CLI and skills
nyxid update --check      # report installed vs latest, install nothing
nyxid update --skills-only
```

Skills you copied manually into a runtime's skill directory (e.g. `~/.claude/skills/nyxid/`) are not tracked by the CLI and will not auto-update — re-run Step 2 to refresh them.

---

## Reporting issues

- GitHub: <https://github.com/ChronoAIProject/NyxID/issues>
- Discord: <https://discord.gg/QMvcs8UQBW>
