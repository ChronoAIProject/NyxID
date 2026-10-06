# Agent plugins: maintenance guide

NyxID ships to agent stores as thin plugin packages around one hosted MCP server, `https://nyx-api.chrono-ai.fun/mcp`. Every behavior lives on that server: tools, OAuth sign-in, consent, tool annotations, connection flows. A package holds only a manifest, a pointer to the server, one MCP-first skill, and icons. It runs no local code.

Server changes therefore reach every installed plugin when the backend deploys. A package only changes when its listing metadata or skill changes.

## Layout

| Path | Store | Generated? |
| --- | --- | --- |
| `integrations/plugin-source/` | Single source for Claude and Codex | Source |
| `integrations/claude-plugin/` | Claude directory, Claude Code | Manifest, `.mcp.json`, skill |
| `integrations/codex-plugin/` | OpenAI plugin directory (ChatGPT, Codex) | Manifest, `mcp.json`, skill |
| `.claude-plugin/marketplace.json` | `claude plugin marketplace add ChronoAIProject/NyxID` | Yes |
| `.agents/plugins/marketplace.json` | `codex plugin marketplace add ChronoAIProject/NyxID` | Yes |
| `.claude-plugin/plugin.json` + `skills/` | `nyxid-cli`: CLI edition for Claude Code, installed by `nyxid ai-setup` | No |
| `integrations/cursor-plugin/` | Cursor | No ([docs/CURSOR_PLUGIN.md](CURSOR_PLUGIN.md)) |

Each package's README, LICENSE and `assets/` stay hand-written.

The Claude marketplace lists two plugins:
- `nyxid` is the package submitted to the Claude directory. Its MCP server is registered as `plugin:nyxid:nyxid`.
- `nyxid-cli` is the CLI skill bundle at the repository root, including the Aevatar family and the service skills.

The Codex marketplace lists only `nyxid`.

## Changing a plugin

1. Edit `integrations/plugin-source/plugin.json` for the version, MCP URL, listing links, descriptions and default prompts, or `integrations/plugin-source/skills/nyxid/SKILL.md` for the skill.
   - The skill template takes `{{clients}}`, `{{assistant}}`, `{{mcp_url}}`, `{{homepage}}` and `{{manage_access_url}}`.
   - Mention MCP tools by their exact `nyx__*` names.
2. Run `python3 scripts/sync-plugins.py` and commit the regenerated files with the source.
3. Bump `version` in `plugin-source/plugin.json` whenever a package's files change. Both packages share one version.
   - Codex also shows `stores.codex.release_notes` to reviewers.
   - The CLI edition keeps its own version in `.claude-plugin/plugin.json`.

Never edit generated files by hand. CI fails when they differ from the source.

## Guardrails

- **Generated files:** `scripts/sync-plugins.py --check` (CI: Codex and Claude Plugin Validate) fails when a generated file drifts from the source.
- **Packages:** `scripts/build-codex-plugin.sh --check` and `scripts/validate-claude-plugin.py --require-claude` validate the packages and the store bundle limits. `claude plugin validate --strict .` validates the Claude marketplace.
- **Skill vs server contract:** the backend test `plugin_skill_only_names_real_annotated_meta_tools` (`backend/src/services/mcp_service.rs`) fails when the skill names a `nyx__*` tool that the server does not define or that has no `title`, `readOnlyHint`, `destructiveHint` and `openWorldHint` annotations. The Claude directory requires a title and a read-only or destructive hint on every tool; `tool_title` and `tool_annotations` in the same file supply them, and `tools/list` publishes the title both at the top level and in `annotations`. Rename a meta tool and the skill in the same change.

## Releasing

| Change | Action |
| --- | --- |
| Backend MCP or OAuth behavior | Deploy the backend. No store action; the stores' scans read the live server. |
| Skill or listing metadata | Bump the version, run `sync-plugins.py`, merge, then publish both stores below. |

- **Codex / ChatGPT:** run `scripts/build-codex-plugin.sh` and upload `dist/nyxid-codex-plugin-<version>.zip` as a new version in the OpenAI plugin portal ([docs/CODEX_PLUGIN.md](CODEX_PLUGIN.md)).
- **Claude:** resubmit from <https://claude.ai/directory/manage> ([docs/CLAUDE_PLUGIN.md](CLAUDE_PLUGIN.md)). Claude Code users get the new version through `claude plugin update nyxid@nyxid`.

## App access and consent

Plugins sign in with OAuth dynamic client registration. On the consent page the user chooses which services the app may use. A restricted or zero-service grant is kept through code exchange and refresh. The skill's "When services are missing" section tells the agent to send the user to `/settings/consents` to revoke and reconnect the app. `nyx__connect_service` returns `service_not_granted` with the same link when a restricted app asks to connect a new service.

MCP sessions created by tokens issued to an OAuth client, or by tokens restricted to certain services or nodes, never authenticate on their own: every request must carry a live bearer token, so the grant always applies.
