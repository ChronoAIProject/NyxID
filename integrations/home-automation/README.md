# Home automation bridge

For the user-facing MQTT connection procedure, see [Connect MQTT devices](../../docs/connecting-services/mqtt.md). For NyxID's service terminology, see [Service types and connections](../../docs/connecting-services/service-types.md).

This bridge gives a NyxID agent three stable operations over a household's mapped Home Assistant entities and MQTT topics: list devices, read one state, and turn an allowed device on or off. It runs on the user's machine beside a NyxID credential node. Home Assistant and MQTT credentials stay on that machine. Each user owns their mapping and can update names or allowed actions in `config.json`.

## Run locally

Requires Node.js 20 or newer and a machine that can reach Home Assistant or the MQTT broker.

```bash
cd integrations/home-automation
npm ci
HA_TOKEN="$HA_TOKEN" npm run setup
```

The setup prompts for Home Assistant, MQTT, or both. For Home Assistant it reads `/api/states` using `HA_TOKEN`, lets you choose supported entities, and starts them read-only unless you explicitly select devices for on/off control. For MQTT it asks for exact state and optional command topics and payloads. It writes `config.json` and a random `bridge.key` with owner-only permissions. Existing config files are never overwritten. You can edit device names, mappings, and allowed actions in `config.json` later; changes require a restart. For a manual setup, start from `config.example.json`. Configure only MQTT devices when using the MQTT catalog entry for an MQTT-only service; the bridge exposes every mapped device.

Set `HA_TOKEN` to a Home Assistant long-lived access token if using Home Assistant. Set `MQTT_USERNAME` and `MQTT_PASSWORD` if the broker requires them. Keep these values out of `config.json` and shell history. When configuring manually, set `BRIDGE_KEY` to a random value of at least 32 characters instead of using `bridge.key`.

```bash
BRIDGE_CONFIG=./config.json HA_TOKEN="$HA_TOKEN" \
  MQTT_USERNAME="$MQTT_USERNAME" MQTT_PASSWORD="$MQTT_PASSWORD" npm start
```

The bridge listens on `127.0.0.1:8787` by default. The MQTT broker should use `mqtts://` when it is not on a trusted local network. MQTT state is null until a matching message arrives; a broker with retained state messages provides immediate state after reconnect. A successful MQTT publish confirms broker receipt, not physical device completion.

## Connect NyxID

Register and start a [NyxID credential node](../../docs/NYXID_NODE.md) on the same machine. For MQTT, add the catalog service from a logged-in NyxID CLI:

```bash
nyxid service add api-mqtt --terminal \
  --label "My MQTT devices" \
  --endpoint-url http://127.0.0.1:8787 \
  --via-node <node-id>
```

On the node machine, store the bridge key under the **connection slug returned by `service add`** (which may have a numeric suffix). The command prompts for the bridge URL and key; read the key from `bridge.key` when prompted:

```bash
nyxid node credentials setup --service <connection-slug>
```

Restart the node daemon if it is already running, then verify the service through `nyxid service list` and agent tool discovery. The hosted OpenAPI spec gives agents named operations and labels the write operation as approval-requiring for clients. That label does not enable NyxID runtime approval; configure the policy separately. NyxID still enforces its normal service access, agent grants, and audit controls.

For Home Assistant without MQTT, add a custom node-routed service with the same bridge URL and hosted spec:

```bash
nyxid service add --custom --terminal \
  --slug home-automation --label "My home" \
  --endpoint-url http://127.0.0.1:8787 \
  --auth-method bearer --via-node <node-id> \
  --openapi-spec-url https://<your-nyxid-host>/api/v1/catalog-specs/home-automation/openapi.json
```

On the node machine, add the bearer key from `bridge.key` under the returned connection slug:

```bash
nyxid node credentials add --service <connection-slug> \
  --url http://127.0.0.1:8787 --header Authorization \
  --secret-format bearer
```

For a family, create the NyxID service under an organization and grant only the members who should use it. Keep security-critical devices such as locks, alarms, and garage openers out of the initial bridge mapping; this version only handles simple on/off actions and does not model their safeguards.
