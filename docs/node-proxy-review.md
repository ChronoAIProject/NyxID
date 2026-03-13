# Node Proxy Code & Security Review

**Reviewed by:** security-reviewer agent
**Date:** 2026-03-12
**Branch:** feature/node-proxy
**Scope:** All new/modified files for the node proxy feature

---

## Summary

- **Critical Issues:** 2
- **High Issues:** 5
- **Medium Issues:** 8
- **Low Issues:** 6
- **Overall Risk Level:** HIGH (due to WebSocket auth and routing issues)

The node proxy feature is architecturally sound with good separation of concerns and proper use of existing patterns (AppError, dedicated response structs, bson datetime helpers, etc.). However, there are security-critical issues in the WebSocket authentication flow, missing rate limiting, and several code quality issues that need attention.

---

## Critical Issues (Fix Immediately)

### C1. WebSocket endpoint has no rate limiting or authentication middleware
**Severity:** CRITICAL
**Category:** Authentication Bypass / DoS
**Location:** `backend/src/routes.rs:544`, `backend/src/handlers/node_ws.rs:50-55`

**Issue:**
The WebSocket endpoint `/api/v1/nodes/ws` is registered in the public router (outside the authenticated `api_v1` nest) with no rate limiting or auth middleware. Any unauthenticated client can open unlimited WebSocket connections, causing resource exhaustion. While in-message auth is performed, the connection is already established and resources allocated before auth occurs.

```rust
// routes.rs:544 - registered outside authenticated routes
.route("/api/v1/nodes/ws", get(handlers::node_ws::ws_handler))
```

**Impact:**
- Unauthenticated DoS: Attackers can open thousands of WebSocket connections exhausting server memory
- Brute force token guessing: No rate limit on auth attempts within WebSocket messages
- Resource exhaustion before auth completes

**Remediation:**
1. Add rate limiting middleware to the WebSocket route (per-IP connection limit)
2. Consider requiring a JWT or short-lived ticket in the WebSocket upgrade request query parameter, validated before upgrade
3. At minimum, track open unauthenticated connections and reject if over a threshold

---

### C2. Node registration token can be used to brute-force via WebSocket
**Severity:** CRITICAL
**Category:** Brute Force / Token Security
**Location:** `backend/src/handlers/node_ws.rs:82-103`

**Issue:**
The WebSocket handler accepts `register` messages with registration tokens. There is no limit on how many register attempts can be made within a single WebSocket connection or across connections. The 10-second auth timeout only applies to receiving the first message, not to the rate of attempts. An attacker who opens many connections can attempt many different tokens within the 10-second windows.

Combined with C1 (no rate limiting on the WS endpoint), this allows rapid brute-force of registration tokens. The `nyx_nreg_` prefix + 32 random bytes makes the token space large, but defense-in-depth requires rate limiting.

**Impact:**
- Potential brute-force of registration tokens (although entropy is high)
- Combined with C1, amplifies the DoS risk

**Remediation:**
1. Add per-IP rate limiting on the WebSocket endpoint
2. Lock out registration tokens after N failed attempts
3. Consider adding a failed-attempt counter per IP in the registration flow

---

## High Issues (Fix Before Production)

### H1. `register_connection` method creates unused channel receiver (memory leak)
**Severity:** HIGH
**Category:** Resource Leak / Code Quality
**Location:** `backend/src/services/node_ws_manager.rs:108`

**Issue:**
The `register_connection` method creates an `mpsc::unbounded_channel()` but immediately discards the receiver `_rx`. The sender is stored in the connection, meaning any messages sent through it will accumulate in the channel buffer forever since no receiver is draining it.

```rust
pub fn register_connection(&self, node_id: &str) -> (...) {
    let (tx, _rx) = mpsc::unbounded_channel(); // _rx is dropped!
    // ...
    self.connections.insert(node_id.to_string(), NodeConnection { tx, pending });
}
```

This method appears unused (the WS handler uses `register_connection_with_sender` instead), but if it were called, it would cause an unbounded memory leak.

**Impact:**
- Potential unbounded memory growth if this method is called
- Dead code that could mislead future developers

**Remediation:**
Remove the `register_connection` method entirely, keeping only `register_connection_with_sender`. If kept, return the receiver to the caller.

---

### H2. `unregister_connection` drops DashMap entries incorrectly for pending request cancellation
**Severity:** HIGH
**Category:** Logic Bug
**Location:** `backend/src/services/node_ws_manager.rs:146-154`

**Issue:**
The `unregister_connection` method iterates over `conn.pending` and calls `drop(entry)` on each, intending to cancel pending requests by dropping the `oneshot::Sender`. However, dropping a DashMap `RefMulti` (the iterator entry) does NOT remove or consume the item from the map. The `oneshot::Sender` is still stored in the DashMap -- the drop only releases the read lock. The subsequent `conn.pending.clear()` then drops them, but between the iteration and clear, pending requests may still try to resolve.

```rust
conn.pending.iter().for_each(|entry| {
    drop(entry); // This drops the iterator ref, NOT the sender
});
conn.pending.clear(); // This is what actually drops the senders
```

**Impact:**
- Pending proxy requests may not receive errors promptly when a node disconnects
- The `iter().for_each(drop)` is a no-op and misleading

**Remediation:**
Simply call `conn.pending.clear()` directly, removing the misleading iteration:

```rust
pub fn unregister_connection(&self, node_id: &str) {
    if let Some((_, conn)) = self.connections.remove(node_id) {
        conn.pending.clear(); // Drops all senders, receivers get RecvError
    }
}
```

---

### H3. Node status field uses raw strings instead of an enum
**Severity:** HIGH
**Category:** Validation Gap
**Location:** `backend/src/models/node.rs:27`, `backend/src/services/node_service.rs:250-275`

**Issue:**
The node `status` field is a `String` with only a comment documenting valid values ("online" | "offline" | "draining"). The `set_node_status` function accepts any `&str` without validation. A typo or invalid status value would silently corrupt the data.

**Impact:**
- Invalid status values could be written to the database
- No compile-time safety for status transitions
- Future developers may introduce typos

**Remediation:**
Create a `NodeStatus` enum with serde support:

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NodeStatus {
    Online,
    Offline,
    Draining,
}
```

Update `Node.status` to use `NodeStatus` type and update all callers.

---

### H4. Node proxy routing query is N+1 (per-binding DB lookup)
**Severity:** HIGH
**Category:** Performance
**Location:** `backend/src/services/node_routing_service.rs:46-63`

**Issue:**
The `resolve_node_route` function first fetches all bindings for a user+service, then for EACH binding does a separate `find_one` query to check if the node is active. With many bindings, this is an N+1 query pattern on the hot path (every proxy request).

```rust
for binding in bindings {
    let node = db.collection::<Node>(NODES)
        .find_one(doc! { "_id": &binding.node_id, "is_active": true })
        .await?;
    // ...
}
```

**Impact:**
- Every proxy request with node bindings makes 1 + N database queries
- Latency increases linearly with the number of bindings
- This is on the critical path for proxy request routing

**Remediation:**
Use a single aggregation pipeline with `$lookup` to join bindings with nodes in one query. Or, collect all node_ids from bindings and fetch them in a single `find` with `_id: { $in: [...] }`, then match locally.

---

### H5. Frontend Zod schema allows single-char names but backend requires 1-64 chars
**Severity:** HIGH
**Category:** Validation Mismatch
**Location:** `frontend/src/schemas/nodes.ts:8-11` vs `backend/src/services/node_service.rs:25-38`

**Issue:**
The frontend Zod schema uses a regex that requires minimum 2 characters (due to `^[a-z0-9][a-z0-9-]*[a-z0-9]$` requiring start char, at least one middle char, and end char), but the backend allows single character names (just `name.len() > 0`). Also, the regex allows names ending with a hyphen if they're exactly 2+ chars, but doesn't match single valid chars like "a". The backend allows "a" but the frontend doesn't.

```typescript
// Frontend: requires min 2 chars due to regex anchors
.regex(/^[a-z0-9][a-z0-9-]*[a-z0-9]$/, "...")
```

```rust
// Backend: allows 1-64 chars
if name.is_empty() || name.len() > 64 { ... }
```

**Impact:**
- Users cannot create single-character node names from the frontend
- Validation behavior is inconsistent between frontend and backend
- Frontend regex does not match the backend validation exactly

**Remediation:**
Align the frontend regex to match backend validation: `^[a-z0-9]([a-z0-9-]*[a-z0-9])?$` to allow single-char names. Or update backend to also require minimum 2 characters.

---

## Medium Issues (Fix When Possible)

### M1. `NodeMetadata.ip_address` is stored without sanitization
**Severity:** MEDIUM
**Category:** Input Validation / XSS
**Location:** `backend/src/models/node.rs:17`, `backend/src/handlers/node_admin.rs:193-194`

**Issue:**
The `ip_address` field in `NodeMetadata` is provided by the node agent and stored without validation. It is returned to the frontend directly. While not a direct XSS vector (React auto-escapes), if this value is used in other contexts (logs, admin panels, emails), it could contain malicious content.

**Remediation:**
Validate that `ip_address` is a valid IP address format before storing. Use a regex or Rust's `std::net::IpAddr` parser.

---

### M2. Handler `list_nodes` performs N+1 binding count queries
**Severity:** MEDIUM
**Category:** Performance
**Location:** `backend/src/handlers/node_admin.rs:149-154`

**Issue:**
For each node in the list, a separate `count_documents` query is executed to get the binding count. This is an N+1 query pattern.

**Remediation:**
Use an aggregation pipeline with `$lookup` to get binding counts in a single query, or use a single query with `$group` on bindings and join results in memory.

---

### M3. `expires_at` in registration token response is computed twice
**Severity:** MEDIUM
**Category:** Logic Bug (Race Condition)
**Location:** `backend/src/handlers/node_admin.rs:117-118`

**Issue:**
The handler computes `expires_at` independently from the service function `create_registration_token`. The service creates the token with its own `Utc::now()` timestamp, then the handler computes another `Utc::now() + ttl`. These two timestamps differ by the time it takes to insert the token into the database. The response's `expires_at` may not match the actual stored value.

```rust
// In handler, AFTER the service call returns:
let expires_at = chrono::Utc::now()
    + chrono::Duration::seconds(state.config.node_registration_token_ttl_secs);
```

**Remediation:**
Have `create_registration_token` return the actual `expires_at` value it stored, and use that in the response.

---

### M4. Node auth token is sent in cleartext via WebSocket register_ok message
**Severity:** MEDIUM
**Category:** Token Exposure
**Location:** `backend/src/handlers/node_ws.rs:86-91`

**Issue:**
When a node registers via WebSocket, the auth token is sent back in the `register_ok` message. While this is necessary for the registration flow, if the WebSocket connection is not over TLS (wss://), the token would be transmitted in cleartext.

**Remediation:**
1. Enforce TLS/WSS for the WebSocket endpoint (document this requirement)
2. Consider a separate HTTPS endpoint for registration that returns the auth token, so the WS connection only uses the token for auth (never receives it)

---

### M5. `delete_binding` handler ignores `node_id` path parameter
**Severity:** MEDIUM
**Category:** Authorization Gap
**Location:** `backend/src/handlers/node_admin.rs:339-342`

**Issue:**
The `delete_binding` handler extracts `(node_id, binding_id)` from the path but only uses `binding_id`. The `node_id` is silently ignored (prefix `_`). The service `delete_binding` only checks `user_id` and `binding_id`. This means a user could call `DELETE /nodes/{any_node_id}/bindings/{binding_id}` and it would succeed even if the binding doesn't belong to that node.

```rust
Path((_node_id, binding_id)): Path<(String, String)>,
```

**Impact:**
- Path semantics are misleading (node_id in URL is ignored)
- No verification that the binding actually belongs to the specified node

**Remediation:**
Add `node_id` verification in the service's `delete_binding` function:

```rust
doc! { "_id": binding_id, "user_id": user_id, "node_id": node_id, "is_active": true }
```

---

### M6. Duplicate `getStatusBadge` function in frontend pages
**Severity:** MEDIUM
**Category:** Code Duplication
**Location:** `frontend/src/pages/nodes.tsx:52-75`, `frontend/src/pages/node-detail.tsx:55-78`

**Issue:**
The `getStatusBadge` function is duplicated identically in both page files.

**Remediation:**
Extract to a shared component, e.g., `frontend/src/components/shared/node-status-badge.tsx`.

---

### M7. `node_service_bindings` unique index may conflict with soft-deleted bindings
**Severity:** MEDIUM
**Category:** Data Integrity
**Location:** `backend/src/db.rs:604-609`

**Issue:**
The unique index on `(node_id, service_id)` does not include `is_active`. If a binding is soft-deleted (is_active = false), a new binding for the same node+service combination would violate the unique index.

```rust
nsb.create_index(
    IndexModel::builder()
        .keys(doc! { "node_id": 1, "service_id": 1 })
        .options(IndexOptions::builder().unique(true).build())
        .build(),
)
```

**Impact:**
- Users cannot re-bind a service to a node after unbinding it

**Remediation:**
Either:
1. Add a partial filter expression: `partialFilterExpression: { is_active: true }` to only enforce uniqueness on active bindings
2. Or use hard deletes instead of soft deletes for bindings

---

### M8. `nodes` unique index on `(user_id, name)` may conflict with soft-deleted nodes
**Severity:** MEDIUM
**Category:** Data Integrity
**Location:** `backend/src/db.rs:573-578`

**Issue:**
Same issue as M7 but for nodes. The unique index on `(user_id, name)` will prevent creating a new node with the same name after deleting an existing one.

**Remediation:**
Add a partial filter expression for `is_active: true` to the unique index, or include `is_active` in the unique key.

---

## Low Issues (Consider Fixing)

### L1. `#[allow(dead_code)]` on NodeRoute struct
**Severity:** LOW
**Category:** Code Quality
**Location:** `backend/src/services/node_routing_service.rs:11`

**Issue:**
`#[allow(dead_code)]` on `NodeRoute` suggests it may not be fully utilized. Verify it's used and remove the annotation, or remove unused fields.

**Remediation:**
Check if `NodeRoute.binding` is used by callers. If not, simplify the struct.

---

### L2. Error message in auth flow leaks internal error details
**Severity:** LOW
**Category:** Information Leakage
**Location:** `backend/src/handlers/node_ws.rs:72-78`, `backend/src/handlers/node_ws.rs:97-99`

**Issue:**
The WebSocket auth error messages include the full error description via `e.to_string()`, which could leak internal details (e.g., database errors) to connecting clients.

```rust
"message": format!("Invalid message: {e}")  // line 76
"message": e.to_string()                     // line 98
```

**Remediation:**
Return generic error messages for auth failures. Log the detailed error server-side:

```rust
tracing::warn!(error = %e, "Node registration failed");
let err_msg = serde_json::json!({
    "type": "auth_error",
    "message": "Registration failed"
});
```

---

### L3. Missing `formatRelativeTime` null safety check in nodes page
**Severity:** LOW
**Category:** Robustness
**Location:** `frontend/src/pages/nodes.tsx:294`

**Issue:**
`formatRelativeTime(node.last_heartbeat_at)` is called with a potentially null value. Verify that `formatRelativeTime` handles null/undefined gracefully.

**Remediation:**
Verify the utility handles null, or add a fallback: `formatRelativeTime(node.last_heartbeat_at) ?? "Never"`.

---

### L4. Handler validation duplicates service validation
**Severity:** LOW
**Category:** Code Duplication
**Location:** `backend/src/handlers/node_admin.rs:102-105`

**Issue:**
The handler checks `if body.name.is_empty()` before calling `node_service::create_registration_token`, which also validates the name (length 1-64, character set). The handler check is redundant.

**Remediation:**
Remove the handler-level name validation, relying on the service-level validation which is more comprehensive.

---

### L5. Heartbeat sweep checks `last_heartbeat_at` but newly connected nodes may not have one yet
**Severity:** LOW
**Category:** Edge Case
**Location:** `backend/src/handlers/node_ws.rs:303`

**Issue:**
In `node_ws_manager_heartbeat_sweep`, if a node's `last_heartbeat_at` is `None` (shouldn't happen due to registration setting it, but defensive coding), the heartbeat timeout check is silently skipped. This is actually correct behavior but should be documented.

**Remediation:**
Add a comment explaining why `None` heartbeat is acceptable (newly registered nodes have it set at registration time).

---

### L6. `useParams` type assertion in node-detail page
**Severity:** LOW
**Category:** Type Safety
**Location:** `frontend/src/pages/node-detail.tsx:81`

**Issue:**
```typescript
const { nodeId } = useParams({ strict: false }) as { nodeId: string };
```
Using `as` type assertion is less safe than using the route type system. If the route changes, this won't produce a compile error.

**Remediation:**
Use TanStack Router's typed route params instead of `as` assertion.

---

## Security Checklist

- [x] No hardcoded secrets
- [x] Registration tokens are one-time use (atomic find_one_and_update with `used: false`)
- [x] Registration tokens are time-limited (TTL + TTL index for auto-expiry)
- [x] Auth tokens are stored as SHA-256 hashes, never in plaintext
- [x] Token rotation immediately invalidates old tokens
- [x] Node ownership verified on all admin endpoints via user_id
- [x] Proper bson datetime helpers used on all DateTime fields
- [x] No `skip_serializing` on model fields
- [x] Dedicated response structs (not serializing models directly)
- [x] Response header allowlist on node proxy responses
- [x] Forward header allowlist prevents leaking internal headers to nodes
- [x] Body size limit (10MB) on proxy requests to nodes
- [x] WebSocket auth timeout (10 seconds)
- [ ] **MISSING: Rate limiting on WebSocket endpoint (C1)**
- [ ] **MISSING: Rate limiting on node admin endpoints**
- [ ] **MISSING: Node status enum validation (H3)**
- [ ] **MISSING: Binding node_id verification on delete (M5)**
- [ ] **MISSING: Partial filter on unique indexes for soft-delete compat (M7, M8)**

---

## Architecture Notes (Not Issues)

The following are observations, not issues:

1. **In-memory WS manager**: The `NodeWsManager` stores connections in-memory using `DashMap`. This is correct for single-instance deployments but would need Redis pub/sub for horizontal scaling. Acceptable for current scope.

2. **Fallback behavior**: When a node is configured but offline, the proxy correctly falls through to the standard credential proxy. This is good for availability.

3. **Audit logging**: All node operations (create token, delete, rotate, bind, unbind) are properly audit-logged.

4. **TTL index**: The `expires_at` TTL index on registration tokens will auto-delete expired tokens. Good use of MongoDB TTL indexes.
