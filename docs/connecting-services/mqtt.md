# Connect MQTT devices

The `MQTT` catalog entry (`api-mqtt`) lets an agent read mapped device states and send allowed on/off commands. A local HTTP bridge connects to your MQTT broker. A NyxID credential node sends requests to that bridge, so the broker and bridge do not need public ports.

```text
Agent -> NyxID -> credential node -> local HTTP bridge -> MQTT broker
```

The bridge runs on the same computer as the node and listens on `127.0.0.1:8787`. The broker can be elsewhere on the network. NyxID stores the connection and node binding; the node stores the bridge key, and the bridge uses your broker credentials locally.

## Prepare the bridge

Use a computer with Node.js 20 or newer and access to the broker. From the repository root, run:

```sh
cd integrations/home-automation
npm ci
npm run setup
```

Choose MQTT when prompted. Add each device's exact state topic. Leave the command topic blank for a read-only device. To permit on/off actions, enter the exact command topic and the payload for each action. The setup writes `config.json` and a random `bridge.key` with owner-only permissions. It does not replace an existing configuration.

The bridge exposes every device in `config.json`. If you also configure Home Assistant devices, they will be reachable through this MQTT catalog connection. Use an MQTT-only configuration when you want an MQTT-only connection.

Set `MQTT_USERNAME` and `MQTT_PASSWORD` in the bridge environment if the broker requires them, then start the bridge:

```sh
npm start
```

The bridge logs `Home automation bridge listening on 127.0.0.1:8787`. Use `mqtts://` for a broker outside a trusted local network. Keep broker credentials and `bridge.key` out of agent prompts and shell history.

## Connect through a node

Register and start a [NyxID credential node](../NYXID_NODE.md) on the bridge computer. Confirm that the node is online in **Credential Nodes** or with `nyxid node list`.

In **AI Services -> Add Service**, choose **MQTT**, select **Via Node**, select that node, and create the connection. The current form also offers **Direct**; a hosted NyxID server cannot reach the bridge's loopback address. Use the node route for this local bridge.

From a logged-in CLI, the equivalent command is:

```sh
nyxid service add api-mqtt --terminal \
  --label "My MQTT devices" \
  --endpoint-url http://127.0.0.1:8787 \
  --via-node NODE_NAME_OR_ID
```

Copy the **connection slug** returned by NyxID. It can differ from `api-mqtt`, for example when you add a second MQTT connection. On the node computer, use that exact slug:

```sh
nyxid node credentials setup --service CONNECTION_SLUG
```

When prompted, enter `http://127.0.0.1:8787` as the instance URL and the raw value from `bridge.key` as the credential. The node adds the `Bearer` prefix. If the node daemon was already running, restart it with `nyxid node daemon restart` so it loads the new credential.

## Check the connection

Use the connection slug from the previous step:

```sh
nyxid catalog endpoints api-mqtt
nyxid proxy request CONNECTION_SLUG devices --method GET
```

The proxy response contains a `devices` array with the configured device IDs and states. A state can be `null` until the bridge receives a matching MQTT message. A retained state message can supply an initial value after reconnect.

The hosted OpenAPI spec defines `list_devices`, `get_device_state`, and `set_device_state`. It marks the action operation as destructive and requiring approval for tool discovery. Those markers do not turn on NyxID runtime approval by themselves. Configure the connection's approval policy and grant each agent only the service access it needs before allowing writes. The bridge also rejects actions that are absent from the device's local `actions` list.

An accepted on/off request confirms that the broker received the publish. It does not confirm that the physical device changed state. Check the device's state topic when you need that confirmation. Avoid mapping locks, alarms, and similar devices until you have a separate safeguard for their actions.

For bridge configuration details and the Home Assistant custom-service path, see the [bridge README](../../integrations/home-automation/README.md). For the difference between a catalog entry, a connection, and a service type, see [Service types and connections](service-types.md).
