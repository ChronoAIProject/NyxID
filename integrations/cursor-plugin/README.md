# NyxID for Cursor

NyxID gives Cursor agents a credential broker and MCP proxy for downstream APIs. Install this plugin once, authenticate NyxID in the browser, and let the agent discover connected services and call their tools without receiving the provider's API key or OAuth token.

## Install

### Cursor Marketplace

In Cursor, open **Settings -> Plugins -> Marketplace**, search for **nyxid**, and install the plugin. Restart or reload Cursor if it does not immediately appear in the MCP tools list.

### Manual install

This repository keeps the plugin source at `integrations/cursor-plugin/`. Clone the public repository and add that directory through Cursor's local plugin/development install flow. The plugin root is the directory containing `.cursor-plugin/plugin.json`; do not select the monorepo root.

If the Cursor build in use does not support local plugin loading, copy `integrations/cursor-plugin/mcp.json` to the project's `.cursor/mcp.json`, replace `${NYXID_BASE_URL}` with the API origin for the NyxID deployment, and copy the bundled `rules/`, `skills/`, and `commands/` content into the corresponding Cursor project directories. The hosted default is `https://nyx-api.chrono-ai.fun`.

## Configuration

The plugin declares one Cursor variable:

| Variable | Default | Purpose |
| --- | --- | --- |
| `NYXID_BASE_URL` | `https://nyx-api.chrono-ai.fun` | API origin where NyxID serves `/mcp` |

Change it in **Settings -> Plugins -> nyxid -> Configure** for a self-hosted deployment, for example `http://localhost:3001`. The resulting MCP URL is `${NYXID_BASE_URL}/mcp`.

No API-key variable is required for the happy path. NyxID's hosted MCP transport authenticates Cursor with browser OAuth and then maintains an MCP session. The backend also supports headless API-key authentication through an `X-API-Key` header, but this plugin does not silently put a secret into Cursor configuration; use the NyxID CLI or an explicitly managed integration when a headless setup is required.

## Use it

After installation, ask the agent to find or connect a service. The bundled skill teaches this flow:

1. `nyx__list_connected_services` or `nyx__discover_services` finds the service.
2. `nyx__connect_service` starts a connection without a credential argument.
3. The user completes the returned hosted URL in a browser.
4. `nyx__wait_for_connection` waits for completion.
5. `nyx__search_tools` finds the exact operation and schema.
6. A typed service tool or `nyx__call_tool` performs the proxied call.

The core built-in tools are `nyx__discover_services`, `nyx__list_connected_services`, `nyx__connect_service`, `nyx__wait_for_connection`, `nyx__search_tools`, and `nyx__call_tool`. The server also registers `nyx__ssh_list_services`, `nyx__ssh_exec`, and the Oracle relay tools (`nyx__oracle_pools`, `nyx__oracle_ask`, `nyx__oracle_result`, `nyx__oracle_attach`, `nyx__oracle_extract`, `nyx__oracle_session`). Connected OpenAPI services add tools whose names come from their service slug and operation ID.

The `/mcp` proxy decrypts and injects stored downstream credentials at call time. Credentials, connection secrets, and raw API keys must never be pasted into chat, committed to a repository, or echoed in a command.

When a shell is available, the CLI is an optional fallback:

```bash
nyxid mcp config --tool cursor
nyxid login --base-url https://nyx-api.chrono-ai.fun
```

The first command prints the same `.cursor/mcp.json` shape used by this plugin. The CLI can also manage services and proxy requests; see the repository's [NyxID skill](../../skills/nyxid/SKILL.md).

## Troubleshooting

- **Browser login does not open:** open the MCP authentication link from Cursor manually, finish OAuth, and reconnect the `nyxid` server.
- **Only `nyx__...` tools appear:** connect a service first. Typed downstream tools appear after a successful connection and tool-list refresh; restart Cursor if its cache is stale.
- **The hosted link expired or was cancelled:** call `nyx__connect_service` again and complete the new browser flow.
- **A tool returns an upstream authorization error:** verify that the intended NyxID service is connected and active. Do not paste a replacement secret into chat; reconnect through the hosted flow or the NyxID web console.
- **Self-hosted deployment fails:** set `NYXID_BASE_URL` to the backend API origin, not the separate web-console origin, and confirm `<NYXID_BASE_URL>/mcp` is reachable.

## Publishing

The root `.cursor-plugin/marketplace.json` points Cursor's marketplace tooling at this directory. To publish, use the open-source repository at <https://github.com/ChronoAIProject/NyxID>, submit through <https://cursor.com/marketplace/publish>, and select the `nyxid` entry. Cursor performs a manual review; every update is reviewed again. See [`docs/CURSOR_PLUGIN.md`](../../docs/CURSOR_PLUGIN.md) for the extraction flow if this plugin is mirrored into a standalone public repository.
