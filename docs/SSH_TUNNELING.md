# SSH Tunneling

NyxID can proxy SSH connections the same way it proxies HTTP: the user authenticates with NyxID first, then NyxID opens a WebSocket-backed TCP tunnel to the registered SSH target.

This guide covers SSH service setup, authentication, short-lived SSH certificates, target machine configuration, node-agent routing for unreachable targets, and the built-in `nyxid ssh` helper used for OpenSSH `ProxyCommand` integration.

SSH is a first-class service type in NyxID. Create the service with `service_type: "ssh"` and an embedded `ssh_config`; the service detail page then renders the SSH target, certificate settings, CA public key, and copyable `nyxid ssh` commands inline.

---

## Endpoints

| Endpoint | Purpose |
|----------|---------|
| `POST /api/v1/ssh/{service_id}/certificate` | Issue a short-lived OpenSSH user certificate |
| `GET /api/v1/ssh/{service_id}` | Open the SSH-over-WebSocket tunnel |
| `POST /api/v1/auth/cli-token` | Issue an access token for the CLI (cookie session auth) |

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

---

## Authentication

The `nyxid` CLI supports three authentication methods, checked in order:

### Option A: Browser Login (recommended)

Opens your browser to the NyxID portal where you can log in with any method (password, SSO, OAuth). The token is saved automatically -- no env vars needed.

```bash
nyxid login --base-url https://auth.example.com
```

The CLI starts a temporary localhost server, opens the browser to `/cli-auth`, and waits for the redirect callback with the access token. The token is saved to `~/.nyxid/access_token` with `0600` permissions.

For headless environments (CI, Docker), use `--password` to enter email and password directly:

```bash
nyxid login --base-url https://auth.example.com --password
```

### Option B: API Key

Create an API key in the NyxID dashboard and export it:

```bash
export NYXID_ACCESS_TOKEN="nyx_..."
```

API keys are long-lived and work well for automation and CI/CD pipelines.

### Option C: Explicit Token

Pass a token directly (useful for scripting):

```bash
nyxid ssh proxy --access-token <token> ...
```

### Token Resolution Order

When running `nyxid ssh` commands, the CLI resolves the access token in this order:

1. `--access-token` flag (explicit)
2. `NYXID_ACCESS_TOKEN` environment variable
3. Saved token from `nyxid login` (`~/.nyxid/access_token`)

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
    "visibility": "private",
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
- Private/internal IPs are allowed (SSH services are admin-configured infrastructure)
- Only `metadata.google.internal` is blocked (cloud metadata SSRF protection)

### Visibility

SSH services default to `"private"` -- only visible to their creator and admins. Set `"visibility": "public"` if the service should be visible to all authenticated users.

To update an SSH service later, use `PUT /api/v1/services/{service_id}` with a replacement `ssh_config` object. `GET /api/v1/services/{service_id}` returns the current SSH config and CA public key.

---

## 2. SSH Certificate Auth

If you enable certificate auth, NyxID generates a per-service SSH CA and stores the private key encrypted at rest. The public key is returned in the service config and certificate issuance response.

### How it works

1. User authenticates with NyxID (via JWT or API key)
2. User sends their SSH public key and requested principal to NyxID
3. NyxID verifies the user's identity and checks the principal is in the service's `allowed_principals` list
4. NyxID signs a short-lived certificate (15-60 minutes) for the user's public key
5. User presents the certificate + private key to the target SSH server
6. Target server verifies the certificate was signed by the trusted CA AND the principal is authorized

The identity file (private key) is required because SSH certificate auth is a two-part proof:
- The **certificate** proves NyxID authorized this public key for this principal
- The **private key** proves the user owns the public key the certificate was issued for

NyxID handles the tunnel transport, but the SSH authentication handshake still happens end-to-end between the client and the target's `sshd`.

### Issue a certificate with the CLI

```bash
nyxid ssh issue-cert \
  --base-url https://auth.example.com \
  --service-id <service_id> \
  --public-key-file ~/.ssh/id_ed25519.pub \
  --principal ubuntu \
  --certificate-file ~/.ssh/nyxid/prod-api-cert.pub \
  --ca-public-key-file ~/.ssh/nyxid/prod-api-ca.pub
```

Replace `~/.ssh/id_ed25519.pub` with your actual SSH public key path (`id_rsa.pub`, `id_ecdsa.pub`, etc.).

---

## 3. Target Machine Setup (Passwordless Login)

For certificate auth to work without a password, the SSH target machine must trust NyxID's CA and authorize the expected principals.

### Step 1: Install the NyxID CA public key

Copy the CA public key from the service detail page or API response and save it on the target:

```bash
echo '<CA public key>' | sudo tee /etc/ssh/nyxid_ca.pub
```

### Step 2: Configure sshd

Add these lines to `/etc/ssh/sshd_config`:

```
TrustedUserCAKeys /etc/ssh/nyxid_ca.pub
AuthorizedPrincipalsFile /etc/ssh/auth_principals/%u
```

- `TrustedUserCAKeys` tells sshd to accept certificates signed by this CA
- `AuthorizedPrincipalsFile` controls which principals can log in as which Unix users (`%u` expands to the target username)

The sshd_config path is the same on Linux and macOS (`/etc/ssh/sshd_config`). On newer macOS versions, you may also check `/etc/ssh/sshd_config.d/` for drop-in config files.

### Step 3: Create authorized principals files

For each Unix user that should be accessible, create a principals file listing which certificate principals can log in as that user:

```bash
sudo mkdir -p /etc/ssh/auth_principals
echo 'ubuntu' | sudo tee /etc/ssh/auth_principals/ubuntu
echo 'deploy' | sudo tee /etc/ssh/auth_principals/deploy
```

This means:
- A certificate with principal `ubuntu` can log in as Unix user `ubuntu`
- A certificate with principal `deploy` can log in as Unix user `deploy`
- A certificate with principal `ubuntu` CANNOT log in as Unix user `deploy` (and vice versa)

### Step 4: Restart sshd

**Linux:**

```bash
sudo systemctl restart sshd
```

**macOS:**

Ensure Remote Login is enabled in **System Settings > General > Sharing > Remote Login**, then restart sshd:

```bash
sudo launchctl kickstart -k system/com.openssh.sshd
```

If Remote Login is not enabled, toggle it on in System Settings or via:

```bash
sudo systemsetup -setremotelogin on
```

### macOS-specific notes

- **SIP (System Integrity Protection)**: Recent macOS versions restrict direct edits to `/etc/ssh/`. Use `sudo` to write config files. If SIP blocks modifications, you may need to create a custom launchd plist pointing to a config copy.
- **File permissions**: CA public key files need `644` (readable by all, writable by root). The `auth_principals/` directory needs `755`, and individual principals files need `644`. Loose permissions cause sshd to silently ignore keys.
- **Keychain integration**: Use `ssh-add --apple-use-keychain ~/.ssh/id_ed25519` to persist your private key across reboots via macOS Keychain, avoiding repeated passphrase prompts.
- **Debugging**: If certificate auth fails, check `/var/log/system.log` for sshd errors, or connect with `ssh -v` to see the certificate validation handshake.
- **`AuthorizedPrincipalsFile`**: macOS OpenSSH supports this directive identically to Linux. Both `%u` (username) and absolute paths work.

### Security model

The security is layered:

1. **NyxID authentication**: User must have a valid JWT or API key
2. **NyxID authorization**: NyxID only signs principals from the service's `allowed_principals` list
3. **Target authorization**: Target sshd independently checks the certificate's principal against `AuthorizedPrincipalsFile`
4. **Certificate expiry**: Certificates are short-lived (15-60 minutes), limiting exposure from compromised credentials

Even if someone obtains a valid NyxID certificate, they can only access accounts whose principals file includes their certificate's principal.

---

## 4. Use OpenSSH ProxyCommand

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
- `ProxyCommand nyxid ssh proxy ...` with automatic certificate refresh
- `CertificateFile` pointing at the short-lived cert written by the helper
- `HostName` set to the host alias (the actual routing is handled by ProxyCommand)

Once the stanza is in place:

```bash
ssh prod-api
```

The helper refreshes the certificate automatically before opening the tunnel.

### Full command breakdown

A complete one-off SSH command with certificate auth looks like this:

```bash
ssh \
  -o ProxyCommand='nyxid ssh proxy \
    --base-url https://auth.example.com \
    --service-id <service_id> \
    --issue-certificate \
    --public-key-file ~/.ssh/id_ed25519.pub \
    --principal ubuntu \
    --certificate-file ~/.ssh/nyxid/prod-api-cert.pub \
    --ca-public-key-file ~/.ssh/nyxid/prod-api-ca.pub' \
  -o CertificateFile=~/.ssh/nyxid/prod-api-cert.pub \
  -o IdentityFile=~/.ssh/id_ed25519 \
  ubuntu@prod-api
```

**ProxyCommand arguments** (run by SSH before connecting):

| Argument | Purpose |
|----------|---------|
| `--base-url` | NyxID backend URL |
| `--service-id` | Which SSH service to tunnel to (determines target host:port) |
| `--issue-certificate` | Request a fresh short-lived certificate before opening the tunnel |
| `--public-key-file` | Your SSH public key -- sent to NyxID to sign into a certificate |
| `--principal` | Unix username to embed in the certificate (must be in the service's `allowed_principals`) |
| `--certificate-file` | Where to save the signed certificate locally |
| `--ca-public-key-file` | Where to save the CA public key (for trust verification) |

**SSH client arguments:**

| Argument | Purpose |
|----------|---------|
| `-o CertificateFile=...` | Tells SSH to present this certificate during authentication |
| `-o IdentityFile=...` | Private key matching the public key the cert was signed for -- proves you own the key |
| `ubuntu@prod-api` | User and host alias (hostname is irrelevant since ProxyCommand handles routing) |

**What happens when you run this:**

1. SSH runs the ProxyCommand (`nyxid ssh proxy`)
2. The CLI authenticates with NyxID using the saved token from `nyxid login` (or env var / flag)
3. It sends your public key to NyxID, receives a signed certificate, saves it locally
4. It opens a WebSocket tunnel: client → NyxID → TCP to SSH target
5. SSH uses the certificate + private key to authenticate with the target's sshd
6. If the target trusts NyxID's CA and the principal is authorized → passwordless login

Replace `~/.ssh/id_ed25519` with your actual key path (`id_rsa`, `id_ecdsa`, etc.).

---

## 5. Transport-Only Mode

Certificate auth is optional. If your target host already uses another SSH auth method (password, authorized_keys), `nyxid ssh proxy` still works as a transport tunnel:

```bash
ssh -o ProxyCommand='nyxid ssh proxy --base-url https://auth.example.com --service-id <service_id>' user@my-service
```

In that mode NyxID only carries the TCP stream. OpenSSH and the downstream host still negotiate authentication end to end. The user will be prompted for their password or use their existing key configuration.

---

## 6. Node Agent (Required for Web Terminal and Exec)

A NyxID node agent is **required** for web terminal sessions and command execution (REST API and MCP). For CLI SSH tunneling (`nyxid ssh proxy`), the node agent is optional but recommended. The NyxID server never makes direct SSH connections -- all SSH operations run on the node agent for security.

Deploy a node agent on a machine that can reach the SSH target. The node agent connects outbound to NyxID via WebSocket -- no inbound ports required on the target network.

### How it works

**SSH tunneling (CLI):**
1. Client connects to `GET /api/v1/ssh/{service_id}`
2. NyxID resolves the user's active node binding for this service
3. NyxID sends `ssh_tunnel_open` to the node over the existing node WebSocket
4. The node agent opens a local TCP connection to `host:port` on its network
5. Raw SSH bytes flow through `ssh_tunnel_data` messages between client and node

**Web terminal:**
1. Browser opens WebSocket to `GET /api/v1/ssh/{service_id}/terminal`
2. NyxID generates ephemeral credentials and sends `web_terminal_open` to the node
3. The node agent spawns `ssh` inside a PTY and bridges PTY I/O through WebSocket
4. Terminal input/output flows as `web_terminal_data` messages

**Command execution (API/MCP):**
1. Client sends `POST /api/v1/ssh/{service_id}/exec` or MCP `nyx__ssh_exec`
2. NyxID generates ephemeral credentials and sends `ssh_exec` to the node
3. The node agent spawns `ssh`, captures stdout/stderr, and returns `ssh_exec_result`

### Setup

**1. Generate a registration token** (via the Nodes page or API):

```bash
curl -X POST https://auth.example.com/api/v1/nodes/register-token \
  -H "Authorization: Bearer <access_token>"
```

**2. Register the node agent** on a machine in the target's network:

```bash
nyxid-node register \
  --token <registration_token> \
  --url wss://auth.example.com/api/v1/nodes/ws
```

**3. Start the agent:**

```bash
nyxid-node start
```

**4. Bind the SSH service to the node** via the Nodes page or API. This tells NyxID to route SSH traffic for this service through the node agent.

### Node-agent SSH policy

The node agent validates SSH targets independently. Configure allowed targets in the node's TOML config:

```toml
[ssh]
max_tunnels = 10
# Timeout for idle SSH tunnel I/O on the node side.
# Default is 3600s (1 hour) to match SSH_MAX_TUNNEL_DURATION_SECS.
io_timeout_secs = 3600

[[ssh.allowed_targets]]
host = "bastion.internal.example"
port = 22

[[ssh.allowed_targets]]
host = "192.168.1.50"
port = 22
```

Public targets can be opened without an allowlist entry. Private/loopback targets require explicit allowlisting in the node config.

### Security

- `ssh_tunnel_open` messages are HMAC-signed by the NyxID server using the node's signing secret
- The node agent verifies the HMAC signature and checks a replay guard (5-minute window, 10k nonce cap)
- Tunnel data is binary (not inspected by the node agent) -- SSH encryption is end-to-end

---

## 7. Audit and Limits

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

---

## 8. Remote Command Execution

NyxID also supports executing individual commands on SSH services programmatically, without opening an interactive tunnel. This is useful for AI agent automation, CI/CD pipelines, monitoring scripts, and infrastructure management.

The exec endpoint is:

```
POST /api/v1/ssh/{service_id}/exec
```

It accepts a command string, optional principal, and optional timeout, and returns structured output (stdout, stderr, exit code, duration, timeout status). The same endpoint is also available as MCP tools (`nyx__ssh_exec` and `nyx__ssh_list_services`) for AI agent integration.

Security controls include a command blocklist, 1 MB output cap, configurable timeouts (max 300s), full audit logging, and the same certificate-based authentication and RBAC as tunneling.

For complete documentation, see [SSH_REMOTE_EXEC.md](SSH_REMOTE_EXEC.md).
