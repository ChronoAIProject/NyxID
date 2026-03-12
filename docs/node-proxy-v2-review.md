# Node Proxy v2 — Code + Security Review

**Reviewed:** 2026-03-12
**Reviewer:** security-reviewer agent
**Scope:** All v2 changes (node-agent crate, server-side v2 features, models, services, handlers, frontend)
**Branch:** `feature/node-proxy`

## Summary

- **Critical Issues:** 5
- **High Issues:** 8
- **Medium Issues:** 8
- **Low Issues:** 5
- **Build Status:** node-agent compiles; backend has syntax error in `config.rs` (in-progress work)
- **Implementation Status:** ~60% complete (ws_client, proxy_executor, streaming, failover, server-side signing still pending)

---

## Critical Issues (Fix Immediately)

### C1: Keyfile created with default permissions before chmod

**Severity:** CRITICAL
**Category:** Local Privilege Escalation
**Location:** `node-agent/src/encryption.rs:37-43`

**Issue:**
`std::fs::write(&keyfile, &key)` creates the file with default permissions (typically 0644 on Unix). The `set_permissions(0o600)` call happens afterwards. In the window between write and chmod, any local user can read the 32-byte encryption key.

**Impact:**
On multi-user systems, a local attacker could read the keyfile during the race window, gaining the ability to decrypt all locally stored credentials (auth token, signing secret, API keys).

**Remediation:**
```rust
// Use std::fs::OpenOptions with mode set atomically
use std::os::unix::fs::OpenOptionsExt;
let mut file = std::fs::OpenOptions::new()
    .write(true)
    .create_new(true)
    .mode(0o600)
    .open(&keyfile)?;
file.write_all(&key)?;
```

---

### C2: register_node returns 3-tuple but WS handler destructures as 2-tuple (compilation error)

**Severity:** CRITICAL
**Category:** Build Break / Missing Signing Secret Delivery
**Location:** `backend/src/handlers/node_ws.rs:102-109` vs `backend/src/services/node_service.rs:90-161`

**Issue:**
`register_node()` returns `AppResult<(Node, String, String)>` (node, auth_token, signing_secret), but the WS handler destructures it as `Ok((node, raw_auth_token))`. This is a compilation error. Additionally, even if it compiled, the `register_ok` WS message does not include the signing secret, so the node agent has no way to receive it.

**Impact:**
- Backend will not compile
- HMAC request signing is broken — the node never receives its signing secret
- The entire v2 signing feature is non-functional

**Remediation:**
```rust
// In node_ws.rs, handle all 3 return values:
Ok((node, raw_auth_token, raw_signing_secret)) => {
    let ok_msg = serde_json::json!({
        "type": "register_ok",
        "node_id": &node.id,
        "auth_token": raw_auth_token,
        "signing_secret": raw_signing_secret,
    });
    let _ = ws_sink.send(Message::Text(ok_msg.to_string().into())).await;
    return Some(node.id);
}
```

---

### C3: rotate_auth_token returns tuple but handler expects single String (compilation error)

**Severity:** CRITICAL
**Category:** Build Break / Missing Signing Secret Return
**Location:** `backend/src/handlers/node_admin.rs:248-273` vs `backend/src/services/node_service.rs:296-325`

**Issue:**
`rotate_auth_token()` now returns `AppResult<(String, String)>` (auth_token, signing_secret), but the `rotate_token` handler assigns the result to a single `let raw_token = ...`. This is a compilation error. The `RotateTokenResponse` also doesn't include the signing secret.

**Impact:**
- Backend will not compile
- Users cannot obtain the new signing secret after rotation
- Node agent cannot verify HMAC signatures after token rotation

**Remediation:**
```rust
let (raw_token, raw_signing_secret) =
    node_service::rotate_auth_token(&state.db, &user_id_str, &node_id).await?;

// Return both in response
Ok(Json(RotateTokenResponse {
    auth_token: raw_token,
    signing_secret: raw_signing_secret,
    message: "Auth token and signing secret rotated. The node must reconnect.".to_string(),
}))
```

---

### C4: Error string truncation panics on multi-byte UTF-8 boundaries

**Severity:** CRITICAL
**Category:** Denial of Service (Panic)
**Location:** `backend/src/services/node_metrics_service.rs:48-51`

**Issue:**
```rust
let error_truncated = if error.len() > 256 {
    format!("{}...", &error[..256])
} else { error };
```
`&error[..256]` will panic if byte index 256 falls within a multi-byte UTF-8 character. This is the same bug class that was fixed in `proxy.rs` (commit 94cc699).

**Impact:**
A node returning an error message with multi-byte characters (e.g., CJK, emoji) at the right length can cause the metrics recording task to panic, potentially crashing the background task or leaving the metrics update incomplete.

**Remediation:**
```rust
let error_truncated = if error.len() > 256 {
    let boundary = error.floor_char_boundary(256);
    format!("{}...", &error[..boundary])
} else { error };
```
Or use the same truncation helper used elsewhere in the codebase.

---

### C5: Backend config.rs has unclosed delimiter (compilation error)

**Severity:** CRITICAL
**Category:** Build Break
**Location:** `backend/src/config.rs:853`

**Issue:**
The file has a mismatched brace, likely from in-progress v2 feature additions to `AppConfig`. The backend does not compile.

**Impact:**
No backend binary can be built. All server-side functionality is blocked.

**Remediation:**
Fix the brace mismatch in `config.rs`. Likely a missing closing brace for the `impl AppConfig` block or the `new()` / `from_env()` method.

---

## High Issues (Fix Before Production)

### H1: Decrypted credentials stored as plain String, not Zeroizing

**Severity:** HIGH
**Category:** Sensitive Data Exposure
**Location:** `node-agent/src/credential_store.rs:10-22`

**Issue:**
`ServiceCredential` stores `header_value` and `param_value` as plain `String`. These decrypted secrets (API keys, bearer tokens) remain in heap memory for the process lifetime. The `zeroize` crate is already a dependency but not used here.

**Impact:**
Core dumps, swap files, or memory forensic tools could extract plaintext credentials. This is especially concerning since the node agent runs on user infrastructure where physical access may be less controlled.

**Remediation:**
```rust
use zeroize::Zeroizing;

pub struct ServiceCredential {
    pub header_value: Zeroizing<String>,
    pub param_value: Zeroizing<String>,
    // ... other fields
}
```

---

### H2: ReplayGuard has no hard cap on nonce set size

**Severity:** HIGH
**Category:** Memory Exhaustion / DoS
**Location:** `node-agent/src/signing.rs:12-61`

**Issue:**
The `ReplayGuard` evicts old nonces only when `seen.len() >= MAX_NONCE_SET_SIZE / 2` (5000). But eviction only removes entries older than 5 minutes. If an attacker sends >10,000 unique requests within 5 minutes (~33/sec), the set grows unbounded because all entries are within the time window.

**Impact:**
A sustained high request rate could cause unbounded memory growth in the node agent, eventually causing OOM.

**Remediation:**
Add a hard cap that drops the oldest entries when at maximum capacity:
```rust
fn evict_old_nonces(&mut self) {
    let cutoff = chrono::Utc::now() - chrono::Duration::seconds(MAX_TIMESTAMP_SKEW_SECS);
    self.seen.retain(|_, ts| *ts > cutoff);

    // Hard cap: if still over max, drop oldest entries
    if self.seen.len() > MAX_NONCE_SET_SIZE {
        let mut entries: Vec<_> = self.seen.drain().collect();
        entries.sort_by_key(|(_, ts)| *ts);
        self.seen = entries.into_iter().skip(entries.len() - MAX_NONCE_SET_SIZE).collect();
    }
}
```

---

### H3: Pending auth counter leak if WS handler future is cancelled

**Severity:** HIGH
**Category:** Resource Leak / DoS
**Location:** `backend/src/handlers/node_ws.rs:69-167`

**Issue:**
`increment_pending_auth()` is called at line 69, but `decrement_pending_auth()` at line 167 is inside the async callback passed to `ws.on_upgrade()`. If the WebSocket upgrade fails (e.g., client disconnects before upgrade completes), or the Tokio task is cancelled, the decrement never executes. Over time, this permanently reduces the effective max connection count.

**Impact:**
Leaked counters permanently reduce `max_connections - pending_auth`, eventually preventing all new node connections.

**Remediation:**
Use a RAII guard pattern:
```rust
struct PendingAuthGuard<'a> {
    manager: &'a NodeWsManager,
}
impl Drop for PendingAuthGuard<'_> {
    fn drop(&mut self) {
        self.manager.decrement_pending_auth();
    }
}
```
Create the guard before the upgrade, move it into the callback.

---

### H4: Unbounded mpsc channel for WS writer allows memory exhaustion

**Severity:** HIGH
**Category:** Memory Exhaustion / DoS
**Location:** `backend/src/handlers/node_ws.rs:186`, `backend/src/services/node_ws_manager.rs:34`

**Issue:**
`mpsc::unbounded_channel()` is used for the WS writer. If a node agent has a slow network connection or stops reading, proxy responses queued via the unbounded channel will accumulate in server memory indefinitely.

**Impact:**
A malicious or slow node could cause the NyxID server to run out of memory by intentionally not reading WS messages while proxy requests keep being sent.

**Remediation:**
Use a bounded channel (e.g., 256 messages) and handle the full-channel case:
```rust
let (tx, mut rx) = mpsc::channel::<String>(256);
// In send_proxy_request, use try_send or send with timeout
```

---

### H5: Admin endpoints return auth_token_hash and signing_secret_hash

**Severity:** HIGH
**Category:** Information Disclosure
**Location:** `backend/src/services/node_service.rs:356-388`

**Issue:**
`list_all_nodes()` and the admin get_node endpoint return full `Node` objects, which include `auth_token_hash` and `signing_secret_hash`. While these are SHA-256 hashes, exposing them is unnecessary and violates the principle of least privilege.

**Impact:**
An admin user (or compromised admin session) could use the hashes for offline brute-force attempts against the token space. Although tokens are 32 random bytes (infeasible to brute-force), the hashes still shouldn't be exposed.

**Remediation:**
Use dedicated admin response structs that omit sensitive hash fields, consistent with the project convention of never serializing model structs to API responses (CLAUDE.md rule: "handlers use dedicated response structs").

---

### H6: No WS connectivity check in node routing before proxy attempt

**Severity:** HIGH
**Category:** Reliability / Fail-Open
**Location:** `backend/src/services/node_routing_service.rs:28-79`

**Issue:**
`resolve_node_route()` checks the database for online nodes but does NOT check `NodeWsManager::is_connected()`. The connectivity check happens separately in `proxy.rs:189-192`. This means the routing service may return a "route" for a node whose WS connection has dropped but whose database status hasn't been updated yet.

The v2 architecture doc explicitly specifies that `resolve_node_route` should accept a `&NodeWsManager` parameter and check actual WS connectivity.

**Impact:**
During the window between a node disconnecting and the heartbeat sweep marking it offline (up to `heartbeat_timeout_secs` = 90s), proxy requests would be sent to the routing service, find a "route", then fail in the handler. This adds unnecessary latency.

**Remediation:**
Pass `ws_manager` to `resolve_node_route` and filter candidates by `ws_manager.is_connected()`:
```rust
pub async fn resolve_node_route(
    db: &mongodb::Database,
    ws_manager: &NodeWsManager,
    user_id: &str,
    service_id: &str,
) -> AppResult<Option<NodeRoute>> {
    // ... fetch bindings and nodes as before ...
    for binding in &bindings {
        if let Some(_node) = online_nodes.get(binding.node_id.as_str()) {
            if ws_manager.is_connected(&binding.node_id) {
                return Ok(Some(NodeRoute { node_id: binding.node_id.clone() }));
            }
        }
    }
    Ok(None)
}
```

---

### H7: No HMAC signing of proxy requests from server to node (not implemented)

**Severity:** HIGH
**Category:** Missing Security Feature
**Location:** `backend/src/services/node_ws_manager.rs:52-66` (WsProxyRequest)

**Issue:**
The v2 architecture specifies HMAC-SHA256 signing of all proxy requests. The `WsProxyRequest` struct does not include `timestamp`, `nonce`, or `signature` fields. The server sends unsigned proxy requests that a man-in-the-middle (between server and node over WS) could tamper with.

**Implementation Status:** Not started (Task #3 in-progress).

**Impact:**
Without request signing, the node agent cannot verify that proxy requests genuinely came from the NyxID server. A MITM on the WS connection could inject malicious requests.

**Remediation:**
Add signing fields to `WsProxyRequest` and sign before sending:
```rust
struct WsProxyRequest {
    // ... existing fields ...
    timestamp: String,    // RFC 3339
    nonce: String,        // UUID v4
    signature: String,    // HMAC-SHA256 hex
}
```
The signing secret must be stored per-node in the connection map and used when constructing the request.

---

### H8: No streaming proxy support (not implemented)

**Severity:** HIGH
**Category:** Missing Feature / Architecture Gap
**Location:** `backend/src/services/node_ws_manager.rs`

**Issue:**
The v2 architecture specifies streaming proxy responses (`proxy_response_start` / `proxy_response_chunk` / `proxy_response_end` WS message types) with backpressure handling. The current implementation only supports oneshot request/response via `oneshot::channel`.

**Implementation Status:** Not started (Task #3 in-progress).

**Impact:**
LLM streaming responses (SSE) cannot be proxied through nodes. The entire response must be buffered, adding latency and memory pressure for large responses.

**Remediation:**
Implement the `PendingRequest` enum with `OneShot` and `Streaming` variants per the architecture doc (Section 2).

---

## Medium Issues (Fix When Possible)

### M1: Config file written with default permissions before chmod

**Severity:** MEDIUM
**Category:** Information Disclosure
**Location:** `node-agent/src/config.rs:83-89`

**Issue:**
Same race condition as C1 but for the config file. The `config.toml` contains encrypted auth tokens and credentials. Between `std::fs::write` and `set_permissions(0o600)`, the file has default permissions.

**Remediation:**
Use `OpenOptions` with `.mode(0o600)` or write to a temp file and rename.

---

### M2: Default WebSocket URL uses wss:// on localhost

**Severity:** MEDIUM
**Category:** Usability / Configuration Error
**Location:** `node-agent/src/main.rs:69`

**Issue:**
The default registration URL is `wss://localhost:3001/api/v1/nodes/ws`. The `wss://` scheme requires TLS, which is typically not configured on localhost during development.

**Remediation:**
Either remove the default (require explicit URL) or use `ws://localhost:3001` for development:
```rust
let ws_url = url.unwrap_or("ws://localhost:3001/api/v1/nodes/ws");
```

---

### M3: list_bindings handler has N+1 query for service names

**Severity:** MEDIUM
**Category:** Performance
**Location:** `backend/src/handlers/node_admin.rs:286-306`

**Issue:**
For each binding, a separate `find_one` query fetches the service name/slug. With 10+ bindings, this becomes 10+ individual queries.

**Remediation:**
Batch-fetch services with `$in`, similar to how `list_nodes` batch-fetches binding counts:
```rust
let service_ids: bson::Array = bindings.iter()
    .map(|b| bson::Bson::String(b.service_id.clone())).collect();
let services: Vec<DownstreamService> = db
    .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
    .find(doc! { "_id": { "$in": service_ids } })
    .await?.try_collect().await?;
let service_map: HashMap<&str, &DownstreamService> =
    services.iter().map(|s| (s.id.as_str(), s)).collect();
```

---

### M4: No validation/length limits on NodeMetadata string fields

**Severity:** MEDIUM
**Category:** Input Validation
**Location:** `backend/src/models/node.rs:28-37`

**Issue:**
`agent_version`, `os`, `arch` fields have no length limits. IP address is validated in `node_service.rs:117-125`, but the other metadata fields accept arbitrarily long strings.

**Impact:**
A malicious node agent could send very large metadata strings, consuming database storage.

**Remediation:**
Add validation in `register_node` and `update_heartbeat`:
```rust
if let Some(ref meta) = metadata {
    if meta.agent_version.as_ref().is_some_and(|v| v.len() > 64) ||
       meta.os.as_ref().is_some_and(|v| v.len() > 64) ||
       meta.arch.as_ref().is_some_and(|v| v.len() > 64) {
        return Err(AppError::ValidationError("Metadata fields too long".into()));
    }
}
```

---

### M5: No audit logging for failed WS auth attempts

**Severity:** MEDIUM
**Category:** Insufficient Logging
**Location:** `backend/src/handlers/node_ws.rs:112-150`

**Issue:**
Failed authentication attempts (invalid token, wrong node_id, registration failure) are logged via `tracing::warn` but not to the audit log. For security monitoring, auth failures should be audit-logged to detect brute-force token guessing.

**Remediation:**
Add `audit_service::log_async` calls for auth failures:
```rust
Err(e) => {
    audit_service::log_async(
        state.db.clone(), None,
        "node_auth_failed".to_string(),
        Some(serde_json::json!({ "reason": "invalid_token" })),
        None, None,
    );
    // ... existing error handling
}
```

---

### M6: TOCTOU in max connections check

**Severity:** MEDIUM
**Category:** Race Condition
**Location:** `backend/src/handlers/node_ws.rs:59-69`

**Issue:**
The check `total_connection_count() >= max_connections()` and the subsequent `increment_pending_auth()` are not atomic. Two concurrent WS upgrade requests could both pass the check before either increments the counter, allowing `max_connections + 1` (or more) connections.

**Impact:**
Slight over-admission of connections (1-2 extra). Not a severe issue since it's bounded by concurrent HTTP request processing.

**Remediation:**
Use `compare_exchange` loop or accept the small over-admission as tolerable (document the tradeoff).

---

### M7: fs::read keyfile data not zeroized

**Severity:** MEDIUM
**Category:** Sensitive Data in Memory
**Location:** `node-agent/src/encryption.rs:24`

**Issue:**
When loading an existing keyfile, `std::fs::read(&keyfile)` returns a `Vec<u8>` containing the raw key. After copying to the `Zeroizing<[u8; 32]>`, the original `Vec<u8>` is dropped without zeroization. The key material may persist in freed heap memory.

**Remediation:**
```rust
let mut data = std::fs::read(&keyfile)?;
// ... validate and copy ...
data.zeroize(); // Explicitly zeroize the source Vec
```

---

### M8: No multi-node failover (not implemented)

**Severity:** MEDIUM
**Category:** Missing Feature
**Location:** `backend/src/services/node_routing_service.rs`

**Issue:**
The v2 architecture specifies multi-node failover with priority-based routing, `fallback_node_ids`, and a retry loop in `execute_proxy()`. Currently, only a single node route is returned and no retry is attempted.

**Implementation Status:** Not started (Task #3 in-progress).

**Remediation:**
Per architecture doc Section 3: return `NodeRoute { node_id, fallback_node_ids }` and implement retry loop in `execute_proxy()`.

---

## Low Issues (Consider Fixing)

### L1: ServiceCredential uses empty strings for unused fields

**Severity:** LOW
**Category:** Code Quality
**Location:** `node-agent/src/credential_store.rs:10-22`

**Issue:**
When `injection_method` is `"header"`, `param_name` and `param_value` are set to empty strings. This flat struct design makes it easy to accidentally use the wrong field.

**Remediation:**
Use an enum:
```rust
pub enum CredentialInjection {
    Header { name: String, value: Zeroizing<String> },
    QueryParam { name: String, value: Zeroizing<String> },
}
```

---

### L2: Node model signing_secret_hash defaults to empty string

**Severity:** LOW
**Category:** Migration Safety
**Location:** `backend/src/models/node.rs:76`

**Issue:**
`#[serde(default)]` on `signing_secret_hash` means existing v1 nodes without this field will deserialize with an empty string. When HMAC signing is enforced, the code must reject operations on nodes with empty signing_secret_hash rather than silently proceeding.

**Remediation:**
When HMAC signing is enforced, add validation:
```rust
if node.signing_secret_hash.is_empty() {
    return Err(AppError::BadRequest("Node requires re-registration for v2 signing".into()));
}
```

---

### L3: Dead code warnings in node-agent

**Severity:** LOW
**Category:** Code Quality
**Location:** `node-agent/src/metrics.rs:29,39`

**Issue:**
`snapshot()` method and `MetricsSnapshot` struct are never used. This is expected for in-progress work but should be resolved before release.

---

### L4: Duplicate comment block in proxy.rs

**Severity:** LOW
**Category:** Code Quality
**Location:** `backend/src/handlers/proxy.rs:30-35`

**Issue:**
The comment "Response headers that are safe to forward back to the client. Uses an allowlist to prevent leaking internal headers from downstream services." is duplicated on consecutive lines.

---

### L5: No index on signing_secret_hash

**Severity:** LOW
**Category:** Performance
**Location:** `backend/src/db.rs`

**Issue:**
There is an index on `auth_token_hash` but not on `signing_secret_hash`. If future code needs to look up nodes by signing secret hash (e.g., for signature verification on the server side), this would require a collection scan.

**Remediation:**
Add index if/when server-side signing verification is implemented.

---

## Incomplete v2 Features Requiring Review When Implemented

The following v2 features from the architecture doc (`docs/node-proxy-v2-architecture.md`) have **not been implemented** yet and will need review when completed:

| Feature | Architecture Section | Status | Blocking Task |
|---------|---------------------|--------|---------------|
| WS client (node-agent) | Section 1 | Not started | #2 |
| Proxy executor (node-agent) | Section 1 | Not started | #2 |
| Streaming proxy (server) | Section 2 | Not started | #3 |
| Multi-node failover | Section 3 | Not started | #3 |
| Server-side HMAC signing | Section 4 | Not started | #3 |
| Health-aware routing | Section 5 | Not started | #3 |
| Admin node endpoints | Section 6 | Partially done | #3 |
| Frontend admin nodes page | Section 6 | Not started | #4 |
| Frontend metrics display | Section 5 | Not started | #4 |

### Security Focus Areas for Future Review

When the above features are implemented, reviewers must pay special attention to:

1. **WS client reconnection** -- Exponential backoff implementation, jitter, max retry limits, credential handling during reconnect
2. **Proxy executor** -- Credential injection safety, request body handling, response size limits, timeout enforcement
3. **Streaming backpressure** -- Bounded channel (256), chunk size cap (64KB), max stream duration (300s), proper cleanup on disconnect
4. **Failover race conditions** -- Request cloning safety, retry count limits, idempotency concerns for non-GET requests
5. **HMAC signing** -- Canonical string construction consistency between server and node, nonce generation quality, timing attack resistance
6. **Admin endpoints** -- Authorization checks (admin role verification), pagination limits, no data leaks

---

## v1 Issues Status (from docs/node-proxy-review.md)

Cross-referencing the 21 issues from the v1 review:

| v1 Issue | Status | Notes |
|----------|--------|-------|
| C1: Max WS connections | **Fixed** | `total_connection_count()` includes `pending_auth` |
| C2: Pending auth counter | **Fixed** | Counter implemented, but has leak risk (see H3) |
| H1: 10s auth timeout | **Fixed** | Implemented in node_ws.rs |
| H2: Non-text WS messages | **Fixed** | Ignored with `continue` |
| H3: NodeStatus enum | **Fixed** | Proper enum with `as_str()` |
| H4: Batch binding count | **Fixed** | Aggregation pipeline in list_nodes |
| H5: Token rotation disconnect | **Fixed** | Disconnects WS and sets offline |
| M1: Close frame code | **Fixed** | Uses 4001 |
| M2: Header allowlist | **Fixed** | Both request and response allowlists |
| M3: Body size limit | **Fixed** | 10MB limit |
| M4: Heartbeat sweep | **Fixed** | Checks last_heartbeat_at with timeout |
| M5: Binding node_id verification | **Fixed** | All binding ops verify node ownership |
| M6: Registration metadata validation | **Fixed** | IP address validation |
| M7: Unique index partial filter (nodes) | **Fixed** | `is_active: true` partial filter |
| M8: Unique index partial filter (bindings) | **Fixed** | `is_active: true` partial filter |
| L1: Audit logging | **Fixed** | All CRUD operations audit-logged |
| L2: Service existence check on binding | **Fixed** | Verified in create_binding handler |
| L3: Graceful node deletion | **Fixed** | Deactivates all bindings |
| L4: Error key for node errors | **Fixed** | All four error types have unique keys |
| L5: TTL index on registration tokens | **Fixed** | `expire_after(0)` on `expires_at` |
| L6: WS endpoint rate limiting | **Documented** | Comment in routes.rs, relies on global limiter |

All 21 v1 issues have been addressed.

---

## Security Checklist

- [x] No hardcoded secrets
- [x] Auth tokens hashed (SHA-256) before storage
- [x] Registration tokens are one-time use (atomic find_one_and_update)
- [x] Node ownership verified on all user-facing operations
- [x] Soft-delete with partial filter indexes
- [x] Error messages don't leak internal details
- [x] Request/response header allowlists
- [x] Body size limit (10MB)
- [x] Rate limiting on HTTP upgrade (global)
- [x] Max concurrent WS connections enforced
- [x] Auth timeout (10s) on WS connections
- [x] Audit logging on all CRUD operations
- [ ] Local keyfile created atomically with correct permissions (C1)
- [ ] Config file created atomically with correct permissions (M1)
- [ ] Signing secret delivered to node agent (C2)
- [ ] Decrypted credentials zeroized (H1)
- [ ] Proxy requests signed with HMAC (H7)
- [ ] Streaming responses supported (H8)
- [ ] Multi-node failover implemented (M8)
- [ ] Admin responses omit sensitive hashes (H5)
- [ ] Failed WS auth attempts audit-logged (M5)

---

## Recommendations

1. **Fix all Critical issues before merging** -- C1-C5 include compilation errors and security vulnerabilities
2. **Prioritize H1 (credential zeroization)** -- The node agent stores API keys; they should not linger in memory
3. **Complete server-side HMAC signing (H7)** before shipping -- without it, the security model is incomplete
4. **Add integration tests** for the WS auth flow, including timeout, invalid token, and concurrent connection scenarios
5. **Consider a WS-specific rate limiter** -- the global per-IP rate limiter may be too permissive for WS connections
6. **Add a `--verify` flag to the node agent** that checks config file permissions and keyfile integrity

---

> Security review performed by Claude Code security-reviewer agent
> Report: docs/node-proxy-v2-review.md
