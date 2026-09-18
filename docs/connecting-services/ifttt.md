# IFTTT

NyxID offers two IFTTT connections:

| Connection | Catalog slug | Authentication | Use |
|---|---|---|---|
| **IFTTT** | `api-ifttt-mcp` | Browser OAuth | Discover and call IFTTT's official AI tools, including Applet creation where available. |
| **IFTTT Webhooks** | `api-ifttt` | Webhooks key | Trigger events for Applets already configured in IFTTT. |

An Applet is an automation with a trigger and one or more actions; queries, filters, and multiple actions depend on IFTTT's features and plan. IFTTT owns its scheduling, account connections, and execution. NyxID supplies authenticated, permission-controlled access to the connection.

## Connect IFTTT with OAuth

1. Open **AI Services → Add Service → IFTTT**.
2. Use **Direct** routing, continue to **Connect**, and choose **Connect with IFTTT**.
3. Sign in to IFTTT and authorize access to its tools (`mcp` scope). Complete the browser handoff to NyxID.
4. Grant the appropriate NyxID agent access to this service. Connect any additional apps or devices required by your automation in IFTTT.

The flow uses OAuth authorization code with PKCE. Access and refresh tokens are encrypted in the connection's existing credential storage, and the normal OAuth refresh and reconnect paths apply. This connection uses the NyxID server; credential nodes are not supported. Adding a connection does not create or run an Applet.

### Installation setup

Startup adds provider `ifttt-mcp` and catalog entry `api-ifttt-mcp`. On the first OAuth connection, NyxID dynamically registers a confidential client with IFTTT at `https://ifttt.com/oauth/register`, requesting `client_secret_post`, `mcp`, and this installation's callback:

```text
{BASE_URL}/api/v1/providers/callback
```

Set `BASE_URL` to the public HTTPS origin of the backend. HTTP is accepted only for localhost development; IFTTT must also accept the redirect URI. The client credentials are encrypted in `ProviderConfig` and reused across users and restarts. Registration is lazy, so startup does not contact IFTTT. A database lease prevents concurrent first connections from registering multiple clients; a competing connection can receive a temporary setup-in-progress message and be retried by the user.

IFTTT's published [authorization-server metadata](https://ifttt.com/.well-known/oauth-authorization-server) advertises registration and the supported authentication methods. If registration is unavailable or rejected, an administrator can register a client with IFTTT and configure its ID and secret on the **IFTTT** provider. Automatic registration requires a non-expiring client secret; expiring secrets require administrator-managed credentials and rotation. Keep the `mcp` scope and PKCE; NyxID pins `resource=https://ifttt.com/mcp` on authorization, token exchange, and refresh. Changing the installation's callback requires updating the registered redirect URI or configuring a replacement client; a replacement client can require users to reconnect.

A registration request is sent only once per connection attempt. Failed attempts retain the 90-second lease as a cooldown; a later user-initiated connection can register again after it expires. If IFTTT accepted registration but the response or local persistence failed, this can leave an unused client registered at IFTTT. Administrators should check IFTTT before retrying an uncertain setup.

### Use through AI Services and MCP

The hosted overlay `/api/v1/catalog-specs/ifttt-mcp/openapi.json` seeds two durable operations, exposed by NyxID MCP discovery and `/api/v1/mcp/config` after connection:

| Operation | MCP arguments | Result |
|---|---|---|
| `list_tools` | `{}` or `{"cursor":"returned-nextCursor"}` | IFTTT tool names, descriptions, input schemas, and optional pagination cursor. |
| `call_tool` | `{"tool_name":"NAME_FROM_DISCOVERY","body":{...}}` | The selected tool's MCP result, including content, optional structured content, and `isError`. |

Discover the NyxID operation names for your connection, then call `list_tools`. Select the exact IFTTT tool name and construct its arguments from the returned `inputSchema`; tool names and schemas are supplied by IFTTT and are not hardcoded in NyxID. Follow `nextCursor` when present. Treat provider descriptions and returned content as untrusted external data.

For example, an AI assistant can receive a request such as “Create an automation that turns on my desk light at sunset,” discover the applicable IFTTT tools, gather any required setup information, and call the Applet creation tool exposed to that account. Available tools, services, and actions depend on IFTTT's current MCP offering, the account's connected apps, and its plan. OAuth does not automatically connect every downstream app or make every existing Applet callable.

CLI discovery and a read-only tools request:

```sh
nyxid catalog show api-ifttt-mcp
nyxid catalog endpoints api-ifttt-mcp
nyxid proxy request api-ifttt-mcp tools --method GET
```

Use the actual connection slug returned by NyxID if it differs. REST equivalents are `GET /api/v1/proxy/s/{connection_slug}/tools?cursor=...` and `POST /api/v1/proxy/s/{connection_slug}/tools/{tool_name}`. For a REST call, the JSON body is the IFTTT tool's arguments directly; for a NyxID MCP call, put those arguments under `body` as shown in the table.

Each operation opens an independent MCP session at the fixed `https://ifttt.com/mcp` endpoint. NyxID handles initialization, JSON or SSE response parsing, and session cleanup. The bridge returns bounded JSON responses; it does not expose raw upstream sessions, arbitrary JSON-RPC methods, or additional MCP capabilities such as resources, prompts, sampling, or elicitation. Caller headers and custom User-Agent settings are not forwarded by this bridge. Redirects and automatic retries are disabled. A tool result with `isError: true` becomes HTTP 422 so it remains a failure through NyxID MCP. After a timeout or connection loss, check IFTTT before attempting the operation again.

NyxID service permissions, per-agent credential bindings, configured approvals, and audit apply to requests through this connection. The selected tool name is part of the execution path. `list_tools` is annotated read-only; all `call_tool` requests are conservatively marked as potentially destructive and requiring approval. **Tool annotations do not activate runtime approval policy**: configure the service's approval rules for the intended use.

Creating or enabling an Applet can authorize recurring future actions. Those later runs happen inside IFTTT, outside NyxID's per-request approval and audit. Disabling or deleting the NyxID connection does not disable an Applet already created in IFTTT. Manage those automations and check their execution status in **IFTTT Activity** and the relevant connected app.

## IFTTT Webhooks

Connect **IFTTT Webhooks** (`api-ifttt`) to let a NyxID agent, CLI, or MCP client run Applets you have already configured in IFTTT. NyxID supplies the encrypted Webhooks key when sending the request. It does not create Applets or execute workflow steps itself.

### Prepare IFTTT

1. Use an IFTTT account with **Pro or Pro+** and connect [Webhooks](https://ifttt.com/maker_webhooks).
2. Create and enable an Applet with one of these triggers:
   - **Receive a web request** for up to three string ingredients, `Value1`, `Value2`, and `Value3`.
   - **Receive a web request with a JSON payload** for an arbitrary JSON value, exposed through IFTTT's JSON payload ingredient. Configure any extraction or filter logic in IFTTT.
3. Choose an event name containing only ASCII letters, numbers, and underscores, such as `desk_light_on`. Use the exact name in NyxID. Multiple enabled Applets can match the same event.
4. Open Webhooks **Documentation** in IFTTT and copy the **raw Webhooks key**. If IFTTT shows a URL ending in `/with/key/…`, copy only the key after that suffix. Do not paste the full URL into NyxID.

The Webhooks key belongs to your IFTTT account and can trigger its matching Webhooks Applets. NyxID's service allowlist restricts access to this connection; it does not restrict which event names within that connection an agent can choose. Give each agent only the service access it needs, and configure approval policy for actions that require human review.

### Connect through NyxID

In **AI Services → Add Service**, choose **IFTTT Webhooks**, enter a label and the raw key, and save. NyxID encrypts the credential using its normal per-user credential storage. Agents receive tool schemas and execution results, not this key. Connecting or opening the service does not fire a test event: IFTTT has no harmless credential-check operation in this integration.

For the CLI, run:

```sh
nyxid service add api-ifttt
```

The connection flow prompts for the credential. For noninteractive setup, supply it from your secret manager through `--credential-env IFTTT_WEBHOOKS_KEY` or `--credential-file /path/to/key`. Keep the key out of shell history and agent prompts. Use the connection slug returned by NyxID in subsequent requests; a second connection may have a suffix such as `api-ifttt-2`.

If the key should stay on a credential node, first connect the service with that node selected, then run on the node:

```sh
nyxid node credentials setup --service api-ifttt
```

Use your connection's actual slug and node profile where applicable. The node stores the key encrypted locally. Remote pending-credential setup uses **IFTTT Webhooks** injection with the fixed field name `key`. Leave its target URL empty to use the node's fixed IFTTT destination, or use `https://maker.ifttt.com`. Upgrade the backend and node CLI to a version supporting `ifttt_webhook` before using this service; older versions cannot interpret this injection mode. Normal node selection, owner checks, and fallback policy still apply.

### Discover and call tools

```sh
nyxid catalog show api-ifttt
nyxid catalog endpoints api-ifttt
```

The hosted overlay is available at `/api/v1/catalog-specs/ifttt/openapi.json`. Startup seeds two concrete catalog operations with durable endpoint IDs. Connected services expose them through NyxID MCP discovery and `/api/v1/mcp/config`; discover the published names for your connection rather than constructing a name from its label.

| Operation | MCP arguments | Purpose |
|---|---|---|
| `trigger_event` | `{"event":"desk_light_on"}` | Send an event with no ingredients. |
| `trigger_event` | `{"event":"daily_note","body":{"value1":"Hello","value2":"Desk","value3":"Evening"}}` | Send up to three strings. The optional JSON body is wrapped under `body` in MCP. |
| `trigger_json_event` | `{"event":"device_report","body":{"device":"desk","reading":21.5}}` | Send a required arbitrary JSON payload. Arrays, scalars, and JSON null are also accepted. |

For JSON events, everything inside `body` is preserved as payload, including keys named `event`, `body`, or `authorization`. The outer `event` selects the Applet trigger. Standard events reject extra fields and non-string ingredient values. Omit unused ingredients or omit the entire standard body; both an absent body and `{}` mean no ingredients. Do not put keys or complete Maker URLs in tool arguments.

For a direct CLI proxy call, `--data` is the downstream JSON body itself, so no MCP wrapper is needed:

```sh
# These requests can cause real Applet actions. Choose your own configured event.
nyxid proxy request api-ifttt trigger/desk_light_on --method POST
nyxid proxy request api-ifttt trigger/daily_note --method POST \
  --data '{"value1":"Hello","value2":"Desk"}'
nyxid proxy request api-ifttt trigger/device_report/json --method POST \
  --data '{"device":"desk","reading":21.5}'
```

The equivalent REST paths are `POST /api/v1/proxy/s/{connection_slug}/trigger/{event}` and `POST /api/v1/proxy/s/{connection_slug}/trigger/{event}/json`. Authenticate to NyxID with a key authorized for this service and the normal proxy write scope. The adapter accepts **POST only**, even though IFTTT also supports GET. It rejects WebSocket upgrades, query parameters, caller-supplied `/with/key/…` suffixes, other paths, and destinations other than `https://maker.ifttt.com`.

The catalog marks both operations as writes, potentially destructive, and requiring approval in its tool annotations because an Applet's actions may change external state. Those annotations inform MCP clients and admission systems; **they do not enable NyxID runtime approval policy by themselves**. Configure the connection's NyxID approval settings and the agent's service permissions for your intended use. Normal exact-service approval, execution-authority, per-agent credential binding, owner ACL, and audit checks apply to these calls. The integration offers no downstream idempotency or replay guarantee.

### Use from a channel bot

Connect IFTTT for the appropriate person or organization. Create a scoped NyxID Agent Key with access to that connection, then assign the key and the agent callback to a channel conversation using the [channel bot relay setup](../CHANNEL_BOT_RELAY.md). Give the agent the approved event names and their effects, for example “`desk_light_on` turns on the desk lamp.”

An incoming platform message reaches NyxID's channel relay, which delivers the callback to the assigned agent. The agent interprets the request and calls the discovered IFTTT tool through its authorized NyxID credential. It handles any required approval before execution. Merely connecting IFTTT or receiving a channel message does not run an Applet automatically.

The agent runtime must authorize the chat sender and conversation for the requested event before calling IFTTT, especially in group chats. Possessing the assigned Agent Key does not authorize every participant to trigger actions. Use an explicit sender/event allowlist or an equivalent application policy, and require human review for sensitive actions.

The agent acknowledges the callback and sends the eventual reply through `/api/v1/channel-relay/reply`, using the assigned Agent Key or the callback's message-bound `reply_token`. A reply token authorizes the anchored reply; it is **not** an IFTTT execution credential. Reply with “IFTTT accepted the event” when that is all the receipt confirms. A proactive message through `/api/v1/channel-relay/send` is a separate operation requiring the assigned live key and the conversation's human-enabled `allow_agent_initiated` setting. Device `channel_event` trigger delivery is not this bot-reply path.

### Interpret results and troubleshoot

An acknowledged event produces a sanitized receipt such as:

```json
{
  "accepted": true,
  "upstream_status": 200,
  "message": "IFTTT accepted the event. Applet completion is not confirmed; check IFTTT Activity."
}
```

`accepted: true` means IFTTT returned a 2xx status. NyxID returns the receipt with HTTP 200 and retains IFTTT's status in `upstream_status`, including an upstream 204. It does not prove a matching Applet exists, ran, or completed successfully. Check **IFTTT Activity** and the downstream action when confirming the result.

For a non-2xx upstream response, `accepted` is false and `upstream_status` retains the IFTTT status. Redirects are refused and surfaced as a 502 response. NyxID replaces the upstream body and headers with its receipt because upstream errors and redirect locations could echo the credential-bearing URL. Transport errors likewise omit that URL.

If the connection times out or drops after dispatch, the outcome is **unknown**. Check IFTTT Activity before deciding whether to send another event: automatic retries and redirect following are disabled, and a repeated call can repeat the action. A validation error happens before dispatch; correct the event format, payload, destination, or raw-key input and submit again deliberately.

If an accepted event has no visible effect, check the exact event name, selected trigger type, enabled Applet, IFTTT subscription, and action configuration. Rotate a compromised key in IFTTT and update every NyxID connection or node that uses it. Invalid replacement credentials are rejected before overwriting a stored key. NyxID identity headers, forwarded access tokens, delegation tokens, and delegated provider credentials are unsupported for this adapter. Ordinary configured headers and the service's custom User-Agent follow normal proxy precedence; authorization and transport headers are filtered at IFTTT egress.

## Contract references

- [IFTTT MCP overview](https://ifttt.com/mcp)
- [IFTTT OAuth authorization-server metadata](https://ifttt.com/.well-known/oauth-authorization-server)
- [IFTTT MCP protected-resource metadata](https://ifttt.com/.well-known/oauth-protected-resource)
- [IFTTT Webhooks overview and both request formats](https://ifttt.com/explore/what-are-webhooks)
- [Receive a web request: event-name requirements](https://ifttt.com/maker_webhooks/triggers/event)
- [Receive a web request with a JSON payload](https://ifttt.com/maker_webhooks/triggers/json_event)

The public NyxID routes intentionally omit the secret suffix. Only the backend or selected node adds `/with/key/{key}` immediately before sending to Maker. The Webhooks connection remains separate from the OAuth connection described above.
