# Node Proxy WebSocket Protocol

This document describes the WebSocket protocol used for communication between NyxID and credential node agents. All messages are JSON with a `type` field discriminator.

---

## Table of Contents

- [Connection Lifecycle](#connection-lifecycle)
- [Authentication Flow](#authentication-flow)
- [Heartbeat Protocol](#heartbeat-protocol)
- [Request/Response Routing](#requestresponse-routing)
- [Streaming Responses](#streaming-responses)
- [HMAC Request Signing](#hmac-request-signing)
- [Message Reference: Node to NyxID](#message-reference-node-to-nyxid)
- [Message Reference: NyxID to Node](#message-reference-nyxid-to-node)
- [Error Handling](#error-handling)
- [Connection Recovery](#connection-recovery)
- [Error Codes](#error-codes)

---

## Connection Lifecycle

```
Node Agent                              NyxID Server
    |                                        |
    |  GET /api/v1/nodes/ws (WS upgrade)     |
    | ──────────────────────────────────────> |
    |                                        |  Check max connections
    |  101 Switching Protocols               |
    | <────────────────────────────────────── |
    |                                        |
    |  { type: "register" } or               |
    |  { type: "auth" }                      |
    | ──────────────────────────────────────> |  10s auth timeout starts
    |                                        |  Validate token
    |  { type: "register_ok" } or            |
    |  { type: "auth_ok" }                   |
    | <────────────────────────────────────── |
    |                                        |  Register in NodeWsManager
    |                                        |  Set status = "online"
    |                                        |
    |  <<< Authenticated session >>>         |
    |  heartbeat_ping / heartbeat_pong       |
    |  proxy_request / proxy_response        |
    |                                        |
    |  WebSocket close                       |
    | ──────────────────────────────────────> |
    |                                        |  Unregister from NodeWsManager
    |                                        |  Set status = "offline"
```

### Connection Limits

NyxID enforces a maximum concurrent WebSocket connections limit (default: 100, configurable via `NODE_MAX_WS_CONNECTIONS`). This includes both authenticated connections and those still in the authentication handshake. If the limit is reached, the WebSocket upgrade request receives HTTP 503 Service Unavailable.

---

## Authentication Flow

The first message on a new WebSocket connection **must** be either `register` (first-time setup) or `auth` (reconnection). NyxID enforces a 10-second timeout for this initial message. Connections that fail to authenticate within the timeout are closed with close code `4001`.

### First-Time Registration

The node sends a `register` message with the one-time registration token:

```json
{
  "type": "register",
  "token": "nyx_nreg_<64_hex_chars>",
  "metadata": {
    "agent_version": "0.1.0",
    "os": "linux",
    "arch": "x86_64"
  }
}
```

NyxID validates the token (hash lookup, checks `used: false`, checks `expires_at`), atomically marks it as used, creates a `Node` record, and returns:

```json
{
  "type": "register_ok",
  "node_id": "<uuid>",
  "auth_token": "nyx_nauth_<64_hex_chars>",
  "signing_secret": "<64_hex_chars>"
}
```

The `auth_token` is the node's long-lived credential for reconnections. The `signing_secret` is the HMAC shared secret for request integrity verification (see [HMAC Request Signing](#hmac-request-signing)). Both must be stored securely.

### Reconnection

For subsequent connections, the node authenticates with its auth token:

```json
{
  "type": "auth",
  "node_id": "<uuid>",
  "token": "nyx_nauth_<64_hex_chars>"
}
```

NyxID validates the token hash and verifies it matches the provided `node_id`. On success:

```json
{
  "type": "auth_ok",
  "node_id": "<uuid>"
}
```

### Authentication Failure

On any authentication failure (invalid token, expired registration token, node_id mismatch), NyxID sends:

```json
{
  "type": "auth_error",
  "message": "Authentication failed"
}
```

The connection is then closed. Error messages are generic to prevent information leakage.

---

## Heartbeat Protocol

NyxID runs a background task that periodically sweeps all connected nodes (configurable interval, default: 30 seconds).

### Ping

NyxID sends to each connected node:

```json
{
  "type": "heartbeat_ping",
  "timestamp": "2026-03-12T10:30:00.000Z"
}
```

### Pong

The node responds with:

```json
{
  "type": "heartbeat_pong",
  "timestamp": "2026-03-12T10:30:00.100Z"
}
```

On receiving a pong, NyxID updates `last_heartbeat_at` in the database for the node.

### Timeout Detection

During each sweep, NyxID checks each node's `last_heartbeat_at`. If the elapsed time exceeds the timeout threshold (default: 90 seconds), the node is considered stale:

1. The WebSocket connection is unregistered from `NodeWsManager`
2. The node status is set to `offline` in the database
3. Any pending proxy requests to the node will receive a `NodeOffline` error

### Ping Failure

If sending a heartbeat ping fails (e.g., the channel is closed), the node is immediately unregistered and marked offline.

---

## Request/Response Routing

When NyxID receives an HTTP proxy request for a service that has a node binding:

1. `node_routing_service::resolve_node_route()` finds an active binding and verifies the bound node is active and online
2. `NodeWsManager::is_connected()` confirms the node has a live WebSocket connection
3. `NodeWsManager::send_proxy_request()` serializes the request as a `proxy_request` message, sends it over the WebSocket, and waits for the correlated response

### Request Correlation

Each proxy request includes a unique `request_id` (UUID v4). The node must include the same `request_id` in its response. NyxID uses oneshot channels internally for request/response correlation with a configurable timeout (default: 30 seconds).

### Proxy Request

NyxID sends to the node:

```json
{
  "type": "proxy_request",
  "request_id": "<uuid>",
  "service_id": "<uuid>",
  "service_slug": "my-api",
  "method": "POST",
  "path": "/v1/chat/completions",
  "query": "stream=true",
  "headers": {
    "content-type": "application/json",
    "accept": "application/json"
  },
  "body": "<base64_encoded_request_body>",
  "timestamp": "2026-03-12T10:30:00.000Z",
  "nonce": "<uuid>",
  "signature": "<hex_encoded_hmac_sha256>"
}
```

- `headers` is a JSON object of allowed request headers (same allowlist as the standard proxy)
- `body` is base64-encoded. Omitted if the request has no body.
- `timestamp`, `nonce`, and `signature` are present when HMAC signing is enabled (see [HMAC Request Signing](#hmac-request-signing)). Omitted when signing is disabled.
- The request does **not** include credentials. The node is responsible for injecting them.

### Proxy Response

The node returns:

```json
{
  "type": "proxy_response",
  "request_id": "<uuid>",
  "status": 200,
  "headers": {
    "content-type": "application/json",
    "x-request-id": "abc123"
  },
  "body": "<base64_encoded_response_body>"
}
```

- `headers` is a JSON object. NyxID filters these through the response header allowlist before returning to the client.
- `body` is base64-encoded. If absent or null, an empty body is returned.

### Proxy Error

If the node encounters an error executing the request:

```json
{
  "type": "proxy_error",
  "request_id": "<uuid>",
  "error": "Connection refused",
  "status": 502
}
```

- `status` defaults to 502 if omitted
- The `error` field is returned as a JSON error body to the client

---

## Streaming Responses

For responses with `Content-Type: text/event-stream` (SSE), the node sends a streaming sequence instead of a single `proxy_response` message. This enables real-time streaming of LLM responses and other SSE-based APIs through the WebSocket tunnel.

### proxy_response_start

Sent by the node when the downstream service begins a streaming response:

```json
{
  "type": "proxy_response_start",
  "request_id": "<uuid>",
  "status": 200,
  "headers": {
    "content-type": "text/event-stream",
    "cache-control": "no-cache"
  }
}
```

On the server side, NyxID upgrades the pending request from a oneshot channel to a streaming `mpsc` channel. The `content-length` header is stripped since the total size is unknown.

### proxy_response_chunk

Sent for each chunk of streaming data:

```json
{
  "type": "proxy_response_chunk",
  "request_id": "<uuid>",
  "data": "<base64_encoded_chunk>"
}
```

Each chunk is limited to 64KB after base64 decoding. Larger chunks from the downstream service are split into multiple `proxy_response_chunk` messages.

NyxID converts these chunks into an `axum::body::Body::from_stream()` for real-time forwarding to the client.

### proxy_response_end

Sent when the stream completes:

```json
{
  "type": "proxy_response_end",
  "request_id": "<uuid>"
}
```

### Stream Error

If the downstream stream encounters an error mid-stream, the node sends a `proxy_error`:

```json
{
  "type": "proxy_error",
  "request_id": "<uuid>",
  "error": "Stream error: connection reset",
  "status": 502
}
```

### Server-Side State Machine

```
PendingRequest::OneShot  ──[proxy_response_start]──>  PendingRequest::Streaming
                                                          |
                                                  [proxy_response_chunk]*
                                                          |
                                                  [proxy_response_end]──> removed
```

If the node sends a standard `proxy_response` instead of the streaming sequence, NyxID handles it as a complete response regardless of the pending request type.

---

## HMAC Request Signing

When HMAC signing is enabled on the server (`NODE_HMAC_SIGNING_ENABLED=true`, default), proxy requests include a cryptographic signature that the node must verify before executing the request.

### Signing Protocol

1. The server generates a timestamp (RFC 3339) and nonce (UUID v4)
2. The server computes an HMAC-SHA256 signature over the canonicalized request
3. The `timestamp`, `nonce`, and `signature` fields are included in the `proxy_request` message
4. The node verifies the signature using the shared secret from registration

### Canonical Message Format

The HMAC message is a newline-delimited string of the following fields:

```
{timestamp}\n{nonce}\n{method}\n{path}\n{query}\n{body_base64}
```

| Field | Source | Empty Value |
|-------|--------|-------------|
| `timestamp` | RFC 3339 datetime | (never empty) |
| `nonce` | UUID v4 | (never empty) |
| `method` | HTTP method (e.g., `POST`) | `""` |
| `path` | Request path (e.g., `/v1/chat/completions`) | `""` |
| `query` | Query string without `?` | `""` |
| `body_base64` | Base64-encoded request body | `""` |

### Verification

```
HMAC-SHA256(shared_secret_bytes, canonical_message) == hex_decode(signature)
```

The node performs constant-time comparison to prevent timing attacks.

### Replay Protection

The node maintains a replay guard with the following rules:

| Rule | Value |
|------|-------|
| Maximum timestamp skew | 300 seconds (5 minutes) |
| Maximum nonce set size | 10,000 entries |
| Eviction policy | Time-based first (remove expired), then oldest-first if over cap |

Requests that fail replay checks are rejected with HTTP 403 and the error message "Request rejected: replay or expired timestamp".

---

## Message Reference: Node to NyxID

| Type | When | Fields |
|------|------|--------|
| `register` | First message on first-time connection | `token` (required), `metadata` (optional: `agent_version`, `os`, `arch`) |
| `auth` | First message on reconnection | `node_id` (required), `token` (required) |
| `heartbeat_pong` | In response to `heartbeat_ping` | `timestamp` (optional) |
| `proxy_response` | After executing a proxied request (non-streaming) | `request_id`, `status`, `headers`, `body` (base64) |
| `proxy_response_start` | Beginning of a streaming response | `request_id`, `status`, `headers` |
| `proxy_response_chunk` | Chunk of streaming data | `request_id`, `data` (base64, max 64KB decoded) |
| `proxy_response_end` | End of streaming response | `request_id` |
| `proxy_error` | If a proxied request fails | `request_id`, `error`, `status` (optional, default 502) |
| `status_update` | Voluntary health/capability update | `agent_version` (optional), `services_ready` (optional) |

---

## Message Reference: NyxID to Node

| Type | When | Fields |
|------|------|--------|
| `register_ok` | After successful registration | `node_id`, `auth_token`, `signing_secret` |
| `auth_ok` | After successful authentication | `node_id` |
| `auth_error` | On authentication failure (connection closes) | `message` |
| `heartbeat_ping` | Periodic keepalive | `timestamp` |
| `proxy_request` | HTTP request to route through the node | `request_id`, `service_id`, `service_slug`, `method`, `path`, `query`, `headers`, `body` (base64), `timestamp`, `nonce`, `signature` (when HMAC enabled) |
| `error` | Server-side error | `message` |

---

## Error Handling

### Authentication Errors

| Scenario | NyxID Behavior |
|----------|----------------|
| No message within 10 seconds | Close with code 4001 |
| Invalid JSON | Send `auth_error`, close connection |
| First message is not `register` or `auth` | Send `auth_error`, close connection |
| Invalid/expired registration token | Send `auth_error`, close connection |
| Invalid auth token | Send `auth_error`, close connection |
| `node_id` does not match token | Send `auth_error`, close connection |

### Proxy Errors

| Scenario | HTTP Status | Error Code |
|----------|-------------|------------|
| Node not found | 404 | 8000 (`node_not_found`) |
| Node not connected | 503 | 8001 (`node_offline`) |
| Proxy request timeout | 504 | 8002 (`node_proxy_timeout`) |
| Node registration failed | 400 | 8003 (`node_registration_failed`) |

### Runtime Errors

- If the node sends an invalid JSON message during an authenticated session, the message is logged and skipped (connection stays open)
- If a WebSocket read error occurs, the connection is closed and the node is marked offline
- If sending a message to the node fails (channel closed), the node is treated as disconnected

---

## Connection Recovery

### Node Reconnection

When a node disconnects (network failure, restart, etc.):

1. NyxID detects the disconnect via WebSocket close or read error
2. The node is unregistered from `NodeWsManager` and marked `offline` in the database
3. Any in-flight proxy requests receive a `NodeOffline` error
4. Proxy requests for the node's bound services fall back to the standard proxy (NyxID-stored credentials)

The node can reconnect at any time by establishing a new WebSocket connection and sending an `auth` message with its stored `node_id` and `auth_token`. NyxID will:

1. Validate the auth token
2. Register the new connection in `NodeWsManager`
3. Update the node status to `online` in the database
4. Resume routing proxy requests through the node

### Graceful Fallback

The node proxy system is designed for graceful degradation:

- If a node is offline, proxy requests automatically use NyxID-stored credentials (if available)
- If no stored credentials exist and no node is available, the proxy returns an appropriate error
- This happens transparently to the client -- no client-side changes are needed

### Token Rotation Recovery

If a node's auth token is rotated via the management API:

1. The old token is immediately invalidated
2. The node's WebSocket connection is closed server-side
3. The node must obtain the new token and reconnect

---

## Implementation Notes

### Dependencies

- `axum` with `ws` feature for WebSocket support
- `dashmap` for concurrent connection tracking
- `tokio::sync::oneshot` for request/response correlation (non-streaming)
- `tokio::sync::mpsc` (bounded) for WS write channels; (unbounded) for streaming chunk delivery
- `base64` for body encoding/decoding
- `hmac` + `sha2` for HMAC-SHA256 request signing
- `async_stream` for converting streaming chunks into `Body::from_stream()`

### Threading Model

Each WebSocket connection has:
- A **reader task** that processes incoming messages (runs on the main connection future)
- A **writer task** (spawned via `tokio::spawn`) that forwards messages from the bounded internal channel (capacity: 256) to the WebSocket sink

The `NodeWsManager` is shared across all connections via `Arc` and uses `DashMap` for lock-free concurrent access.

### Pending Request Types

Each pending proxy request is tracked as one of:
- `PendingRequest::OneShot` -- standard request/response (initial state)
- `PendingRequest::Streaming` -- streaming response (upgraded on `proxy_response_start`)

When a `proxy_response_start` arrives, the oneshot sender is dropped and replaced with an `mpsc::unbounded_channel` for streaming chunks. The proxy handler in `proxy.rs` receives `ProxyResponseType::Complete` or `ProxyResponseType::Streaming` based on which path the response takes.
