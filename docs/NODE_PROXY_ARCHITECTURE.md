# Node Proxy Architecture

## Overview

The Node Proxy feature introduces a **control plane / data plane** architecture for NyxID's credential proxy. Users can run lightweight "credential nodes" on their own infrastructure. Instead of storing credentials in NyxID's database, credentials stay on the user's node. When a proxy request arrives, NyxID routes it to the user's node via WebSocket, and the node injects credentials locally before forwarding the request to the downstream service.

This is an opt-in feature. Users without nodes continue using the existing proxy (credentials stored in NyxID). Users with nodes can selectively route specific services through their nodes while keeping others on NyxID.

For the WebSocket protocol specification, see [NODE_PROXY_PROTOCOL.md](NODE_PROXY_PROTOCOL.md). For end-user documentation, see [NODE_PROXY.md](NODE_PROXY.md) and [NYXID_NODE.md](NYXID_NODE.md).

## Architecture Diagram

```
                        +-----------------------------------+
                        |          NyxID Server              |
    Client --HTTP-->    |  +---------+  +--------------+     |
                        |  |  Proxy  |  |   Node WS    |     |
                        |  | Handler +-->   Manager     |     |
                        |  +----+----+  +------+-------+     |
                        |       |              |             |
                        |  +----v----+    WebSocket          |
                        |  |  Node   |         |             |
                        |  |  Router |         |             |
                        |  +----+----+         |             |
                        +-------+--------------+-------------+
                                |              |
                     +----------+              |
                     | (fallback)              | (node route)
                     v                         v
              +----------+            +-----------------+
              |Downstream|            |  User's Node    |
              | Service  |<---HTTP----|  (credentials   |
              |          |            |   stored here)  |
              +----------+            +-----------------+
```

---

## MongoDB Models

### Node (collection: `nodes`)

Represents a registered node instance owned by a user.

```rust
// File: backend/src/models/node.rs

pub const COLLECTION_NAME: &str = "nodes";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeMetadata {
    pub agent_version: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub ip_address: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NodeMetrics {
    pub total_requests: u64,
    pub success_count: u64,
    pub error_count: u64,
    pub avg_latency_ms: f64,
    pub last_error: Option<String>,
    pub last_error_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    #[serde(rename = "_id")]
    pub id: String,                    // UUID v4
    pub user_id: String,
    pub name: String,                  // User-given name, e.g. "home-server"
    pub status: NodeStatus,            // Online | Offline | Draining
    pub auth_token_hash: String,       // SHA-256 hash of the node's auth token
    pub signing_secret_hash: String,   // SHA-256 hash of the HMAC signing secret
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub connected_at: Option<DateTime<Utc>>,
    pub metadata: Option<NodeMetadata>,
    #[serde(default)]
    pub metrics: NodeMetrics,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

### NodeServiceBinding (collection: `node_service_bindings`)

Maps which services a node handles for its owner. When a proxy request arrives for a bound service, NyxID routes it to the node instead of using stored credentials.

```rust
// File: backend/src/models/node_service_binding.rs

pub const COLLECTION_NAME: &str = "node_service_bindings";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeServiceBinding {
    #[serde(rename = "_id")]
    pub id: String,                    // UUID v4
    pub node_id: String,
    pub user_id: String,
    pub service_id: String,
    pub is_active: bool,
    /// Lower value = higher priority (for multi-node failover)
    #[serde(default)]
    pub priority: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

### NodeRegistrationToken (collection: `node_registration_tokens`)

One-time tokens for registering new nodes. Created via the management API, consumed during WebSocket registration handshake.

```rust
// File: backend/src/models/node_registration_token.rs

pub const COLLECTION_NAME: &str = "node_registration_tokens";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeRegistrationToken {
    #[serde(rename = "_id")]
    pub id: String,                    // UUID v4
    pub user_id: String,
    pub token_hash: String,            // SHA-256 hash of the one-time token
    pub name: String,                  // Pre-assigned name for the node
    pub used: bool,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}
```

### MongoDB Indexes

```rust
// nodes
{ "user_id": 1, "name": 1 }               // unique
{ "user_id": 1, "is_active": 1 }
{ "auth_token_hash": 1 }

// node_service_bindings
{ "node_id": 1, "service_id": 1 }         // unique
{ "user_id": 1, "service_id": 1, "is_active": 1 }
{ "node_id": 1, "is_active": 1 }

// node_registration_tokens
{ "token_hash": 1 }
{ "expires_at": 1 }                        // TTL index (auto-cleanup)
```

---

## Backend Services

### node_service (`backend/src/services/node_service.rs`)

Core CRUD and lifecycle operations for nodes.

```rust
/// Create a one-time registration token for a new node.
/// Returns (token_id, raw_token). The raw token is shown once and never stored.
pub async fn create_registration_token(
    db: &mongodb::Database,
    user_id: &str,
    name: &str,
    max_nodes_per_user: u32,
    ttl_secs: i64,
) -> AppResult<(String, String)>;

/// Consume a registration token and create a new Node record.
/// Returns (Node, raw_auth_token, raw_signing_secret). Called during WebSocket registration.
pub async fn register_node(
    db: &mongodb::Database,
    raw_token: &str,
    metadata: Option<NodeMetadata>,
) -> AppResult<(Node, String, String)>;

/// Get a single node by ID, verifying ownership.
pub async fn get_node(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
) -> AppResult<Node>;

/// List all nodes for a user.
pub async fn list_user_nodes(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<Node>>;

/// Soft-delete a node. Closes WebSocket if connected.
pub async fn delete_node(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
) -> AppResult<()>;

/// Update last_heartbeat_at and optionally metadata.
pub async fn update_heartbeat(
    db: &mongodb::Database,
    node_id: &str,
    metadata: Option<NodeMetadata>,
) -> AppResult<()>;

/// Set node status (Online | Offline | Draining).
pub async fn set_node_status(
    db: &mongodb::Database,
    node_id: &str,
    status: NodeStatus,
) -> AppResult<()>;

/// Rotate the node's auth token and signing secret. Invalidates old values immediately.
/// Returns (new_raw_auth_token, new_raw_signing_secret).
pub async fn rotate_auth_token(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
) -> AppResult<(String, String)>;

/// Validate a raw auth token. Returns the Node if valid.
pub async fn validate_auth_token(
    db: &mongodb::Database,
    raw_token: &str,
) -> AppResult<Node>;

// --- Binding operations ---

/// Create a service binding for a node.
pub async fn create_binding(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
    service_id: &str,
) -> AppResult<NodeServiceBinding>;

/// List all bindings for a node.
pub async fn list_bindings(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
) -> AppResult<Vec<NodeServiceBinding>>;

/// Delete a binding.
pub async fn delete_binding(
    db: &mongodb::Database,
    user_id: &str,
    binding_id: &str,
) -> AppResult<()>;

/// Update a binding's priority.
pub async fn update_binding_priority(
    db: &mongodb::Database,
    user_id: &str,
    binding_id: &str,
    priority: i32,
) -> AppResult<()>;
```

### node_routing_service (`backend/src/services/node_routing_service.rs`)

Routing decision logic: should a proxy request go through a user's node or through NyxID's standard proxy?

```rust
/// Result of a routing decision.
pub struct NodeRoute {
    pub node_id: String,
    /// Ordered list of fallback node IDs (for failover)
    pub fallback_node_ids: Vec<String>,
}

/// Check if a user has a node binding for this service.
/// Returns Some(NodeRoute) if the user has an active binding to an active,
/// online node. Returns None to fall through to standard proxy.
///
/// Selection logic:
/// 1. Find active bindings for (user_id, service_id) ordered by priority
/// 2. Batch-fetch the corresponding nodes
/// 3. Filter to nodes that are both DB-online AND WS-connected
/// 4. Skip unhealthy nodes (>50% error rate with >10 samples)
/// 5. Return first viable node as primary, rest as fallbacks
/// 6. Return None if no connected node found
pub async fn resolve_node_route(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
    ws_manager: &NodeWsManager,
) -> AppResult<Option<NodeRoute>>;
```

### node_ws_manager (`backend/src/services/node_ws_manager.rs`)

In-memory WebSocket connection pool. Manages active connections and provides request/response correlation for proxy forwarding. Shared via `Arc` in `AppState`; uses `DashMap` for lock-free concurrent access.

```rust
/// Request sent to a node via WebSocket.
#[derive(Clone)]
pub struct NodeProxyRequest {
    pub request_id: String,
    pub service_id: String,
    pub service_slug: String,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

/// Response received from a node via WebSocket.
pub struct NodeProxyResponse {
    pub request_id: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A pending proxy request that may receive a single response or a stream.
enum PendingRequest {
    /// Standard request/response
    OneShot(oneshot::Sender<NodeProxyResponse>),
    /// Streaming response: sends chunks through an mpsc channel
    Streaming(mpsc::UnboundedSender<StreamChunk>),
}

/// A chunk in a streaming response.
pub enum StreamChunk {
    Start { status: u16, headers: Vec<(String, String)> },
    Data(Vec<u8>),
    End,
    Error(String),
}

/// Handle for sending messages to a connected node.
struct NodeConnection {
    /// Bounded channel to send WS messages to the node's write task (capacity: 256).
    /// try_send treats full buffers as node offline.
    tx: mpsc::Sender<String>,
    /// Pending proxy request correlation map
    pending: Arc<DashMap<String, PendingRequest>>,
}

pub struct NodeWsManager {
    /// Active connections: node_id -> NodeConnection
    connections: DashMap<String, NodeConnection>,
    /// Proxy request timeout
    proxy_timeout_secs: u64,
}

impl NodeWsManager {
    pub fn new(proxy_timeout_secs: u64) -> Self;

    /// Register a new WebSocket connection for a node.
    pub fn register_connection(
        &self,
        node_id: &str,
    ) -> (mpsc::Sender<String>, Arc<DashMap<String, PendingRequest>>);

    /// Remove a node's connection (called on WS close).
    pub fn unregister_connection(&self, node_id: &str);

    /// Check if a node has an active WebSocket connection.
    pub fn is_connected(&self, node_id: &str) -> bool;

    /// Send a proxy request to a node and wait for the response.
    /// HMAC-signs the request if a signing secret is provided.
    /// Returns Err on timeout or if the node is not connected.
    pub async fn send_proxy_request(
        &self,
        node_id: &str,
        request: NodeProxyRequest,
        signing_secret: Option<&[u8]>,
    ) -> AppResult<NodeProxyResponse>;

    /// Send a heartbeat ping to a node. Non-blocking.
    pub fn send_heartbeat_ping(&self, node_id: &str) -> AppResult<()>;

    /// Get the IDs of all currently connected nodes.
    pub fn connected_node_ids(&self) -> Vec<String>;
}
```

**Streaming dispatch in the WS reader task (`node_ws.rs`):**

- `proxy_response` resolves the OneShot sender (non-streaming path)
- `proxy_response_start` upgrades the pending entry from OneShot to Streaming, sends `StreamChunk::Start`
- `proxy_response_chunk` sends `StreamChunk::Data` through the channel
- `proxy_response_end` sends `StreamChunk::End` and removes the entry

### node_metrics_service (`backend/src/services/node_metrics_service.rs`)

Per-node request/error/latency tracking. Metrics are recorded asynchronously (fire-and-forget) after each proxy request and stored as an embedded `NodeMetrics` document on the Node model.

```rust
/// Record a successful proxy request.
/// Uses exponential moving average (alpha=0.1) for latency.
/// Implemented as a MongoDB aggregation pipeline update for atomicity.
pub async fn record_success(
    db: &mongodb::Database,
    node_id: &str,
    latency_ms: u64,
) -> AppResult<()>;

/// Record a failed proxy request.
pub async fn record_error(
    db: &mongodb::Database,
    node_id: &str,
    error: &str,
) -> AppResult<()>;
```

---

## Backend Handlers

### node_admin.rs -- Node Management API

User-facing endpoints for managing nodes and bindings. All require standard `AuthUser` authentication (session/JWT/API key). Placed in `api_v1_human_only`.

```
POST   /api/v1/nodes/register-token                         Create a registration token
GET    /api/v1/nodes                                        List user's nodes
GET    /api/v1/nodes/{node_id}                              Get node details
DELETE /api/v1/nodes/{node_id}                              Delete/deregister a node
POST   /api/v1/nodes/{node_id}/rotate-token                 Rotate auth token + signing secret
GET    /api/v1/nodes/{node_id}/bindings                     List service bindings
POST   /api/v1/nodes/{node_id}/bindings                     Create a service binding
PATCH  /api/v1/nodes/{node_id}/bindings/{binding_id}        Update binding priority
DELETE /api/v1/nodes/{node_id}/bindings/{binding_id}        Remove a binding
```

**Request/Response schemas:**

```rust
// POST /nodes/register-token
#[derive(Deserialize)]
pub struct CreateRegistrationTokenRequest {
    pub name: String,  // 1-64 chars, alphanumeric + hyphens
}
#[derive(Serialize)]
pub struct CreateRegistrationTokenResponse {
    pub token_id: String,
    pub token: String,       // Raw token, shown only once: "nyx_nreg_..."
    pub name: String,
    pub expires_at: String,  // ISO 8601
}

// GET /nodes, GET /nodes/{node_id}
#[derive(Serialize)]
pub struct NodeInfo {
    pub id: String,
    pub name: String,
    pub status: String,
    pub is_connected: bool,  // Real-time from NodeWsManager
    pub last_heartbeat_at: Option<String>,
    pub connected_at: Option<String>,
    pub metadata: Option<NodeMetadata>,
    pub metrics: Option<NodeMetricsInfo>,
    pub binding_count: u64,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct NodeMetricsInfo {
    pub total_requests: u64,
    pub success_count: u64,
    pub error_count: u64,
    pub success_rate: f64,     // computed: success_count / total_requests
    pub avg_latency_ms: f64,
    pub last_error: Option<String>,
    pub last_error_at: Option<String>,
    pub last_success_at: Option<String>,
}

// POST /nodes/{node_id}/rotate-token
#[derive(Serialize)]
pub struct RotateTokenResponse {
    pub auth_token: String,       // New raw token: "nyx_nauth_..."
    pub signing_secret: String,   // New raw signing secret
    pub message: String,
}

// GET /nodes/{node_id}/bindings
#[derive(Serialize)]
pub struct BindingInfo {
    pub id: String,
    pub service_id: String,
    pub service_name: String,
    pub service_slug: String,
    pub is_active: bool,
    pub priority: i32,
    pub created_at: String,
}

// PATCH /nodes/{node_id}/bindings/{binding_id}
#[derive(Deserialize)]
pub struct UpdateBindingRequest {
    pub priority: Option<i32>,
}

// POST /nodes/{node_id}/bindings
#[derive(Deserialize)]
pub struct CreateBindingRequest {
    pub service_id: String,
}
```

### node_ws.rs -- WebSocket Handler

Handles WebSocket connections from node agents. Not behind standard auth middleware; authentication happens via the WS protocol.

```
GET /api/v1/nodes/ws    WebSocket upgrade (no standard auth)
```

```rust
/// WebSocket upgrade handler for node agent connections.
/// Authentication happens in the first message (register or auth).
/// If no valid auth message within 10 seconds, connection is closed.
pub async fn ws_handler(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse;

async fn handle_node_connection(state: AppState, socket: WebSocket) {
    // 1. Split into reader/writer
    // 2. Wait for auth/register message (10s timeout)
    // 3. Validate token, identify node
    // 4. Register connection in NodeWsManager
    // 5. Mark node as "online" in DB
    // 6. Spawn reader + writer tasks
    // 7. On disconnect: unregister, mark "offline"
}
```

### admin_nodes.rs -- Admin Node Management

System-wide node management for admins. Requires admin role; no ownership check.

```
GET    /api/v1/admin/nodes                          List all nodes across all users (paginated)
GET    /api/v1/admin/nodes/{node_id}                Get node details (any user's node)
POST   /api/v1/admin/nodes/{node_id}/disconnect     Force-disconnect a node
DELETE /api/v1/admin/nodes/{node_id}                Admin force-delete a node
```

```rust
#[derive(Deserialize)]
pub struct AdminNodeListQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub status: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Serialize)]
pub struct AdminNodeInfo {
    pub id: String,
    pub name: String,
    pub user_id: String,
    pub user_email: Option<String>,
    pub status: String,
    pub is_connected: bool,
    pub last_heartbeat_at: Option<String>,
    pub connected_at: Option<String>,
    pub metadata: Option<NodeMetadata>,
    pub metrics: Option<NodeMetricsInfo>,
    pub binding_count: u64,
    pub created_at: String,
}
```

---

## Proxy Handler Integration

The node routing decision point is inserted in `execute_proxy()` after the approval check and before the identity headers / credential resolution block.

### Routing and Failover

```rust
// In execute_proxy(), after approval check:
if let Some(node_route) = resolve_node_route(
    &state.db, &user_id_str, service_id, &state.node_ws_manager,
).await? {
    let all_nodes = std::iter::once(&node_route.node_id)
        .chain(node_route.fallback_node_ids.iter());

    for node_id in all_nodes {
        let start = std::time::Instant::now();
        match state.node_ws_manager.send_proxy_request(
            node_id, request.clone(), signing_secret.as_deref(),
        ).await {
            Ok(response) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                tokio::spawn(node_metrics_service::record_success(
                    state.db.clone(), node_id.clone(), latency_ms,
                ));
                // Build axum Response, audit with node_id, return
                return Ok(build_response(response));
            }
            Err(AppError::NodeOffline(_) | AppError::NodeProxyTimeout) => {
                tracing::warn!(node_id = %node_id, "Node failed, trying next");
                tokio::spawn(node_metrics_service::record_error(
                    state.db.clone(), node_id.clone(), e.to_string(),
                ));
                continue;
            }
            Err(e) => return Err(e), // Non-retryable error
        }
    }
    // All nodes failed -- fall through to standard proxy
}
// ... existing credential resolution and forwarding logic ...
```

On failover retry, a **new** `request_id` is generated to avoid correlation conflicts. `NodeProxyRequest` derives `Clone` to support this.

### Streaming Responses

When the server receives a `StreamChunk::Start` from the WS manager, it converts the channel receiver into an `axum::body::Body::from_stream()`:

```rust
match response_type {
    ResponseType::Complete(response) => {
        // Standard: build response from NodeProxyResponse
    }
    ResponseType::Streaming(mut rx) => {
        let start = rx.recv().await.ok_or(AppError::NodeOffline(...))?;
        let StreamChunk::Start { status, headers } = start else { ... };

        let stream = async_stream::stream! {
            while let Some(chunk) = rx.recv().await {
                match chunk {
                    StreamChunk::Data(bytes) => yield Ok(Bytes::from(bytes)),
                    StreamChunk::End => break,
                    StreamChunk::Error(e) => { tracing::error!(%e); break; }
                    _ => {}
                }
            }
        };

        Ok(Response::builder()
            .status(status)
            .body(Body::from_stream(stream))?)
    }
}
```

### Backpressure

WebSocket does not have native backpressure. Mitigation strategies:

1. **Chunk size limit**: Each `proxy_response_chunk` is capped at 64KB of base64-encoded data. Larger downstream chunks are split.
2. **Bounded channel**: The streaming channel uses `mpsc::channel(256)`. If the channel is full, the node-side stream pauses (backpressure propagates to the downstream HTTP connection).
3. **Proxy timeout**: The overall proxy timeout still applies to streaming requests.
4. **Max stream duration**: A separate configurable max stream duration (default: 300 seconds, via `NODE_MAX_STREAM_DURATION_SECS`) prevents runaway streams.

### Audit Trail

Node-routed proxy requests include extra fields in audit data:

```json
{
    "service_id": "...",
    "method": "POST",
    "path": "/v1/chat/completions",
    "response_status": 200,
    "routed_via": "node",
    "node_id": "..."
}
```

---

## HMAC Request Signing

### Shared Secret Lifecycle

- **Generated** server-side during registration: 32 bytes of cryptographic randomness, hex-encoded
- **Stored** server-side as a SHA-256 hash in `Node.signing_secret_hash`
- **Delivered** to the node agent in the `register_ok` WebSocket message (raw, shown once)
- **Stored** on the node agent as AES-256-GCM encrypted ciphertext in the config file (or in OS keychain)
- **Rotated** alongside auth token rotation (`rotate-token` endpoint returns both new auth token and new signing secret)
- **Configurable**: `NODE_HMAC_SIGNING_ENABLED` (default: `true`) controls whether signing is active

### Signature Computation

The server signs every `proxy_request` message with HMAC-SHA256 over a canonical string:

```
HMAC-SHA256(
    key = shared_secret_bytes,
    message = "{timestamp}\n{nonce}\n{method}\n{path}\n{query_or_empty}\n{body_b64_or_empty}"
)
```

The signature is hex-encoded in the `signature` field of the proxy_request message.

```rust
fn compute_hmac_signature(
    secret: &[u8],
    timestamp: &str,
    nonce: &str,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: Option<&[u8]>,
) -> String {
    let body_b64 = body
        .map(|b| base64::engine::general_purpose::STANDARD.encode(b))
        .unwrap_or_default();

    let message = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        timestamp, nonce, method, path,
        query.unwrap_or(""),
        body_b64,
    );

    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}
```

### Node-Side Verification

The node agent verifies signatures using constant-time comparison:

```rust
fn verify_request_signature(
    request: &serde_json::Value,
    secret: &str,
    expected_signature: &str,
) -> bool {
    // Recompute HMAC over the same canonical string
    // Use mac.verify_slice() for constant-time comparison
}
```

### Replay Protection

- **Timestamp check**: Node rejects requests with timestamps older than 5 minutes (`MAX_TIMESTAMP_SKEW_SECS = 300`)
- **Nonce tracking**: Node maintains a bounded set of recently seen nonces (last 10,000). Duplicate nonces are rejected. Nonces older than the timestamp skew window are evicted.

---

## Heartbeat and Health

### Server-Side Heartbeat Task

A background task runs at `NODE_HEARTBEAT_INTERVAL_SECS` (default: 30s) intervals:

```rust
// Spawned in main.rs
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(heartbeat_interval));
    loop {
        interval.tick().await;
        node_ws_manager_heartbeat_sweep(&db, &ws_manager, heartbeat_timeout).await;
    }
});
```

The sweep sends `heartbeat_ping` to all connected nodes. Nodes that have not responded within `NODE_HEARTBEAT_TIMEOUT_SECS` (default: 90s) are marked offline and their WebSocket connections are closed.

### Health-Aware Routing

The routing service uses metrics to skip unhealthy nodes during failover:

```rust
// Skip nodes with >50% error rate (if they have enough samples)
if node.metrics.total_requests > 10 {
    let error_rate = node.metrics.error_count as f64 / node.metrics.total_requests as f64;
    if error_rate > 0.5 {
        tracing::warn!(node_id = %node.id, error_rate, "Skipping unhealthy node");
        continue;
    }
}
```

---

## Node Agent Crate (`node-agent/`)

The `nyxid-node` agent is a separate workspace member alongside `backend/`.

### Crate Structure

```
node-agent/
  Cargo.toml
  src/
    main.rs              # CLI entry point, command dispatch
    cli.rs               # Clap subcommand definitions
    config.rs            # TOML config loading, secret storage backend selection
    ws_client.rs         # WebSocket connection + reconnection loop
    proxy_executor.rs    # HTTP request execution, credential injection, SSE streaming
    credential_store.rs  # In-memory decrypted credential store
    signing.rs           # HMAC-SHA256 verification, replay guard
    metrics.rs           # Local atomic counters (total_requests, success_count, error_count)
    encryption.rs        # AES-256-GCM local encryption, keyfile management (0600 mode)
    error.rs             # Error enum with thiserror
```

### Key Dependencies

```toml
tokio-tungstenite   # WebSocket client (with rustls-tls)
reqwest             # HTTP client for downstream requests (with stream support)
clap                # CLI parsing
toml                # Config file format
aes-gcm             # Local credential encryption
hmac + sha2         # HMAC signature verification
zeroize             # Secure memory cleanup
directories         # Platform-specific config paths
```

### Configuration File

Default path: `~/.nyxid-node/config.toml` (overridable with `--config`).

```toml
storage_backend = "file"  # or "keychain"

[server]
url = "wss://auth.nyxid.dev/api/v1/nodes/ws"

[node]
id = "uuid-string"
name = "my-home-server"
auth_token_encrypted = "base64-of-aes-gcm-ciphertext"

[credentials.openai]
injection_method = "header"
header_name = "Authorization"
header_value_encrypted = "base64-of-aes-gcm-ciphertext"

[signing]
shared_secret_encrypted = "base64-of-aes-gcm-ciphertext"
```

### Secret Storage Backends

**File backend** (default): Secrets are encrypted with AES-256-GCM using a file-based key at `~/.nyxid-node/.keyfile` (32-byte random key, permissions 0600). Works on all platforms including headless servers and containers.

**Keychain backend** (`--keychain` at registration): Secrets are stored in the OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service). The config file contains only non-secret metadata.

Migration between backends: `nyxid-node migrate --to keychain` or `nyxid-node migrate --to file`.

### WebSocket Client

The connection loop uses exponential backoff reconnection:

```rust
pub async fn run_connection_loop(config: &NodeConfig, credentials: &CredentialStore) -> ! {
    let mut backoff = ExponentialBackoff::new(
        Duration::from_millis(100), // initial
        Duration::from_secs(60),    // max
        2.0,                        // multiplier
    );

    loop {
        match connect_and_serve(config, credentials).await {
            Ok(()) => {
                // Clean disconnect: reset backoff, reconnect immediately
                backoff.reset();
            }
            Err(e) => {
                let delay = backoff.next_delay();
                tracing::warn!(error = %e, delay_ms = delay.as_millis(), "Connection failed");
                tokio::time::sleep(delay).await;
            }
        }
    }
}
```

A single connection lifecycle: connect via TLS WebSocket, send `auth` message, wait for `auth_ok`, then enter the main message loop handling `heartbeat_ping`, `proxy_request`, and `error` messages. Reader and writer run as separate tasks communicating through an mpsc channel.

### Proxy Executor

Handles both standard and streaming proxy responses:

1. Verify HMAC signature if signing is enabled
2. Look up credentials for the target service slug
3. Build the downstream HTTP request with forwarded headers
4. Inject credentials (header or query parameter)
5. Attach the request body (base64-decoded)
6. Execute the request
7. If the response is SSE/streaming (`text/event-stream`), use the streaming path
8. Otherwise, collect the full response and send a single `proxy_response`

**Streaming path:**

```rust
async fn stream_proxy_response(
    request_id: &str,
    response: reqwest::Response,
    tx: &mpsc::UnboundedSender<String>,
) {
    // Send proxy_response_start with status + headers
    // Stream chunks as proxy_response_chunk (base64-encoded)
    // Send proxy_response_end when the stream completes
    // On error, send proxy_error instead
}
```

### Credential Store

```rust
#[derive(Clone)]
pub struct CredentialStore {
    credentials: Arc<HashMap<String, ServiceCredential>>,
}

pub struct ServiceCredential {
    pub service_slug: String,
    pub injection_method: String, // "header" or "query_param"
    pub header_name: String,
    pub header_value: String,     // decrypted value
    pub param_name: String,       // for query_param injection
    pub param_value: String,
}
```

### Graceful Shutdown

On SIGINT/SIGTERM, the agent stops accepting new proxy requests and drains in-flight requests with a 30-second deadline before forcing shutdown.

---

## AppState Integration

```rust
pub struct AppState {
    // ... existing fields ...
    pub node_ws_manager: Arc<NodeWsManager>,
}
```

Initialized in `main()`:

```rust
let node_ws_manager = Arc::new(NodeWsManager::new(config.node_proxy_timeout_secs));
```

---

## Error Handling

Error variants for node operations (8000-series):

```rust
#[error("Node not found: {0}")]
NodeNotFound(String),           // 8000, HTTP 404

#[error("Node offline: {0}")]
NodeOffline(String),            // 8001, HTTP 503

#[error("Node proxy timeout")]
NodeProxyTimeout,               // 8002, HTTP 504

#[error("Node registration failed: {0}")]
NodeRegistrationFailed(String), // 8003, HTTP 400
```

---

## Environment Variables

All optional with sensible defaults:

| Variable | Default | Description |
|----------|---------|-------------|
| `NODE_HEARTBEAT_INTERVAL_SECS` | `30` | Heartbeat ping interval |
| `NODE_HEARTBEAT_TIMEOUT_SECS` | `90` | Mark offline after N seconds without heartbeat |
| `NODE_PROXY_TIMEOUT_SECS` | `30` | Timeout for proxy requests through nodes |
| `NODE_REGISTRATION_TOKEN_TTL_SECS` | `3600` | Registration token validity (1 hour) |
| `NODE_MAX_PER_USER` | `10` | Maximum nodes per user |
| `NODE_MAX_WS_CONNECTIONS` | `100` | Maximum concurrent node WebSocket connections |
| `NODE_MAX_STREAM_DURATION_SECS` | `300` | Maximum duration for streaming proxy responses |
| `NODE_HMAC_SIGNING_ENABLED` | `true` | Enable HMAC request signing for node proxy |

---

## Frontend

### Types (`frontend/src/types/`)

```typescript
interface NodeInfo {
    readonly id: string;
    readonly name: string;
    readonly status: string;
    readonly is_connected: boolean;
    readonly last_heartbeat_at: string | null;
    readonly connected_at: string | null;
    readonly metadata: NodeMetadata | null;
    readonly metrics: NodeMetricsInfo | null;
    readonly binding_count: number;
    readonly created_at: string;
}

interface NodeMetricsInfo {
    readonly total_requests: number;
    readonly success_count: number;
    readonly error_count: number;
    readonly success_rate: number;
    readonly avg_latency_ms: number;
    readonly last_error: string | null;
    readonly last_error_at: string | null;
    readonly last_success_at: string | null;
}

interface AdminNodeInfo extends NodeInfo {
    readonly user_id: string;
    readonly user_email: string | null;
}
```

### Hooks (`frontend/src/hooks/use-nodes.ts`, `use-admin-nodes.ts`)

```typescript
// Query hooks
useNodes()                              // GET /nodes
useNode(nodeId: string)                 // GET /nodes/{nodeId}
useNodeBindings(nodeId: string)         // GET /nodes/{nodeId}/bindings

// Mutation hooks
useCreateRegistrationToken()            // POST /nodes/register-token
useDeleteNode()                         // DELETE /nodes/{nodeId}
useRotateNodeToken()                    // POST /nodes/{nodeId}/rotate-token
useCreateBinding()                      // POST /nodes/{nodeId}/bindings
useDeleteBinding()                      // DELETE /nodes/{nodeId}/bindings/{bindingId}
useUpdateBindingPriority()              // PATCH /nodes/{nodeId}/bindings/{bindingId}
```

### Schemas (`frontend/src/schemas/nodes.ts`)

```typescript
const createRegistrationTokenSchema = z.object({
    name: z.string()
        .min(1, "Name is required")
        .max(64, "Name must be 64 characters or less")
        .regex(/^[a-z0-9][a-z0-9-]*[a-z0-9]$/, "Lowercase alphanumeric and hyphens only"),
});

const createBindingSchema = z.object({
    service_id: z.string().uuid("Invalid service ID"),
});
```

### Pages

| Page | Route | Description |
|------|-------|-------------|
| `pages/nodes.tsx` | `/nodes` | Node list with status indicators and management actions |
| `pages/node-detail.tsx` | `/nodes/$nodeId` | Node detail: status, metadata, metrics, bindings, token management |
| `pages/admin-nodes.tsx` | `/admin/nodes` | Admin: all nodes across all users, filter/search, force-disconnect/delete |

---

## Route Registration

Node management routes in `api_v1_human_only`:

```rust
let node_routes = Router::new()
    .route("/register-token", post(node_admin::create_registration_token))
    .route("/", get(node_admin::list_nodes))
    .route("/{node_id}", get(node_admin::get_node))
    .route("/{node_id}", delete(node_admin::delete_node))
    .route("/{node_id}/rotate-token", post(node_admin::rotate_token))
    .route("/{node_id}/bindings", get(node_admin::list_bindings))
    .route("/{node_id}/bindings", post(node_admin::create_binding))
    .route("/{node_id}/bindings/{binding_id}", patch(node_admin::update_binding))
    .route("/{node_id}/bindings/{binding_id}", delete(node_admin::delete_binding));
```

WebSocket route outside authenticated routes:

```rust
.route("/api/v1/nodes/ws", get(node_ws::ws_handler))
```

Admin routes under the admin router:

```rust
.route("/admin/nodes", get(admin_nodes::admin_list_nodes))
.route("/admin/nodes/{node_id}", get(admin_nodes::admin_get_node))
.route("/admin/nodes/{node_id}", delete(admin_nodes::admin_delete_node))
.route("/admin/nodes/{node_id}/disconnect", post(admin_nodes::admin_disconnect_node))
```

---

## Security Considerations

### Token Security
- Registration tokens (`nyx_nreg_`) and auth tokens (`nyx_nauth_`) are 32-byte cryptographic random values
- Only SHA-256 hashes are stored in the database; raw tokens are shown once
- Distinguishable prefixes enable identification and leak scanning
- Registration tokens are one-time use with configurable TTL

### Credential Isolation
- Credentials never transit through NyxID when using node proxy
- NyxID sends only request metadata (method, path, headers, body) to the node
- The node injects credentials locally before forwarding to the downstream service

### ACL Enforcement
- NyxID validates service bindings before forwarding to a node
- A node can only receive requests for services it has active bindings for
- Bindings are checked server-side, not trusting the node's self-reported capabilities

### WebSocket Security
- Connections must use WSS (TLS) in production
- Auth tokens are sent in the initial WS message, not as URL parameters (avoids server logs)
- Connections without valid auth within 10 seconds are terminated
- Heartbeat mechanism detects stale connections

### Node Limits
- Configurable max nodes per user (default: 10) prevents abuse
- Configurable max concurrent WS connections (default: 100)
- Proxy requests through nodes have a timeout (default: 30s)
- Streaming responses have a max duration (default: 300s)

### Audit Trail
- All node management operations are audit-logged
- Proxy requests routed through nodes include `routed_via: "node"` and `node_id` in audit data
- Node connection/disconnection events are logged
