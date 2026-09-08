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

| Skill | Purpose | Source |
|---|---|---|
| `nyxid` | Credential broker for downstream APIs (OpenAI, Anthropic, GitHub, Slack, internal APIs, SSH, MCP tools). The skill is a thin wrapper over the `nyxid` CLI. | [`nyxid/`](nyxid/) |

### Aevatar platform (control-plane family)

These skills drive the [Aevatar](https://aevatar.ai) control plane over REST, authenticated through the NyxID broker — so they need the `nyxid` CLI plus a NyxID login, exactly like the `nyxid` skill. They mirror the public `aevatar-platform` skillset on Ornn. Start at `aevatar-platform-map`; each spoke is self-contained.

| Skill | Purpose | Source |
|---|---|---|
| `aevatar-platform-map` | Entry point: object model, auth, and router to the right companion skill for each step. | [`aevatar-platform-map/`](aevatar-platform-map/) |
| `aevatar-agent-profile-management` | Create, validate, and publish Agent Profiles with exact skill pins and bounded tool authority. | [`aevatar-agent-profile-management/`](aevatar-agent-profile-management/) |
| `aevatar-feasibility-advisor` | Decide whether a goal is buildable on Aevatar, and its prerequisites, before building. | [`aevatar-feasibility-advisor/`](aevatar-feasibility-advisor/) |
| `aevatar-workflow-authoring` | Author, validate, and persist an executable Aevatar workflow from a natural-language request. | [`aevatar-workflow-authoring/`](aevatar-workflow-authoring/) |
| `aevatar-team-builder` | Create a team and its members, bind implementations, and set the team entry member. | [`aevatar-team-builder/`](aevatar-team-builder/) |
| `aevatar-service-publisher` | Publish a member/team/workflow as an invocable service, register it with NyxID, and invoke it. | [`aevatar-service-publisher/`](aevatar-service-publisher/) |
| `aevatar-scheduler` | Route and manage Team-member automation or generic service schedules with honest credential semantics. | [`aevatar-scheduler/`](aevatar-scheduler/) |
| `aevatar-automation` | Create independent scheduled Ornn skill agents and reminders with durable credential semantics. | [`aevatar-automation/`](aevatar-automation/) |
| `aevatar-channels-delivery` | Configure channel capabilities, delivery targets, and typed credential failure triage. | [`aevatar-channels-delivery/`](aevatar-channels-delivery/) |
| `aevatar-codex-exec-node-setup` | Configure and prove the managed-sandbox or private-SSH `codex_exec` target. | [`aevatar-codex-exec-node-setup/`](aevatar-codex-exec-node-setup/) |
| `aevatar-codex-exec-workflow-sample` | Run harmless workflows that verify either `codex_exec` target end to end. | [`aevatar-codex-exec-workflow-sample/`](aevatar-codex-exec-workflow-sample/) |
| `firecrawl-via-nyxid` | Run Firecrawl research jobs through a caller-owned NyxID connection. | [`firecrawl-via-nyxid/`](firecrawl-via-nyxid/) |
| `github-via-nyxid` | Use the GitHub REST API through a caller-owned NyxID connection. | [`github-via-nyxid/`](github-via-nyxid/) |
| `aevatar-triage` | Triage a failure across Aevatar / NyxID / Ornn and file a code-grounded issue or give usage guidance. | [`aevatar-triage/`](aevatar-triage/) |
| `fallback-to-calling-agent` | Safety net: hand the original task back to the calling agent when Aevatar can't finish it server-side. | [`fallback-to-calling-agent/`](fallback-to-calling-agent/) |

A plugin install (`/plugin install nyxid@nyxid`) bundles every skill above automatically across the Claude Code / Codex / Cursor manifests. For manual installs, copy each skill directory you want from `skills/`. This manifest will list more as they ship.

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

Skills you copied manually into a runtime's skill directory (e.g. `~/.claude/skills/nyxid/`) are not tracked by the CLI and will not auto-update — re-run Step 2 to refresh them.

---

## Reporting issues

- GitHub: <https://github.com/ChronoAIProject/NyxID/issues>
- Discord: <https://discord.gg/QMvcs8UQBW>
