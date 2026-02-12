# Service Accounts Architecture Plan

## Overview

Service accounts are non-human (machine-to-machine) identities in NyxID that authenticate programmatically via OAuth2 Client Credentials Grant. They are stored in a dedicated `service_accounts` collection (separate from `users`), managed by admins, and can access downstream services, providers, the LLM gateway, and the proxy -- the same resources available to regular users.

This document covers all data model changes, API endpoints, service layer logic, middleware modifications, frontend pages, and security considerations needed for a complete implementation.

---

## 1. Data Model

### 1.1 New Collection: `service_accounts`

**File:** `backend/src/models/service_account.rs`

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "service_accounts";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceAccount {
    /// UUID v4 string, serves as the identity subject (`sub` in JWT claims).
    #[serde(rename = "_id")]
    pub id: String,

    /// Human-readable name (e.g. "CI/CD Pipeline", "Monitoring Agent").
    pub name: String,

    /// Optional description of what this service account does.
    pub description: Option<String>,

    /// Unique client_id for OAuth2 Client Credentials Grant.
    /// Format: "sa_" + 24 random hex chars (e.g. "sa_a1b2c3d4e5f6...").
    pub client_id: String,

    /// SHA-256 hash of the client_secret.
    /// The raw secret is shown once at creation, never stored.
    pub client_secret_hash: String,

    /// First 8 chars of client_secret for UI identification.
    pub secret_prefix: String,

    /// Directly assigned role IDs (no group membership for service accounts).
    #[serde(default)]
    pub role_ids: Vec<String>,

    /// Space-separated allowed scopes. Token requests can request a subset.
    /// Examples: "openid proxy:* llm:proxy services:read connections:read"
    pub allowed_scopes: String,

    /// Whether this service account can authenticate.
    pub is_active: bool,

    /// Optional per-account rate limit override (requests per second).
    /// When None, the global rate limit applies.
    pub rate_limit_override: Option<u64>,

    /// The admin user ID who created this service account.
    pub created_by: String,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,

    #[serde(default, with = "bson_datetime::optional")]
    pub last_authenticated_at: Option<DateTime<Utc>>,
}
```

**Indexes** (added to `db.rs` `ensure_indexes()`):

```rust
// -- service_accounts --
let sa = db.collection::<Document>("service_accounts");
sa.create_index(
    IndexModel::builder()
        .keys(doc! { "client_id": 1 })
        .options(IndexOptions::builder().unique(true).build())
        .build(),
).await?;
sa.create_index(
    IndexModel::builder()
        .keys(doc! { "is_active": 1 })
        .build(),
).await?;
sa.create_index(
    IndexModel::builder()
        .keys(doc! { "created_by": 1 })
        .build(),
).await?;
```

### 1.2 New Collection: `service_account_tokens`

Tracks JWT access tokens issued to service accounts for revocation support. Without this, issued JWTs cannot be revoked before expiry.

**File:** `backend/src/models/service_account_token.rs`

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "service_account_tokens";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceAccountToken {
    #[serde(rename = "_id")]
    pub id: String,

    /// The JTI (JWT ID) of the issued token, for revocation lookups.
    pub jti: String,

    /// The service account that owns this token.
    pub service_account_id: String,

    /// Space-separated scopes granted to this token.
    pub scope: String,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,

    pub revoked: bool,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
```

**Indexes:**

```rust
// -- service_account_tokens --
let sat = db.collection::<Document>("service_account_tokens");
sat.create_index(
    IndexModel::builder()
        .keys(doc! { "jti": 1 })
        .options(IndexOptions::builder().unique(true).build())
        .build(),
).await?;
sat.create_index(
    IndexModel::builder()
        .keys(doc! { "service_account_id": 1 })
        .build(),
).await?;
sat.create_index(
    IndexModel::builder()
        .keys(doc! { "expires_at": 1 })
        .options(
            IndexOptions::builder()
                .expire_after(Duration::from_secs(0))
                .build(),
        )
        .build(),
).await?;
```

### 1.3 Existing Collections -- No Changes Required

The following collections already use a generic `user_id: String` field. Service account IDs will be stored in these fields when a service account connects to services or providers. **No schema changes are needed.**

- `user_service_connections` -- `user_id` will hold the service account ID
- `user_provider_tokens` -- `user_id` will hold the service account ID
- `audit_log` -- `user_id` is already `Option<String>`
- `api_keys` -- Service accounts will NOT use API keys (they use client credentials instead)

### 1.4 Module Registration

**File:** `backend/src/models/mod.rs` -- Add:

```rust
pub mod service_account;
pub mod service_account_token;
```

---

## 2. Authentication Flow

### 2.1 OAuth2 Client Credentials Grant

Service accounts authenticate at the **existing** `POST /oauth/token` endpoint with `grant_type=client_credentials`.

**Request:**

```
POST /oauth/token
Content-Type: application/x-www-form-urlencoded

grant_type=client_credentials
&client_id=sa_a1b2c3d4e5f6...
&client_secret=sas_xxxxxxxxxxxxxxxx
&scope=proxy:* llm:proxy          (optional; defaults to all allowed_scopes)
```

**Or** via HTTP Basic Auth:

```
POST /oauth/token
Content-Type: application/x-www-form-urlencoded
Authorization: Basic base64(client_id:client_secret)

grant_type=client_credentials
&scope=proxy:* llm:proxy
```

**Response:**

```json
{
  "access_token": "eyJhbGci...",
  "token_type": "Bearer",
  "expires_in": 3600,
  "scope": "proxy:* llm:proxy"
}
```

**JWT Claims for Service Account Tokens:**

```json
{
  "sub": "<service_account._id>",
  "iss": "nyxid",
  "aud": "http://localhost:3001",
  "exp": 1700003600,
  "iat": 1700000000,
  "jti": "uuid-v4",
  "scope": "proxy:* llm:proxy",
  "token_type": "access",
  "sa": true
}
```

Key differences from user tokens:
- `sub` is the service account ID (not a user ID)
- `sa: true` custom claim identifies this as a service account token
- No `roles`, `groups`, `permissions` claims (RBAC is checked at request time via the service account's `role_ids`)
- No `sid` (no session concept)
- No `act` or `delegated` (service accounts act on their own behalf)

### 2.2 Token TTL

Service account tokens default to **1 hour** (3600 seconds). This can be configured via a new env var:

```
SA_TOKEN_TTL_SECS=3600  # default: 1 hour
```

**File:** `backend/src/config.rs` -- Add:

```rust
/// Service account token TTL in seconds (default: 3600 = 1 hour)
pub sa_token_ttl_secs: i64,
```

No refresh tokens are issued for service accounts. When a token expires, the service account must re-authenticate with client credentials.

### 2.3 JWT Changes

**File:** `backend/src/crypto/jwt.rs`

Add a new `sa` field to `Claims`:

```rust
/// True if this token was issued to a service account.
#[serde(skip_serializing_if = "Option::is_none")]
pub sa: Option<bool>,
```

Add a new function:

```rust
/// Generate an access token for a service account.
///
/// Like a regular access token, but with `sa: true` and no RBAC claims
/// embedded (RBAC is resolved at request time for service accounts).
pub fn generate_service_account_token(
    keys: &JwtKeys,
    config: &AppConfig,
    service_account_id: &str,
    scope: &str,
    ttl_secs: i64,
) -> Result<(String, String), AppError> {
    let now = Utc::now().timestamp();
    let jti = Uuid::new_v4().to_string();

    let claims = Claims {
        sub: service_account_id.to_string(),
        iss: config.jwt_issuer.clone(),
        aud: config.base_url.clone(),
        exp: now + ttl_secs,
        iat: now,
        jti: jti.clone(),
        scope: scope.to_string(),
        token_type: "access".to_string(),
        roles: None,
        groups: None,
        permissions: None,
        sid: None,
        act: None,
        delegated: None,
        sa: Some(true),
    };

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(keys.kid.clone());

    let token = encode(&header, &claims, &keys.encoding)
        .map_err(|e| AppError::Internal(format!("Failed to encode SA token: {e}")))?;

    Ok((token, jti))
}
```

---

## 3. Auth Middleware Changes

### 3.1 `AuthUser` Extractor

**File:** `backend/src/mw/auth.rs`

Add a field to `AuthUser` to distinguish service accounts:

```rust
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub session_id: Option<Uuid>,
    pub scope: String,
    pub acting_client_id: Option<String>,
    /// True if authenticated as a service account (not a regular user).
    pub is_service_account: bool,
}
```

### 3.2 Bearer Token Extraction -- Service Account Path

In the `FromRequestParts` implementation, after verifying the JWT and extracting claims, add a branch for `sa: true` tokens:

```rust
// After: let claims = jwt::verify_token(...)
// Check if this is a service account token
if claims.sa == Some(true) {
    let sa_id = claims.sub.clone();

    // Verify the service account exists, is active, and token is not revoked
    let sa = state
        .db
        .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .find_one(doc! { "_id": &sa_id, "is_active": true })
        .await
        .map_err(|e| AppError::Internal(format!("SA lookup failed: {e}")))?
        .ok_or_else(|| {
            AppError::Unauthorized("Service account is inactive or not found".to_string())
        })?;

    // Check token revocation
    let token_record = state
        .db
        .collection::<ServiceAccountToken>(SA_TOKENS)
        .find_one(doc! { "jti": &claims.jti })
        .await
        .map_err(|e| AppError::Internal(format!("SA token lookup failed: {e}")))?;

    if let Some(record) = token_record {
        if record.revoked {
            return Err(AppError::Unauthorized("Token has been revoked".to_string()));
        }
    }

    // Parse UUID from service account ID
    let sa_uuid = Uuid::parse_str(&sa_id).map_err(|_| {
        AppError::Unauthorized("Invalid service account ID".to_string())
    })?;

    return Ok(AuthUser {
        user_id: sa_uuid,
        session_id: None,
        scope: claims.scope.clone(),
        acting_client_id: None,
        is_service_account: true,
    });
}
```

This goes **before** the existing user verification logic in the Bearer token branch.

### 3.3 Endpoint Access Control

Service accounts can access the following route groups:
- `/api/v1/proxy/{service_id}/{path}` -- Proxy requests
- `/api/v1/llm/*` -- LLM gateway
- `/api/v1/connections/*` -- Service connections
- `/api/v1/providers/*` -- Provider management
- `/api/v1/delegation/*` -- Delegation token refresh

Service accounts **cannot** access:
- `/api/v1/auth/*` -- Human auth flows (login, register, MFA)
- `/api/v1/users/*` -- Human user profile
- `/api/v1/sessions/*` -- Human sessions
- `/api/v1/api-keys/*` -- API keys (for human users only)
- `/api/v1/admin/*` -- Admin panel (uses `require_admin()` which checks `users` collection)
- `/api/v1/services/*` -- Service definition management (admin concern)
- `/api/v1/mcp/*` -- MCP config (human user concern)

Add a middleware function to reject service accounts from human-only routes:

```rust
/// Middleware that rejects service account tokens from human-only endpoints.
pub async fn reject_service_account_tokens(
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<impl IntoResponse, AppError> {
    if is_service_account_request(&request) {
        return Err(AppError::Forbidden(
            "Service accounts cannot access this endpoint".to_string(),
        ));
    }
    Ok(next.run(request).await)
}
```

This middleware is applied in `routes.rs` to the `api_v1_protected` group alongside `reject_delegated_tokens`.

### 3.4 Route Changes

**File:** `backend/src/routes.rs`

```rust
// Routes that BLOCK service account tokens (human-only endpoints)
let api_v1_human_only = Router::new()
    .nest("/auth", auth_routes)
    .nest("/users", user_routes)
    .nest("/api-keys", api_key_routes)
    .nest("/services", service_routes)
    .nest("/sessions", session_routes)
    .nest("/mcp", mcp_routes)
    .nest("/admin", admin_routes)
    .route("/public/config", get(handlers::health::public_config))
    .layer(middleware::from_fn(reject_delegated_tokens))
    .layer(middleware::from_fn(reject_service_account_tokens));

// Routes accessible by both users and service accounts
let api_v1_shared = Router::new()
    .nest("/connections", connection_routes)
    .nest("/providers", provider_routes)
    .layer(middleware::from_fn(reject_delegated_tokens));

// Routes that ALLOW delegated tokens (proxy, LLM, delegation)
let api_v1_delegated = Router::new()
    .nest("/llm", llm_routes)
    .nest("/delegation", delegation_routes)
    .route("/proxy/{service_id}/{*path}", any(handlers::proxy::proxy_request));

let api_v1 = api_v1_delegated
    .merge(api_v1_shared)
    .merge(api_v1_human_only);
```

---

## 4. Service Layer

### 4.1 New Service: `service_account_service.rs`

**File:** `backend/src/services/service_account_service.rs`

```rust
// Core functions:

/// Create a new service account. Returns (ServiceAccount, raw_client_secret).
pub async fn create_service_account(
    db: &Database,
    name: &str,
    description: Option<&str>,
    allowed_scopes: &str,
    role_ids: &[String],
    rate_limit_override: Option<u64>,
    created_by: &str,
) -> AppResult<(ServiceAccount, String)>

/// List all service accounts (paginated).
pub async fn list_service_accounts(
    db: &Database,
    page: u64,
    per_page: u64,
    search: Option<&str>,
) -> AppResult<(Vec<ServiceAccount>, u64)>

/// Get a service account by ID.
pub async fn get_service_account(
    db: &Database,
    sa_id: &str,
) -> AppResult<ServiceAccount>

/// Update a service account's mutable fields.
pub async fn update_service_account(
    db: &Database,
    sa_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    allowed_scopes: Option<&str>,
    role_ids: Option<&[String]>,
    rate_limit_override: Option<Option<u64>>,
    is_active: Option<bool>,
) -> AppResult<ServiceAccount>

/// Rotate the client secret. Revokes all outstanding tokens.
/// Returns (updated ServiceAccount, new raw_client_secret).
pub async fn rotate_secret(
    db: &Database,
    sa_id: &str,
) -> AppResult<(ServiceAccount, String)>

/// Soft-delete (deactivate) a service account and revoke all tokens.
pub async fn delete_service_account(
    db: &Database,
    sa_id: &str,
) -> AppResult<()>

/// Revoke all active tokens for a service account.
pub async fn revoke_all_tokens(
    db: &Database,
    sa_id: &str,
) -> AppResult<u64>

/// Authenticate via client credentials: validate client_id + client_secret,
/// issue a JWT, and persist a token record.
pub async fn authenticate_client_credentials(
    db: &Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    client_id: &str,
    client_secret: &str,
    requested_scope: Option<&str>,
) -> AppResult<ClientCredentialsResponse>

pub struct ClientCredentialsResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub scope: String,
}
```

**Key implementation details:**

- `client_id` generation: `format!("sa_{}", hex::encode(random_bytes(12)))` -- 24 hex chars, prefixed with `sa_` for easy identification.
- `client_secret` generation: `format!("sas_{}", hex::encode(random_bytes(32)))` -- 64 hex chars, prefixed with `sas_`.
- Secret hashing: SHA-256 (same as `crypto::token::hash_token`), consistent with API keys and OAuth clients.
- `secret_prefix`: first 8 chars of the raw secret for UI display.
- Scope validation: requested scopes must be a subset of `allowed_scopes`.
- `last_authenticated_at` is updated on each successful authentication.

### 4.2 Module Registration

**File:** `backend/src/services/mod.rs` -- Add:

```rust
pub mod service_account_service;
```

---

## 5. Handler Layer

### 5.1 New Handler: `admin_service_accounts.rs`

**File:** `backend/src/handlers/admin_service_accounts.rs`

All endpoints are under `/api/v1/admin/service-accounts` and require admin authentication.

#### Request/Response Types

```rust
// --- Create ---
#[derive(Deserialize)]
pub struct CreateServiceAccountRequest {
    pub name: String,
    pub description: Option<String>,
    pub allowed_scopes: String,
    pub role_ids: Option<Vec<String>>,
    pub rate_limit_override: Option<u64>,
}

#[derive(Serialize)]
pub struct CreateServiceAccountResponse {
    pub id: String,
    pub name: String,
    pub client_id: String,
    /// Only returned once at creation time!
    pub client_secret: String,
    pub allowed_scopes: String,
    pub role_ids: Vec<String>,
    pub is_active: bool,
    pub created_at: String,
    pub message: String,
}

// --- List ---
#[derive(Deserialize)]
pub struct ServiceAccountListQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub search: Option<String>,
}

#[derive(Serialize)]
pub struct ServiceAccountItem {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub client_id: String,
    pub secret_prefix: String,
    pub allowed_scopes: String,
    pub role_ids: Vec<String>,
    pub is_active: bool,
    pub rate_limit_override: Option<u64>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_authenticated_at: Option<String>,
}

#[derive(Serialize)]
pub struct ServiceAccountListResponse {
    pub service_accounts: Vec<ServiceAccountItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

// --- Update ---
#[derive(Deserialize)]
pub struct UpdateServiceAccountRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub allowed_scopes: Option<String>,
    pub role_ids: Option<Vec<String>>,
    pub rate_limit_override: Option<Option<u64>>,
    pub is_active: Option<bool>,
}

// --- Rotate ---
#[derive(Serialize)]
pub struct RotateSecretResponse {
    pub client_id: String,
    /// Only returned once!
    pub client_secret: String,
    pub secret_prefix: String,
    pub message: String,
}

// --- Revoke ---
#[derive(Serialize)]
pub struct RevokeTokensResponse {
    pub revoked_count: u64,
    pub message: String,
}
```

#### Endpoint Handlers

```rust
/// POST /api/v1/admin/service-accounts
pub async fn create_service_account(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Json(body): Json<CreateServiceAccountRequest>,
) -> AppResult<Json<CreateServiceAccountResponse>>

/// GET /api/v1/admin/service-accounts
pub async fn list_service_accounts(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ServiceAccountListQuery>,
) -> AppResult<Json<ServiceAccountListResponse>>

/// GET /api/v1/admin/service-accounts/:sa_id
pub async fn get_service_account(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(sa_id): Path<String>,
) -> AppResult<Json<ServiceAccountItem>>

/// PUT /api/v1/admin/service-accounts/:sa_id
pub async fn update_service_account(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(sa_id): Path<String>,
    Json(body): Json<UpdateServiceAccountRequest>,
) -> AppResult<Json<ServiceAccountItem>>

/// DELETE /api/v1/admin/service-accounts/:sa_id
pub async fn delete_service_account(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(sa_id): Path<String>,
) -> AppResult<Json<AdminActionResponse>>

/// POST /api/v1/admin/service-accounts/:sa_id/rotate-secret
pub async fn rotate_secret(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(sa_id): Path<String>,
) -> AppResult<Json<RotateSecretResponse>>

/// POST /api/v1/admin/service-accounts/:sa_id/revoke-tokens
pub async fn revoke_tokens(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(sa_id): Path<String>,
) -> AppResult<Json<RevokeTokensResponse>>
```

### 5.2 Handler Module Registration

**File:** `backend/src/handlers/mod.rs` -- Add:

```rust
pub mod admin_service_accounts;
```

### 5.3 Route Registration

**File:** `backend/src/routes.rs` -- Add within `admin_routes`:

```rust
let sa_admin_routes = Router::new()
    .route("/", get(handlers::admin_service_accounts::list_service_accounts)
        .post(handlers::admin_service_accounts::create_service_account))
    .route("/{sa_id}", get(handlers::admin_service_accounts::get_service_account)
        .put(handlers::admin_service_accounts::update_service_account)
        .delete(handlers::admin_service_accounts::delete_service_account))
    .route("/{sa_id}/rotate-secret",
        post(handlers::admin_service_accounts::rotate_secret))
    .route("/{sa_id}/revoke-tokens",
        post(handlers::admin_service_accounts::revoke_tokens));

// Nest under admin_routes:
let admin_routes = Router::new()
    // ... existing admin routes ...
    .nest("/service-accounts", sa_admin_routes);
```

### 5.4 OAuth Token Endpoint Extension

**File:** `backend/src/handlers/oauth.rs` -- `token()` function

Add a new match arm for `client_credentials`:

```rust
"client_credentials" => {
    let client_id = body.client_id.as_deref()
        .ok_or_else(|| AppError::BadRequest("Missing client_id".to_string()))?;
    let client_secret = body.client_secret.as_deref()
        .ok_or_else(|| AppError::BadRequest("Missing client_secret".to_string()))?;

    let result = service_account_service::authenticate_client_credentials(
        &state.db,
        &state.config,
        &state.jwt_keys,
        client_id,
        client_secret,
        body.scope.as_deref(),
    ).await?;

    audit_service::log_async(
        state.db.clone(),
        None,
        "sa.token_issued".to_string(),
        Some(serde_json::json!({
            "client_id": client_id,
            "scope": &result.scope,
        })),
        None,
        None,
    );

    Ok(Json(TokenResponse {
        access_token: result.access_token,
        token_type: result.token_type,
        expires_in: result.expires_in,
        refresh_token: None,
        id_token: None,
        scope: Some(result.scope),
        issued_token_type: None,
    }))
}
```

---

## 6. RBAC Integration

### 6.1 Role Assignment

Service accounts support direct role assignment (via `role_ids`), consistent with how users have `role_ids`. **No group membership** for service accounts -- groups are an organizational construct for humans.

Role assignment/revocation is done through the admin update endpoint:

```
PUT /api/v1/admin/service-accounts/:sa_id
{ "role_ids": ["role-id-1", "role-id-2"] }
```

### 6.2 Permission Checking

When a service account makes a request, its permissions can be resolved from its `role_ids` using the existing `rbac_helpers` module. The `role_service::get_user_roles()` pattern can be adapted:

```rust
/// Resolve permissions for a service account.
pub async fn resolve_service_account_permissions(
    db: &Database,
    sa_id: &str,
) -> AppResult<Vec<String>> {
    let sa = db
        .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .find_one(doc! { "_id": sa_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Service account not found".to_string()))?;

    if sa.role_ids.is_empty() {
        return Ok(vec![]);
    }

    let roles: Vec<Role> = db
        .collection::<Role>(ROLES)
        .find(doc! { "_id": { "$in": &sa.role_ids } })
        .await?
        .try_collect()
        .await?;

    let permissions: Vec<String> = roles
        .into_iter()
        .flat_map(|r| r.permissions)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    Ok(permissions)
}
```

### 6.3 Scope System

Service account scopes control what API resources are accessible:

| Scope | Access |
|-------|--------|
| `proxy:*` | All proxy endpoints |
| `proxy:{service_id}` | Specific service proxy |
| `llm:proxy` | LLM gateway proxy requests |
| `llm:status` | LLM status endpoint |
| `connections:read` | List connections |
| `connections:write` | Connect/disconnect services |
| `providers:read` | List providers and tokens |
| `providers:write` | Connect to providers |
| `openid` | Standard OIDC scope |
| `profile` | Profile information |

---

## 7. Connection/Provider Access

Service accounts access connections and providers the same way users do -- the `user_id` field in `user_service_connections` and `user_provider_tokens` stores the service account ID.

**No code changes needed** in:
- `connection_service.rs` -- Already takes `user_id: &str`
- `user_token_service.rs` -- Already takes `user_id: &str`
- `proxy_service.rs` -- Reads connection by `user_id`
- `llm_gateway_service.rs` -- Reads provider tokens by `user_id`

The existing handlers in `connections.rs`, `user_tokens.rs`, `proxy.rs`, and `llm_gateway.rs` already extract `auth_user.user_id.to_string()` and pass it to services. Since `AuthUser.user_id` will contain the service account's UUID when authenticated as a service account, everything works transparently.

---

## 8. Audit Logging

All service account operations are audit logged with distinct event types:

| Event Type | Trigger |
|-----------|---------|
| `admin.sa.created` | Service account created |
| `admin.sa.updated` | Service account updated |
| `admin.sa.deleted` | Service account deactivated |
| `admin.sa.secret_rotated` | Client secret rotated |
| `admin.sa.tokens_revoked` | All tokens revoked |
| `sa.token_issued` | Client credentials token issued |
| `sa.token_revoked` | Specific token revoked (via introspection/revoke) |

**event_data** includes relevant context (e.g., `client_id`, `scope`, `target_sa_id`).

**user_id** in audit log entries:
- For admin operations: the admin's user ID
- For `sa.token_issued`: `None` (the `client_id` is in `event_data`)

---

## 9. Security Considerations

### 9.1 Secret Management

- Client secrets are hashed with SHA-256 (same as API keys and OAuth client secrets)
- Raw secret is shown **once** at creation time, never stored or retrievable
- Secret rotation generates a new secret and immediately revokes all outstanding tokens
- Secret prefix (first 8 chars) is stored for UI identification

### 9.2 Token Revocation

- Tokens are tracked in `service_account_tokens` collection with `jti` and `revoked` flag
- Auth middleware checks `revoked` flag on every request (same pattern as refresh tokens)
- TTL index on `expires_at` auto-cleans expired token records
- `revoke_all_tokens()` does a batch `update_many` setting `revoked: true`
- Secret rotation triggers automatic token revocation

### 9.3 Deactivation

- Setting `is_active: false` immediately blocks all authentication
- Auth middleware checks `is_active` on every request
- Existing issued tokens are checked against `is_active` during verification
- Admin delete performs soft-delete (sets `is_active: false`) + revokes all tokens

### 9.4 Rate Limiting

- Service accounts use the global rate limiter by default
- Optional `rate_limit_override` allows per-account rate limit configuration
- Rate limit middleware would need a minor extension to check for service account overrides (future enhancement; not in initial implementation)

### 9.5 Input Validation

- `name`: 1-100 characters, required
- `description`: 0-500 characters, optional
- `allowed_scopes`: validated against known scope set
- `role_ids`: each ID verified to exist in `roles` collection
- `client_id` uniqueness enforced by unique index

### 9.6 Scope Restriction at Token Endpoint

When requesting a token, `requested_scope` must be a subset of the service account's `allowed_scopes`. The existing `oauth_service::validate_scopes()` function can be reused.

### 9.7 No Escalation Path

- Service accounts cannot create other service accounts (admin-only)
- Service accounts cannot modify their own configuration
- Service accounts cannot access admin endpoints
- Service accounts cannot impersonate users (no token exchange / delegation)

---

## 10. Error Handling

### 10.1 New Error Variants

**File:** `backend/src/errors/mod.rs` -- Add:

```rust
#[error("Service account not found: {0}")]
ServiceAccountNotFound(String),

#[error("Service account is inactive")]
ServiceAccountInactive,
```

With corresponding entries in `status_code()`, `error_code()`, and `error_key()`:

| Variant | HTTP Status | Error Code | Error Key |
|---------|------------|------------|-----------|
| `ServiceAccountNotFound` | 404 | 5000 | `service_account_not_found` |
| `ServiceAccountInactive` | 403 | 5001 | `service_account_inactive` |

---

## 11. Frontend Pages

### 11.1 Admin Service Accounts List Page

**File:** `frontend/src/pages/admin-service-accounts.tsx`

- Table listing all service accounts with columns: Name, Client ID (truncated), Status, Scopes, Created, Last Used
- Search bar for filtering by name
- Pagination
- "Create Service Account" button opens dialog
- Row actions: View, Edit, Rotate Secret, Revoke Tokens, Delete

### 11.2 Create Service Account Dialog

**File:** `frontend/src/components/dashboard/create-service-account-dialog.tsx`

- Form fields: Name, Description, Allowed Scopes (multi-select), Role Assignment (multi-select)
- On submit, shows the client_id and client_secret in a one-time copy dialog with warning
- "Copy to Clipboard" buttons for both values
- Clear warning: "Save this secret now. It cannot be retrieved later."

### 11.3 Service Account Detail Page

**File:** `frontend/src/pages/admin-service-account-detail.tsx`

- Header: Name, Status badge, Client ID
- Tabs:
  - **Overview**: Description, scopes, roles, rate limit, creation info, last authenticated
  - **Connections**: Service connections for this SA (reuse connection list component)
  - **Providers**: Provider tokens for this SA (reuse provider token list component)
  - **Audit Log**: Filtered audit log for this SA's client_id
- Actions: Edit, Rotate Secret (with confirmation), Revoke All Tokens (with confirmation), Delete (with confirmation)

### 11.4 Frontend Hooks

**File:** `frontend/src/hooks/use-service-accounts.ts`

TanStack Query hooks:

```typescript
// Query keys
const SA_KEYS = {
  all: ['service-accounts'] as const,
  list: (params: { page: number; search?: string }) =>
    [...SA_KEYS.all, 'list', params] as const,
  detail: (id: string) => [...SA_KEYS.all, id] as const,
};

// Hooks
export function useServiceAccounts(params) { ... }
export function useServiceAccount(id: string) { ... }
export function useCreateServiceAccount() { ... }
export function useUpdateServiceAccount() { ... }
export function useDeleteServiceAccount() { ... }
export function useRotateSecret() { ... }
export function useRevokeTokens() { ... }
```

### 11.5 Frontend Schema

**File:** `frontend/src/schemas/service-account-schema.ts`

```typescript
import { z } from 'zod';

export const createServiceAccountSchema = z.object({
  name: z.string().min(1).max(100),
  description: z.string().max(500).optional(),
  allowed_scopes: z.string().min(1),
  role_ids: z.array(z.string()).optional(),
  rate_limit_override: z.number().int().positive().optional(),
});

export const updateServiceAccountSchema = z.object({
  name: z.string().min(1).max(100).optional(),
  description: z.string().max(500).optional(),
  allowed_scopes: z.string().min(1).optional(),
  role_ids: z.array(z.string()).optional(),
  rate_limit_override: z.number().int().positive().nullable().optional(),
  is_active: z.boolean().optional(),
});
```

### 11.6 Router Integration

**File:** `frontend/src/router.tsx` -- Add routes:

```typescript
// Under admin routes:
'/admin/service-accounts': AdminServiceAccountsPage,
'/admin/service-accounts/$saId': AdminServiceAccountDetailPage,
```

### 11.7 Navigation

Add "Service Accounts" to the admin sidebar navigation, between "Users" and "Roles".

---

## 12. Configuration Changes

### 12.1 New Environment Variable

```bash
# Service account token TTL (default: 3600 = 1 hour)
SA_TOKEN_TTL_SECS=3600
```

**File:** `backend/src/config.rs` -- Add field and parsing:

```rust
pub sa_token_ttl_secs: i64,

// In from_env():
sa_token_ttl_secs: env::var("SA_TOKEN_TTL_SECS")
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(3600),
```

---

## 13. Implementation Phases

### Phase 1: Core Backend (Models + Service + Auth)

1. **Create model files** (`service_account.rs`, `service_account_token.rs`)
   - Register in `models/mod.rs`
   - Add BSON roundtrip tests

2. **Add indexes** to `db.rs`

3. **Add `sa` claim** to JWT `Claims` struct in `crypto/jwt.rs`
   - Add `generate_service_account_token()` function
   - Add tests

4. **Add `is_service_account` field** to `AuthUser` in `mw/auth.rs`
   - Add service account token verification branch
   - Add `reject_service_account_tokens` middleware
   - Update all existing `AuthUser` construction sites to set `is_service_account: false`

5. **Add `sa_token_ttl_secs`** to `AppConfig`

6. **Add error variants** to `errors/mod.rs`

7. **Create `service_account_service.rs`**
   - Register in `services/mod.rs`
   - Implement all functions

### Phase 2: Admin API

8. **Create `admin_service_accounts.rs`** handler
   - Register in `handlers/mod.rs`

9. **Add routes** to `routes.rs`
   - Admin CRUD routes
   - `client_credentials` grant type in oauth token handler

10. **Update route grouping** for service account access control

### Phase 3: Frontend

11. **Create Zod schemas** (`service-account-schema.ts`)
12. **Create TanStack Query hooks** (`use-service-accounts.ts`)
13. **Create admin pages** (list, detail, create dialog)
14. **Add navigation** to admin sidebar
15. **Add routes** to router

### Phase 4: Testing

16. **Backend unit tests** for service_account_service
17. **Backend unit tests** for JWT SA token generation/verification
18. **Backend unit tests** for auth middleware SA path
19. **Frontend schema tests**
20. **Integration tests** (client credentials flow end-to-end)

---

## 14. Files Changed Summary

### New Files

| File | Purpose |
|------|---------|
| `backend/src/models/service_account.rs` | ServiceAccount model |
| `backend/src/models/service_account_token.rs` | ServiceAccountToken model |
| `backend/src/services/service_account_service.rs` | Business logic |
| `backend/src/handlers/admin_service_accounts.rs` | Admin API handlers |
| `frontend/src/pages/admin-service-accounts.tsx` | List page |
| `frontend/src/pages/admin-service-account-detail.tsx` | Detail page |
| `frontend/src/components/dashboard/create-service-account-dialog.tsx` | Create dialog |
| `frontend/src/hooks/use-service-accounts.ts` | TanStack Query hooks |
| `frontend/src/schemas/service-account-schema.ts` | Zod schemas |

### Modified Files

| File | Change |
|------|--------|
| `backend/src/models/mod.rs` | Add module declarations |
| `backend/src/services/mod.rs` | Add module declaration |
| `backend/src/handlers/mod.rs` | Add module declaration |
| `backend/src/crypto/jwt.rs` | Add `sa` claim, `generate_service_account_token()` |
| `backend/src/mw/auth.rs` | Add `is_service_account` to `AuthUser`, SA verification branch, `reject_service_account_tokens` middleware |
| `backend/src/routes.rs` | Add admin SA routes, `client_credentials` grant, route group restructuring |
| `backend/src/handlers/oauth.rs` | Add `client_credentials` match arm in `token()` |
| `backend/src/config.rs` | Add `sa_token_ttl_secs` |
| `backend/src/db.rs` | Add indexes for new collections |
| `backend/src/errors/mod.rs` | Add `ServiceAccountNotFound`, `ServiceAccountInactive` |
| `frontend/src/router.tsx` | Add SA admin routes |
| Admin sidebar component | Add "Service Accounts" nav item |

---

## 15. Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| SA tokens used in user-only endpoints | Medium | `reject_service_account_tokens` middleware on human-only routes |
| Secret leak in logs | High | Never log raw secrets; only log prefixes and hashes |
| Stale token after deactivation | Medium | Auth middleware checks `is_active` + `revoked` on every request |
| Scope escalation via role assignment | Medium | Admin-only role management; validate role_ids exist |
| Token revocation performance | Low | JTI index ensures O(1) lookup; TTL index auto-cleans |
| Breaking existing auth flow | High | SA branch is additive; existing user auth is unchanged; feature flag not needed since `sa: true` only set on SA tokens |

---

## 16. Success Criteria

- [ ] Service accounts can be created, listed, updated, and deleted by admins
- [ ] Client credentials grant issues valid JWT tokens
- [ ] Service account tokens are verified by the auth middleware
- [ ] Service accounts can access proxy, LLM, connections, and provider endpoints
- [ ] Service accounts are blocked from human-only endpoints (auth, users, sessions, admin)
- [ ] Secret rotation works and revokes all existing tokens
- [ ] Token revocation works (individual and bulk)
- [ ] All operations are audit logged
- [ ] Frontend admin pages render and function correctly
- [ ] All backend tests pass with 80%+ coverage for new code
- [ ] No security vulnerabilities in the implementation
