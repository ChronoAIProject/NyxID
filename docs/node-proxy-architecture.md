# Node Proxy Architecture

## Overview

The Node Proxy feature introduces a **control plane / data plane** architecture for NyxID's credential proxy. Users can run lightweight "credential nodes" on their own infrastructure. Instead of storing credentials in NyxID's database, credentials stay on the user's node. When a proxy request arrives, NyxID routes it to the user's node via WebSocket, and the node injects credentials locally before forwarding the request to the downstream service.

This is an opt-in feature. Users without nodes continue using the existing proxy (credentials stored in NyxID). Users with nodes can selectively route specific services through their nodes while keeping others on NyxID.

## Architecture Diagram

```
                        ┌─────────────────────────────────┐
                        │          NyxID Server            │
    Client ──HTTP──►    │  ┌─────────┐  ┌──────────────┐   │
                        │  │  Proxy  │  │   Node WS    │   │
                        │  │ Handler ├──► Manager       │   │
                        │  └────┬────┘  └──────┬───────┘   │
                        │       │              │           │
                        │  ┌────▼────┐    WebSocket        │
                        │  │ Node    │         │           │
                        │  │ Router  │         │           │
                        │  └────┬────┘         │           │
                        └───────┼──────────────┼───────────┘
                                │              │
                     ┌──────────┘              │
                     │ (fallback)              │ (node route)
                     ▼                         ▼
              ┌──────────┐            ┌─────────────────┐
              │Downstream│            │  User's Node    │
              │ Service  │◄───HTTP────│  (credentials   │
              │          │            │   stored here)  │
              └──────────┘            └─────────────────┘
```

---

## New MongoDB Models

### 1. `Node` (collection: `nodes`)

Represents a registered node instance owned by a user.

```rust
// File: backend/src/models/node.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "nodes";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    #[serde(rename = "_id")]
    pub id: String,                    // UUID v4
    pub user_id: String,
    pub name: String,                  // User-given name, e.g. "home-server"
    /// "online" | "offline" | "draining"
    pub status: String,
    /// SHA-256 hash of the node's long-lived auth token
    pub auth_token_hash: String,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub connected_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<NodeMetadata>,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
```

### 2. `NodeServiceBinding` (collection: `node_service_bindings`)

Maps which services a node handles for its owner. When a proxy request arrives for a bound service, NyxID routes it to the node instead of using stored credentials.

```rust
// File: backend/src/models/node_service_binding.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "node_service_bindings";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeServiceBinding {
    #[serde(rename = "_id")]
    pub id: String,                    // UUID v4
    pub node_id: String,
    pub user_id: String,
    pub service_id: String,
    pub is_active: bool,
    /// Lower value = higher priority (for future multi-node failover)
    #[serde(default)]
    pub priority: i32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
```

### 3. `NodeRegistrationToken` (collection: `node_registration_tokens`)

One-time tokens for registering new nodes. Created via the management API, consumed during WebSocket registration handshake.

```rust
// File: backend/src/models/node_registration_token.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "node_registration_tokens";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeRegistrationToken {
    #[serde(rename = "_id")]
    pub id: String,                    // UUID v4
    pub user_id: String,
    /// SHA-256 hash of the one-time registration token
    pub token_hash: String,
    /// Pre-assigned name for the node that will be created
    pub name: String,
    pub used: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
```

---

## New Services

### 1. `node_service` (File: `backend/src/services/node_service.rs`)

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
/// Returns (Node, raw_auth_token). Called during WebSocket registration.
pub async fn register_node(
    db: &mongodb::Database,
    raw_token: &str,
    metadata: Option<NodeMetadata>,
) -> AppResult<(Node, String)>;

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

/// Set node status ("online" | "offline" | "draining").
pub async fn set_node_status(
    db: &mongodb::Database,
    node_id: &str,
    status: &str,
) -> AppResult<()>;

/// Rotate the node's auth token. Invalidates the old token immediately.
/// Returns the new raw auth token.
pub async fn rotate_auth_token(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
) -> AppResult<String>;

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
```

### 2. `node_routing_service` (File: `backend/src/services/node_routing_service.rs`)

Routing decision logic: should a proxy request go through a user's node or through NyxID's standard proxy?

```rust
/// Result of a routing decision.
pub struct NodeRoute {
    pub node_id: String,
    pub binding: NodeServiceBinding,
}

/// Check if a user has a node binding for this service.
/// Returns Some(NodeRoute) if the user has an active binding to an active,
/// online node. Returns None to fall through to standard proxy.
///
/// Selection logic:
/// 1. Find active bindings for (user_id, service_id) ordered by priority
/// 2. For each binding, check if the node is active
/// 3. Return the first binding whose node is connected (checked via NodeWsManager)
/// 4. Return None if no connected node found
pub async fn resolve_node_route(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<Option<NodeRoute>>;
```

### 3. `node_ws_manager` (File: `backend/src/services/node_ws_manager.rs`)

In-memory WebSocket connection pool. Manages active connections and provides request/response correlation for proxy forwarding.

```rust
use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::{mpsc, oneshot};

/// Request sent to a node via WebSocket.
pub struct NodeProxyRequest {
    pub request_id: String,
    pub service_id: String,
    pub service_slug: String,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,   // Raw bytes (not base64 -- serialized to base64 in WS message)
}

/// Response received from a node via WebSocket.
pub struct NodeProxyResponse {
    pub request_id: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Handle for sending messages to a connected node.
struct NodeConnection {
    /// Channel to send WS messages to the node's write task
    tx: mpsc::UnboundedSender<String>,
    /// Pending proxy request correlation map
    pending: Arc<DashMap<String, oneshot::Sender<NodeProxyResponse>>>,
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
    /// Returns a receiver for incoming WS messages from the node.
    pub fn register_connection(
        &self,
        node_id: &str,
    ) -> (mpsc::UnboundedSender<String>, Arc<DashMap<String, oneshot::Sender<NodeProxyResponse>>>);

    /// Remove a node's connection (called on WS close).
    pub fn unregister_connection(&self, node_id: &str);

    /// Check if a node has an active WebSocket connection.
    pub fn is_connected(&self, node_id: &str) -> bool;

    /// Send a proxy request to a node and wait for the response.
    /// Returns Err on timeout or if the node is not connected.
    pub async fn send_proxy_request(
        &self,
        node_id: &str,
        request: NodeProxyRequest,
    ) -> AppResult<NodeProxyResponse>;

    /// Send a heartbeat ping to a node. Non-blocking.
    pub fn send_heartbeat_ping(&self, node_id: &str) -> AppResult<()>;

    /// Get the IDs of all currently connected nodes.
    pub fn connected_node_ids(&self) -> Vec<String>;
}
```

---

## New Handlers

### 1. `handlers/node_admin.rs` -- Node Management API

User-facing endpoints for managing nodes and bindings. All require standard `AuthUser` authentication (session/JWT/API key). Placed in `api_v1_human_only`.

```
POST   /api/v1/nodes/register-token          Create a registration token
GET    /api/v1/nodes                          List user's nodes
GET    /api/v1/nodes/{node_id}                Get node details
DELETE /api/v1/nodes/{node_id}                Delete/deregister a node
POST   /api/v1/nodes/{node_id}/rotate-token   Rotate auth token
GET    /api/v1/nodes/{node_id}/bindings       List service bindings
POST   /api/v1/nodes/{node_id}/bindings       Create a service binding
DELETE /api/v1/nodes/{node_id}/bindings/{binding_id}  Remove a binding
```

**Request/Response schemas:**

```rust
// POST /api/v1/nodes/register-token
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

// GET /api/v1/nodes
#[derive(Serialize)]
pub struct NodeListResponse {
    pub nodes: Vec<NodeInfo>,
}
#[derive(Serialize)]
pub struct NodeInfo {
    pub id: String,
    pub name: String,
    pub status: String,
    pub is_connected: bool,  // Real-time from NodeWsManager
    pub last_heartbeat_at: Option<String>,
    pub connected_at: Option<String>,
    pub metadata: Option<NodeMetadata>,
    pub binding_count: u64,
    pub created_at: String,
}

// GET /api/v1/nodes/{node_id}
// Returns: NodeInfo (same as above)

// DELETE /api/v1/nodes/{node_id}
// Returns: 204 No Content

// POST /api/v1/nodes/{node_id}/rotate-token
#[derive(Serialize)]
pub struct RotateTokenResponse {
    pub auth_token: String,  // New raw token: "nyx_nauth_..."
    pub message: String,
}

// GET /api/v1/nodes/{node_id}/bindings
#[derive(Serialize)]
pub struct BindingListResponse {
    pub bindings: Vec<BindingInfo>,
}
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

// POST /api/v1/nodes/{node_id}/bindings
#[derive(Deserialize)]
pub struct CreateBindingRequest {
    pub service_id: String,
}
#[derive(Serialize)]
pub struct CreateBindingResponse {
    pub id: String,
    pub service_id: String,
    pub service_name: String,
    pub message: String,
}

// DELETE /api/v1/nodes/{node_id}/bindings/{binding_id}
// Returns: 204 No Content
```

### 2. `handlers/node_ws.rs` -- WebSocket Handler

Handles WebSocket connections from node agents. Not behind standard auth middleware; authentication happens via the WS protocol.

```
GET /api/v1/nodes/ws    WebSocket upgrade (no standard auth)
```

**Handler logic:**

```rust
/// GET /api/v1/nodes/ws
///
/// WebSocket upgrade handler for node agent connections.
/// Authentication happens in the first message (register or auth).
/// If no valid auth message within 10 seconds, connection is closed.
pub async fn ws_handler(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_node_connection(state, socket))
}

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

---

## WebSocket Protocol

All messages are JSON with a `type` field discriminator.

### Node -> NyxID (Client -> Server)

#### `register` -- First-time registration
```json
{
  "type": "register",
  "token": "nyx_nreg_<random_64_hex_chars>",
  "metadata": {
    "agent_version": "0.1.0",
    "os": "linux",
    "arch": "x86_64"
  }
}
```

#### `auth` -- Reconnection with auth token
```json
{
  "type": "auth",
  "node_id": "<uuid>",
  "token": "nyx_nauth_<random_64_hex_chars>"
}
```

#### `heartbeat_pong` -- Response to keepalive
```json
{
  "type": "heartbeat_pong",
  "timestamp": "2026-03-12T10:30:00Z"
}
```

#### `proxy_response` -- Response to a proxied request
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

#### `proxy_error` -- Error executing a proxy request
```json
{
  "type": "proxy_error",
  "request_id": "<uuid>",
  "error": "Connection refused",
  "status": 502
}
```

#### `status_update` -- Node health/capability update
```json
{
  "type": "status_update",
  "agent_version": "0.1.0",
  "services_ready": ["<service_id_1>", "<service_id_2>"]
}
```

### NyxID -> Node (Server -> Client)

#### `register_ok` -- Registration successful
```json
{
  "type": "register_ok",
  "node_id": "<uuid>",
  "auth_token": "nyx_nauth_<random_64_hex_chars>"
}
```

#### `auth_ok` -- Authentication successful
```json
{
  "type": "auth_ok",
  "node_id": "<uuid>"
}
```

#### `auth_error` -- Authentication failed (connection will be closed)
```json
{
  "type": "auth_error",
  "message": "Invalid or expired token"
}
```

#### `heartbeat_ping` -- Keepalive ping
```json
{
  "type": "heartbeat_ping",
  "timestamp": "2026-03-12T10:30:00Z"
}
```

#### `proxy_request` -- Forward an HTTP request for the node to execute
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
  "body": "<base64_encoded_request_body>"
}
```
Note: The `proxy_request` does NOT include credentials. The node is responsible for injecting its locally stored credentials.

#### `config_sync` -- Push binding configuration changes
```json
{
  "type": "config_sync",
  "bindings": [
    {
      "service_id": "<uuid>",
      "service_slug": "my-api",
      "service_name": "My API"
    }
  ]
}
```

#### `error` -- Server-side error
```json
{
  "type": "error",
  "message": "Internal error"
}
```

---

## Sequence Diagrams

### Node Registration Flow

```
User (Browser)          NyxID Server           Node Agent
     │                       │                      │
     │  POST /nodes/         │                      │
     │  register-token       │                      │
     │  {name: "my-node"}   │                      │
     │ ─────────────────────►│                      │
     │                       │ Create token record  │
     │  {token: "nyx_nreg_..│.", expires_at: ...}  │
     │ ◄─────────────────────│                      │
     │                       │                      │
     │  (User configures     │                      │
     │   node with token)    │                      │
     │                       │                      │
     │                       │  WS Connect          │
     │                       │  GET /api/v1/nodes/ws│
     │                       │ ◄────────────────────│
     │                       │  101 Switching Proto  │
     │                       │ ────────────────────►│
     │                       │                      │
     │                       │  { type: "register", │
     │                       │    token: "nyx_nreg_.│.." }
     │                       │ ◄────────────────────│
     │                       │                      │
     │                       │ Validate token       │
     │                       │ Create Node record   │
     │                       │ Generate auth token  │
     │                       │ Mark token as used   │
     │                       │                      │
     │                       │ { type: "register_ok│",
     │                       │   node_id: "...",    │
     │                       │   auth_token: "nyx_na│uth_..." }
     │                       │ ────────────────────►│
     │                       │                      │
     │                       │ (Node stores auth    │
     │                       │  token for reconnect)│
```

### Proxy Routing Flow (Node Route)

```
Client              NyxID Server          NodeWsManager       Node Agent        Downstream
  │                      │                     │                  │                │
  │ POST /proxy/s/my-api │                     │                  │                │
  │ /v1/completions      │                     │                  │                │
  │ ────────────────────►│                     │                  │                │
  │                      │                     │                  │                │
  │                      │ Auth user           │                  │                │
  │                      │ Check approval      │                  │                │
  │                      │                     │                  │                │
  │                      │ resolve_node_route  │                  │                │
  │                      │ (user, service)     │                  │                │
  │                      │ ───► Found binding  │                  │                │
  │                      │                     │                  │                │
  │                      │ is_connected?       │                  │                │
  │                      │ ───────────────────►│                  │                │
  │                      │ ◄──── true          │                  │                │
  │                      │                     │                  │                │
  │                      │ send_proxy_request  │                  │                │
  │                      │ ───────────────────►│                  │                │
  │                      │                     │ proxy_request    │                │
  │                      │                     │ (WS message)     │                │
  │                      │                     │ ────────────────►│                │
  │                      │                     │                  │                │
  │                      │                     │                  │ Inject creds   │
  │                      │                     │                  │ POST /v1/compl │
  │                      │                     │                  │ ──────────────►│
  │                      │                     │                  │ ◄──────────────│
  │                      │                     │                  │ 200 OK + body  │
  │                      │                     │                  │                │
  │                      │                     │ proxy_response   │                │
  │                      │                     │ (WS message)     │                │
  │                      │                     │ ◄────────────────│                │
  │                      │ ◄───────────────────│                  │                │
  │                      │ NodeProxyResponse   │                  │                │
  │                      │                     │                  │                │
  │ ◄────────────────────│                     │                  │                │
  │ 200 OK (response)    │                     │                  │                │
```

### Proxy Routing Flow (Fallback -- No Node)

```
Client              NyxID Server                           Downstream
  │                      │                                     │
  │ POST /proxy/s/my-api │                                     │
  │ ────────────────────►│                                     │
  │                      │ resolve_node_route -> None          │
  │                      │ (no binding or node offline)        │
  │                      │                                     │
  │                      │ resolve_proxy_target (existing)     │
  │                      │ Decrypt credential from DB          │
  │                      │ forward_request                     │
  │                      │ ───────────────────────────────────►│
  │                      │ ◄───────────────────────────────────│
  │ ◄────────────────────│                                     │
  │ 200 OK               │                                     │
```

### Heartbeat Flow

```
NyxID Server              Node Agent
     │                         │
     │ heartbeat_ping          │
     │ { timestamp: "..." }    │
     │ ───────────────────────►│
     │                         │
     │ heartbeat_pong          │
     │ { timestamp: "..." }    │
     │ ◄───────────────────────│
     │                         │
     │ (update last_heartbeat) │
     │                         │
     │  ... 30s later ...      │
     │                         │
     │ heartbeat_ping          │
     │ ───────────────────────►│
     │                         │
     │  ... no response ...    │
     │  ... 30s later ...      │
     │                         │
     │ heartbeat_ping          │
     │ ───────────────────────►│
     │                         │
     │  ... no response ...    │
     │  ... 30s later ...      │
     │                         │
     │ 3 missed: mark offline  │
     │ Close WebSocket         │
```

---

## Changes to Existing Files

### 1. `Cargo.toml` (workspace root)

Add `ws` feature to axum:

```toml
[workspace.dependencies]
axum = { version = "0.8.8", features = ["macros", "ws"] }
```

### 2. `backend/Cargo.toml`

Add `dashmap` dependency:

```toml
[dependencies]
dashmap = "6"
```

### 3. `backend/src/models/mod.rs`

Add new model modules:

```rust
pub mod node;
pub mod node_registration_token;
pub mod node_service_binding;
```

### 4. `backend/src/services/mod.rs`

Add new service modules:

```rust
pub mod node_routing_service;
pub mod node_service;
pub mod node_ws_manager;
```

### 5. `backend/src/handlers/mod.rs`

Add new handler modules:

```rust
pub mod node_admin;
pub mod node_ws;
```

### 6. `backend/src/main.rs` -- AppState

Add `node_ws_manager` field to `AppState`:

```rust
use crate::services::node_ws_manager::NodeWsManager;

pub struct AppState {
    // ... existing fields ...
    /// WebSocket connection manager for credential nodes
    pub node_ws_manager: Arc<NodeWsManager>,
}
```

Initialize in `main()`:

```rust
let node_ws_manager = Arc::new(NodeWsManager::new(
    config.node_proxy_timeout_secs,
));
```

Spawn the heartbeat background task:

```rust
let heartbeat_db = state.db.clone();
let heartbeat_ws = state.node_ws_manager.clone();
let heartbeat_interval = config.node_heartbeat_interval_secs;
let heartbeat_timeout = config.node_heartbeat_timeout_secs;
tokio::spawn(async move {
    let mut interval = tokio::time::interval(
        std::time::Duration::from_secs(heartbeat_interval),
    );
    loop {
        interval.tick().await;
        node_ws_manager_heartbeat_sweep(
            &heartbeat_db, &heartbeat_ws, heartbeat_timeout,
        ).await;
    }
});
```

### 7. `backend/src/config.rs` -- New Environment Variables

Add to `AppConfig`:

```rust
/// Heartbeat ping interval in seconds (default: 30)
pub node_heartbeat_interval_secs: u64,
/// Mark node offline after this many seconds without heartbeat (default: 90)
pub node_heartbeat_timeout_secs: u64,
/// Timeout for proxy requests routed through nodes (default: 30)
pub node_proxy_timeout_secs: u64,
/// Registration token validity in seconds (default: 3600 = 1 hour)
pub node_registration_token_ttl_secs: i64,
/// Maximum nodes per user (default: 10)
pub node_max_per_user: u32,
```

Parse from env in `from_env()`:

```rust
node_heartbeat_interval_secs: env::var("NODE_HEARTBEAT_INTERVAL_SECS")
    .ok().and_then(|v| v.parse().ok()).unwrap_or(30),
node_heartbeat_timeout_secs: env::var("NODE_HEARTBEAT_TIMEOUT_SECS")
    .ok().and_then(|v| v.parse().ok()).unwrap_or(90),
node_proxy_timeout_secs: env::var("NODE_PROXY_TIMEOUT_SECS")
    .ok().and_then(|v| v.parse().ok()).unwrap_or(30),
node_registration_token_ttl_secs: env::var("NODE_REGISTRATION_TOKEN_TTL_SECS")
    .ok().and_then(|v| v.parse().ok()).unwrap_or(3600),
node_max_per_user: env::var("NODE_MAX_PER_USER")
    .ok().and_then(|v| v.parse().ok()).unwrap_or(10),
```

### 8. `backend/src/routes.rs` -- New Routes

Add node management routes (human-only, authenticated):

```rust
let node_routes = Router::new()
    .route("/register-token", post(handlers::node_admin::create_registration_token))
    .route("/", get(handlers::node_admin::list_nodes))
    .route("/{node_id}", get(handlers::node_admin::get_node))
    .route("/{node_id}", delete(handlers::node_admin::delete_node))
    .route("/{node_id}/rotate-token", post(handlers::node_admin::rotate_token))
    .route("/{node_id}/bindings", get(handlers::node_admin::list_bindings))
    .route("/{node_id}/bindings", post(handlers::node_admin::create_binding))
    .route(
        "/{node_id}/bindings/{binding_id}",
        delete(handlers::node_admin::delete_binding),
    );
```

Add to `api_v1_human_only`:

```rust
let api_v1_human_only = Router::new()
    // ... existing routes ...
    .nest("/nodes", node_routes)  // NEW
    // ...
```

Add WebSocket route outside authenticated routes (in `private`):

```rust
let private = Router::new()
    .route("/health", get(handlers::health::health_check))
    .nest("/api/v1/webhooks", webhook_routes)
    .nest("/api/v1", api_v1)
    .route("/api/v1/nodes/ws", get(handlers::node_ws::ws_handler))  // NEW
    .route("/mcp", ...);
```

### 9. `backend/src/db.rs` -- New Indexes

Add to `ensure_indexes()`:

```rust
// ── nodes ──
let nodes = db.collection::<mongodb::bson::Document>("nodes");
nodes
    .create_index(
        IndexModel::builder()
            .keys(doc! { "user_id": 1, "name": 1 })
            .options(IndexOptions::builder().unique(true).build())
            .build(),
    )
    .await?;
nodes
    .create_index(
        IndexModel::builder()
            .keys(doc! { "user_id": 1, "is_active": 1 })
            .build(),
    )
    .await?;
nodes
    .create_index(
        IndexModel::builder()
            .keys(doc! { "auth_token_hash": 1 })
            .build(),
    )
    .await?;

// ── node_service_bindings ──
let nsb = db.collection::<mongodb::bson::Document>("node_service_bindings");
nsb.create_index(
    IndexModel::builder()
        .keys(doc! { "node_id": 1, "service_id": 1 })
        .options(IndexOptions::builder().unique(true).build())
        .build(),
)
.await?;
nsb.create_index(
    IndexModel::builder()
        .keys(doc! { "user_id": 1, "service_id": 1, "is_active": 1 })
        .build(),
)
.await?;
nsb.create_index(
    IndexModel::builder()
        .keys(doc! { "node_id": 1, "is_active": 1 })
        .build(),
)
.await?;

// ── node_registration_tokens ──
let nrt = db.collection::<mongodb::bson::Document>("node_registration_tokens");
nrt.create_index(
    IndexModel::builder()
        .keys(doc! { "token_hash": 1 })
        .build(),
)
.await?;
nrt.create_index(
    IndexModel::builder()
        .keys(doc! { "expires_at": 1 })
        .options(
            IndexOptions::builder()
                .expire_after(Duration::from_secs(0))
                .build(),
        )
        .build(),
)
.await?;
```

### 10. `backend/src/handlers/proxy.rs` -- Routing Integration

Modify `execute_proxy()` to add node routing decision point. Insert after the approval check block and before the identity headers / credential resolution block:

```rust
// === NEW: Node Proxy Routing ===
// Check if the user has a node binding for this service.
// If a node is online and connected, route the request through it.
// The node handles credential injection locally.
if let Some(node_route) = node_routing_service::resolve_node_route(
    &state.db,
    &user_id_str,
    service_id,
).await? {
    if state.node_ws_manager.is_connected(&node_route.node_id) {
        let method_str = request.method().as_str().to_string();
        let query = request.uri().query().map(String::from);
        let headers: Vec<(String, String)> = request.headers().iter()
            .filter_map(|(name, value)| {
                let name_lower = name.as_str().to_lowercase();
                if ALLOWED_FORWARD_HEADERS.contains(&name_lower.as_str()) {
                    value.to_str().ok().map(|v| (name.to_string(), v.to_string()))
                } else {
                    None
                }
            })
            .collect();

        let body_bytes = axum::body::to_bytes(request.into_body(), 10 * 1024 * 1024)
            .await
            .map_err(|e| AppError::BadRequest(format!("Failed to read body: {e}")))?;

        let node_request = NodeProxyRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            service_id: service_id.to_string(),
            service_slug: target.service.slug.clone(),
            method: method_str.clone(),
            path: path.to_string(),
            query,
            headers,
            body: if body_bytes.is_empty() { None } else { Some(body_bytes.to_vec()) },
        };

        let node_response = state.node_ws_manager
            .send_proxy_request(&node_route.node_id, node_request)
            .await?;

        // Build axum Response from node response
        let status = StatusCode::from_u16(node_response.status)
            .unwrap_or(StatusCode::BAD_GATEWAY);
        let mut response_builder = Response::builder().status(status);
        for (name, value) in &node_response.headers {
            let name_lower = name.to_lowercase();
            if ALLOWED_RESPONSE_HEADERS.contains(&name_lower.as_str()) {
                if let (Ok(hn), Ok(hv)) = (
                    axum::http::header::HeaderName::from_bytes(name.as_bytes()),
                    axum::http::header::HeaderValue::from_bytes(value.as_bytes()),
                ) {
                    response_builder = response_builder.header(hn, hv);
                }
            }
        }
        let response = response_builder
            .body(Body::from(node_response.body))
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))?;

        audit_service::log_async(
            state.db.clone(),
            Some(user_id_str),
            "proxy_request".to_string(),
            Some(serde_json::json!({
                "service_id": service_id,
                "method": method_str,
                "path": path,
                "response_status": status.as_u16(),
                "routed_via": "node",
                "node_id": node_route.node_id,
            })),
            None,
            None,
        );

        return Ok(response);
    }
    // Node not connected -- fall through to standard proxy
}
// === END Node Proxy Routing ===

// ... existing credential resolution and forwarding logic ...
```

### 11. `backend/src/errors/mod.rs` -- New Error Variants

Add to `AppError`:

```rust
#[error("Node not found: {0}")]
NodeNotFound(String),

#[error("Node offline: {0}")]
NodeOffline(String),

#[error("Node proxy timeout")]
NodeProxyTimeout,

#[error("Node registration failed: {0}")]
NodeRegistrationFailed(String),
```

Status codes:
```rust
Self::NodeNotFound(_) => StatusCode::NOT_FOUND,
Self::NodeOffline(_) => StatusCode::SERVICE_UNAVAILABLE,
Self::NodeProxyTimeout => StatusCode::GATEWAY_TIMEOUT,
Self::NodeRegistrationFailed(_) => StatusCode::BAD_REQUEST,
```

Error codes (8000-series for node errors):
```rust
Self::NodeNotFound(_) => 8000,
Self::NodeOffline(_) => 8001,
Self::NodeProxyTimeout => 8002,
Self::NodeRegistrationFailed(_) => 8003,
```

Error keys:
```rust
Self::NodeNotFound(_) => "node_not_found",
Self::NodeOffline(_) => "node_offline",
Self::NodeProxyTimeout => "node_proxy_timeout",
Self::NodeRegistrationFailed(_) => "node_registration_failed",
```

---

## New Environment Variables

```bash
# Node Proxy Configuration (all optional with defaults)
NODE_HEARTBEAT_INTERVAL_SECS=30        # Heartbeat ping interval (default: 30)
NODE_HEARTBEAT_TIMEOUT_SECS=90         # Mark offline after N seconds without heartbeat (default: 90)
NODE_PROXY_TIMEOUT_SECS=30             # Timeout for proxy requests through nodes (default: 30)
NODE_REGISTRATION_TOKEN_TTL_SECS=3600  # Registration token validity (default: 1 hour)
NODE_MAX_PER_USER=10                   # Maximum nodes per user (default: 10)
```

---

## Frontend Changes

### New Files

#### `frontend/src/hooks/use-nodes.ts`

TanStack Query hooks following the existing pattern (see `use-services.ts`):

```typescript
// Query hooks
export function useNodes()                                    // GET /nodes
export function useNode(nodeId: string)                       // GET /nodes/{nodeId}
export function useNodeBindings(nodeId: string)               // GET /nodes/{nodeId}/bindings

// Mutation hooks
export function useCreateRegistrationToken()                  // POST /nodes/register-token
export function useDeleteNode()                               // DELETE /nodes/{nodeId}
export function useRotateNodeToken()                          // POST /nodes/{nodeId}/rotate-token
export function useCreateBinding()                            // POST /nodes/{nodeId}/bindings
export function useDeleteBinding()                            // DELETE /nodes/{nodeId}/bindings/{bindingId}
```

#### `frontend/src/schemas/nodes.ts`

Zod schemas:

```typescript
import { z } from "zod";

export const createRegistrationTokenSchema = z.object({
  name: z.string()
    .min(1, "Name is required")
    .max(64, "Name must be 64 characters or less")
    .regex(/^[a-z0-9][a-z0-9-]*[a-z0-9]$/, "Lowercase alphanumeric and hyphens only"),
});

export const createBindingSchema = z.object({
  service_id: z.string().uuid("Invalid service ID"),
});

export type CreateRegistrationTokenFormData = z.infer<typeof createRegistrationTokenSchema>;
export type CreateBindingFormData = z.infer<typeof createBindingSchema>;
```

#### `frontend/src/types/api.ts` -- New Types

```typescript
export interface NodeInfo {
  readonly id: string;
  readonly name: string;
  readonly status: string;
  readonly is_connected: boolean;
  readonly last_heartbeat_at: string | null;
  readonly connected_at: string | null;
  readonly metadata: NodeMetadata | null;
  readonly binding_count: number;
  readonly created_at: string;
}

export interface NodeMetadata {
  readonly agent_version: string | null;
  readonly os: string | null;
  readonly arch: string | null;
  readonly ip_address: string | null;
}

export interface NodeBindingInfo {
  readonly id: string;
  readonly service_id: string;
  readonly service_name: string;
  readonly service_slug: string;
  readonly is_active: boolean;
  readonly priority: number;
  readonly created_at: string;
}

export interface CreateRegistrationTokenResponse {
  readonly token_id: string;
  readonly token: string;
  readonly name: string;
  readonly expires_at: string;
}

export interface RotateNodeTokenResponse {
  readonly auth_token: string;
  readonly message: string;
}
```

#### `frontend/src/pages/nodes.tsx`

Node list page showing all user's nodes with status indicators and management actions.

#### `frontend/src/pages/node-detail.tsx`

Node detail page showing status, metadata, service bindings, and management actions (rotate token, delete, manage bindings).

#### `frontend/src/pages/lazy.ts`

Add lazy imports:
```typescript
export const NodesPage = lazy(() => import("./nodes").then(m => ({ default: m.NodesPage })));
export const NodeDetailPage = lazy(() => import("./node-detail").then(m => ({ default: m.NodeDetailPage })));
```

#### `frontend/src/router.tsx`

Add routes under dashboard layout:

```typescript
const nodesRoute = createRoute({
  path: "/nodes",
  getParentRoute: () => dashboardLayout,
  component: NodesPage,
});

const nodeDetailRoute = createRoute({
  path: "/nodes/$nodeId",
  getParentRoute: () => dashboardLayout,
  component: NodeDetailPage,
});
```

Add to route tree:
```typescript
dashboardLayout.addChildren([
  // ... existing routes ...
  nodesRoute,
  nodeDetailRoute,
])
```

---

## Security Considerations

### 1. Token Security
- Registration tokens and auth tokens are cryptographically random (32 bytes / 64 hex chars)
- Only SHA-256 hashes are stored in the database; raw tokens are shown once
- Tokens use distinguishable prefixes (`nyx_nreg_` for registration, `nyx_nauth_` for auth) for identification and leak scanning
- Registration tokens are one-time use and have a configurable TTL (default 1 hour)

### 2. Credential Isolation
- Credentials never transit through NyxID when using node proxy
- NyxID sends only request metadata (method, path, headers, body) to the node
- The node injects credentials locally before forwarding to the downstream service
- This is the primary security benefit of the node proxy architecture

### 3. ACL Enforcement
- NyxID validates service bindings before forwarding to a node
- A node can only receive requests for services it has active bindings for
- Bindings are checked on the NyxID side, not trusting the node's self-reported capabilities
- Each binding requires explicit user creation via the authenticated management API

### 4. WebSocket Security
- WebSocket connections must use WSS (TLS) in production
- Auth tokens are sent in the initial message, not as URL parameters (avoids server logs)
- Connections without valid auth within 10 seconds are terminated
- Heartbeat mechanism detects stale connections (3 missed pings = disconnect)

### 5. Request Signing (Future Enhancement)
- Future: Sign proxy requests with HMAC so nodes can verify request integrity
- Future: Add request replay protection via nonce/timestamp

### 6. Node Limits
- Configurable max nodes per user (default 10) prevents abuse
- Registration tokens have TTL to prevent token hoarding
- Proxy requests through nodes have a timeout (default 30s) to prevent hanging

### 7. Audit Trail
- All node management operations are audit-logged
- Proxy requests routed through nodes include `routed_via: "node"` and `node_id` in audit data
- Node connection/disconnection events are logged

---

## Implementation Phases

### Phase 1: Core Infrastructure (Backend)
1. Add new models (node, binding, registration token)
2. Add DB indexes
3. Add config vars
4. Implement `node_service` (CRUD, token validation)
5. Implement `node_ws_manager` (in-memory connection pool, request/response correlation)
6. Implement `node_ws` handler (WebSocket lifecycle)
7. Implement `node_admin` handler (management API)
8. Add error variants
9. Add routes
10. Update AppState

### Phase 2: Proxy Integration (Backend)
1. Implement `node_routing_service` (routing decision)
2. Modify `execute_proxy` in proxy handler (add routing decision point)
3. Add heartbeat background task
4. Verify existing tests still pass

### Phase 3: Frontend
1. Add TypeScript types
2. Add Zod schemas
3. Add TanStack Query hooks
4. Build nodes list page
5. Build node detail page
6. Add routes to router
7. Add navigation link to sidebar

### Future Enhancements (Not in v1)
- **Streaming proxy responses**: Support `proxy_response_start` / `proxy_response_chunk` / `proxy_response_end` for SSE/streaming
- **Multi-node failover**: Route to secondary node if primary is offline (use `priority` field)
- **Request signing**: HMAC-sign proxy requests for integrity verification
- **Node metrics**: Track request counts, latency, error rates per node
- **Admin view**: Admin dashboard showing all nodes across users
