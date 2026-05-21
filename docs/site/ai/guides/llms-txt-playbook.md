---
title: The llms.txt playbook
description: How NyxID's /llms.txt and /llms-full.txt endpoints work, what they contain, and how AI agents use them to self-orient on a live deployment.
---

NyxID serves two machine-readable context files at the top level of every deployment:

- `/llms.txt` — a short summary optimized for context-window efficiency
- `/llms-full.txt` — the full AI Agent Playbook with all commands, API routes, and configuration examples

These files follow the emerging [`llms.txt` convention](https://llmstxt.org/) for helping AI agents understand a site or service without requiring the agent to crawl HTML pages.

Both endpoints require no authentication. Any agent can fetch them at startup to orient itself on the deployment it is talking to.

## What the files contain

### `/llms.txt` — the short version

A concise summary of what NyxID is, how to authenticate, and the most commonly used CLI commands and API routes. Intended to fit in the early portion of a context window without crowding out working context.

A typical agent use case: include the content of `/llms.txt` in a system prompt or task preamble so the agent knows the base URL, how to authenticate, and how to call `nyxid service add` before the conversation begins.

### `/llms-full.txt` — the full playbook

The complete AI Agent Playbook: every CLI subcommand, every API route, all configuration examples, node proxy setup, approval workflows, SSH services, channel bots, and troubleshooting guides.

Critically, `/llms-full.txt` replaces placeholder URLs with the **live deployment URLs**. When the server serves this file, it substitutes the actual backend API URL and frontend dashboard URL into every code example. An agent that fetches `/llms-full.txt` gets runnable commands, not examples with `http://localhost:3001` that need manual substitution.

```bash
# Fetch the full playbook and pipe to a pager or save locally
curl https://nyx-api.chrono-ai.fun/llms-full.txt | less
curl https://nyx-api.chrono-ai.fun/llms-full.txt > /tmp/nyxid-playbook.txt
```

## How agents use these files

### Self-orientation at session start

Before beginning any NyxID-related task, an agent can fetch `/llms.txt` to learn:

- What NyxID is and what it can do
- The base URL for all API calls
- How to install and authenticate the CLI
- How to add a service and make a proxy request
- Where to find the full playbook

```bash
# From a shell
curl https://nyx-api.chrono-ai.fun/llms.txt
```

In an agentic framework that supports system prompt injection, the content of `/llms.txt` can be fetched at agent initialization and prepended to the system prompt so the agent is already oriented before the user speaks.

### Full task context

When the user asks for something non-trivial — setting up a node proxy, configuring approvals, wrapping a REST API as MCP tools — the agent can fetch `/llms-full.txt` to get exact, runnable instructions for that specific deployment.

### Claude Code + `nyxid ai-setup`

For deployments with the `nyxid` CLI installed, the `ai-setup` subcommand automates skill installation for Claude Code:

```bash
# Install NyxID skill and playbook into Claude Code
nyxid ai-setup install --tool claude-code
```

This fetches the current playbook from the server, installs a Claude Code skill, and adds a CLAUDE.md context file so that every Claude Code session in the project is already aware of NyxID. The skill is kept up to date with:

```bash
nyxid ai-setup update                    # update all installed tools
nyxid ai-setup update --tool claude-code # update a specific tool
nyxid ai-setup status                    # check what's installed and version
```

Under the hood, `ai-setup install` fetches `/llms-full.txt` and writes it to the relevant AI tool's config directory, scoped to the current project or user.

## URL substitution in `/llms-full.txt`

The playbook source uses two placeholder URLs:

- `http://localhost:3001` — backend API
- `http://localhost:3000` — frontend dashboard

When the server serves `/llms-full.txt`, these placeholders are replaced with the live deployment URLs. The result is a fully runnable playbook with no manual URL editing required.

:::note
`/llms.txt` does not perform URL substitution — it is a static short summary. Only `/llms-full.txt` has live URL injection.
:::

## Integrating into your own agent framework

If you build on top of NyxID and want your own AI agents to be self-orienting, fetch `/llms.txt` at agent startup and include its content as a system prompt preamble:

```python
import httpx

NYXID_API = "https://nyx-api.chrono-ai.fun"

def get_nyxid_context() -> str:
    resp = httpx.get(f"{NYXID_API}/llms.txt", timeout=5)
    resp.raise_for_status()
    return resp.text

system_prompt = get_nyxid_context() + "\n\n" + YOUR_SYSTEM_PROMPT
```

For tasks that involve complex NyxID configuration, fetch `/llms-full.txt` instead and include only the relevant sections (search for the section heading and extract the surrounding text).

## OpenClaw integration

When NyxID is used as an OpenClaw plugin, the skill installation copies the playbook content into OpenClaw's managed context:

```bash
mkdir -p ~/.openclaw/skills
cp -r skills/nyxid ~/.openclaw/skills/nyxid
openclaw skills check
```

The NyxID skill uses the `nyxid` CLI for all operations and relies on `~/.nyxid/base_url` (set by `nyxid login`) for the deployment URL — no environment variables needed in the skill itself.

## Reference

| Path | Auth required | Description |
|---|---|---|
| `GET /llms.txt` | No | Short NyxID summary for agents |
| `GET /llms-full.txt` | No | Full playbook with live URL substitution |
| `nyxid ai-setup install --tool <tool>` | CLI session | Install skill + playbook for a specific AI tool |
| `nyxid ai-setup update` | CLI session | Update installed skills to the current playbook |
| `nyxid ai-setup status` | CLI session | Show installed tools and playbook version |
