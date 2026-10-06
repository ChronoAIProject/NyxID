# Claude plugin

`integrations/claude-plugin/` is the self-contained plugin folder submitted to the Claude directory. It bundles the remote MCP server (`.mcp.json`), an MCP-first `nyxid` skill that works in Claude chat, Cowork, and Claude Code, a README with the data disclosure, the license, and the icon. It runs no local code.

The repository-root `.claude-plugin/` marketplace is separate: it installs the CLI-oriented skills from `skills/` for Claude Code users and is unchanged by this folder.

## Validate

```bash
python3 scripts/validate-claude-plugin.py                  # package checks
python3 scripts/validate-claude-plugin.py --require-claude # also require `claude plugin validate --strict`
python3 scripts/validate-claude-plugin.py --repo-only      # repository-wide directory limits
```

These are conservative local checks, not the directory's full validation or security scan. They cover the manifest (https listing URLs, icon), the remote MCP server declaration, skill frontmatter, README length outside code blocks, LICENSE, and the bundle limits (512 files and 256 KiB per non-image file, which the directory treats as review holds; a 5 MiB cap on any file; no symlinks, executables, archives, or OS metadata files). `--repo-only` checks the limits the directory reads across the whole repository: under 10,000 files and folders, 256 MiB unpacked, and 50 MiB archived.

CI runs the repository check on every PR as **Claude Plugin Validate**, and the package checks with a pinned Claude Code (`CLAUDE_CODE_VERSION`) whenever the folder or validator changes.

## Submit

Submit from the Claude organization that should own the listing (on Team or Enterprise plans an Owner submits), at <https://claude.ai/directory/manage> → **Submit new**:

1. **MCP connector**: `https://nyx-api.chrono-ai.fun/mcp`, OAuth via dynamic client registration.
2. **Plugin**: this repository and the `integrations/claude-plugin` folder.

Before submitting, connect the server as a custom connector in Claude and run the example prompts from the README end to end. Reviewer access uses SSO (Google, GitHub, or Apple); provide a dedicated reviewer account with services already connected.
