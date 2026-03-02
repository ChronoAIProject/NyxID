# API Proxy Access Architecture Plan

## Overview

Enable developers to access downstream services through direct API calls using NyxID API keys with developer-friendly slug-based URLs, without requiring MCP. The system already supports API key authentication for proxy requests; this plan adds slug-based routing, a service discovery endpoint, and populates API key scopes in the auth context.

---

## 1. Current State Analysis

### 1.1 What Already Works

**API key authentication for proxy requests** is fully functional today:

1. `mw/auth.rs:264-301` -- The `AuthUser` extractor checks for `X-API-Key` header, calls `key_service::validate_api_key()`, verifies the user is active, and returns an `AuthUser` struct.
2. `handlers/proxy.rs:40-256` -- The proxy handler (`ANY /api/v1/proxy/{service_id}/{*path}`) accepts any `AuthUser` including API-key-authenticated users. No scope check is performed.
3. `routes.rs:253-256` -- The proxy route lives in `api_v1_delegated`, which has no middleware blocking API key auth or service account tokens.

**A developer can already do this today:**
```bash
curl http://localhost:3001/api/v1/proxy/d1e2f3a4-b5c6-7890-1234-567890abcdef/v1/reports \
  -H "X-API-Key: nyx_k_a1b2c3d4..."
```

**Service slugs already exist** on the `DownstreamService` model (`models/downstream_service.rs:11`) with a unique index on `slug` (`db.rs:206-212`).

**The `proxy` scope** is defined in `VALID_API_KEY_SCOPES` (`key_service.rs:31`) but is never enforced anywhere.

### 1.2 What's Missing

| Gap | Description |
|-----|-------------|
| **Slug-based proxy route** | Only UUID-based route exists (`/proxy/{service_id}/{*path}`). Developers must know the UUID. |
| **Slug-based resolution** | `proxy_service::resolve_proxy_target()` only looks up by `_id` (`proxy_service.rs:48-49`). No slug lookup. |
| **Service discovery** | No endpoint for API key users to list available services and their proxy URLs. |
| **API key scopes in AuthUser** | `mw/auth.rs:270` discards the `ApiKey` struct (`_key`), and line 299 sets `scope: String::new()`. API key scopes are never populated into `AuthUser`. |
| **Proxy scope enforcement** | The `proxy` scope exists but is never checked. Any API key can proxy regardless of scopes. |

---

## 2. Proposed Changes

### 2.1 Service Layer: Add Slug-Based Resolution

**File:** `backend/src/services/proxy_service.rs`

Add a new function `resolve_service_by_slug` that looks up a `DownstreamService` by slug:

```rust
/// Resolve a downstream service by its slug.
/// Returns the service document or NotFound.
pub async fn resolve_service_by_slug(
    db: &mongodb::Database,
    slug: &str,
) -> AppResult<DownstreamService> {
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": slug })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Service with slug '{}' not found", slug)))
}
```

This function is intentionally separate from `resolve_proxy_target` -- it only resolves the slug to a service. The existing `resolve_proxy_target` then takes the service ID for credential resolution.

**No changes to `resolve_proxy_target`** -- it continues to accept `service_id: &str` (the UUID). The slug handler resolves slug -> service_id first, then calls `resolve_proxy_target`.

### 2.2 Handler Layer: Slug-Based Proxy Handler + Core Extraction

**File:** `backend/src/handlers/proxy.rs`

The existing `proxy_request` handler is 257 lines. Rather than duplicating all that logic, extract the core proxy execution into a shared function:

#### 2.2.1 Extract Shared Core Function

```rust
/// Core proxy execution logic shared by UUID and slug handlers.
///
/// Takes the resolved service_id (UUID string) and executes the full proxy
/// pipeline: resolve target, build identity headers, inject delegation token,
/// resolve delegated credentials, forward request, and return response.
async fn execute_proxy(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
    path: &str,
    request: Request<Body>,
) -> AppResult<Response> {
    // ... existing proxy_request body from lines 46-255, using service_id param ...
}
```

#### 2.2.2 Refactor Existing Handler

```rust
/// ANY /api/v1/proxy/{service_id}/{*path}
pub async fn proxy_request(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((service_id, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response> {
    execute_proxy(&state, &auth_user, &service_id, &path, request).await
}
```

#### 2.2.3 Add Slug-Based Handler

```rust
/// ANY /api/v1/proxy/s/{slug}/{*path}
///
/// Resolve the service by slug, then forward via the shared proxy pipeline.
pub async fn proxy_request_by_slug(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response> {
    let service = proxy_service::resolve_service_by_slug(&state.db, &slug).await?;
    execute_proxy(&state, &auth_user, &service.id, &path, request).await
}
```

### 2.3 Handler Layer: Service Discovery Endpoint

**File:** `backend/src/handlers/proxy.rs` (add to the same file)

Add a discovery endpoint that returns available services with their proxy URLs:

```rust
#[derive(Debug, Serialize)]
pub struct ProxyServiceItem {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub service_category: String,
    /// Whether the user has an active connection to this service
    pub connected: bool,
    /// Whether a connection is required before proxying
    pub requires_connection: bool,
    /// UUID-based proxy URL
    pub proxy_url: String,
    /// Slug-based proxy URL (developer-friendly)
    pub proxy_url_slug: String,
}

#[derive(Debug, Serialize)]
pub struct ProxyServicesResponse {
    pub services: Vec<ProxyServiceItem>,
}

/// GET /api/v1/proxy/services
///
/// List downstream services available for proxying with their proxy URLs.
/// Excludes "provider" category services (not proxyable).
pub async fn list_proxy_services(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<ProxyServicesResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let base = state.config.base_url.trim_end_matches('/');

    // Get all active, non-provider services
    let services: Vec<DownstreamService> = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(doc! {
            "is_active": true,
            "service_category": { "$ne": "provider" },
        })
        .sort(doc! { "name": 1 })
        .await?
        .try_collect()
        .await?;

    // Get user's active connections in a single query
    let service_ids: Vec<&str> = services.iter().map(|s| s.id.as_str()).collect();
    let connections: Vec<UserServiceConnection> = if service_ids.is_empty() {
        vec![]
    } else {
        state
            .db
            .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
            .find(doc! {
                "user_id": &user_id_str,
                "service_id": { "$in": &service_ids },
                "is_active": true,
            })
            .await?
            .try_collect()
            .await?
    };

    let connected_set: std::collections::HashSet<&str> = connections
        .iter()
        .map(|c| c.service_id.as_str())
        .collect();

    let items: Vec<ProxyServiceItem> = services
        .iter()
        .map(|s| ProxyServiceItem {
            id: s.id.clone(),
            name: s.name.clone(),
            slug: s.slug.clone(),
            description: s.description.clone(),
            service_category: s.service_category.clone(),
            connected: connected_set.contains(s.id.as_str()),
            requires_connection: s.requires_user_credential,
            proxy_url: format!("{base}/api/v1/proxy/{}/{{}}", s.id),
            proxy_url_slug: format!("{base}/api/v1/proxy/s/{}/{{}}", s.slug),
        })
        .collect();

    Ok(Json(ProxyServicesResponse { services: items }))
}
```

### 2.4 Auth Middleware: Populate API Key Scopes

**File:** `backend/src/mw/auth.rs`

Change the API key auth block (lines 264-302) to populate `AuthUser.scope` from the API key's scopes:

**Before (line 270, 296-301):**
```rust
let (user_id_str, _key) =
    crate::services::key_service::validate_api_key(&state.db, api_key).await?;
// ...
return Ok(AuthUser {
    user_id,
    session_id: None,
    scope: String::new(),
    acting_client_id: None,
});
```

**After:**
```rust
let (user_id_str, key) =
    crate::services::key_service::validate_api_key(&state.db, api_key).await?;
// ...
return Ok(AuthUser {
    user_id,
    session_id: None,
    scope: key.scopes.clone(),
    acting_client_id: None,
});
```

**Also update the doc comment** on `AuthUser.scope` (line 30):
```rust
/// Space-separated scopes from the access token or API key (empty for session auth).
pub scope: String,
```

**Impact analysis:** This change populates `AuthUser.scope` with API key scopes (e.g., `"read write"`). Currently no handler checks `AuthUser.scope` for API key requests. The only places that read `auth_user.scope` are:
- `handlers/delegation.rs:42` -- delegation token refresh, uses scope from delegated JWT tokens
- `handlers/oauth.rs:963` -- OAuth userinfo, uses scope from OAuth access tokens

Neither of these code paths is reachable via API key auth (delegation requires a delegated Bearer token, OAuth userinfo requires a Bearer token). So this change is **backward compatible** and has no side effects on existing functionality.

### 2.5 Route Registration

**File:** `backend/src/routes.rs`

Add the new routes to the `api_v1_delegated` router (lines 253-256):

**Before:**
```rust
let api_v1_delegated = Router::new()
    .nest("/llm", llm_routes)
    .nest("/delegation", delegation_routes)
    .route("/proxy/{service_id}/{*path}", axum::routing::any(handlers::proxy::proxy_request));
```

**After:**
```rust
let api_v1_delegated = Router::new()
    .nest("/llm", llm_routes)
    .nest("/delegation", delegation_routes)
    .route("/proxy/services", get(handlers::proxy::list_proxy_services))
    .route("/proxy/s/{slug}/{*path}", axum::routing::any(handlers::proxy::proxy_request_by_slug))
    .route("/proxy/{service_id}/{*path}", axum::routing::any(handlers::proxy::proxy_request));
```

**Route ordering matters:** The `/proxy/services` and `/proxy/s/{slug}/{*path}` routes MUST be registered before `/proxy/{service_id}/{*path}` because Axum matches routes in registration order when path segments could conflict. The literal `services` and `s` segments will match before the `{service_id}` parameter.

**Import addition** (line 4): Add `get` to the routing import:
```rust
use axum::routing::{delete, get, patch, post, put};
```
(Already imported -- `get` is already in the import.)

---

## 3. API Surface Design

### 3.1 Slug-Based Proxy (New)

```
ANY /api/v1/proxy/s/{slug}/{*path}
```

Identical behavior to the existing UUID-based route, but uses the service slug instead of UUID.

**curl examples:**

```bash
# GET request via slug
curl http://localhost:3001/api/v1/proxy/s/stripe/v1/charges \
  -H "X-API-Key: nyx_k_a1b2c3d4..."

# POST request via slug
curl -X POST http://localhost:3001/api/v1/proxy/s/stripe/v1/charges \
  -H "X-API-Key: nyx_k_a1b2c3d4..." \
  -H "Content-Type: application/json" \
  -d '{"amount": 2000, "currency": "usd"}'

# Also works with Bearer token
curl http://localhost:3001/api/v1/proxy/s/my-internal-api/health \
  -H "Authorization: Bearer eyJhbGciOiJSUzI1NiIs..."

# Also works with delegated tokens (for downstream services calling other services)
curl http://localhost:3001/api/v1/proxy/s/analytics-api/v1/events \
  -H "Authorization: Bearer <delegated_token>" \
  -H "Content-Type: application/json" \
  -d '{"event": "page_view"}'
```

### 3.2 Service Discovery (New)

```
GET /api/v1/proxy/services
```

Returns all proxyable services with their connection status and proxy URLs.

**curl example:**

```bash
curl http://localhost:3001/api/v1/proxy/services \
  -H "X-API-Key: nyx_k_a1b2c3d4..."
```

**Response:**

```json
{
  "services": [
    {
      "id": "d1e2f3a4-b5c6-7890-1234-567890abcdef",
      "name": "Stripe API",
      "slug": "stripe",
      "description": "Payment processing",
      "service_category": "connection",
      "connected": true,
      "requires_connection": true,
      "proxy_url": "http://localhost:3001/api/v1/proxy/d1e2f3a4-b5c6-7890-1234-567890abcdef/{}",
      "proxy_url_slug": "http://localhost:3001/api/v1/proxy/s/stripe/{}"
    },
    {
      "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
      "name": "Internal Analytics",
      "slug": "analytics",
      "description": "Internal analytics service",
      "service_category": "internal",
      "connected": false,
      "requires_connection": false,
      "proxy_url": "http://localhost:3001/api/v1/proxy/a1b2c3d4-e5f6-7890-abcd-ef1234567890/{}",
      "proxy_url_slug": "http://localhost:3001/api/v1/proxy/s/analytics/{}"
    }
  ]
}
```

### 3.3 Existing UUID-Based Proxy (Unchanged)

```
ANY /api/v1/proxy/{service_id}/{*path}
```

Continues to work exactly as before. No changes.

---

## 4. Security Considerations

### 4.1 Slug Validation

Service slugs are validated at creation time (`handlers/services.rs`) and stored in a unique index. The `resolve_service_by_slug` function uses a MongoDB query with the slug as a string match -- no risk of injection since MongoDB document queries are not string-interpolated.

### 4.2 Path Traversal

The slug-based handler delegates to the same `execute_proxy` function which calls `proxy_service::forward_request`. That function already rejects paths containing `..` or `//` (`proxy_service.rs:151-153`).

### 4.3 API Key Scope Enforcement (Future Enhancement)

The `proxy` scope exists in `VALID_API_KEY_SCOPES` but is not enforced in this implementation. This is intentional for backward compatibility:

- Existing API keys have default scope `"read"` (no `proxy`)
- Enforcing `proxy` scope would break all existing API key proxy users
- The scope population change (Section 2.4) lays the groundwork for future enforcement

**Recommended future work:** Add optional per-service scope enforcement. Add a `required_api_key_scopes: Option<String>` field to `DownstreamService`. When set, the proxy handler checks that the API key's scopes include the required scopes. This gives admins per-service control without a global breaking change.

### 4.4 Rate Limiting

All new routes inherit the existing global rate limiter from `mw/rate_limit.rs`. The discovery endpoint (`GET /proxy/services`) is lightweight (two MongoDB queries) and should be fine under the global rate limit.

### 4.5 Audit Trail

The slug-based handler delegates to `execute_proxy` which produces the same `proxy_request` and `proxy_request_denied` audit events. The `service_id` in audit logs will be the UUID (resolved from slug), maintaining consistency with existing audit entries.

---

## 5. Implementation Steps

Ordered list for the backend developer:

### Step 1: Populate API Key Scopes in AuthUser

**File:** `backend/src/mw/auth.rs`

1. Change line 270: `_key` -> `key`
2. Change line 299: `scope: String::new()` -> `scope: key.scopes.clone()`
3. Update doc comment on `AuthUser.scope` (line 30)

**Why first:** This is a foundational change that all other features can build on. It has zero side effects (verified in Section 2.4).

**Verification:** Run `cargo test` -- all existing tests should pass unchanged.

### Step 2: Add Slug-Based Resolution to Proxy Service

**File:** `backend/src/services/proxy_service.rs`

1. Add `resolve_service_by_slug()` function (Section 2.1)

**Verification:** Add a unit test for the new function (if integration test infra exists) or verify in Step 5.

### Step 3: Extract Core Proxy Logic and Add Slug Handler

**File:** `backend/src/handlers/proxy.rs`

1. Extract `execute_proxy()` from `proxy_request` (Section 2.2.1)
2. Refactor `proxy_request` to call `execute_proxy` (Section 2.2.2)
3. Add `proxy_request_by_slug` handler (Section 2.2.3)
4. Add necessary imports: `futures::TryStreamExt`, `serde::Serialize`, `models::downstream_service::*`, `models::user_service_connection::*`

**Verification:** `cargo build` should compile. Existing proxy functionality should work identically (refactor is behavior-preserving).

### Step 4: Add Service Discovery Handler

**File:** `backend/src/handlers/proxy.rs`

1. Add response structs `ProxyServiceItem` and `ProxyServicesResponse` (Section 2.3)
2. Add `list_proxy_services` handler (Section 2.3)
3. Add import for `axum::Json`

**Verification:** `cargo build` should compile.

### Step 5: Register New Routes

**File:** `backend/src/routes.rs`

1. Add three new routes to `api_v1_delegated` (Section 2.5)
2. Ensure route ordering: `/proxy/services` and `/proxy/s/{slug}/{*path}` before `/proxy/{service_id}/{*path}`

**Verification:** `cargo build` and `cargo test`. Then manual smoke test:
```bash
# Discovery
curl http://localhost:3001/api/v1/proxy/services -H "X-API-Key: <key>"

# Slug-based proxy
curl http://localhost:3001/api/v1/proxy/s/<slug>/health -H "X-API-Key: <key>"
```

### Step 6: Update Tests

1. Add test for `resolve_service_by_slug` in `proxy_service.rs`
2. Update `mw/auth.rs` tests to verify API key scope population (add a test that checks `AuthUser.scope` is non-empty when API key has scopes)
3. Add integration tests for slug-based proxy route if test infra supports it

---

## 6. Backward Compatibility

| Change | Impact | Risk |
|--------|--------|------|
| New route `/proxy/s/{slug}/{*path}` | Purely additive | None |
| New route `/proxy/services` | Purely additive | None |
| Existing route `/proxy/{service_id}/{*path}` | Unchanged | None |
| `AuthUser.scope` populated for API keys | No handler checks this for API key auth paths | None (verified in Section 2.4) |
| `execute_proxy` extraction | Internal refactor, same behavior | None |

All changes are additive or internal refactors. No existing API contracts are modified. The UUID-based proxy route continues to work identically.

---

## 7. Files Changed Summary

| File | Change Type | Description |
|------|-------------|-------------|
| `backend/src/mw/auth.rs` | **Modify** | Populate `AuthUser.scope` from API key scopes (3 lines) |
| `backend/src/services/proxy_service.rs` | **Modify** | Add `resolve_service_by_slug()` (~10 lines) |
| `backend/src/handlers/proxy.rs` | **Modify** | Extract `execute_proxy()`, add `proxy_request_by_slug`, add `list_proxy_services` (~100 lines net new) |
| `backend/src/routes.rs` | **Modify** | Add 2 new routes (2 lines) |

**No new files.** No model changes. No index changes. No migration needed.
