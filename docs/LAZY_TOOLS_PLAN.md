# Lazy Tool Loading for MCP -- Implementation Plan

## Problem

When many downstream services are connected, the MCP server exposes 80+ tools upfront via `tools/list`. This degrades LLM performance because large tool lists consume context window and confuse tool selection.

## Solution

Start each MCP session with only 3 meta-tools. Dynamically activate service tools when the LLM calls `nyx__search_tools` or `nyx__connect_service`. Send `notifications/tools/list_changed` so clients refresh their tool list.

---

## Architecture Overview

```
                  tools/list
Client  <-----------------------------  Server
         returns: meta-tools + activated

                  tools/call nyx__search_tools("github")
Client  ----------------------------->  Server
         activates github tools
         <--- SSE: notifications/tools/list_changed
         <--- tool result with matches

                  tools/list (client auto-refreshes)
Client  <-----------------------------  Server
         returns: meta-tools + github tools
```

---

## File Changes

### 1. `backend/src/models/mcp_session.rs` -- Add per-session activated state

**Current state:** `McpSession` stores only `user_id` and `last_active`.

**Changes:**

Add an `activated_service_ids` set and a notification channel to `McpSession`:

```rust
use std::collections::HashSet;
use tokio::sync::mpsc;

pub struct McpSession {
    pub user_id: String,
    pub last_active: DateTime<Utc>,
    /// Service IDs whose tools are currently exposed in tools/list.
    pub activated_service_ids: HashSet<String>,
    /// Channel to send JSON-RPC notifications to the SSE stream.
    /// None if no SSE listener is connected.
    pub notification_tx: Option<mpsc::Sender<serde_json::Value>>,
}
```

Add a constant for maximum activated services per session:

```rust
/// Maximum number of services that can be activated per session.
/// Prevents unbounded memory growth.
pub const MAX_ACTIVATED_SERVICES: usize = 20;
```

Add methods to `McpSessionStore`:

```rust
impl McpSessionStore {
    /// Create a new session. Returns (session_id, notification_rx).
    /// The rx end is held by the SSE stream; the tx end is stored in the session.
    pub fn create(&self, user_id: &str) -> (String, mpsc::Receiver<serde_json::Value>) {
        let (tx, rx) = mpsc::channel(32);
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = McpSession {
            user_id: user_id.to_string(),
            last_active: Utc::now(),
            activated_service_ids: HashSet::new(),
            notification_tx: Some(tx),
        };
        self.sessions
            .write()
            .expect("lock poisoned")
            .insert(session_id.clone(), session);
        (session_id, rx)
    }

    /// Activate services for a session. Returns true if any were newly activated.
    /// Enforces MAX_ACTIVATED_SERVICES.
    pub fn activate_services(
        &self,
        session_id: &str,
        service_ids: &[String],
    ) -> bool {
        let mut sessions = self.sessions.write().expect("lock poisoned");
        let session = match sessions.get_mut(session_id) {
            Some(s) => s,
            None => return false,
        };
        let mut changed = false;
        for id in service_ids {
            if session.activated_service_ids.len() >= MAX_ACTIVATED_SERVICES {
                break;
            }
            if session.activated_service_ids.insert(id.clone()) {
                changed = true;
            }
        }
        changed
    }

    /// Get the set of activated service IDs for a session.
    pub fn get_activated_service_ids(&self, session_id: &str) -> HashSet<String> {
        self.sessions
            .read()
            .expect("lock poisoned")
            .get(session_id)
            .map(|s| s.activated_service_ids.clone())
            .unwrap_or_default()
    }

    /// Send a JSON-RPC notification to the session's SSE stream.
    /// Returns true if sent successfully, false if no listener or channel full.
    pub fn send_notification(
        &self,
        session_id: &str,
        notification: serde_json::Value,
    ) -> bool {
        let sessions = self.sessions.read().expect("lock poisoned");
        if let Some(session) = sessions.get(session_id) {
            if let Some(tx) = &session.notification_tx {
                return tx.try_send(notification).is_ok();
            }
        }
        false
    }

    /// Attach a new notification sender (e.g., when SSE reconnects).
    pub fn set_notification_tx(
        &self,
        session_id: &str,
        tx: mpsc::Sender<serde_json::Value>,
    ) {
        if let Some(session) = self
            .sessions
            .write()
            .expect("lock poisoned")
            .get_mut(session_id)
        {
            session.notification_tx = Some(tx);
        }
    }
}
```

**Note on `create()` signature change:** The return type changes from `String` to `(String, mpsc::Receiver<serde_json::Value>)`. All call sites must be updated.

---

### 2. `backend/src/services/mcp_service.rs` -- Filter tool generation by activated set

**Changes to `generate_tool_definitions()`:**

Rename the current function and add a new one that filters:

```rust
/// Generate MCP tool definitions for ONLY the activated services.
/// Always includes the three `nyx__` meta-tools.
/// `activated_service_ids` controls which services' tools are included.
/// If None, all services are included (backward compat for REST /mcp/config).
pub fn generate_tool_definitions(
    services: &[McpToolService],
    activated_service_ids: Option<&HashSet<String>>,
) -> Vec<McpToolDefinition> {
    let mut tools = Vec::new();

    // -- Meta-tools (always present, unchanged) --
    tools.push(/* nyx__search_tools -- same as current */);
    tools.push(/* nyx__discover_services -- same as current */);
    tools.push(/* nyx__connect_service -- same as current */);

    // -- Per-service tools (filtered by activated set) --
    for service in services {
        let included = match activated_service_ids {
            Some(ids) => ids.contains(&service.service_id),
            None => true, // No filter = include all
        };
        if !included {
            continue;
        }
        for endpoint in &service.endpoints {
            // ... (existing per-endpoint logic, unchanged)
        }
    }

    tools
}
```

**Changes to `search_tools()`:**

The function currently searches already-generated tool definitions. For lazy loading, it needs to search ALL user tools (not just activated), return matches, AND return which service IDs matched so the caller can activate them.

Add a new struct and function:

```rust
pub struct SearchResult {
    pub matches: Vec<McpToolDefinition>,
    /// Service IDs that had matching tools (for activation).
    pub matched_service_ids: Vec<String>,
}

/// Search ALL user tools (regardless of activation state) and return matches
/// plus the service IDs they belong to.
pub fn search_all_tools(
    services: &[McpToolService],
    query: &str,
) -> SearchResult {
    let q_lower = query.to_lowercase();
    let mut matches = Vec::new();
    let mut matched_ids: HashSet<String> = HashSet::new();

    for service in services {
        for endpoint in &service.endpoints {
            let name = format!("{}__{}", service.service_slug, endpoint.name);
            let description = format!(
                "[{}] {}",
                service.service_name,
                endpoint.description.as_deref().unwrap_or(&endpoint.name),
            );

            if name.to_lowercase().contains(&q_lower)
                || description.to_lowercase().contains(&q_lower)
            {
                matched_ids.insert(service.service_id.clone());
                matches.push(McpToolDefinition {
                    name,
                    description,
                    input_schema: build_input_schema(endpoint),
                });
                if matches.len() >= MAX_SEARCH_RESULTS {
                    break;
                }
            }
        }
        if matches.len() >= MAX_SEARCH_RESULTS {
            break;
        }
    }

    SearchResult {
        matches,
        matched_service_ids: matched_ids.into_iter().collect(),
    }
}
```

Keep the old `search_tools()` function but mark it `#[allow(dead_code)]` or remove it, since the transport handler will use `search_all_tools()` instead.

---

### 3. `backend/src/handlers/mcp_transport.rs` -- Wire up lazy loading

#### 3a. `handle_initialize()` -- Advertise `listChanged: true`, store rx

**Before:**
```rust
"capabilities": {
    "tools": { "listChanged": false },
},
```

**After:**
```rust
"capabilities": {
    "tools": { "listChanged": true },
},
```

The `create()` call now returns `(session_id, notification_rx)`. The `notification_rx` must be stored somewhere accessible to the SSE handler. Since `mcp_post` and `mcp_get` are separate handler functions, we need a way to pass the rx to the SSE stream.

**Strategy:** Store the `Receiver` in a separate map on `AppState` (or within `McpSessionStore`). When the SSE handler (`mcp_get`) opens, it takes the receiver from the map.

Add to `McpSessionStore`:

```rust
/// Pending notification receivers, waiting for SSE connection.
/// Key: session_id, Value: Receiver
pending_receivers: Arc<RwLock<HashMap<String, mpsc::Receiver<serde_json::Value>>>>,
```

In `create()`, store the rx:

```rust
self.pending_receivers
    .write()
    .expect("lock poisoned")
    .insert(session_id.clone(), rx);
```

Add a method to take the receiver:

```rust
/// Take the pending notification receiver for a session.
/// Returns None if already taken or session doesn't exist.
pub fn take_notification_rx(
    &self,
    session_id: &str,
) -> Option<mpsc::Receiver<serde_json::Value>> {
    self.pending_receivers
        .write()
        .expect("lock poisoned")
        .remove(session_id)
}
```

In `handle_initialize`, update the call:

```rust
fn handle_initialize(state: &AppState, user_id: &str, request: &JsonRpcRequest) -> Response {
    let session_id = state.mcp_sessions.create(user_id);
    // notification_rx is stored internally, SSE handler will take it

    let result = serde_json::json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {
            "tools": { "listChanged": true },
        },
        "serverInfo": {
            "name": "NyxID",
            "version": env!("CARGO_PKG_VERSION"),
        }
    });
    // ... rest same as before
}
```

#### 3b. `handle_tools_list()` -- Return only meta-tools + activated tools

```rust
async fn handle_tools_list(
    state: &AppState,
    user_id: &str,
    session_id: &str,  // NEW parameter
    request: &JsonRpcRequest,
) -> Response {
    let services = match mcp_service::load_user_tools(&state.db, user_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to load user tools: {e}");
            return rpc_error(request.id.clone(), -32603, "Failed to load tools");
        }
    };

    // Get activated service IDs for this session
    let activated = state.mcp_sessions.get_activated_service_ids(session_id);

    // Generate only meta-tools + activated service tools
    let tool_defs = mcp_service::generate_tool_definitions(&services, Some(&activated));

    let tools_json: Vec<serde_json::Value> = tool_defs
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })
        })
        .collect();

    rpc_success(
        request.id.clone(),
        serde_json::json!({ "tools": tools_json }),
    )
}
```

Update the call site in `mcp_post` to pass `session_id`:

```rust
"tools/list" => {
    let sid = match require_session(&headers) { ... };
    if let Err(r) = validate_session(...) { ... }
    handle_tools_list(&state, &user_id, &sid, &request).await
}
```

#### 3c. `handle_meta_search()` -- Activate matching tools + send notification

```rust
async fn handle_meta_search(
    state: &AppState,
    user_id: &str,
    session_id: &str,  // NEW parameter
    arguments: &serde_json::Value,
    request_id: Option<serde_json::Value>,
) -> Response {
    let query = arguments
        .get("query")
        .and_then(|q| q.as_str())
        .unwrap_or("");

    if query.is_empty() {
        return tool_result(request_id, "Search query is required", true);
    }

    // Load ALL user tools (not just activated)
    let services = match mcp_service::load_user_tools(&state.db, user_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to load tools for search: {e}");
            return tool_result(request_id, "Failed to load tools", true);
        }
    };

    // Search across ALL tools
    let search_result = mcp_service::search_all_tools(&services, query);

    // Activate the services that had matches
    let changed = state.mcp_sessions.activate_services(
        session_id,
        &search_result.matched_service_ids,
    );

    // If tools changed, send notification so client refreshes
    if changed {
        send_tools_list_changed(state, session_id);
    }

    // Return search results
    let results: Vec<serde_json::Value> = search_result
        .matches
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
            })
        })
        .collect();

    let activated_count = state
        .mcp_sessions
        .get_activated_service_ids(session_id)
        .len();

    let text = serde_json::to_string_pretty(&serde_json::json!({
        "matches": results,
        "count": results.len(),
        "services_activated": search_result.matched_service_ids.len(),
        "total_activated_services": activated_count,
        "note": if changed {
            "Matching service tools have been activated. Your tool list has been updated."
        } else {
            "Tools were already activated."
        },
    }))
    .unwrap_or_default();

    tool_result(request_id, &text, false)
}
```

#### 3d. `handle_meta_connect()` -- Activate connected service + send notification

After a successful `mcp_service::connect_service()`, activate the newly connected service:

```rust
async fn handle_meta_connect(
    state: &AppState,
    user_id: &str,
    session_id: &str,  // NEW parameter
    arguments: &serde_json::Value,
    request_id: Option<serde_json::Value>,
) -> Response {
    let service_id = match arguments.get("service_id").and_then(|s| s.as_str()) {
        Some(id) => id,
        None => return tool_result(request_id, "service_id is required", true),
    };
    // ... (existing credential extraction, unchanged) ...

    match mcp_service::connect_service(/* same args */).await {
        Ok(result) => {
            // Activate the newly connected service
            let changed = state.mcp_sessions.activate_services(
                session_id,
                &[service_id.to_string()],
            );

            if changed {
                send_tools_list_changed(state, session_id);
            }

            // Audit log (unchanged)
            audit_service::log_async(/* same */);

            // Enhanced response
            let mut result_value = result;
            if let Some(obj) = result_value.as_object_mut() {
                obj.insert(
                    "note".to_string(),
                    serde_json::Value::String(
                        "Service tools are now available. Your tool list has been updated."
                            .to_string(),
                    ),
                );
            }
            let text = serde_json::to_string_pretty(&result_value).unwrap_or_default();
            tool_result(request_id, &text, false)
        }
        Err(e) => tool_result(request_id, &e.to_string(), true),
    }
}
```

#### 3e. `send_tools_list_changed()` -- Helper to push notification

```rust
/// Send a `notifications/tools/list_changed` JSON-RPC notification
/// to the session's SSE stream.
fn send_tools_list_changed(state: &AppState, session_id: &str) {
    let notification = serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "method": "notifications/tools/list_changed",
    });

    if !state.mcp_sessions.send_notification(session_id, notification) {
        tracing::debug!(
            session_id,
            "Failed to send tools/list_changed notification (no SSE listener)"
        );
    }
}
```

#### 3f. `mcp_get()` -- SSE stream backed by notification channel

Replace the `futures::stream::pending()` with a stream that receives from the notification channel:

```rust
pub async fn mcp_get(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user_id = match authenticate_mcp(&state, &headers).await {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };

    let sid = match require_session(&headers) {
        Ok(s) => s,
        Err(r) => return r,
    };

    if let Err(r) = validate_session(&state, &sid, &user_id, None) {
        return r;
    }

    // Take the notification receiver for this session.
    // If already taken (reconnect), create a new channel pair.
    let rx = match state.mcp_sessions.take_notification_rx(&sid) {
        Some(rx) => rx,
        None => {
            // Reconnect: create new channel, update session's tx
            let (tx, rx) = tokio::sync::mpsc::channel(32);
            state.mcp_sessions.set_notification_tx(&sid, tx);
            rx
        }
    };

    // Convert mpsc::Receiver into an SSE-compatible stream
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|notification| {
        Ok::<_, Infallible>(
            Event::default()
                .event("message")
                .data(notification.to_string()),
        )
    });

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(30))
                .text("keepalive"),
        )
        .into_response()
}
```

**New import needed:** `use tokio_stream::StreamExt;` (or use `futures::StreamExt`).

#### 3g. `mcp_post()` -- Pass session_id to meta-tool handlers

Update the match arms to pass `&sid` to the affected handlers:

```rust
"tools/list" => {
    // ... session validation ...
    handle_tools_list(&state, &user_id, &sid, &request).await
}

"tools/call" => {
    // ... session validation ...
    handle_tools_call(&state, &user_id, &sid, &request).await  // pass sid
}
```

Update `handle_tools_call` to pass `session_id` to meta-tool handlers:

```rust
async fn handle_tools_call(
    state: &AppState,
    user_id: &str,
    session_id: &str,  // NEW parameter
    request: &JsonRpcRequest,
) -> Response {
    // ... parse params ...

    match tool_name {
        "nyx__search_tools" => {
            return handle_meta_search(state, user_id, session_id, &arguments, request.id.clone()).await;
        }
        "nyx__connect_service" => {
            return handle_meta_connect(state, user_id, session_id, &arguments, request.id.clone()).await;
        }
        "nyx__discover_services" => {
            // discover_services does NOT activate tools (it's for browsing)
            return handle_meta_discover(state, user_id, &arguments, request.id.clone()).await;
        }
        _ => {}
    }

    // Service tool execution: verify the tool's service is activated
    let activated = state.mcp_sessions.get_activated_service_ids(session_id);

    let services = match mcp_service::load_user_tools(&state.db, user_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to load tools for execution: {e}");
            return tool_result(request.id.clone(), "Failed to load tools", true);
        }
    };

    let (service, endpoint) = match mcp_service::resolve_tool_call(tool_name, &services) {
        Some(pair) => pair,
        None => {
            return tool_result(
                request.id.clone(),
                &format!("Unknown tool: {tool_name}. Use nyx__search_tools to find and activate tools."),
                true,
            );
        }
    };

    // Guard: only allow execution if the service is activated
    if !activated.contains(&service.service_id) {
        return tool_result(
            request.id.clone(),
            &format!(
                "Tool '{tool_name}' belongs to service '{}' which is not activated. \
                 Use nyx__search_tools to activate it first.",
                service.service_name,
            ),
            true,
        );
    }

    // ... rest of execution unchanged ...
}
```

---

### 4. Dependency: `tokio-stream` crate

The `ReceiverStream` wrapper requires `tokio-stream`. Add to `backend/Cargo.toml`:

```toml
[dependencies]
tokio-stream = "0.1"
```

This is a lightweight, widely-used crate from the tokio ecosystem.

---

### 5. `backend/src/handlers/mcp.rs` (REST `/api/v1/mcp/config`) -- No changes needed

The REST config endpoint is for external tooling (not MCP protocol) and should continue to return all tools. No changes.

---

### 6. `backend/src/models/mcp_session.rs` -- Updated `reap_expired()`

The reaper already removes expired sessions. The `activated_service_ids` and `notification_tx` are cleaned up automatically when the session is dropped. The `pending_receivers` map also needs cleanup:

```rust
pub fn reap_expired(&self, max_idle: Duration) {
    let cutoff = Utc::now()
        - chrono::Duration::from_std(max_idle).unwrap_or(chrono::Duration::hours(1));
    let mut sessions = self.sessions.write().expect("lock poisoned");
    let before = sessions.len();

    // Collect expired session IDs
    let expired_ids: Vec<String> = sessions
        .iter()
        .filter(|(_, s)| s.last_active <= cutoff)
        .map(|(id, _)| id.clone())
        .collect();

    for id in &expired_ids {
        sessions.remove(id);
    }

    drop(sessions); // Release lock before acquiring pending_receivers lock

    // Also clean up pending receivers for expired sessions
    let mut receivers = self.pending_receivers.write().expect("lock poisoned");
    for id in &expired_ids {
        receivers.remove(id);
    }

    let removed = before - before + expired_ids.len(); // = expired_ids.len()
    if !expired_ids.is_empty() {
        tracing::info!(removed = expired_ids.len(), "Reaped expired MCP sessions");
    }
}
```

---

## JSON-RPC Notification Format

The `notifications/tools/list_changed` notification follows the MCP spec:

```json
{
    "jsonrpc": "2.0",
    "method": "notifications/tools/list_changed"
}
```

No `id` field (it's a notification, not a request). No `params` needed.

The client receives this over the SSE stream and should call `tools/list` again to get the updated tool list.

---

## Edge Cases

### 1. Tool called before activation

If an LLM tries to call a tool whose service isn't activated, return a helpful error:

```json
{
    "content": [{"type": "text", "text": "Tool 'github__list_repos' belongs to service 'GitHub' which is not activated. Use nyx__search_tools to activate it first."}],
    "isError": true
}
```

### 2. Service deactivated/deleted while tools are activated

If a service is deactivated by an admin while its tools are activated in a session:
- `tools/list` will not include them because `load_user_tools()` filters by `is_active: true`
- `tools/call` will fail at `resolve_tool_call()` since the service won't be in the loaded services
- The activated set in the session retains stale IDs, but they're harmless (filtered out at generation time)

No special handling needed -- the existing `load_user_tools()` filter naturally handles this.

### 3. MAX_ACTIVATED_SERVICES reached

When the limit is reached, `activate_services()` stops adding new services. The search result still shows matches, but includes a note:

Add to `handle_meta_search`:
```rust
let activated_count = state.mcp_sessions.get_activated_service_ids(session_id).len();
if activated_count >= mcp_session::MAX_ACTIVATED_SERVICES {
    // Include warning in response
    "max_activated_services_warning": "Maximum activated services reached. Some tools may not have been activated."
}
```

### 4. SSE not connected when notification is sent

`send_notification()` uses `try_send()` which is non-blocking. If no SSE listener exists (StreamableHTTP-only clients), the notification is dropped. The LLM still gets the `note` field in the tool result telling it to re-list tools. Most MCP clients also poll `tools/list` periodically.

### 5. SSE reconnect

When a client reconnects to `GET /mcp`, the handler creates a new `(tx, rx)` pair and updates the session's `notification_tx`. The old `tx` is replaced, and the old SSE stream's `rx` will see the channel close (stream ends). This is the correct behavior.

### 6. Session cleanup

When `DELETE /mcp` is called, `remove()` drops the session including its `notification_tx`. The SSE stream's `rx` sees the channel close and the stream ends. The `pending_receivers` entry is also removed.

---

## Data Flow Summary

### `tools/list` (lazy mode)

1. Load all user services via `load_user_tools()`
2. Get `activated_service_ids` from session store
3. Call `generate_tool_definitions(services, Some(&activated))` -- filters to activated only
4. Return meta-tools + activated tools

### `nyx__search_tools`

1. Load all user services via `load_user_tools()`
2. Search across ALL services via `search_all_tools()`
3. Activate matched service IDs via `activate_services()`
4. If changed, send `notifications/tools/list_changed` via notification channel
5. Return search results with activation status note

### `nyx__connect_service`

1. Connect user via `connection_service::connect_user()`
2. Activate the connected service via `activate_services()`
3. If changed, send `notifications/tools/list_changed`
4. Return connection result with note

### `nyx__discover_services`

1. No changes -- this is a browsing tool, does NOT activate services

### Service tool execution (`tools/call` for non-meta tools)

1. Check that tool's service is in `activated_service_ids`
2. If not activated, return error with guidance to use `nyx__search_tools`
3. If activated, execute as before (unchanged)

---

## Testing Plan

### Unit tests (`mcp_session.rs`)

1. `activate_services` -- adds service IDs, returns `true` on change
2. `activate_services` -- returns `false` when re-activating same IDs
3. `activate_services` -- enforces `MAX_ACTIVATED_SERVICES` limit
4. `get_activated_service_ids` -- returns correct set after activation
5. `send_notification` -- sends to channel successfully
6. `send_notification` -- returns false when no listener
7. `take_notification_rx` -- returns rx once, None on second call
8. `set_notification_tx` -- replaces the tx for reconnect
9. `reap_expired` -- cleans up both sessions and pending_receivers
10. `create` -- returns session_id string (API remains clean)

### Unit tests (`mcp_service.rs`)

1. `generate_tool_definitions(services, Some(empty_set))` -- returns only meta-tools
2. `generate_tool_definitions(services, Some(subset))` -- returns meta + subset tools
3. `generate_tool_definitions(services, None)` -- returns all tools (backward compat)
4. `search_all_tools` -- returns matching tools and their service IDs
5. `search_all_tools` -- respects MAX_SEARCH_RESULTS limit

### Integration tests (`mcp_transport`)

1. `tools/list` after `initialize` returns only 3 meta-tools
2. `nyx__search_tools` call activates matching services
3. `tools/list` after search returns meta-tools + activated
4. `nyx__connect_service` activates connected service
5. Calling non-activated tool returns helpful error
6. SSE stream receives `notifications/tools/list_changed` after activation

---

## Summary of Files to Modify

| File | Change |
|------|--------|
| `backend/Cargo.toml` | Add `tokio-stream = "0.1"` |
| `backend/src/models/mcp_session.rs` | Add `activated_service_ids`, notification channel, new methods |
| `backend/src/services/mcp_service.rs` | Add `activated_service_ids` param to `generate_tool_definitions()`, add `search_all_tools()` |
| `backend/src/handlers/mcp_transport.rs` | Wire session_id through handlers, filter tools/list, activate on search/connect, SSE notifications |

Files with NO changes:
- `backend/src/handlers/mcp.rs` (REST config endpoint)
- `backend/src/main.rs` (McpSessionStore::new() API unchanged)
- `backend/src/routes.rs` (routes unchanged)
- All other files
