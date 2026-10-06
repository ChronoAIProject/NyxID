# Codex and ChatGPT plugin

`integrations/codex-plugin/` is the self-contained package for OpenAI's plugin directory (ChatGPT and Codex) and for Codex CLI installs. It contains `plugin.json` (listing metadata, icons, default prompts), `mcp.json` (NyxID's remote MCP server), an MCP-first `nyxid` skill, a README with the data disclosure, the license, and the icons. It runs no local code and does not ship the CLI-oriented skills from the repository's `skills/` folder.

## Build and validate

```bash
scripts/build-codex-plugin.sh --check   # validate only (CI: Codex Plugin Validate)
scripts/build-codex-plugin.sh           # validate, then write dist/nyxid-codex-plugin-<version>.zip
```

The ZIP contains only the package folder's allowlisted entries (`plugin.json`, `mcp.json`, `README.md`, `LICENSE`, `assets/`, `skills/`); any other top-level entry, symlink, or `.DS_Store` fails the build. Bump `version` in `plugin.json` before building a new upload.

## Publish to the OpenAI plugin directory

Upload the ZIP in the plugin portal with **Upload new version**. Skill and metadata changes need a new ZIP; MCP tool changes are picked up from the live server by the portal's scan.

## Install with Codex CLI

The repository-root `.agents/plugins/marketplace.json` points Codex at this folder:

```bash
codex plugin marketplace add ChronoAIProject/NyxID
codex plugin add nyxid@nyxid
codex mcp login nyxid
```
