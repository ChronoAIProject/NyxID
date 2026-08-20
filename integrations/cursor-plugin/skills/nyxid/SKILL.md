---
name: nyxid
description: Use NyxID's hosted MCP server to discover connected services, connect a service through a browser-hosted link, and call downstream APIs with credentials kept out of the agent context.
---

# NyxID for Cursor

Use NyxID when the user wants a credential-backed downstream API call, service discovery, or a connection to an external service. NyxID stores the provider credential and injects it when it proxies a tool call. The agent should see the downstream response, not the provider secret.

## MCP-first workflow

The plugin configures the hosted `${NYXID_BASE_URL}/mcp` endpoint. Authenticate in the browser when Cursor prompts. The built-in MCP tools are:

- `nyx__discover_services` lists catalog services that are not connected. Use its optional `query` or `category` filters when useful.
- `nyx__list_connected_services` lists the user's connected services and availability.
- `nyx__connect_service` starts a connection for a `service_id` returned by discovery. Omit `credential` by default: when a human connection is needed, the result contains a hosted connection URL and `connect_link_id`.
- `nyx__wait_for_connection` waits on that `connect_link_id`; call it after the user completes the hosted flow, then retry the original operation.
- `nyx__search_tools` searches the connected service tools and returns their input schemas.
- `nyx__call_tool` invokes a discovered tool by its full name. Pass `arguments_json` as a JSON string matching the schema returned by `nyx__search_tools`.

Per-service tools may also be listed directly after a connection. Their names are derived from the service slug and OpenAPI operation ID; do not guess a name when `nyx__search_tools` can find it.

## Connecting without handling secrets

Never ask the user to paste a raw API key, OAuth token, password, or private credential into chat, a prompt, or a repository. Start with `nyx__connect_service` using only the discovered `service_id`. Tell the user to open the returned hosted URL and finish the provider flow in the browser. Then call `nyx__wait_for_connection` with the returned `connect_link_id`. Do not repeat or log the URL's token, and do not put credentials in command arguments or files.

If the user already has a service connected, skip connection and use `nyx__list_connected_services`, `nyx__search_tools`, and the matching service tool. If the hosted link expires or is cancelled, start a new connection rather than requesting the secret.

## Calling a downstream API

1. Use `nyx__list_connected_services` to confirm the service, or `nyx__discover_services` to find one.
2. If it is not connected, use the hosted connection flow above.
3. Search with `nyx__search_tools` and inspect the returned `inputSchema`.
4. Prefer a typed per-service tool. Otherwise invoke it through `nyx__call_tool` with exact JSON arguments.
5. Report the downstream response and any service error without exposing credential material.

NyxID's MCP proxy supports the user's connected catalog and user-managed services. A service with no OpenAPI operation may still be available through its generic proxy tool; use the search results as the source of truth.

## Optional shell fallback

Use the `nyxid` CLI only when a shell is available and the user wants a terminal workflow. Install it with the repository's documented installer, then authenticate once:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
nyxid login --base-url https://nyx-api.chrono-ai.fun
```

The CLI stores its session locally and can list services, manage keys, and issue proxy requests. Prefer the MCP tools in Cursor because they preserve the browser OAuth flow and keep credentials in NyxID. Never print a stored token or ask the user to paste one into this chat.

## Oracle and SSH tools

The hosted MCP server also exposes these NyxID-owned tools when the account has the corresponding capability:

- SSH: `nyx__ssh_list_services`, `nyx__ssh_exec`.
- Browser-backed Oracle relay: `nyx__oracle_pools`, `nyx__oracle_ask`, `nyx__oracle_result`, `nyx__oracle_attach`, `nyx__oracle_extract`, `nyx__oracle_session`.

Use `nyx__oracle_pools` before submitting Oracle work. Poll a pending task with `nyx__oracle_result`. Use `nyx__oracle_attach` only when the user supplies an existing ChatGPT conversation URL and asks to import it.
