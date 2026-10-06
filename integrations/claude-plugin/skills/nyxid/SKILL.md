---
name: nyxid
description: Use NyxID's hosted MCP server to list connected services, connect a service through a browser-hosted link, and call downstream APIs with credentials kept out of the conversation.
---

# NyxID for Claude

Use NyxID when the user wants to call an API that needs their credentials, see which services they have connected, or connect a new service. NyxID stores the provider credential and injects it when it proxies a tool call. Claude sees the downstream response, never the provider secret.

## Tools

The plugin connects Claude to NyxID's MCP server at `https://nyx-api.chrono-ai.fun/mcp`. The user signs in to NyxID in the browser the first time a tool is used. The core tools are:

- `nyx__list_connected_services` lists the user's connected services and whether each is currently available.
- `nyx__discover_services` lists catalog services the user has not connected yet. Use its optional `query` or `category` filters.
- `nyx__connect_service` starts a connection for a `service_id` returned by discovery. Omit `credential`: the result contains a hosted connection URL and a `connect_link_id`.
- `nyx__wait_for_connection` waits on that `connect_link_id` after the user completes the hosted flow.
- `nyx__search_tools` searches the connected services' tools and returns their input schemas.
- `nyx__call_tool` invokes a tool found by `nyx__search_tools`. Pass `arguments_json` as a JSON string matching the returned schema.

Typed per-service tools may also appear directly after a connection. Their names come from the service slug and operation ID; use `nyx__search_tools` instead of guessing a name.

## Connecting without handling secrets

Never ask the user to paste an API key, OAuth token, password, or other credential into the conversation. Call `nyx__connect_service` with only the `service_id`, give the user the returned hosted URL to complete in their browser, then call `nyx__wait_for_connection` with the `connect_link_id`. Report success only after the wait confirms the connection. If a link expires or is cancelled, start a new one rather than asking for the secret.

## Calling a downstream API

1. Confirm the service with `nyx__list_connected_services`, or find it with `nyx__discover_services`.
2. If it is not connected, use the hosted connection flow above.
3. Find the operation with `nyx__search_tools` and read its `inputSchema`.
4. Prefer a typed per-service tool; otherwise call it through `nyx__call_tool` with exact JSON arguments.
5. Report the downstream response or error without exposing credential material.

Ask the user before calling an operation that sends messages, changes data, or deletes anything, and describe exactly what it will do.

## Out of scope

NyxID cannot disconnect services, delete connections, or change account settings from this plugin; direct the user to the NyxID web app at https://nyx.chrono-ai.fun for those. Only the signed-in user's own services are visible; never attempt to access another person's account.
