# Reach localhost from a cloud-hosted agent (NyxID Node Proxy)

**TL;DR** — I have a Grafana running on my home server's `localhost:3000`. No public URL. No port forwarding. I want my cloud-hosted Claude Code (or n8n, or anything pointed at NyxID) to query its API. Run a NyxID **node** on the home server. It opens an outbound WebSocket to NyxID. NyxID routes proxy requests through it. Done.

```
Cloud agent ──(HTTPS)──►  NyxID  ──(outbound WSS)──►  Node on home server  ──►  localhost:3000
                                                       (injects bearer token)
```

The node never opens an inbound port. Credentials never leave the home server. Same NyxID Agent Key and same proxy URL pattern (`/api/v1/proxy/s/<slug>/...`) you'd use for any public API — the node-routing is invisible to the caller.

> This walkthrough assumes you've done [Step 0 of the n8n quickstart](n8n.md#step-0--get-nyxid-running-and-create-an-agent-key) — i.e., you have NyxID reachable somewhere and a logged-in `nyxid` CLI on your laptop. If not, see [docs/SETUP.md](../SETUP.md) first.

---

## The problem

Three things I'd otherwise reach for, and why they didn't fit:

- **Cloudflare Tunnel** — gives a public hostname, but doesn't inject the Authorization header, so my agent has to hold the bearer token. That's the credential I most don't want a hosted LLM to hold.
- **VPN (WireGuard / Tailscale)** — fine for me on a laptop, but my n8n is in a hosted Docker container that I'd rather not put inside my private mesh.
- **Port-forwarding 3000 to the public internet** — bypasses my firewall by design. Hard pass.

What I actually want: my agent calls a normal NyxID URL with a NyxID Agent Key; NyxID routes the request through a process I control on the home server; that process injects the real bearer token and forwards to `localhost:3000`. Token never leaves the home server, no inbound port, no public hostname.

That's what a NyxID node does.

---

## Setup

### 1. Mint a node registration token

Web UI: **Credential Nodes → Register Node**, name it (e.g. `home-server`), click **Create**. Copy the `nyx_nreg_...` token (shown once, expires in 1 hour).

### 2. Install and register the node on your home server

SSH into the home server (or whichever machine sits next to the localhost service). Install the `nyxid` CLI per [docs/SETUP.md](../SETUP.md), then:

```bash
nyxid node register \
  --token nyx_nreg_<your-reg-token> \
  --url wss://<your-nyxid-host>/api/v1/nodes/ws
# (use ws://localhost:3001/api/v1/nodes/ws for self-host on the same machine)
```

That stores the long-lived auth token (`nyx_nauth_...`) and HMAC signing secret in `~/.nyxid-node/`. Both are shown once during registration — back them up.

> **OS keychain instead of a file:** add `--keychain` (uses macOS Keychain, Windows Credential Manager, or Linux Secret Service). Migration: `nyxid node migrate --to keychain`.

### 3. Add the service in NyxID, routed through the node

From your laptop (where you're logged into the `nyxid` CLI as the user), grab the node id:

```bash
nyxid node list
# ID                                    Name           Owner   Status   Last Seen
# 33333333-cccc-...                     home-server    you     online   2026-05-08 14:01:22
```

Then add Grafana as a custom service routed through that node. Setting `--via-node` skips the credential prompt — credentials live on the node, not in NyxID:

```bash
nyxid service add --custom \
  --slug grafana \
  --label "Home Grafana" \
  --endpoint-url "http://localhost:3000" \
  --auth-method bearer \
  --auth-key-name "Authorization" \
  --via-node 33333333-cccc-...
```

`--endpoint-url` is `http://localhost:3000` because that's the URL the **node** will dial — it's relative to the node's network, not to NyxID's. The CLI prints the slug NyxID landed on (`grafana` on a fresh account, or `grafana-2` if you already had one).

### 4. Add the local credential on the node

Back on the home server:

```bash
nyxid node credentials add \
  --service grafana \
  --url "http://localhost:3000" \
  --header "Authorization" \
  --secret-format bearer
# Prompts for the Grafana token. Stored encrypted in the node's local store.
```

The token never leaves this machine. NyxID sees only that "service `grafana` is bound to node `home-server`."

### 5. Start the node

Foreground (good for testing):

```bash
nyxid node start
```

You should see `Connected to NyxID` and a heartbeat ping every ~30s.

Background (the actual answer):

```bash
nyxid node daemon install
nyxid node daemon start
nyxid node daemon logs --follow   # tail
```

This installs a launchd LaunchAgent on macOS or a systemd user unit on Linux. It restarts on boot and on failure.

### 6. Call it from anywhere

From your laptop, your cloud-hosted n8n, or your CI runner — same proxy URL, same Agent Key:

```bash
curl "$NYXID_BASE/api/v1/proxy/s/grafana/api/dashboards/home" \
  -H "X-API-Key: $NYX_API_KEY"
```

The response is whatever Grafana returns. The bearer token never leaves your home server. The audit log shows `routed_via: node, node_id: <home-server-uuid>`.

---

## Before vs After

| | Before | After |
|---|---|---|
| Public URL | Cloudflare Tunnel hostname | None — node dials out only |
| Inbound port open | Tunnel daemon (Cloudflare) | None |
| Where the bearer token lives | Wherever the agent runs | Only on the home server |
| Credential rotation | Update every agent | `nyxid node credentials add` to overwrite, restart node |
| Caller URL | Cloudflare hostname | Same `/api/v1/proxy/s/<slug>/...` as any public service |
| Agent's view of routing | Different per service | Identical — node-routing is invisible |

---

## Gotchas

**`--endpoint-url` is dialed by the node, not by NyxID.** Putting `http://localhost:3000` is correct; the node interprets `localhost` as itself. If you ever migrate the node to a separate machine that reaches the service over the LAN, update to `http://<lan-ip>:3000`.

**The registration token expires in 1 hour by default** (`NODE_REGISTRATION_TOKEN_TTL_SECS=3600`). If you mint one and don't use it in time, mint another. Admin-issued tokens still work even after the admin's role is revoked, until the TTL elapses — see the security note in [docs/NODE_PROXY.md](../NODE_PROXY.md#token-rotation).

**HMAC signing is on by default** (`NODE_HMAC_SIGNING_ENABLED=true`). Server signs proxy frames with a per-node secret; node verifies. Don't disable it unless you have a specific reason — it catches in-flight tampering and is essentially free.

**WS frames are bounded** (writer channel capacity 256, max stream duration 5 min by default). For long-running streaming responses (e.g., LLM chat completions through the node), watch the `NODE_MAX_STREAM_DURATION_SECS` ceiling.

**Multiple nodes for the same service give automatic failover.** Bind two nodes, set priorities. If the primary's WebSocket goes idle past `NODE_HEARTBEAT_TIMEOUT_SECS` (default 90s), traffic shifts to the secondary. See [docs/NODE_PROXY.md#multi-node-failover](../NODE_PROXY.md#multi-node-failover).

**Multiple instances on one machine via `--profile`.** `nyxid node register --profile work` and `--profile personal` give you two independent daemons (`dev.nyxid.node.work` / `.personal` on macOS; `nyxid-node-work.service` on Linux), each with its own config dir under `~/.nyxid-node/profiles/`.

---

## Next

- **Same flow but with OpenClaw as the local service** (one-step bind for self-hosted AI gateway): `nyxid node openclaw connect --url http://localhost:18789`
- **Reference for the node protocol, security model, metrics, and admin endpoints:** [docs/NODE_PROXY.md](../NODE_PROXY.md)
- **Other quickstarts:** [n8n](n8n.md) · [Claude Code per-agent keys](claude-code.md) · [MCP wrapping](mcp-wrapping.md)
