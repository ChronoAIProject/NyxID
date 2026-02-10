# Security Review Report

**Component:** Service Proxy Overhaul (Backend + Frontend + MCP Proxy)
**Reviewed:** 2026-02-10
**Reviewer:** security-reviewer agent

## Summary

- **Critical Issues:** 1
- **High Issues:** 3
- **Medium Issues:** 5
- **Low Issues:** 3
- **Risk Level:** HIGH

---

## Critical Issues (Fix Immediately)

### SEC-C1: Reconnection blocked by unique index - data integrity / denial of service

**Severity:** CRITICAL
**Category:** Authorization / Data Integrity
**Location:** `backend/src/db.rs:201-208`, `backend/src/services/connection_service.rs:84-130`

**Issue:**
The `user_service_connections` collection has a unique index on `(user_id, service_id)`. When a user disconnects, `disconnect_user` sets `is_active: false` on the existing document but does not delete it. When the user attempts to reconnect, `connect_user` checks for an active connection (lines 84-92) - which finds none - then calls `insert_one` (line 128-130) which fails because the unique index rejects the duplicate `(user_id, service_id)` pair from the inactive document.

**Impact:**
Once a user disconnects from any service, they can never reconnect. This is a permanent denial of access that requires manual database intervention. An attacker who can trigger a disconnect (e.g., CSRF on the DELETE endpoint, or social engineering) can permanently block a user from a service.

**Attack Scenario:**
1. User connects to service X
2. User (or attacker via CSRF) disconnects from service X
3. User tries to reconnect - `insert_one` fails with duplicate key error
4. User is permanently locked out of service X

**Remediation:**
Change `connect_user` to use `update_one` with upsert for reconnection, or delete the document on disconnect instead of soft-deleting, or make the unique index a partial index filtering on `is_active: true`:

```rust
// Option A: Partial unique index (preferred)
IndexModel::builder()
    .keys(doc! { "user_id": 1, "service_id": 1 })
    .options(
        IndexOptions::builder()
            .unique(true)
            .partial_filter_expression(doc! { "is_active": true })
            .build(),
    )
    .build()

// Option B: In connect_user, delete the old inactive document first
db.collection::<UserServiceConnection>(CONNECTIONS)
    .delete_one(doc! {
        "user_id": user_id,
        "service_id": service_id,
        "is_active": false,
    })
    .await?;
```

**References:**
- CWE-400: Uncontrolled Resource Consumption
- MongoDB Partial Indexes: https://www.mongodb.com/docs/manual/core/index-partial/

---

## High Issues (Fix Before Production)

### SEC-H1: SSRF via DNS rebinding bypasses base_url validation

**Severity:** HIGH
**Category:** Server-Side Request Forgery (SSRF)
**Location:** `backend/src/handlers/services_helpers.rs:89-148`, `backend/src/services/proxy_service.rs:113-117`

**Issue:**
`validate_base_url` checks whether a URL points to a private/internal address, but only at service creation/update time. At proxy time (`forward_request`), the hostname is resolved again by the HTTP client. An attacker-controlled DNS server could return a public IP during validation and then a private IP (e.g., 169.254.169.254, 10.0.0.1) during actual proxy requests.

Additionally, hostnames that resolve to private IPs are not caught. Only literal IP addresses and specific blocked hostnames are validated. A hostname like `internal.attacker.com` resolving to `10.0.0.1` would pass validation.

**Impact:**
A malicious or compromised admin could create a service with a hostname that initially resolves to a public IP, then changes DNS to point to internal infrastructure (cloud metadata services, internal APIs, databases).

**Attack Scenario:**
1. Attacker registers `proxy-target.evil.com` with a short TTL
2. During `validate_base_url`, DNS returns `1.2.3.4` (public IP) - passes validation
3. Attacker updates DNS to return `169.254.169.254`
4. User proxies a request - hits cloud metadata service
5. Attacker reads cloud credentials from the response

**Remediation:**
Re-validate the resolved IP at proxy time. Use a custom DNS resolver or reqwest's `resolve` feature to check the IP before connecting:

```rust
// In proxy_service::forward_request, before sending:
// 1. Resolve the hostname
// 2. Check if the resolved IP is private
// 3. Only then send the request
//
// Or use reqwest's connect callback to reject private IPs at connection time
```

**Mitigating Factor:** Only admins can create/update services, reducing the attack surface. But a compromised admin account or a multi-tenant admin scenario increases risk.

**References:**
- CWE-918: Server-Side Request Forgery
- OWASP SSRF Prevention Cheat Sheet

---

### SEC-H2: Debug derive on credential-bearing request structs

**Severity:** HIGH
**Category:** Sensitive Data Exposure
**Location:**
- `backend/src/handlers/connections.rs:23` - `ConnectRequest`
- `backend/src/handlers/connections.rs:34` - `UpdateCredentialRequest`
- `backend/src/handlers/services.rs:27` - `CreateServiceRequest`

**Issue:**
These request structs derive `Debug`, which includes the `credential` field in their `Debug` output. If any code path (error handler, middleware, framework internals) debug-prints these structs, plaintext credentials will appear in application logs.

Axum's error handling and tracing ecosystem can trigger debug formatting of extractors under certain conditions (e.g., deserialization errors with debug-level logging).

**Impact:**
Plaintext API keys, bearer tokens, and passwords could appear in log files, log aggregation services, or error reporting tools.

**Remediation:**
Implement a custom Debug that redacts sensitive fields:

```rust
// Remove Debug derive and implement manually:
impl std::fmt::Debug for ConnectRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectRequest")
            .field("credential", &self.credential.as_ref().map(|_| "[REDACTED]"))
            .field("credential_label", &self.credential_label)
            .finish()
    }
}
```

**References:**
- CWE-532: Insertion of Sensitive Information into Log File
- OWASP Logging Cheat Sheet

---

### SEC-H3: Path traversal in proxy URL construction

**Severity:** HIGH
**Category:** Path Traversal / SSRF
**Location:** `backend/src/services/proxy_service.rs:113-117`

**Issue:**
The proxy URL is constructed by directly concatenating `base_url` with the user-supplied `path`:

```rust
let url = format!("{}/{}", target.base_url.trim_end_matches('/'), path.trim_start_matches('/'));
```

The `path` comes from `{*path}` in the route, which Axum decodes. An attacker could send `../sensitive-endpoint` or URL-encoded traversal sequences. While the host cannot be changed through path traversal alone, an attacker could access unintended paths on the downstream service that the endpoint definitions didn't intend to expose.

Similarly in MCP proxy (`mcp-proxy/src/tools.ts:231`):
```typescript
path = path.replace(`{${key}}`, String(value));
```
Path parameter values are not sanitized, allowing traversal through user input.

**Impact:**
An authenticated user could access arbitrary paths on a downstream service, bypassing the intended endpoint-level access control.

**Attack Scenario:**
1. Service has endpoints `/api/users` and `/api/data`
2. User sends proxy request with path `../admin/delete-all`
3. Proxy forwards to `https://api.example.com/../admin/delete-all`
4. Downstream service resolves this to `https://api.example.com/admin/delete-all`

**Remediation:**
Normalize and validate the path before constructing the URL:

```rust
// Reject paths containing traversal sequences
if path.contains("..") || path.contains("//") {
    return Err(AppError::BadRequest("Invalid proxy path".to_string()));
}
```

**References:**
- CWE-22: Improper Limitation of a Pathname to a Restricted Directory

---

## Medium Issues (Fix When Possible)

### SEC-M1: No per-endpoint rate limiting on credential operations

**Severity:** MEDIUM
**Category:** Insufficient Rate Limiting
**Location:** `backend/src/handlers/services.rs:111-114` (existing TODO), `backend/src/routes.rs:56-66`

**Issue:**
Credential-sensitive endpoints share the global rate limiter:
- `POST /connections/{service_id}` (connect with credential)
- `PUT /connections/{service_id}/credential` (update credential)
- `GET /services/{service_id}/oidc-credentials` (retrieve OIDC secrets)
- `POST /services/{service_id}/regenerate-secret` (regenerate OIDC secret)
- `ANY /proxy/{service_id}/{*path}` (proxy with credential injection)

The global rate limit (default 10/s per IP, burst 30) is too permissive for credential operations.

**Impact:**
An attacker could brute-force credential operations or abuse the proxy endpoint for cost amplification against downstream services.

**Remediation:**
Add per-endpoint rate limiters:
- Credential endpoints: 5 requests/minute
- Proxy endpoints: configurable per-service
- OIDC credential access: 10 requests/minute

---

### SEC-M2: Service deactivation does not wipe user credentials

**Severity:** MEDIUM
**Category:** Insecure Data Retention
**Location:** `backend/src/handlers/services.rs:345-383`

**Issue:**
When a service is deactivated via `delete_service`, only the service's `is_active` flag is set to `false`. All `user_service_connections` documents for that service remain active with encrypted credentials intact. While the proxy correctly rejects requests to inactive services, the encrypted credentials persist unnecessarily.

**Impact:**
If the encryption key is compromised, credentials for deactivated services can still be decrypted. Defense-in-depth requires wiping credentials when they're no longer needed.

**Remediation:**
When deactivating a service, also deactivate all connections and wipe their credentials:

```rust
// After deactivating the service:
db.collection::<UserServiceConnection>(CONNECTIONS)
    .update_many(
        doc! { "service_id": &service_id, "is_active": true },
        doc! { "$set": {
            "is_active": false,
            "credential_encrypted": Bson::Null,
            "updated_at": bson::DateTime::from_chrono(now),
        }},
    )
    .await?;
```

---

### SEC-M3: Encryption key parsed on every request

**Severity:** MEDIUM
**Category:** Performance / Defense in Depth
**Location:**
- `backend/src/handlers/proxy.rs:24`
- `backend/src/handlers/connections.rs:145`
- `backend/src/handlers/connections.rs:192`
- `backend/src/handlers/services.rs:265-266,270`

**Issue:**
`aes::parse_hex_key(&state.config.encryption_key)?` is called on every request that involves encryption/decryption. This performs hex decoding on every call. While not directly exploitable, it means:
1. Unnecessary CPU work on every request
2. The parsed key exists as multiple copies in memory across concurrent requests
3. An invalid key would only be detected at request time, not startup

**Remediation:**
Parse the key once at startup and store the raw bytes in `AppState`:

```rust
pub struct AppState {
    // ...
    pub encryption_key: Vec<u8>,  // Pre-parsed at startup
}
```

---

### SEC-M4: Error messages leak internal details

**Severity:** MEDIUM
**Category:** Information Disclosure
**Location:**
- `backend/src/services/proxy_service.rs:90` - `"Credential is not valid UTF-8: {e}"`
- `backend/src/services/proxy_service.rs:168` - `"Proxy request failed: {e}"`
- `backend/src/handlers/services_helpers.rs:63-64` - `"Unknown service category: {}"`

**Issue:**
Error messages include internal details that could help attackers understand the system:
- UTF-8 validation failures reveal credential storage format
- Proxy error messages could expose downstream service addresses, connection errors, or TLS details
- Internal category values reveal data model details

**Impact:**
Information disclosure helps attackers map the system's internal architecture and identify attack vectors.

**Remediation:**
Return generic error messages to clients and log detailed errors server-side:

```rust
// Instead of:
AppError::Internal(format!("Proxy request failed: {e}"))

// Use:
tracing::error!("Proxy request to {} failed: {e}", target.base_url);
AppError::Internal("Proxy request failed".to_string())
```

---

### SEC-M5: Disconnect does not clear credential metadata

**Severity:** MEDIUM
**Category:** Insecure Data Retention
**Location:** `backend/src/services/connection_service.rs:218-232`

**Issue:**
`disconnect_user` correctly sets `credential_encrypted` to Null, but does not clear `credential_type` and `credential_label`. While these fields don't contain the credential itself, `credential_label` is user-provided and could contain sensitive identifiers (e.g., "Production Stripe Key for acme-corp").

**Remediation:**
Clear all credential-related fields on disconnect:

```rust
doc! { "$set": {
    "is_active": false,
    "credential_encrypted": Bson::Null,
    "credential_type": Bson::Null,
    "credential_label": Bson::Null,
    "updated_at": bson::DateTime::from_chrono(now),
}}
```

---

## Low Issues (Consider Fixing)

### SEC-L1: MCP config endpoint exposes service base_url to all connected users

**Severity:** LOW
**Category:** Information Disclosure
**Location:** `backend/src/handlers/mcp.rs:170`

**Issue:**
The MCP config response includes `base_url` for each service. While necessary for the MCP proxy architecture, this exposes the internal/external API URLs of all services a user is connected to.

**Impact:**
Users can discover the actual API endpoints of downstream services. For "internal" category services, this may reveal internal infrastructure URLs.

**Remediation:**
Consider whether `base_url` is needed in the MCP config response, since the MCP proxy routes through NyxID's proxy endpoint anyway. If removed, the MCP proxy should use the proxy_base_url exclusively.

---

### SEC-L2: MCP proxy path parameter values not sanitized

**Severity:** LOW
**Category:** Input Validation
**Location:** `mcp-proxy/src/tools.ts:231`

**Issue:**
Path parameter substitution uses simple string replacement:
```typescript
path = path.replace(`{${key}}`, String(value));
```

User-provided values are not URL-encoded or sanitized before substitution. A value containing `/` or `..` could manipulate the path.

**Impact:**
Limited - the proxy ultimately goes through the NyxID backend proxy which provides a second layer of validation. But defense in depth suggests sanitizing here too.

**Remediation:**
URL-encode path parameter values:
```typescript
path = path.replace(`{${key}}`, encodeURIComponent(String(value)));
```

---

### SEC-L3: Non-atomic OIDC secret regeneration

**Severity:** LOW
**Category:** Data Integrity
**Location:** `backend/src/handlers/services.rs:685-720`

**Issue:**
The `regenerate_oidc_secret` handler performs two sequential MongoDB updates (update hash on OauthClient, then update encrypted secret on DownstreamService) without a transaction. A crash between the two operations leaves the system in an inconsistent state.

**Impact:**
Low probability but if it occurs, the OIDC service becomes unusable until manually repaired. This is already documented as a TODO (SEC-5).

**Remediation:**
Use a MongoDB multi-document transaction when running on a replica set, as noted in the existing TODO.

---

## Security Checklist

- [x] No hardcoded secrets in source code
- [x] `.env` file in `.gitignore`
- [x] All user inputs validated (length, format)
- [x] NoSQL injection prevention (parameterized doc! queries)
- [x] XSS prevention (React auto-escapes, credential input uses type="password")
- [x] Authentication required on all endpoints (AuthUser extractor)
- [x] Authorization verified (admin checks, user-scoped queries)
- [x] AES-256-GCM encryption for credentials at rest
- [x] Random nonces for each encryption operation
- [x] Encryption key validated at startup (length, not all-zeros)
- [x] Credentials never returned in API responses (has_credential bool only)
- [x] Credential input uses autoComplete="off"
- [x] Header allowlist on proxy forwarding
- [x] Hop-by-hop headers stripped from proxy responses
- [x] CORS configured via frontend_url
- [x] Secure cookie flag based on environment
- [x] Sessions expire (TTL index on expires_at)
- [x] Audit logging on all sensitive operations
- [ ] **MISSING:** Per-endpoint rate limiting on credential operations (SEC-M1)
- [ ] **MISSING:** SSRF protection at proxy time / DNS rebinding (SEC-H1)
- [ ] **MISSING:** Path traversal prevention in proxy (SEC-H3)
- [ ] **BUG:** Reconnection blocked by unique index (SEC-C1)
- [ ] **MISSING:** Credential wipe on service deactivation (SEC-M2)

## Recommendations

1. **Fix SEC-C1 immediately** - The unique index bug prevents reconnection, which is a fundamental functional failure
2. **Add path validation to proxy** - Reject paths containing `..` or other traversal sequences
3. **Add DNS-level SSRF protection** - Either re-validate resolved IPs at proxy time or use a deny-list resolving proxy
4. **Remove Debug derive** from credential-bearing request types or implement custom redacting Debug
5. **Parse encryption key once at startup** and store in AppState
6. **Add per-endpoint rate limiters** for credential and proxy operations
7. **Cascade deactivation** - When a service is deactivated, wipe all associated user credentials
8. **Sanitize error messages** - Return generic errors to clients, log details server-side
