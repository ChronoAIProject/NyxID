# WebSocket Auth-Frame Injection

NyxID can inject a held downstream credential into a WebSocket frame after the HTTP upgrade. This is for protocols that do not authenticate the upgrade request itself and instead send a challenge frame that expects a credential-bearing response frame.

Rules live on user-owned `UserService.ws_frame_injections` and, for platform
catalog defaults, `DownstreamService.ws_frame_injections`. `direction` is the
trigger direction: `downstream` means the rule matches frames sent by the
downstream service toward the client, and NyxID sends the injected frame back
to the downstream service. At proxy time, a non-empty user-owned rule list wins;
catalog-backed services fall back to catalog rules only when the user list is
empty.

## Home Assistant

Home Assistant sends an auth challenge immediately after `/api/websocket` upgrades:

```json
{"type":"auth_required","ha_version":"..."}
```

A Home Assistant rule:

```json
{
  "trigger": {
    "json_field_equals": {
      "path": "$.type",
      "value": "auth_required"
    }
  },
  "template": "{\"type\":\"auth\",\"access_token\":\"${credential}\"}",
  "frame_kind": "text",
  "consume_trigger": true,
  "direction": "downstream"
}
```

Expected on-wire behavior:

```text
Downstream -> NyxID: {"type":"auth_required","ha_version":"..."}
NyxID -> Downstream: {"type":"auth","access_token":"<held credential>"}
Downstream -> Client: {"type":"auth_ok"}
```

With `consume_trigger: true`, NyxID hides the challenge from the client. The client sees only the post-auth downstream frames.

## User-Level Configuration

For Home Assistant or any custom WebSocket-authenticated service, configure the
rules on the user service itself:

```bash
nyxid service add --custom \
  --slug my-ha \
  --label "Home Assistant" \
  --endpoint-url "https://ha.local:8123/api" \
  --auth-method bearer \
  --auth-key-name Authorization \
  --credential-env HA_TOKEN \
  --ws-frame-preset home-assistant
```

Existing user services can be updated or cleared:

```bash
nyxid service update "$USER_SERVICE_ID" --ws-frame-preset home-assistant
nyxid service update "$USER_SERVICE_ID" --ws-frame-clear
```

The REST endpoint is the user-service update route:

```bash
curl -X PUT "$NYXID_BASE_URL/api/v1/user-services/$USER_SERVICE_ID" \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"ws_frame_injections":[{
    "trigger":{"json_field_equals":{"path":"$.type","value":"auth_required"}},
    "template":"{\"type\":\"auth\",\"access_token\":\"${credential}\"}",
    "frame_kind":"text",
    "consume_trigger":true,
    "direction":"downstream"
  }]}'
```

The route is `PUT /api/v1/user-services/{service_id}`. Send
`{"ws_frame_injections":[]}` to clear the user-owned rules.

## Catalog Defaults

Platform operators can set catalog-level defaults with the admin service editor
or `PUT /api/v1/services/{service_id}`. These defaults apply to catalog-backed
user services that have no user-owned rules. Custom endpoints do not have a
catalog fallback, so configure them through the user-level path above.

## Limits And Security

Each service can define at most 4 WebSocket frame injection rules. Each template is limited to 4096 bytes and only supports `${credential}` interpolation.

Each WebSocket connection can fire at most 8 injected frames. This prevents repeated challenge frames from causing unbounded credential replay.

Credentials are never included in logs, errors, or audit payloads. Successful injection emits the metadata-only audit event `ws_frame_auth_injected` with the service id, trigger kind, frame index, and node routing metadata when applicable.
