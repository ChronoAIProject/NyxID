# Claude plugin

`integrations/claude-plugin/` is the self-contained plugin folder submitted to the Claude directory. It bundles the remote MCP server (`.mcp.json`), an MCP-first `nyxid` skill that works in Claude chat, Cowork, and Claude Code, a README with the data disclosure, the license, and the icon. It runs no local code.

The repository-root `.claude-plugin/marketplace.json` lists this folder as plugin `nyxid` (installed with `claude plugin install nyxid@nyxid`; its MCP server is `plugin:nyxid:nyxid`) and the CLI skill bundle at the repository root as `nyxid-cli`. The manifest, `.mcp.json`, skill and marketplace are generated from `integrations/plugin-source/`; see [PLUGINS.md](PLUGINS.md).

## Validate

`claude plugin validate --strict` needs Claude Code 2.1.284 or later, the version CI pins (see [PLUGINS.md](PLUGINS.md#requirements)).

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

Before submitting, connect the server as a custom connector in Claude and run the example prompts from the README end to end.

### Reviewer account

The directory reviews the tools listed to the reviewer's session, so prepare a dedicated NyxID account that shows only what passes review:

- **Sign-in:** the hosted instance is SSO-only (Google, GitHub or Apple; email/password login is disabled). Provide a dedicated Google or GitHub account for reviewers, or confirm an alternative with Anthropic before submitting.
- **Services:** connect only catalog services with curated API specs (for example GitHub). Do not connect custom services without an OpenAPI spec. They publish a generic `{slug}__request` tool whose `method` parameter mixes safe and unsafe HTTP methods, a pattern the directory rejects.
- **Tool names:** keep to services whose tool names stay within 64 characters (`{slug}__{operation}`). The curated catalog overlays do; specs discovered automatically may not.
- **Consent:** approve the connector with the services to review (or all services), so `nyx__list_connected_services` is not empty.
