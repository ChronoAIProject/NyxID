# IFTTT Webhooks

Connect **IFTTT Webhooks** (`api-ifttt`) to let a NyxID agent, CLI, or MCP client run Applets you have already configured in IFTTT. NyxID supplies the encrypted Webhooks key when sending the request. It does not create Applets or execute workflow steps itself.

## Prepare IFTTT

1. Use an IFTTT account with **Pro or Pro+** and connect [Webhooks](https://ifttt.com/maker_webhooks).
2. Create and enable an Applet with one of these triggers:
   - **Receive a web request** for up to three string ingredients, `Value1`, `Value2`, and `Value3`.
   - **Receive a web request with a JSON payload** for an arbitrary JSON value, exposed through IFTTT's JSON payload ingredient. Configure any extraction or filter logic in IFTTT.
3. Choose an event name containing only ASCII letters, numbers, and underscores, such as `desk_light_on`. Use the exact name in NyxID. Multiple enabled Applets can match the same event.
4. Open Webhooks **Documentation** in IFTTT and copy the **raw Webhooks key**. If IFTTT shows a URL ending in `/with/key/…`, copy only the key after that suffix. Do not paste the full URL into NyxID.

The Webhooks key belongs to your IFTTT account and can trigger its matching Webhooks Applets. NyxID's service allowlist restricts access to this connection; it does not restrict which event names within that connection an agent can choose. Give each agent only the service access it needs, and configure approval policy for actions that require human review.

## Connect through NyxID

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

## Discover and call tools

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

## Use from a channel bot

Connect IFTTT for the appropriate person or organization. Create a scoped NyxID Agent Key with access to that connection, then assign the key and the agent callback to a channel conversation using the [channel bot relay setup](../CHANNEL_BOT_RELAY.md). Give the agent the approved event names and their effects, for example “`desk_light_on` turns on the desk lamp.”

An incoming platform message reaches NyxID's channel relay, which delivers the callback to the assigned agent. The agent interprets the request and calls the discovered IFTTT tool through its authorized NyxID credential. It handles any required approval before execution. Merely connecting IFTTT or receiving a channel message does not run an Applet automatically.

The agent runtime must authorize the chat sender and conversation for the requested event before calling IFTTT, especially in group chats. Possessing the assigned Agent Key does not authorize every participant to trigger actions. Use an explicit sender/event allowlist or an equivalent application policy, and require human review for sensitive actions.

The agent acknowledges the callback and sends the eventual reply through `/api/v1/channel-relay/reply`, using the assigned Agent Key or the callback's message-bound `reply_token`. A reply token authorizes the anchored reply; it is **not** an IFTTT execution credential. Reply with “IFTTT accepted the event” when that is all the receipt confirms. A proactive message through `/api/v1/channel-relay/send` is a separate operation requiring the assigned live key and the conversation's human-enabled `allow_agent_initiated` setting. Device `channel_event` trigger delivery is not this bot-reply path.

## Interpret results and troubleshoot

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

- [IFTTT Webhooks overview and both request formats](https://ifttt.com/explore/what-are-webhooks)
- [Receive a web request: event-name requirements](https://ifttt.com/maker_webhooks/triggers/event)
- [Receive a web request with a JSON payload](https://ifttt.com/maker_webhooks/triggers/json_event)

The public NyxID routes intentionally omit the secret suffix. Only the backend or selected node adds `/with/key/{key}` immediately before sending to Maker. This version does not connect to IFTTT's separate OAuth MCP server or import upstream MCP tools.
