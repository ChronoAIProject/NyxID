# SSH Tunneling

NyxID can proxy SSH connections the same way it proxies HTTP: the user authenticates with NyxID first, then NyxID opens a WebSocket-backed TCP tunnel to the registered SSH target.

This guide covers SSH service setup, short-lived SSH certificates, and the built-in `nyxid ssh` helper used for OpenSSH `ProxyCommand` integration.

SSH is a first-class service type in NyxID. Create the service with `service_type: "ssh"` and an embedded `ssh_config`; the service detail page then renders the SSH target, certificate settings, CA public key, and copyable `nyxid ssh` commands inline.

---

## Endpoints

| Endpoint | Purpose |
|----------|---------|
| `POST /api/v1/ssh/{service_id}/certificate` | Issue a short-lived OpenSSH user certificate |
| `GET /api/v1/ssh/{service_id}` | Open the SSH-over-WebSocket tunnel |

`GET /api/v1/ssh/{service_id}` upgrades to WebSocket and accepts binary frames only. In practice you should use the `nyxid ssh proxy` helper instead of speaking to the tunnel directly.

---

## Install the Helper

The SSH helper ships with the main `nyxid` backend binary.

From the repository root:

```bash
cargo install --path backend
nyxid ssh --help
```

For local development without installing the binary globally:

```bash
cargo run -p nyxid -- ssh --help
```

Before using the helper, export a NyxID access token in the shell where you plan to run `ssh`:

```bash
export NYXID_ACCESS_TOKEN=<access_token>
```

---

## 1. Create an SSH Service

Create the service as `service_type: "ssh"` instead of bolting SSH onto an HTTP service later:

```bash
curl -X POST http://localhost:3001/api/v1/services \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Production Bastion",
    "service_type": "ssh",
    "service_category": "internal",
    "ssh_config": {
      "host": "ssh.internal.example",
      "port": 22,
      "certificate_auth_enabled": true,
      "certificate_ttl_minutes": 30,
      "allowed_principals": ["ubuntu"]
    }
  }'
```

Rules enforced by NyxID:
- `host` must be present and at most 255 characters
- `port` must be greater than zero
- `certificate_ttl_minutes` must be between `15` and `60`
- `allowed_principals` is required when certificate auth is enabled

To update an SSH service later, use `PUT /api/v1/services/{service_id}` with a replacement `ssh_config` object. `GET /api/v1/services/{service_id}` returns the current SSH config and CA public key.

---

## 2. Enable SSH Certificate Auth

If you enable certificate auth, NyxID generates a per-service SSH CA and stores the private key encrypted at rest. The public key is returned in the service config and certificate issuance response.

The downstream SSH server must trust that CA using your normal OpenSSH CA policy. At minimum, install the returned CA public key on the target host and wire it into your `sshd` configuration. The principal you request from NyxID must also be accepted by the target host's OpenSSH authorization rules.

Issue a certificate with the built-in helper:

```bash
nyxid ssh issue-cert \
  --base-url https://auth.example.com \
  --service-id <service_id> \
  --public-key-file ~/.ssh/id_ed25519.pub \
  --principal ubuntu \
  --certificate-file ~/.ssh/nyxid/prod-api-cert.pub \
  --ca-public-key-file ~/.ssh/nyxid/prod-api-ca.pub
```

By default the helper reads the NyxID access token from `NYXID_ACCESS_TOKEN`. Pass `--access-token` if you want to provide it directly.

---

## 3. Use OpenSSH ProxyCommand

The easiest way to wire OpenSSH to NyxID is to let the helper print a ready-made `~/.ssh/config` stanza:

```bash
nyxid ssh config \
  --host-alias prod-api \
  --base-url https://auth.example.com \
  --service-id <service_id> \
  --principal ubuntu \
  --identity-file ~/.ssh/id_ed25519 \
  --certificate-file ~/.ssh/nyxid/prod-api-cert.pub \
  --ca-public-key-file ~/.ssh/nyxid/prod-api-ca.pub
```

That emits a config block using:
- `ProxyCommand nyxid ssh proxy ...`
- `CertificateFile` pointing at the short-lived cert written by the helper
- `HostName ssh.invalid` so OpenSSH never talks to the target directly

Once the stanza is in place:

```bash
export NYXID_ACCESS_TOKEN=<access_token>
ssh prod-api
```

The helper can refresh the certificate automatically before opening the tunnel.

---

## 4. Transport-Only Mode

Certificate auth is optional. If your target host already uses another SSH auth method, `nyxid ssh proxy` still works as a transport tunnel:

```bash
nyxid ssh proxy \
  --base-url https://auth.example.com \
  --service-id <service_id>
```

In that mode NyxID only carries the TCP stream. OpenSSH and the downstream host still negotiate authentication end to end.

---

## 5. Node-Routed SSH

If the service is bound to a NyxID credential node, NyxID resolves that route before opening a direct TCP connection. The flow becomes:

1. client connects to `GET /api/v1/ssh/{service_id}`
2. NyxID resolves the user's active node binding
3. NyxID sends `ssh_tunnel_open` to the node over the node WebSocket
4. the node opens the local TCP connection to `host:port`
5. raw SSH bytes flow through `ssh_tunnel_data` messages

If no healthy node route is available, NyxID falls back to opening the TCP connection itself.

Operational requirement: the selected node must be able to reach the target SSH host and port from its own network.

For node-routed SSH, NyxID now validates the downstream banner as SSH before the tunnel is exposed to the client, and the node agent only allows private or loopback targets when you explicitly allowlist them in the node config. Public targets can still be opened without an allowlist entry.

Example node-agent policy:

```toml
[ssh]
max_tunnels = 10

[[ssh.allowed_targets]]
host = "bastion.internal.example"
port = 22
```

---

## 6. Audit and Limits

NyxID emits audit events for:
- `service_created` and `service_updated` when SSH services are created or edited
- `ssh_certificate_issued`
- `ssh_tunnel_connected`
- `ssh_tunnel_disconnected`
- `ssh_tunnel_connect_failed`

Relevant environment variables:

| Variable | Default | Purpose |
|----------|---------|---------|
| `SSH_MAX_SESSIONS_PER_USER` | `4` | Maximum concurrent SSH tunnels per authenticated user |
| `SSH_CONNECT_TIMEOUT_SECS` | `10` | Timeout when NyxID or a node opens the downstream TCP connection |
| `SSH_MAX_TUNNEL_DURATION_SECS` | `3600` | Maximum lifetime for a single SSH tunnel session before NyxID closes it |

Every disconnect audit entry includes session duration plus byte counts in each direction.
