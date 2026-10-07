# Plugin source

Single source for the Claude (`integrations/claude-plugin/`) and Codex (`integrations/codex-plugin/`) packages and both repository marketplaces. Edit the files here, then run:

```bash
python3 scripts/sync-plugins.py          # regenerate
python3 scripts/sync-plugins.py --check  # what CI runs
```

- `plugin.json`: shared version, MCP URL and listing links, plus per-store descriptions, keywords and OpenAI interface fields.
- `skills/nyxid/SKILL.md`: the skill template rendered into both packages.

See [docs/PLUGINS.md](../../docs/PLUGINS.md) for the layout, guardrails and release steps.
