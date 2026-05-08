# Reach a Localhost API from a Cloud-Hosted Agent (Node Proxy)

Expose an internal API running on a private host (home server, on-prem VM, anything not reachable from the public internet) to a cloud-hosted agent — without VPN, port forwarding, or a tunneling service. NyxID's `Credential Node` is a small agent installed on the private host that opens an outbound WebSocket to NyxID. NyxID routes proxy requests through that WebSocket; the node injects credentials locally and forwards to the localhost API.

```
Cloud agent ──(HTTPS)──►  NyxID  ──(outbound WSS)──►  Node on private host  ──►  localhost:3000
                                                       (injects auth header)
```

The node never opens an inbound port. Credentials never leave the private host. Callers see the same proxy URL pattern as for any public API (`/api/v1/proxy/s/<slug>/...`); node routing is invisible.

This guide uses Grafana on `localhost:3000` as the worked example.

## Prerequisites

- A NyxID account and a logged-in `nyxid` CLI on your laptop. Follow [Step 0 of the n8n quickstart](n8n.md#step-0--get-nyxid-running-and-create-an-agent-key) if not already done.
- SSH access to the private host running the localhost API.
- The downstream API token (e.g. a Grafana service-account token) ready to enter on that host.
- Outbound WebSocket connectivity from the private host to NyxID (port 443 for hosted, 3001 for self-host).

## Procedure

### 1. Mint a node registration token

In the web console, open `Credential Nodes` → `Register Node`. Enter a name (for example, `home-server`) and click `Create`. Copy the displayed `nyx_nreg_…` token; it is shown once and expires after one hour by default (`NODE_REGISTRATION_TOKEN_TTL_SECS`).

### 2. Install and register the node on the private host

SSH into the private host. Install the `nyxid` CLI per [docs/SETUP.md](../SETUP.md), then register the node using the token from Step 1:

```bash
nyxid node register \
  --token nyx_nreg_<token> \
  --url wss://<your-nyxid-host>/api/v1/nodes/ws
```

For a self-host instance running on the same machine, use `ws://localhost:3001/api/v1/nodes/ws`.

The CLI stores the long-lived auth token (`nyx_nauth_…`) and HMAC signing secret in `~/.nyxid-node/`. Both values are shown once during registration. To use the OS keychain instead of a file, append `--keychain` (macOS Keychain, Windows Credential Manager, or Linux Secret Service). Migrate later with `nyxid node migrate --to keychain`.

### 3. Register the service in NyxID, routed through the node

From your laptop, look up the registered node's UUID:

```bash
nyxid node list
# ID                                     Name           Owner   Status   Last Seen
# 33333333-cccc-…                        home-server    you     online   2026-05-08 14:01:22
```

Add the localhost service as a custom NyxID service, routed through the node. Setting `--via-node` skips the credential prompt; the node, not NyxID, holds the upstream credential.

```bash
nyxid service add --custom \
  --slug grafana \
  --label "Home Grafana" \
  --endpoint-url "http://localhost:3000" \
  --auth-method bearer \
  --auth-key-name "Authorization" \
  --via-node 33333333-cccc-…
```

`--endpoint-url` is the URL the **node** will dial. Because the node runs on the private host, `http://localhost:3000` resolves to Grafana on that host. If you later move the node to a separate machine on the same LAN, update the endpoint to `http://<lan-ip>:3000`.

The CLI prints the slug NyxID landed on (`grafana` on a fresh account; suffixed with `-2`, `-3`, or a random suffix if you already have a `grafana` service).

### 4. Add the upstream credential on the node

Back on the private host, add the Grafana token to the node's local credential store:

```bash
nyxid node credentials add \
  --service grafana \
  --url "http://localhost:3000" \
  --header "Authorization" \
  --secret-format bearer
```

The CLI prompts for the token; it is encrypted and stored in `~/.nyxid-node/`. The token never leaves this host. NyxID stores only metadata (the binding between service slug `grafana` and node `home-server`).

### 5. Start the node

For a manual test, run the node in the foreground:

```bash
nyxid node start
```

Look for `Connected to NyxID` and a heartbeat ping every ~30 seconds.

For long-running deployments, install the node as a system service (launchd on macOS, systemd on Linux) and start it in the background:

```bash
nyxid node daemon install
nyxid node daemon start
nyxid node daemon logs --follow
```

The daemon restarts on boot and on failure. Multi-instance deployments (e.g. one node per environment) use `--profile {name}` on every node command; profiles isolate config under `~/.nyxid-node/profiles/{name}/` and use distinct service labels.

### 6. Verify

From your laptop or any cloud-hosted environment with NyxID network access, call the proxy URL:

```bash
NYX_API_KEY="nyx_…"
NYXID_BASE="https://<your-nyxid-host>"

curl -sf "$NYXID_BASE/api/v1/proxy/s/grafana/api/dashboards/home" \
  -H "X-API-Key: $NYX_API_KEY"
```

A `200 OK` with the Grafana JSON response confirms the route. The token never left the private host.

NyxID's audit log records each node-routed request with `routed_via: node` and the `node_id` set to the home-server UUID. Filter on these fields under `Admin` → `Audit Log` to confirm node routing was used.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `8002 NodeOffline` from the proxy | Node has not sent a heartbeat in `NODE_HEARTBEAT_TIMEOUT_SECS` (default 90 s) | Confirm the daemon is running (`nyxid node daemon status`); check outbound WebSocket connectivity |
| `8003 NodeRegistrationFailed` | Registration token expired or already consumed | Mint a new token (Step 1) |
| `401` from the upstream API | Local credential is wrong or revoked | Re-add the credential on the node: `nyxid node credentials add --service grafana --url http://localhost:3000 --header Authorization --secret-format bearer` (overwrites the existing entry) |
| Connection times out at registration | Outbound WebSocket blocked by firewall or proxy | Allow `wss://<your-nyxid-host>` (port 443 hosted, 3001 self-host) outbound from the private host |
| Audit log shows the request was routed direct (not via node) | Service is missing the `node_id` binding | Re-add the service with `--via-node <node-id>`, run `nyxid service update <service-id> --node-id <node-id>`, or open `AI Services` → `[your service]` and bind the node in the `Your Routing` card |

## Operational notes

- **HMAC signing.** Frames between NyxID and the node are signed with a per-node secret (`NODE_HMAC_SIGNING_ENABLED=true` by default). Tamper-detection is essentially free; do not disable.
- **Multi-node failover.** Bind two nodes to the same service with priorities. If the primary's WebSocket goes idle past `NODE_HEARTBEAT_TIMEOUT_SECS`, traffic shifts to the secondary. See [docs/NODE_PROXY.md#multi-node-failover](../NODE_PROXY.md#multi-node-failover).
- **Streaming responses.** Long-running streams (e.g. LLM chat completions through a node) are bounded by `NODE_MAX_STREAM_DURATION_SECS` (default 5 min).
- **Multi-instance.** Run two nodes on one machine via `nyxid node register --profile <name>`; each profile gets its own daemon and config directory.

## Reference

- **Node protocol, security model, metrics, admin endpoints**: [docs/NODE_PROXY.md](../NODE_PROXY.md)
- **Self-hosted AI gateway (OpenClaw) over a node**: `nyxid node openclaw connect --url http://localhost:18789`
- **Other quickstarts**: [n8n](n8n.md) · [Claude Code per-agent keys](claude-code.md) · [MCP wrapping](mcp-wrapping.md)
