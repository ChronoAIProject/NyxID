# Cursor Marketplace Plugin

NyxID's Cursor plugin lives in [`integrations/cursor-plugin/`](../integrations/cursor-plugin/). It is deliberately self-contained so the directory can be mirrored into a standalone public repository without changing its manifest paths.

## Publish from this repository

1. Keep the repository public and make sure the plugin manifest, logo, rules, skills, commands, MCP config, and README are committed.
2. Run `python3 scripts/validate-cursor-plugin.py` from the repository root.
3. Open <https://cursor.com/marketplace/publish> and submit the public GitHub repository.
4. The root `.cursor-plugin/marketplace.json` identifies the `nyxid` plugin and its `integrations/cursor-plugin` source directory for the marketplace submission.
5. Complete Cursor's manual review. Marketplace updates are reviewed again, so publish a new version for substantive changes and rerun validation before submitting.

The root marketplace file is needed because the plugin is nested in this monorepo. Its `source` is `integrations/cursor-plugin`, relative to the repository root.

## Mirror into a standalone repository

For a dedicated marketplace repository, copy the complete `integrations/cursor-plugin/` directory to the repository root, preserving `.cursor-plugin/plugin.json` and all relative paths. The standalone repository can submit that root plugin directly; it does not need the monorepo marketplace index. Keep the repository public and include the plugin README and Apache-2.0 license notice.

After mirroring, run the validator against the standalone plugin path if needed:

```bash
python3 scripts/validate-cursor-plugin.py /path/to/standalone-repo
```

Do not change `${NYXID_BASE_URL}` into a secret-bearing variable. The default browser OAuth flow is intentionally zero-config; users configure only the NyxID API origin in Cursor's plugin settings.
