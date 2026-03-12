# Node Proxy v2 Architecture

## Overview

This document describes the v2 enhancements to the NyxID Node Proxy system. The v1 foundation (server-side WebSocket handler, node models, node service, routing service, WS manager, admin endpoints, and proxy handler integration) is already implemented on `feature/node-proxy`. The v2 deliverables are:

1. **nyxid-node agent binary** -- a new Rust crate that users run on their infrastructure
2. **Streaming proxy responses** -- SSE/chunked responses through the WebSocket tunnel
3. **Multi-node failover** -- priority-based routing with health-aware fallback
4. **HMAC request signing** -- integrity verification between server and node
5. **Node metrics** -- per-node request/error/latency tracking
6. **Admin view** -- system-wide node management for admins

## 1. nyxid-node Agent Binary

### 1.1 Crate Structure

A new workspace member `node-agent/` alongside `backend/`:

```
Cargo.toml (workspace)
  members = ["backend", "node-agent"]

node-agent/
  Cargo.toml
  src/
    main.rs           # CLI entry point (clap)
    cli.rs            # Subcommand definitions
    config.rs         # Config file loading (TOML)
    ws_client.rs      # WebSocket connection + reconnection loop
    proxy_executor.rs # HTTP request execution + credential injection
    credential_store.rs # Encrypted local credential storage
    signing.rs        # HMAC request verification
    metrics.rs        # Local metrics counters (for status_update)
```

### 1.2 Cargo.toml Dependencies

```toml
[package]
name = "nyxid-node"
version = "0.1.0"
edition = "2024"

[dependencies]
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
uuid = { workspace = true }
clap = { version = "4.5", features = ["derive"] }
toml = "0.8"
tokio-tungstenite = { version = "0.26", features = ["rustls-tls-native-roots"] }
futures = { workspace = true }
reqwest = { version = "0.12", features = ["json", "stream", "rustls-tls"], default-features = false }
base64 = "0.22"
sha2 = "0.10"
hmac = "0.12"
hex = "0.4"
aes-gcm = "0.10"
rand = "0.8"
zeroize = "1"
directories = "5"
```

### 1.3 CLI Interface

```
nyxid-node <SUBCOMMAND>

SUBCOMMANDS:
    register       Register this node with a NyxID server
    start          Start the node agent (connect and serve)
    status         Show node connection status
    credentials    Manage local credentials
    version        Show version information
```

#### `register`
```
nyxid-node register --token <nyx_nreg_...> [--url <wss://server/api/v1/nodes/ws>] [--name <node-name>] [--config <path>]
```

Connects to the NyxID server via WebSocket, sends the `register` message with the one-time token, receives `register_ok` with `node_id` and `auth_token`, and writes them to the config file. Exits after registration completes.

#### `start`
```
nyxid-node start [--config <path>] [--log-level <level>]
```

Loads the config file, connects via WebSocket, authenticates with the stored `auth_token`, and enters the main event loop (heartbeat responses + proxy request execution). Runs until SIGINT/SIGTERM.

#### `status`
```
nyxid-node status [--config <path>]
```

Reads the config file and prints node_id, server URL, number of configured credentials, and whether a connection is likely active (checks PID file). Does not connect to the server.

#### `credentials`
```
nyxid-node credentials add --service <slug> --header "Authorization: Bearer <key>"
nyxid-node credentials add --service <slug> --query-param "api_key=<key>"
nyxid-node credentials list [--config <path>]
nyxid-node credentials remove --service <slug>
```

### 1.4 Configuration File

Default path: `~/.nyxid-node/config.toml` (overridable with `--config`).

Created by `register`, updated by `credentials` commands.

```toml
[server]
url = "wss://auth.nyxid.dev/api/v1/nodes/ws"

[node]
id = "uuid-string"
name = "my-home-server"
# Auth token is stored encrypted; plaintext only in memory
auth_token_encrypted = "base64-of-aes-gcm-ciphertext"

[credentials.openai]
# service_slug = injection rules
injection_method = "header"
header_name = "Authorization"
header_value_encrypted = "base64-of-aes-gcm-ciphertext"

[credentials.github-api]
injection_method = "header"
header_name = "Authorization"
header_value_encrypted = "base64-of-aes-gcm-ciphertext"

[signing]
# HMAC shared secret, established during registration
# Stored encrypted like credentials
shared_secret_encrypted = "base64-of-aes-gcm-ciphertext"
```

#### Local Encryption

Credentials in the config file are encrypted at rest using AES-256-GCM. The encryption key is derived from a passphrase using Argon2id, or from a machine-specific key stored in the OS keychain (platform-dependent). For v2 MVP, we use a file-based key at `~/.nyxid-node/.keyfile` (a 32-byte random key generated on first `register`, permissions 0600). The `directories` crate provides platform-appropriate paths.

```rust
/// Encryption for local credential storage.
/// Key is loaded from ~/.nyxid-node/.keyfile (generated on first register).
pub struct LocalEncryption {
    key: Zeroizing<[u8; 32]>,
}

impl LocalEncryption {
    /// Load or generate the local encryption key.
    pub fn load_or_generate(config_dir: &Path) -> Result<Self, Error>;

    /// Encrypt a plaintext string. Returns base64-encoded nonce+ciphertext.
    pub fn encrypt(&self, plaintext: &str) -> Result<String, Error>;

    /// Decrypt a base64-encoded nonce+ciphertext. Returns plaintext string.
    pub fn decrypt(&self, ciphertext: &str) -> Result<String, Error>;
}
```

### 1.5 WebSocket Client

```rust
/// Main connection loop with exponential backoff reconnection.
pub async fn run_connection_loop(config: &NodeConfig, credentials: &CredentialStore) -> ! {
    let mut backoff = ExponentialBackoff::new(
        Duration::from_millis(100), // initial
        Duration::from_secs(60),    // max
        2.0,                        // multiplier
    );

    loop {
        match connect_and_serve(config, credentials).await {
            Ok(()) => {
                // Clean disconnect (e.g., server sent close frame)
                tracing::info!("Disconnected cleanly, reconnecting...");
                backoff.reset();
            }
            Err(e) => {
                let delay = backoff.next_delay();
                tracing::warn!(error = %e, delay_ms = delay.as_millis(), "Connection failed, retrying");
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// Single connection lifecycle: connect, authenticate, serve requests.
async fn connect_and_serve(
    config: &NodeConfig,
    credentials: &CredentialStore,
) -> Result<(), Error> {
    // 1. WebSocket connect (TLS)
    let (ws_stream, _) = tokio_tungstenite::connect_async(&config.server_url).await?;
    let (mut ws_sink, mut ws_stream) = ws_stream.split();

    // 2. Authenticate
    let auth_msg = serde_json::json!({
        "type": "auth",
        "node_id": config.node_id,
        "token": config.auth_token_decrypted,
    });
    ws_sink.send(Message::Text(auth_msg.to_string())).await?;

    // 3. Wait for auth_ok
    let response = ws_stream.next().await??;
    let parsed: serde_json::Value = serde_json::from_str(&response.into_text()?)?;
    if parsed["type"] != "auth_ok" {
        return Err(Error::AuthFailed(parsed["message"].as_str().unwrap_or("unknown").to_string()));
    }

    tracing::info!("Authenticated with NyxID server");

    // 4. Main message loop
    // Split into reader + writer with an mpsc channel
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Writer task
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_sink.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    // Reader loop
    while let Some(msg) = ws_stream.next().await {
        let text = msg?.into_text()?;
        let parsed: serde_json::Value = serde_json::from_str(&text)?;

        match parsed["type"].as_str() {
            Some("heartbeat_ping") => {
                let pong = serde_json::json!({
                    "type": "heartbeat_pong",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                });
                tx.send(pong.to_string())?;
            }
            Some("proxy_request") => {
                let tx_clone = tx.clone();
                let creds = credentials.clone();
                let request_id = parsed["request_id"].as_str().unwrap_or("").to_string();
                let signing_secret = config.signing_secret.clone();

                tokio::spawn(async move {
                    let response = execute_proxy_request(&parsed, &creds, signing_secret.as_deref()).await;
                    let _ = tx_clone.send(response);
                });
            }
            Some("error") => {
                tracing::error!(message = %parsed["message"], "Server error");
            }
            _ => {
                tracing::debug!(msg_type = %parsed["type"], "Unknown message type");
            }
        }
    }

    writer.abort();
    Ok(())
}
```

### 1.6 Proxy Executor

```rust
/// Execute a proxied HTTP request locally with credential injection.
pub async fn execute_proxy_request(
    request: &serde_json::Value,
    credentials: &CredentialStore,
    signing_secret: Option<&str>,
) -> String {
    let request_id = request["request_id"].as_str().unwrap_or("");
    let service_slug = request["service_slug"].as_str().unwrap_or("");

    // 1. Verify HMAC signature if signing is enabled
    if let Some(secret) = signing_secret {
        if let Some(signature) = request["signature"].as_str() {
            if !verify_request_signature(request, secret, signature) {
                return proxy_error_response(request_id, "HMAC signature verification failed", 403);
            }
        }
    }

    // 2. Look up credentials for this service
    let cred = match credentials.get(service_slug) {
        Some(c) => c,
        None => {
            return proxy_error_response(
                request_id,
                &format!("No credentials configured for service '{service_slug}'"),
                502,
            );
        }
    };

    // 3. Build the downstream HTTP request
    let method = request["method"].as_str().unwrap_or("GET");
    let path = request["path"].as_str().unwrap_or("/");
    let query = request["query"].as_str();

    let base_url = request["base_url"].as_str().unwrap_or("");
    let mut url = format!("{}{}", base_url.trim_end_matches('/'), path);
    if let Some(q) = query {
        url = format!("{url}?{q}");
    }

    let client = reqwest::Client::new();
    let mut req_builder = client.request(
        reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET),
        &url,
    );

    // 4. Forward headers from the proxy_request
    if let Some(headers) = request["headers"].as_object() {
        for (name, value) in headers {
            if let Some(v) = value.as_str() {
                req_builder = req_builder.header(name.as_str(), v);
            }
        }
    }

    // 5. Inject credentials
    match cred.injection_method.as_str() {
        "header" => {
            req_builder = req_builder.header(&cred.header_name, &cred.header_value);
        }
        "query_param" => {
            let separator = if url.contains('?') { "&" } else { "?" };
            url = format!("{url}{separator}{}={}", cred.param_name, cred.param_value);
            req_builder = client.request(
                reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET),
                &url,
            );
            if let Some(headers) = request["headers"].as_object() {
                for (name, value) in headers {
                    if let Some(v) = value.as_str() {
                        req_builder = req_builder.header(name.as_str(), v);
                    }
                }
            }
        }
        _ => {}
    }

    // 6. Attach body
    if let Some(body_b64) = request["body"].as_str() {
        if let Ok(body_bytes) = base64::engine::general_purpose::STANDARD.decode(body_b64) {
            req_builder = req_builder.body(body_bytes);
        }
    }

    // 7. Execute request
    match req_builder.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let is_streaming = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.contains("text/event-stream"));

            if is_streaming {
                return proxy_stream_start(request_id, status, &response);
            }

            let headers = extract_response_headers(&response);
            let body = response.bytes().await.unwrap_or_default();
            let body_b64 = base64::engine::general_purpose::STANDARD.encode(&body);

            serde_json::json!({
                "type": "proxy_response",
                "request_id": request_id,
                "status": status,
                "headers": headers,
                "body": body_b64,
            }).to_string()
        }
        Err(e) => {
            proxy_error_response(request_id, &format!("Downstream request failed: {e}"), 502)
        }
    }
}

fn proxy_error_response(request_id: &str, error: &str, status: u16) -> String {
    serde_json::json!({
        "type": "proxy_error",
        "request_id": request_id,
        "error": error,
        "status": status,
    }).to_string()
}
```

### 1.7 Credential Store

```rust
/// In-memory credential store loaded from config file.
#[derive(Clone)]
pub struct CredentialStore {
    credentials: Arc<HashMap<String, ServiceCredential>>,
}

/// A single service's credential configuration.
#[derive(Clone)]
pub struct ServiceCredential {
    pub service_slug: String,
    pub injection_method: String, // "header" or "query_param"
    pub header_name: String,      // e.g., "Authorization"
    pub header_value: String,     // decrypted value, e.g., "Bearer sk-..."
    pub param_name: String,       // for query_param injection
    pub param_value: String,      // for query_param injection
}

impl CredentialStore {
    /// Load credentials from config file, decrypting each value.
    pub fn from_config(config: &NodeConfig, encryption: &LocalEncryption) -> Result<Self, Error>;

    /// Get credentials for a service slug.
    pub fn get(&self, service_slug: &str) -> Option<&ServiceCredential>;
}
```

### 1.8 Graceful Shutdown

```rust
/// Graceful shutdown: drain in-flight requests before exit.
async fn run_with_shutdown(config: NodeConfig, credentials: CredentialStore) {
    let shutdown = tokio::signal::ctrl_c();
    let in_flight = Arc::new(AtomicUsize::new(0));

    tokio::select! {
        _ = run_connection_loop(&config, &credentials, in_flight.clone()) => {},
        _ = shutdown => {
            tracing::info!("Shutdown signal received, draining in-flight requests...");
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            while in_flight.load(Ordering::Relaxed) > 0
                && tokio::time::Instant::now() < deadline
            {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let remaining = in_flight.load(Ordering::Relaxed);
            if remaining > 0 {
                tracing::warn!(remaining, "Forcing shutdown with in-flight requests");
            }
            tracing::info!("Shutdown complete");
        }
    }
}
```

---

## 2. Streaming Proxy Responses

### 2.1 New WebSocket Message Types

Three new message types for streaming responses through the WebSocket tunnel:

```
Node -> NyxID:

proxy_response_start:
{
    "type": "proxy_response_start",
    "request_id": "<uuid>",
    "status": 200,
    "headers": { "content-type": "text/event-stream", ... }
}

proxy_response_chunk:
{
    "type": "proxy_response_chunk",
    "request_id": "<uuid>",
    "data": "<base64_encoded_chunk>"
}

proxy_response_end:
{
    "type": "proxy_response_end",
    "request_id": "<uuid>"
}
```

### 2.2 Server-Side Changes (node_ws_manager.rs)

Replace the single `oneshot::Sender<NodeProxyResponse>` with a new `StreamingResponse` type:

```rust
/// A pending proxy request that may receive a single response or a stream.
enum PendingRequest {
    /// Standard request/response (v1 behavior)
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
```

The WS reader task in `node_ws.rs` dispatches:
- `proxy_response` -> resolves the OneShot sender (unchanged)
- `proxy_response_start` -> upgrades the pending entry from OneShot to Streaming, sends `StreamChunk::Start`
- `proxy_response_chunk` -> sends `StreamChunk::Data` through the channel
- `proxy_response_end` -> sends `StreamChunk::End` and removes the entry

### 2.3 Server-Side Changes (proxy.rs)

When the server receives a `StreamChunk::Start`, it converts the mpsc receiver into an `axum::body::Body::from_stream()`:

```rust
// In execute_proxy(), after send_proxy_request:
match response_type {
    ResponseType::Complete(response) => {
        // Existing v1 behavior: build response from NodeProxyResponse
    }
    ResponseType::Streaming(mut rx) => {
        // Wait for Start chunk
        let start = rx.recv().await.ok_or(AppError::NodeOffline(...))?;
        let StreamChunk::Start { status, headers } = start else { ... };

        // Build streaming response
        let stream = async_stream::stream! {
            while let Some(chunk) = rx.recv().await {
                match chunk {
                    StreamChunk::Data(bytes) => yield Ok::<_, std::io::Error>(bytes::Bytes::from(bytes)),
                    StreamChunk::End => break,
                    StreamChunk::Error(e) => {
                        tracing::error!(error = %e, "Stream error from node");
                        break;
                    }
                    _ => {}
                }
            }
        };

        let body = Body::from_stream(stream);
        let mut response = Response::builder().status(status);
        // Add headers...
        Ok(response.body(body)?)
    }
}
```

### 2.4 Node-Side Streaming

In the proxy executor, when the downstream response is SSE/streaming:

```rust
async fn stream_proxy_response(
    request_id: &str,
    response: reqwest::Response,
    tx: &mpsc::UnboundedSender<String>,
) {
    let status = response.status().as_u16();
    let headers = extract_response_headers(&response);

    // Send start
    let start_msg = serde_json::json!({
        "type": "proxy_response_start",
        "request_id": request_id,
        "status": status,
        "headers": headers,
    });
    let _ = tx.send(start_msg.to_string());

    // Stream chunks
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                let chunk_msg = serde_json::json!({
                    "type": "proxy_response_chunk",
                    "request_id": request_id,
                    "data": base64::engine::general_purpose::STANDARD.encode(&bytes),
                });
                if tx.send(chunk_msg.to_string()).is_err() {
                    break; // WS disconnected
                }
            }
            Err(e) => {
                let err_msg = serde_json::json!({
                    "type": "proxy_error",
                    "request_id": request_id,
                    "error": format!("Stream error: {e}"),
                    "status": 502,
                });
                let _ = tx.send(err_msg.to_string());
                return;
            }
        }
    }

    // Send end
    let end_msg = serde_json::json!({
        "type": "proxy_response_end",
        "request_id": request_id,
    });
    let _ = tx.send(end_msg.to_string());
}
```

### 2.5 Backpressure

WebSocket does not have native backpressure. Mitigation:

1. **Chunk size limit**: Each `proxy_response_chunk` is capped at 64KB of base64-encoded data. Larger downstream chunks are split.
2. **Bounded channel**: Use `mpsc::channel(256)` instead of unbounded for the streaming channel. If the channel is full, the node-side stream pauses (backpressure propagates to the downstream HTTP connection).
3. **Timeout**: The overall proxy timeout still applies to streaming requests. If the stream doesn't complete within `NODE_PROXY_TIMEOUT_SECS`, the server cancels it.
4. **Max stream duration**: A separate configurable max stream duration (default: 300 seconds) prevents runaway streams.

---

## 3. Multi-Node Failover

### 3.1 Enhanced Routing Logic

The existing `resolve_node_route` already fetches bindings ordered by `priority` and batch-fetches nodes. Enhance it to support failover:

```rust
/// Result of a routing decision.
pub struct NodeRoute {
    pub node_id: String,
    /// Ordered list of fallback node IDs (for failover)
    pub fallback_node_ids: Vec<String>,
}

pub async fn resolve_node_route(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
    ws_manager: &NodeWsManager,
) -> AppResult<Option<NodeRoute>> {
    let bindings = /* ... fetch ordered by priority ... */;
    let nodes = /* ... batch fetch ... */;
    let online_nodes: HashMap<&str, &Node> = /* ... */;

    // Filter to nodes that are both DB-online AND WS-connected
    let mut viable_nodes: Vec<String> = Vec::new();
    for binding in &bindings {
        if let Some(node) = online_nodes.get(binding.node_id.as_str()) {
            if ws_manager.is_connected(&node.id) {
                viable_nodes.push(node.id.clone());
            }
        }
    }

    if viable_nodes.is_empty() {
        return Ok(None);
    }

    Ok(Some(NodeRoute {
        node_id: viable_nodes[0].clone(),
        fallback_node_ids: viable_nodes[1..].to_vec(),
    }))
}
```

### 3.2 Failover in Proxy Handler

In `execute_proxy()`, when the primary node fails (offline, timeout, error), try fallback nodes:

```rust
if let Some(node_route) = resolve_node_route(&state.db, &user_id_str, service_id, &state.node_ws_manager).await? {
    let all_nodes = std::iter::once(&node_route.node_id)
        .chain(node_route.fallback_node_ids.iter());

    for node_id in all_nodes {
        match state.node_ws_manager.send_proxy_request(node_id, request.clone()).await {
            Ok(response) => {
                // Success -- return response, audit with node_id
                return Ok(build_response(response));
            }
            Err(AppError::NodeOffline(_) | AppError::NodeProxyTimeout) => {
                tracing::warn!(node_id = %node_id, "Node failed, trying next");
                node_metrics::record_error(&state.db, node_id).await;
                continue;
            }
            Err(e) => return Err(e), // Non-retryable error
        }
    }
    // All nodes failed -- fall through to standard proxy
}
```

### 3.3 Priority Management API

Add an endpoint to update binding priority:

```
PATCH /api/v1/nodes/{node_id}/bindings/{binding_id}
Body: { "priority": 1 }
```

Handler in `node_admin.rs`:

```rust
/// PATCH /api/v1/nodes/{node_id}/bindings/{binding_id}
pub async fn update_binding(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((node_id, binding_id)): Path<(String, String)>,
    Json(body): Json<UpdateBindingRequest>,
) -> AppResult<impl IntoResponse> { ... }

#[derive(Debug, Deserialize)]
pub struct UpdateBindingRequest {
    pub priority: Option<i32>,
}
```

### 3.4 NodeProxyRequest Must Be Cloneable

For failover retry, `NodeProxyRequest` needs `Clone`:

```rust
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
```

Note: On failover retry, generate a **new** `request_id` to avoid correlation conflicts.

---

## 4. HMAC Request Signing

### 4.1 Shared Secret Establishment

During node registration (`register_ok` response), the server includes a newly generated HMAC shared secret:

```json
{
    "type": "register_ok",
    "node_id": "<uuid>",
    "auth_token": "nyx_nauth_<64_hex_chars>",
    "signing_secret": "<64_hex_chars>"
}
```

The signing secret is:
- Generated server-side: 32 bytes of cryptographic randomness, hex-encoded
- SHA-256 hash stored in the Node model (new field: `signing_secret_hash`)
- Raw secret shown once and stored encrypted in the node agent config
- Rotated alongside auth token rotation (`rotate-token` endpoint returns a new signing secret too)

### 4.2 Node Model Changes

Add to `Node` struct:

```rust
pub struct Node {
    // ... existing fields ...
    /// SHA-256 hash of the HMAC signing secret
    pub signing_secret_hash: String,
}
```

### 4.3 Request Signing Protocol

The server signs every `proxy_request` message with HMAC-SHA256. New fields added to the proxy_request message:

```json
{
    "type": "proxy_request",
    "request_id": "<uuid>",
    "timestamp": "2026-03-12T10:30:00.000Z",
    "nonce": "<uuid>",
    "service_id": "...",
    "service_slug": "...",
    "method": "POST",
    "path": "/v1/chat/completions",
    "query": "stream=true",
    "headers": { ... },
    "body": "<base64>",
    "signature": "<hex_encoded_hmac>"
}
```

#### Signature Computation

The HMAC-SHA256 is computed over a canonical string:

```
HMAC-SHA256(
    key = shared_secret_bytes,
    message = "{timestamp}\n{nonce}\n{method}\n{path}\n{query_or_empty}\n{body_b64_or_empty}"
)
```

The signature is hex-encoded in the `signature` field.

### 4.4 Server-Side Signing (node_ws_manager.rs)

```rust
impl NodeWsManager {
    pub async fn send_proxy_request(
        &self,
        node_id: &str,
        request: NodeProxyRequest,
        signing_secret: Option<&[u8]>,
    ) -> AppResult<NodeProxyResponse> {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let nonce = uuid::Uuid::new_v4().to_string();

        let signature = signing_secret.map(|secret| {
            compute_hmac_signature(
                secret,
                &timestamp,
                &nonce,
                &request.method,
                &request.path,
                request.query.as_deref(),
                request.body.as_deref(),
            )
        });

        let ws_msg = WsProxyRequest {
            msg_type: "proxy_request",
            request_id: request.request_id.clone(),
            timestamp: Some(timestamp),
            nonce: Some(nonce),
            signature,
            // ... existing fields ...
        };

        // ... rest unchanged ...
    }
}

fn compute_hmac_signature(
    secret: &[u8],
    timestamp: &str,
    nonce: &str,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: Option<&[u8]>,
) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let body_b64 = body.map(|b| base64::engine::general_purpose::STANDARD.encode(b))
        .unwrap_or_default();

    let message = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        timestamp, nonce, method, path,
        query.unwrap_or(""),
        body_b64,
    );

    let mut mac = Hmac::<Sha256>::new_from_slice(secret)
        .expect("HMAC accepts any key size");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}
```

### 4.5 Node-Side Verification

```rust
/// Verify the HMAC signature on a proxy request.
fn verify_request_signature(
    request: &serde_json::Value,
    secret: &str,
    expected_signature: &str,
) -> bool {
    let secret_bytes = hex::decode(secret).unwrap_or_default();

    let timestamp = request["timestamp"].as_str().unwrap_or("");
    let nonce = request["nonce"].as_str().unwrap_or("");
    let method = request["method"].as_str().unwrap_or("");
    let path = request["path"].as_str().unwrap_or("");
    let query = request["query"].as_str().unwrap_or("");
    let body = request["body"].as_str().unwrap_or("");

    let message = format!("{timestamp}\n{nonce}\n{method}\n{path}\n{query}\n{body}");

    let mut mac = Hmac::<Sha256>::new_from_slice(&secret_bytes).unwrap();
    mac.update(message.as_bytes());

    // Constant-time comparison
    mac.verify_slice(&hex::decode(expected_signature).unwrap_or_default()).is_ok()
}
```

### 4.6 Replay Protection

- **Timestamp check**: Node rejects requests with timestamps older than 5 minutes (`MAX_TIMESTAMP_SKEW_SECS = 300`)
- **Nonce tracking**: Node maintains a bounded set of recently seen nonces (last 10,000). Duplicate nonces are rejected. Nonces older than the timestamp skew window are evicted.

---

## 5. Node Metrics

### 5.1 New Model: NodeMetrics

Stored as fields on the existing `Node` document (not a separate collection) to avoid an extra query on every request:

```rust
/// Per-node metrics. Stored as an embedded document in the Node model.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NodeMetrics {
    /// Total proxy requests handled
    pub total_requests: u64,
    /// Successful proxy responses (2xx-4xx from downstream)
    pub success_count: u64,
    /// Failed proxy requests (node errors, timeouts, 5xx)
    pub error_count: u64,
    /// Average response latency in milliseconds (exponential moving average)
    pub avg_latency_ms: f64,
    /// Last error message (for diagnostics)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Timestamp of the last error
    #[serde(default, with = "bson_datetime::optional")]
    pub last_error_at: Option<DateTime<Utc>>,
    /// Timestamp of the last successful request
    #[serde(default, with = "bson_datetime::optional")]
    pub last_success_at: Option<DateTime<Utc>>,
}
```

Add to Node model:

```rust
pub struct Node {
    // ... existing fields ...
    #[serde(default)]
    pub metrics: NodeMetrics,
}
```

### 5.2 Metrics Recording Service

```rust
// services/node_metrics_service.rs

/// Record a successful proxy request.
pub async fn record_success(
    db: &mongodb::Database,
    node_id: &str,
    latency_ms: u64,
) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());
    db.collection::<Node>(NODES)
        .update_one(
            doc! { "_id": node_id },
            vec![doc! {
                "$set": {
                    "metrics.total_requests": { "$add": ["$metrics.total_requests", 1] },
                    "metrics.success_count": { "$add": ["$metrics.success_count", 1] },
                    "metrics.avg_latency_ms": {
                        "$add": [
                            { "$multiply": [0.9, { "$ifNull": ["$metrics.avg_latency_ms", latency_ms as f64] }] },
                            { "$multiply": [0.1, latency_ms as f64] },
                        ]
                    },
                    "metrics.last_success_at": now,
                    "updated_at": now,
                }
            }],
        )
        .await?;
    Ok(())
}

/// Record a failed proxy request.
pub async fn record_error(
    db: &mongodb::Database,
    node_id: &str,
    error: &str,
) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());
    db.collection::<Node>(NODES)
        .update_one(
            doc! { "_id": node_id },
            doc! {
                "$inc": {
                    "metrics.total_requests": 1,
                    "metrics.error_count": 1,
                },
                "$set": {
                    "metrics.last_error": error,
                    "metrics.last_error_at": &now,
                    "updated_at": &now,
                }
            },
        )
        .await?;
    Ok(())
}
```

### 5.3 Metrics in Proxy Handler

In `execute_proxy()`, wrap the node proxy call with timing:

```rust
let start = std::time::Instant::now();
let result = state.node_ws_manager.send_proxy_request(&node_id, request).await;
let latency_ms = start.elapsed().as_millis() as u64;

match &result {
    Ok(_) => {
        tokio::spawn(node_metrics_service::record_success(
            state.db.clone(), node_id.clone(), latency_ms,
        ));
    }
    Err(e) => {
        tokio::spawn(node_metrics_service::record_error(
            state.db.clone(), node_id.clone(), e.to_string(),
        ));
    }
}
```

### 5.4 Metrics in API Responses

Add metrics to `NodeInfo` response:

```rust
#[derive(Debug, Serialize)]
pub struct NodeInfo {
    // ... existing fields ...
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<NodeMetricsInfo>,
}

#[derive(Debug, Serialize)]
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
```

### 5.5 Health-Aware Routing

Use metrics for smarter failover decisions in `resolve_node_route`:

```rust
// Skip nodes with >50% error rate in the last hour (if they have enough samples)
if node.metrics.total_requests > 10 {
    let error_rate = node.metrics.error_count as f64 / node.metrics.total_requests as f64;
    if error_rate > 0.5 {
        tracing::warn!(node_id = %node.id, error_rate, "Skipping unhealthy node");
        continue;
    }
}
```

---

## 6. Admin View

### 6.1 New Admin Endpoints

```
GET    /api/v1/admin/nodes              List all nodes across all users (paginated)
GET    /api/v1/admin/nodes/{node_id}    Get node details (any user's node)
POST   /api/v1/admin/nodes/{node_id}/disconnect   Force-disconnect a node
DELETE /api/v1/admin/nodes/{node_id}    Admin force-delete a node
```

### 6.2 Admin Handler

```rust
// handlers/admin_nodes.rs (new file)

/// GET /api/v1/admin/nodes
pub async fn admin_list_nodes(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(query): Query<AdminNodeListQuery>,
) -> AppResult<Json<AdminNodeListResponse>> {
    // List all nodes with optional status filter, pagination
    // Include user_email for each node (join with users collection)
}

#[derive(Debug, Deserialize)]
pub struct AdminNodeListQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub status: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AdminNodeListResponse {
    pub nodes: Vec<AdminNodeInfo>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

#[derive(Debug, Serialize)]
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

/// POST /api/v1/admin/nodes/{node_id}/disconnect
pub async fn admin_disconnect_node(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(node_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    if state.node_ws_manager.is_connected(&node_id) {
        state.node_ws_manager.unregister_connection(&node_id);
        node_service::set_node_status(&state.db, &node_id, NodeStatus::Offline).await?;
    }

    audit_service::log_async(
        state.db.clone(),
        None,
        "admin_node_disconnected".to_string(),
        Some(serde_json::json!({ "node_id": &node_id })),
        None,
        None,
    );

    Ok(StatusCode::NO_CONTENT)
}
```

### 6.3 Routes Registration

Add to `routes.rs` under the admin router:

```rust
.route("/admin/nodes", get(handlers::admin_nodes::admin_list_nodes))
.route("/admin/nodes/{node_id}", get(handlers::admin_nodes::admin_get_node))
.route("/admin/nodes/{node_id}", delete(handlers::admin_nodes::admin_delete_node))
.route("/admin/nodes/{node_id}/disconnect", post(handlers::admin_nodes::admin_disconnect_node))
```

### 6.4 Frontend Admin Page

New page at `/admin/nodes` (added to admin section in sidebar):

- Table listing all nodes across all users
- Columns: Name, User, Status (with badge), Connected, Heartbeat, Requests, Error Rate, Latency, Actions
- Filter by status (Online/Offline/All)
- Search by user email or node name
- Actions column: Disconnect (force), Delete
- Click row to navigate to node detail with full metrics

---

## 7. Protocol Changes Summary

### New WebSocket Messages (Node -> NyxID)

| Type | When | Fields |
|------|------|--------|
| `proxy_response_start` | Beginning of a streaming proxy response | `request_id`, `status`, `headers` |
| `proxy_response_chunk` | Each chunk of streaming data | `request_id`, `data` (base64) |
| `proxy_response_end` | End of streaming response | `request_id` |

### Modified WebSocket Messages (NyxID -> Node)

| Type | Change | New Fields |
|------|--------|------------|
| `register_ok` | Add signing secret | `signing_secret` |
| `proxy_request` | Add signing fields + base_url | `timestamp`, `nonce`, `signature`, `base_url` |

### New API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `PATCH` | `/api/v1/nodes/{node_id}/bindings/{binding_id}` | Update binding priority |
| `GET` | `/api/v1/admin/nodes` | Admin: list all nodes |
| `GET` | `/api/v1/admin/nodes/{node_id}` | Admin: get node details |
| `DELETE` | `/api/v1/admin/nodes/{node_id}` | Admin: force-delete node |
| `POST` | `/api/v1/admin/nodes/{node_id}/disconnect` | Admin: force-disconnect node |

### New Error Codes

No new error codes needed. The existing 8000-8003 range covers all node error scenarios.

---

## 8. New Files

### Backend (server-side)

| File | Purpose |
|------|---------|
| `backend/src/services/node_metrics_service.rs` | Record success/error metrics per node |
| `backend/src/handlers/admin_nodes.rs` | Admin node management endpoints |

### Node Agent (new crate)

| File | Purpose |
|------|---------|
| `node-agent/Cargo.toml` | Crate manifest |
| `node-agent/src/main.rs` | CLI entry point |
| `node-agent/src/cli.rs` | Clap subcommand definitions |
| `node-agent/src/config.rs` | TOML config loading + local encryption |
| `node-agent/src/ws_client.rs` | WebSocket connection + reconnection loop |
| `node-agent/src/proxy_executor.rs` | HTTP request execution + credential injection |
| `node-agent/src/credential_store.rs` | In-memory credential store |
| `node-agent/src/signing.rs` | HMAC signature verification + replay protection |
| `node-agent/src/metrics.rs` | Local metrics counters |

### Frontend

| File | Purpose |
|------|---------|
| `frontend/src/pages/admin-nodes.tsx` | Admin nodes list page |

---

## 9. Model Changes Summary

### Node (modified)

```rust
pub struct Node {
    // ... existing fields ...
    pub signing_secret_hash: String,  // NEW: SHA-256 hash of HMAC secret
    #[serde(default)]
    pub metrics: NodeMetrics,          // NEW: embedded metrics document
}
```

### NodeMetrics (new embedded struct)

```rust
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
```

---

## 10. Environment Variables (new/modified)

| Variable | Default | Description |
|----------|---------|-------------|
| `NODE_MAX_STREAM_DURATION_SECS` | `300` | Maximum duration for streaming proxy responses |
| `NODE_HMAC_SIGNING_ENABLED` | `true` | Enable HMAC request signing (disable for backwards compat) |

---

## 11. Implementation Order

### Phase 1: nyxid-node Agent Binary (Task #2)
1. Create `node-agent/` crate with workspace setup
2. Implement CLI (`clap` subcommands)
3. Implement config file loading/saving (TOML + local encryption)
4. Implement WebSocket client with auth handshake
5. Implement credential store + `credentials` CLI commands
6. Implement proxy executor (non-streaming)
7. Implement heartbeat response
8. Implement reconnection with exponential backoff
9. Implement graceful shutdown
10. Test end-to-end: register, start, proxy request, disconnect, reconnect

### Phase 2: Server-Side v2 Features (Task #3)
1. Add `signing_secret_hash` and `metrics` to Node model
2. Implement `node_metrics_service.rs` (record_success, record_error)
3. Wire metrics recording into `execute_proxy()`
4. Add metrics to `NodeInfo` API responses
5. Implement HMAC signing in `node_ws_manager.rs`
6. Update `register_ok` to include `signing_secret`
7. Update `rotate-token` to return new `signing_secret`
8. Add streaming WS message types (`proxy_response_start/chunk/end`)
9. Implement `PendingRequest` enum (OneShot vs Streaming) in `node_ws_manager`
10. Wire streaming into `execute_proxy()` (Body::from_stream)
11. Enhance `resolve_node_route` for multi-node failover
12. Add failover retry loop in `execute_proxy()`
13. Make `NodeProxyRequest` Clone
14. Add binding priority update endpoint
15. Implement admin node endpoints (`admin_nodes.rs`)
16. Register admin routes in `routes.rs`
17. Add `base_url` to proxy_request WS messages

### Phase 3: Node Agent v2 Features (back to Task #2)
1. Add HMAC verification to proxy executor
2. Add replay protection (timestamp check + nonce tracking)
3. Implement streaming proxy responses (stream_proxy_response)
4. Add `base_url` handling to proxy executor
5. Update `register` flow to save signing_secret

### Phase 4: Frontend (Task #4)
1. Add metrics display to node detail page
2. Add streaming indicators to proxy status
3. Create admin nodes page
4. Add admin nodes link to sidebar

---

## 12. Risks and Mitigations

| Risk | Severity | Mitigation |
|------|----------|------------|
| WebSocket message ordering during streaming | HIGH | Use request_id correlation; chunks are ordered within a single WS connection |
| Large streaming responses filling WS buffers | HIGH | Bounded channel (256 chunks), chunk size cap (64KB), max stream duration |
| HMAC clock skew between server and node | MEDIUM | 5-minute timestamp tolerance; document NTP requirement |
| Config file corruption losing auth token | HIGH | Atomic writes (write temp, rename); backup on rotation |
| Failover retry with request body for non-idempotent methods | MEDIUM | Only retry on node-level errors (offline/timeout), not downstream errors; log warning for non-GET retries |
| In-flight requests during token rotation | LOW | Rotation closes WS, pending requests get RecvError, client retries |
| Node agent binary size (Rust + TLS) | LOW | Use `strip = true` in release profile; static linking optional |

---

## 13. Testing Strategy

### Unit Tests
- HMAC signature computation + verification (round-trip)
- Credential store encryption/decryption
- Exponential backoff timing
- Config file serialization/deserialization
- Nonce replay detection
- Metrics EMA calculation

### Integration Tests
- End-to-end: register node, send proxy_request, receive proxy_response
- Streaming: start -> chunks -> end flow
- Failover: primary fails, secondary handles request
- HMAC: signed request accepted, tampered request rejected
- Token rotation: old connection closed, new connection works
- Graceful shutdown: in-flight requests complete

### E2E Tests (manual or CI)
- `nyxid-node register` with real server
- `nyxid-node credentials add/list/remove`
- `nyxid-node start` with proxy traffic
- Network disconnect + automatic reconnection
- Admin force-disconnect from dashboard
