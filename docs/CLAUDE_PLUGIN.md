# Claude plugin

`integrations/claude-plugin/` is the self-contained plugin folder submitted to the Claude directory. It bundles the remote MCP server (`.mcp.json`), an MCP-first `nyxid` skill that works in Claude chat, Cowork, and Claude Code, a README with the data disclosure, the license, and the icon. It runs no local code.

The repository-root `.claude-plugin/` marketplace is separate: it installs the CLI-oriented skills from `skills/` for Claude Code users and is unchanged by this folder.

## Validate

```bash
python3 scripts/validate-claude-plugin.py
```

The script checks the manifest (https listing URLs, icon), the remote MCP server declaration, skill frontmatter, README length, LICENSE, and the directory's bundle limits (512 files, 256 KiB per non-image file, no symlinks, no `.ico`/`.pdf`/`.zip`/`.DS_Store`). When the `claude` CLI is on `PATH`, it also runs `claude plugin validate --strict`. CI runs it as **Claude Plugin Validate** whenever the folder or script changes.

## Submit

Submit from the Claude organization that should own the listing (on Team or Enterprise plans an Owner submits), at <https://claude.ai/directory/manage> → **Submit new**:

1. **MCP connector**: `https://nyx-api.chrono-ai.fun/mcp`, OAuth via dynamic client registration.
2. **Plugin**: this repository and the `integrations/claude-plugin` folder.

Before submitting, connect the server as a custom connector in Claude and run the example prompts from the README end to end. Reviewer access uses SSO (Google, GitHub, or Apple); provide a dedicated reviewer account with services already connected.
