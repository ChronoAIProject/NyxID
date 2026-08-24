---
name: nyxid-connect
description: Connect a catalog service through NyxID without placing its credential in the agent context.
---

Use NyxID's MCP tools to connect a service for the current user.

1. Call `nyx__discover_services` and show a concise list of matching services. Ask which service the user wants if the request did not identify one.
2. Call `nyx__connect_service` with the selected `service_id` only. Do not request or pass a raw API key, OAuth token, password, or other credential.
3. If the result is `pending_connection`, give the user the hosted connection URL and wait for them to complete it in their browser. Then call `nyx__wait_for_connection` with `connect_link_id`.
4. After a successful connection, call `nyx__search_tools` for the service and report that its tools are available. Do not claim a downstream call succeeded until you actually call one when the user asks for verification.
