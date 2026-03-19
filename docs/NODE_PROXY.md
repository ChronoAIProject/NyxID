# NyxID Credential Nodes

NyxID Credential Nodes let you run a lightweight agent on your own infrastructure that handles credential injection for proxy requests. Instead of storing credentials in NyxID's database, credentials stay on your node. When a proxy request arrives, NyxID routes it to your node via WebSocket, and the node injects credentials locally before forwarding the request to the downstream service.

This is an opt-in feature. Users without nodes continue using the existing proxy (credentials stored in NyxID). Users with nodes can selectively route specific services through their nodes while keeping others on NyxID.

---

## Table of Contents

- [Why Use a Credential Node](#why-use-a-credential-node)
- [Prerequisites](#prerequisites)
- [Node Registration Flow](#node-registration-flow)
- [Service Bindings](#service-bindings)
- [Streaming Proxy](#streaming-proxy)
- [SSH Tunnel Transport](#ssh-tunnel-transport)
- [Multi-Node Failover](#multi-node-failover)
- [HMAC Request Signing](#hmac-request-signing)
- [Node Metrics](#node-metrics)
- [Health Monitoring](#health-monitoring)
- [Token Rotation](#token-rotation)
- [Admin Node Management](#admin-node-management)
- [Troubleshooting](#troubleshooting)
- [Security Model](#security-model)

---

## Why Use a Credential Node

- **Credential isolation:** API keys and tokens never leave your infrastructure. NyxID sends only request metadata (method, path, headers, body) to the node. The node injects credentials locally before forwarding to the downstream service.
- **Compliance:** Meet data residency or regulatory requirements by keeping secrets on-premises.
- **Selective routing:** Bind specific services to your node while keeping others on NyxID's standard proxy. If a node goes offline, requests fall back to NyxID-stored credentials automatically.

---

## Prerequisites

| Requirement | Details |
|-------------|---------|
| NyxID account | Active user account with at least one downstream service configured |
| Node agent | A running instance of the NyxID node agent on your infrastructure |
| Network | Outbound WebSocket (WSS) connectivity from the node to the NyxID server |

---

## Node Registration Flow

Registration is a two-step process: create a registration token in the NyxID dashboard, then use it to connect the node agent.

### Step 1: Create a Registration Token

In the NyxID dashboard, navigate to **Credential Nodes** and click **Register Node**. Enter a name for the node (lowercase alphanumeric and hyphens, 1-64 characters).

Alternatively, use the API:

```bash
curl -X POST https://your-nyxid-server/api/v1/nodes/register-token \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{"name": "my-home-server"}'
```

Response:

```json
{
  "token_id": "...",
  "token": "nyx_nreg_<64_hex_chars>",
  "name": "my-home-server",
  "expires_at": "2026-03-12T17:30:00Z"
}
```

The registration token is shown **only once** and expires after the configured TTL (default: 1 hour). Copy it immediately.

### Step 2: Connect the Node Agent

Configure the node agent with the registration token:

```bash
# File-based storage (default, works on all platforms)
nyxid-node register --token nyx_nreg_<64_hex_chars>

# OS keychain storage (macOS Keychain, Windows Credential Manager, Linux Secret Service)
nyxid-node register --token nyx_nreg_<64_hex_chars> --keychain
```

The node agent will:

1. Open a WebSocket connection to `wss://your-nyxid-server/api/v1/nodes/ws`
2. Send a `register` message with the token and optional metadata (agent version, OS, architecture)
3. Receive a `register_ok` response containing the permanent `node_id` and `auth_token`
4. Store the auth token locally using the selected storage backend (encrypted file or OS keychain)

The auth token (`nyx_nauth_...`) is the node's long-lived credential for reconnecting. The signing secret is the HMAC shared secret for request integrity verification. Both are shown only once during registration. Store them securely.

To migrate an existing node between storage backends later, use `nyxid-node migrate --to keychain` (or `--to file`).

### Reconnecting

On subsequent connections, the node authenticates with its auth token:

```json
{
  "type": "auth",
  "node_id": "<uuid>",
  "token": "nyx_nauth_<64_hex_chars>"
}
```

---

## Service Bindings

A service binding tells NyxID to route proxy requests for a specific service through your node instead of using NyxID-stored credentials.

### Create a Binding

In the node detail page, click **Bind Service** and select a service from the dropdown. Or use the API:

```bash
curl -X POST https://your-nyxid-server/api/v1/nodes/<node_id>/bindings \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{"service_id": "<service_uuid>"}'
```

### How Routing Works

When a proxy request arrives (`/api/v1/proxy/s/{slug}/{path}` or `/api/v1/proxy/{service_id}/{path}`):

1. NyxID authenticates the user and checks approval requirements
2. NyxID checks if the user has an active service binding for the target service
3. If a binding exists and the bound node is online and connected via WebSocket, NyxID forwards the request to the node
4. The node injects credentials locally and forwards the request to the downstream service
5. The node returns the response to NyxID, which returns it to the client

If no binding exists, or all bound nodes are offline/unhealthy, the request falls back to the standard proxy flow (NyxID-stored credentials).

### Binding Priority

Each binding has a `priority` field (default: 0, lower values = higher priority). When multiple nodes are bound to the same service, NyxID routes to the node with the lowest priority value first. Update priority via the API:

```bash
curl -X PATCH https://your-nyxid-server/api/v1/nodes/<node_id>/bindings/<binding_id> \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{"priority": 10}'
```

### Remove a Binding

In the node detail page, click the delete icon next to the binding. Or use the API:

```bash
curl -X DELETE https://your-nyxid-server/api/v1/nodes/<node_id>/bindings/<binding_id> \
  -H "Authorization: Bearer <access_token>"
```

---

## Streaming Proxy

Node-proxied requests support streaming responses (SSE / Server-Sent Events). When the downstream service returns `Content-Type: text/event-stream`, the node agent streams chunks back to NyxID via WebSocket instead of buffering the entire response. NyxID reconstructs the stream and forwards it to the client in real time.

This enables streaming LLM responses (e.g., OpenAI `stream=true`) through the node proxy with the same latency characteristics as a direct connection.

The streaming protocol uses three message types:

1. `proxy_response_start` -- status code and headers
2. `proxy_response_chunk` -- base64-encoded data chunks (max 64KB each)
3. `proxy_response_end` -- signals stream completion

The `content-length` header is stripped from streaming responses since the total size is unknown. The maximum streaming duration is configurable via `NODE_MAX_STREAM_DURATION_SECS` (default: 300 seconds).

---

## SSH Tunnel Transport

The same node routing layer can also carry SSH sessions for services that have SSH tunneling enabled in NyxID. Instead of forwarding HTTP, NyxID asks the node to open a raw TCP connection to the configured SSH target and then relays SSH bytes over the existing node WebSocket.

### How It Works

1. A user opens `GET /api/v1/ssh/{service_id}` through NyxID's SSH helper
2. NyxID resolves the user's bound node for that service
3. NyxID sends `ssh_tunnel_open` with the target `host` and `port`
4. The node opens a local TCP connection to the SSH daemon
5. Both sides exchange base64-encoded `ssh_tunnel_data` messages until either side closes the session

### Routing Behavior

- SSH uses the same per-user service bindings as HTTP proxy traffic
- The selected node must be online and able to reach the target SSH host from its own network
- Private, loopback, and metadata SSH targets must be explicitly allowlisted in the node agent config before the node will open them
- The node agent enforces a bounded `max_tunnels` limit so the server cannot open unbounded concurrent SSH sessions
- The node agent enforces `io_timeout_secs` per SSH TCP read and write so stalled tunnel workers are eventually reclaimed; the default `3600` seconds matches NyxID's tunnel cap, and operators can lower it for more aggressive idle-session cleanup
- If no healthy bound node can open the SSH target, NyxID falls back to opening the TCP connection itself

For operator-facing setup, see [SSH_TUNNELING.md](./SSH_TUNNELING.md). For the exact message shapes, see [NODE_PROXY_PROTOCOL.md](./NODE_PROXY_PROTOCOL.md).

---

## Multi-Node Failover

When multiple nodes are bound to the same service for a user, NyxID implements priority-based routing with automatic failover:

1. Bindings are sorted by `priority` (ascending -- lower values tried first)
2. Nodes are filtered to those that are active, online, and WebSocket-connected
3. Nodes with >50% error rate (with at least 10 requests) are skipped as unhealthy
4. The first viable node receives the request
5. If the primary node fails (offline or timeout), the next viable node is tried
6. If all nodes fail, the request falls back to the standard proxy (NyxID-stored credentials)

Each failover attempt generates a new `request_id` to avoid correlation conflicts. Metrics are recorded for each attempt (success or failure).

---

## HMAC Request Signing

NyxID signs proxy requests sent to nodes using HMAC-SHA256 to ensure integrity and authenticity. This is enabled by default and can be toggled via `NODE_HMAC_SIGNING_ENABLED` (default: `true`).

### How It Works

1. During node registration, the server generates a shared HMAC secret and returns it alongside the auth token
2. When routing a proxy request through a node, the server computes an HMAC-SHA256 signature over the request fields
3. The node verifies the signature before executing the request
4. Requests with invalid signatures are rejected with HTTP 403

### Signed Fields

The HMAC message is computed as:

```
{timestamp}\n{nonce}\n{method}\n{path}\n{query}\n{body_base64}
```

The `timestamp` is an RFC 3339 datetime, `nonce` is a UUID v4, and `body_base64` is the base64-encoded request body. The signature is hex-encoded.

### Token Rotation

When a node's token is rotated (`POST /api/v1/nodes/{node_id}/rotate-token`), both the auth token and signing secret are regenerated. The node must reconnect and re-register to receive the new signing secret.

---

## Node Metrics

NyxID tracks per-node proxy metrics as an embedded document on the Node model. Metrics are updated asynchronously (fire-and-forget) after each proxy request.

### Tracked Metrics

| Metric | Description |
|--------|-------------|
| `total_requests` | Total proxy requests routed through this node |
| `success_count` | Requests that completed successfully |
| `error_count` | Requests that failed (node errors, timeouts) |
| `avg_latency_ms` | Exponential moving average response latency (alpha=0.1) |
| `last_error` | Most recent error message (truncated to 256 chars) |
| `last_error_at` | Timestamp of the most recent error |
| `last_success_at` | Timestamp of the most recent success |

### Viewing Metrics

Metrics are included in node list and detail API responses:

```json
{
  "metrics": {
    "total_requests": 1542,
    "success_count": 1500,
    "error_count": 42,
    "success_rate": 0.9728,
    "avg_latency_ms": 234.5,
    "last_error": "Connection refused",
    "last_error_at": "2026-03-12T10:15:00Z",
    "last_success_at": "2026-03-12T10:30:00Z"
  }
}
```

The `success_rate` field is computed by the API handler (`success_count / total_requests`).

### Health-Based Routing

The failover system uses metrics for health-aware routing. Nodes with an error rate exceeding 50% (and at least 10 total requests) are skipped during route resolution. This prevents routing traffic to nodes that are consistently failing.

---

## Admin Node Management

Administrators can view and manage all nodes across all users through the admin API and dashboard.

### Admin Dashboard

The **Admin > Node Management** page displays all nodes with:

- Node name, user email, status, connection state
- Binding count, total requests, success rate, average latency
- Agent version from node metadata
- Actions: disconnect (force-close WebSocket) and delete (soft-delete with binding cleanup)

Supports filtering by status (online/offline/draining) and user ID, with pagination.

### Admin API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/admin/nodes` | List all nodes (paginated, filterable) |
| `GET` | `/api/v1/admin/nodes/{node_id}` | Get node details (includes user email) |
| `POST` | `/api/v1/admin/nodes/{node_id}/disconnect` | Force-disconnect a node's WebSocket |
| `DELETE` | `/api/v1/admin/nodes/{node_id}` | Delete a node (no ownership check) |

#### List All Nodes

```bash
curl "https://your-nyxid-server/api/v1/admin/nodes?page=1&per_page=50&status=online" \
  -H "Authorization: Bearer <admin_access_token>"
```

Query parameters: `page`, `per_page` (max 100), `status` (online/offline/draining), `user_id`.

#### Force-Disconnect

```bash
curl -X POST "https://your-nyxid-server/api/v1/admin/nodes/<node_id>/disconnect" \
  -H "Authorization: Bearer <admin_access_token>"
```

This closes the WebSocket connection and sets the node status to offline. The node can reconnect at any time.

#### Admin Delete

```bash
curl -X DELETE "https://your-nyxid-server/api/v1/admin/nodes/<node_id>" \
  -H "Authorization: Bearer <admin_access_token>"
```

Soft-deletes the node and all its bindings. The node's WebSocket connection is closed if active. All admin node operations are audit-logged.

---

## Health Monitoring

NyxID monitors node health through a heartbeat protocol:

1. NyxID sends `heartbeat_ping` messages to connected nodes at a configurable interval (default: every 30 seconds)
2. Nodes respond with `heartbeat_pong`
3. If a node fails to respond within the timeout window (default: 90 seconds), NyxID marks it offline and closes the WebSocket connection

### Node Statuses

| Status | Meaning |
|--------|---------|
| **Online** | Node is connected via WebSocket and responding to heartbeats |
| **Offline** | Node is not connected (disconnected, timed out, or deleted) |
| **Draining** | Node is preparing to shut down (future use) |

The dashboard shows real-time connection status alongside the database status. The `is_connected` field reflects live WebSocket connectivity from `NodeWsManager`.

### Monitoring via API

```bash
# List all nodes with status
curl https://your-nyxid-server/api/v1/nodes \
  -H "Authorization: Bearer <access_token>"

# Get single node details
curl https://your-nyxid-server/api/v1/nodes/<node_id> \
  -H "Authorization: Bearer <access_token>"
```

---

## Token Rotation

Rotate a node's auth token and signing secret to invalidate the current ones:

1. In the node detail page, click **Rotate Token**
2. Copy the new auth token and signing secret -- they are shown only once
3. The node is immediately disconnected and must reconnect with the new credentials

Via API:

```bash
curl -X POST https://your-nyxid-server/api/v1/nodes/<node_id>/rotate-token \
  -H "Authorization: Bearer <access_token>"
```

Response:

```json
{
  "auth_token": "nyx_nauth_<new_64_hex_chars>",
  "signing_secret": "<64_hex_chars>",
  "message": "Auth token and signing secret rotated. The node must reconnect with the new token."
}
```

---

## Troubleshooting

### Node shows "Offline" but the agent is running

- Verify the agent can reach `wss://your-nyxid-server/api/v1/nodes/ws`
- Check that the auth token has not been rotated
- Ensure TLS certificates are valid (WebSocket requires WSS in production)
- Check if the maximum concurrent connections limit has been reached (`NODE_MAX_WS_CONNECTIONS`, default: 100)

### Registration token expired

Registration tokens expire after the configured TTL (default: 1 hour). Create a new one from the dashboard or API.

### Proxy requests not routing through the node

- Verify the node is online and connected (check `is_connected` in the node list)
- Verify an active binding exists for the target service
- Check that the service is active (`is_active: true`)
- Review audit logs -- node-routed requests include `"routed_via": "node"` in audit data

### Proxy request timeouts

Node proxy requests have a configurable timeout (default: 30 seconds, `NODE_PROXY_TIMEOUT_SECS`). If the downstream service is slow:
- Increase the timeout via the environment variable
- Check the node's network connectivity to the downstream service

### Maximum nodes reached

Each user can register up to `NODE_MAX_PER_USER` nodes (default: 10). Delete unused nodes to free capacity.

---

## Security Model

### Token Security

- Registration tokens (`nyx_nreg_...`) and auth tokens (`nyx_nauth_...`) are 32 bytes of cryptographic randomness (64 hex characters)
- Only SHA-256 hashes are stored in the database; raw tokens are never stored
- Distinguishable prefixes enable leak scanning and token type identification
- Registration tokens are single-use and expire after a configurable TTL (default: 1 hour)

### Credential Isolation

- Credentials never transit through NyxID when using node proxy
- NyxID sends only request metadata (HTTP method, path, headers, body) to the node
- The node is responsible for injecting credentials locally before forwarding to the downstream service
- This is the primary security benefit of the node proxy architecture

### ACL Enforcement

- NyxID validates service bindings server-side before routing to a node
- A node only receives requests for services it has active bindings for
- Bindings require explicit creation through the authenticated management API

### WebSocket Security

- WebSocket connections must use WSS (TLS) in production
- Auth tokens are transmitted in the first WebSocket message, not as URL parameters (avoids server access logs)
- Connections without valid authentication within 10 seconds are terminated with close code 4001
- A configurable maximum concurrent connections limit prevents resource exhaustion (`NODE_MAX_WS_CONNECTIONS`, default: 100)

### Proxy Request Integrity

- Request and response headers are filtered through the same allowlists as the standard proxy
- Body size limits (10 MB) apply to node-proxied requests
- The node cannot influence which services it receives requests for -- routing is controlled entirely by NyxID

### Audit Trail

- All node management operations (registration, deletion, token rotation, binding changes) are audit-logged
- Proxy requests routed through nodes include `routed_via: "node"` and `node_id` in the audit event data
- Node connection and disconnection events are logged via the tracing framework

---

## Environment Variables

All node proxy settings are optional with sensible defaults:

| Variable | Default | Description |
|----------|---------|-------------|
| `NODE_HEARTBEAT_INTERVAL_SECS` | `30` | Interval between heartbeat pings to connected nodes |
| `NODE_HEARTBEAT_TIMEOUT_SECS` | `90` | Mark node offline after this many seconds without heartbeat |
| `NODE_PROXY_TIMEOUT_SECS` | `30` | Timeout for proxy requests routed through nodes |
| `NODE_REGISTRATION_TOKEN_TTL_SECS` | `3600` | Registration token validity (1 hour) |
| `NODE_MAX_PER_USER` | `10` | Maximum nodes per user |
| `NODE_MAX_WS_CONNECTIONS` | `100` | Maximum concurrent WebSocket connections (authenticated + pending) |
| `NODE_MAX_STREAM_DURATION_SECS` | `300` | Maximum duration for streaming proxy responses |
| `NODE_HMAC_SIGNING_ENABLED` | `true` | Enable HMAC request signing for node proxy requests |

---

## API Endpoints

All endpoints require authentication (session, JWT, or API key) unless noted otherwise.

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/v1/nodes/register-token` | Create a one-time registration token |
| `GET` | `/api/v1/nodes` | List user's nodes |
| `GET` | `/api/v1/nodes/{node_id}` | Get node details |
| `DELETE` | `/api/v1/nodes/{node_id}` | Delete/deregister a node |
| `POST` | `/api/v1/nodes/{node_id}/rotate-token` | Rotate the node's auth token |
| `GET` | `/api/v1/nodes/{node_id}/bindings` | List service bindings |
| `POST` | `/api/v1/nodes/{node_id}/bindings` | Create a service binding |
| `PATCH` | `/api/v1/nodes/{node_id}/bindings/{binding_id}` | Update binding priority |
| `DELETE` | `/api/v1/nodes/{node_id}/bindings/{binding_id}` | Remove a binding |
| `GET` | `/api/v1/nodes/ws` | WebSocket upgrade (no standard auth -- auth via WS protocol) |

### Admin Endpoints (requires admin role)

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/admin/nodes` | List all nodes across all users (paginated) |
| `GET` | `/api/v1/admin/nodes/{node_id}` | Get any node's details (includes user email) |
| `POST` | `/api/v1/admin/nodes/{node_id}/disconnect` | Force-disconnect a node |
| `DELETE` | `/api/v1/admin/nodes/{node_id}` | Admin-delete a node |
