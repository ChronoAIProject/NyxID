# Nyx Skills — Install Manifest (for AI agents)

You (the AI agent — Claude Code, Cursor, Codex, OpenClaw, etc.) have been asked to install **Nyx skills** or **NyxID** on the user's machine. Default to the lightweight `nyxid` CLI plus skill files. Do not install or start the Docker backend unless the user explicitly asks to self-host a NyxID backend.

This file is the canonical install entry point for any Nyx skill. It is intentionally short — the bulk of each skill's behavior lives in its own `SKILL.md` and `references/`.

---

## Agent safety requirements

Before running anything, observe these constraints:

- **Default to the `nyxid` CLI installer.** "Install NyxID" means install the CLI client unless the user explicitly asks to run their own backend/server.
- **Do not run Docker, `docker compose`, or backend setup from `docs/SETUP.md`** unless the user explicitly asks to self-host.
- **Installation alone does not authorize login or credential access.** After successful installation, offer the optional connection in Step 3. If the user chooses it, resolve their selected NyxID instance through the normal human-approved login flow.
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

NyxID's CLI skill bundle contains only these two guides:

| Skill | Purpose | Source |
|---|---|---|
| `nyxid` | Connect services, proxy requests, and manage credentials, nodes, approvals, and MCP through the NyxID CLI. | [`nyxid/`](nyxid/) |
| `nyxid-service-skill-authoring` | Find or author API guidance for a connected service that has no machine-readable specification. | [`nyxid-service-skill-authoring/`](nyxid-service-skill-authoring/) |

`nyxid ai-setup install` installs this core bundle for Codex and OpenClaw, and
through `nyxid-cli@nyxid` for hosted Claude Code installations. Cursor and the
single-skill fallback install only `nyxid`. The hosted `nyxid` MCP plugin has
its own MCP-oriented guide.

Codex and OpenClaw downloads use an explicit core-skill list. Keep the root
`skills/` tree limited to this same bundle because plugin clients discover its
subdirectories. Additional service guidance is discovered when needed through
the connected service's recommendations.

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
nyxid --version
```

### Step 2 — Place the skill files in your agent's skill directory

For **Codex / Codex CLI**, use the managed installer and verify registration:

```bash
nyxid ai-setup install --tool codex
nyxid ai-setup status
test -f "$HOME/.agents/skills/nyxid/SKILL.md"
```

The current documented Codex user skill directory is `~/.agents/skills/`.
`CODEX_HOME` selects Codex configuration and credential storage, not this skill
directory. NyxID conservatively updates existing managed legacy installations.
If an older CLI only wrote `~/.codex/skills/nyxid/`, link that managed directory
into `~/.agents/skills/nyxid/` when the destination does not exist. Codex supports
symlinked skill directories. Never replace an unrelated existing path. Verify
the canonical path above, then reload Codex if its skill list is already cached.

The remaining manual instructions apply to other runtimes or manual Codex installs.

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
- **Codex / Codex CLI:** `~/.agents/skills/nyxid/`
- **Cursor / other runtimes:** consult your runtime's skills or instructions documentation

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

### Step 3 — Offer the optional Codex connection

Only after verifying the CLI and skills, proactively ask:

> NyxID is installed. Would you like to upload and save your existing Codex
> credentials in your NyxID account so compatible AI features can use them?
> Choose Connect or Skip. API-key reuse uses paid OpenAI API access; ChatGPT
> OAuth requires separate authorization to preserve local Codex.

Declining or cancelling completes installation successfully. Do not inspect
Codex configuration, credentials, or the OS credential store. Noninteractive
installation finishes without waiting; the surrounding conversation may offer
this optional step afterward. Generic unattended flags are not credential
consent. Failed installation must not start this step.

Check `nyxid provider connect-codex --help` before invoking the helper. The
published `v0.15.0` predates this operation, so version text alone is insufficient.
If unavailable, offer an update to a supporting release or separate **OpenAI
Codex** authorization in **AI Services**. Installation remains successful.

After Connect, follow the [credential connection procedure](../docs/CODEX_CONNECTION.md):
resolve the chosen instance with normal human-approved login, inspect redacted
status, show the live account and destination, and obtain separate explicit
transfer approval. Existing connections require approval of their ID **and
version**. Only the dedicated local helper may read supported credential files;
raw credentials must never enter the conversation, arguments, logs or clipboard.
Report AI access ready only when the helper returns `usable` after a completed
request. Skip or unsupported storage always leaves the installation usable.

Connect later with `nyxid provider connect-codex`; skip explicitly with
`nyxid provider connect-codex --skip`. The website recommendation must ship with
a compatible CLI release and backend deployment; see the release checklist.

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

Updates remove retired packages recorded in `.nyxid-managed.json`, including
when the installer falls back to the single NyxID guide. Manually installed
companion skills without that ownership record are left alone. A failed removal
prints a warning and keeps the package's ownership record for a later retry;
other packages and tools continue updating. Claude Code
updates its CLI plugin through the marketplace; reload the agent after updating
to clear its cached skill list.

---

## Reporting issues

- GitHub: <https://github.com/ChronoAIProject/NyxID/issues>
- Discord: <https://discord.gg/QMvcs8UQBW>
