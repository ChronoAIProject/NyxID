# NyxID Developer Guide

This guide covers common development tasks, codebase conventions, and how to extend NyxID with new features.

---

## Table of Contents

- [Local Development Setup](#local-development-setup)
- [Codebase Conventions](#codebase-conventions)
- [Adding a New Endpoint](#adding-a-new-endpoint)
- [Adding an OAuth Provider](#adding-an-oauth-provider)
- [Configuring Identity Propagation](#configuring-identity-propagation)
- [Configuring Credential Delegation](#configuring-credential-delegation)
- [Working with Encrypted Data](#working-with-encrypted-data)
- [Error Handling Patterns](#error-handling-patterns)
- [Testing](#testing)

---

## Local Development Setup

### Prerequisites

| Tool    | Version | Purpose                       |
|---------|---------|-------------------------------|
| Rust    | 1.85+   | Backend compiler (edition 2024)|
| Node.js | 20+     | Frontend build tooling        |
| Docker  | 24+     | MongoDB + Mailpit             |

### Quick Start

```bash
# 1. Start infrastructure
docker compose up -d

# 2. Configure environment
cp .env.example .env
# Replace ENCRYPTION_KEY with: openssl rand -hex 32

# 3. Start backend (auto-generates RSA keys in dev mode)
cargo run --manifest-path backend/Cargo.toml

# 4. Start frontend
cd frontend && npm install && npm run dev

# 5. Create initial admin
curl -X POST http://localhost:3001/api/v1/auth/setup \
  -H "Content-Type: application/json" \
  -d '{"email": "admin@example.com", "password": "securepassword123"}'
```

### Services

| Service  | Port  | Purpose                    |
|----------|-------|----------------------------|
| Backend  | 3001  | Axum HTTP server           |
| Frontend | 3000  | Vite dev server            |
| MongoDB  | 27017 | Database                   |
| Mailpit  | 8025  | Email testing (web UI)     |
| Mailpit  | 1025  | Email testing (SMTP)       |

---

## Codebase Conventions

### Layered Architecture

Dependencies flow strictly downward:

```
handlers/ --> services/ --> models/
                |
                +--> crypto/
```

- **handlers/** -- Parse HTTP requests, call services, return JSON. No business logic.
- **services/** -- Business logic. No HTTP types (`axum::*`). Takes `&mongodb::Database` and string IDs.
- **models/** -- Plain structs with serde. Each has a `COLLECTION_NAME` constant. No logic.
- **crypto/** -- Pure cryptographic operations. No database or HTTP dependencies.
- **mw/** -- Axum middleware (auth extraction, rate limiting, security headers).

### ID Conventions

- All IDs are UUID v4 stored as strings in MongoDB `_id` fields
- Handlers convert `AuthUser.user_id` (Uuid) to string before passing to services
- `crypto/jwt.rs` still takes `&Uuid` for JWT signing (kept for type safety)

### MongoDB Patterns

- Use `#[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]` on `DateTime<Utc>` fields
- Use custom `bson_datetime::optional` helper (in `models/bson_datetime.rs`) for `Option<DateTime<Utc>>`
- Never use `#[serde(skip_serializing)]` on model fields -- it prevents `insert_one` from storing them
- Use `futures::TryStreamExt` for `.try_collect()` on MongoDB cursors
- Batch fetch related documents to avoid N+1 queries (see `delegation_service.rs` for pattern)

### Response Patterns

- Handlers use dedicated response structs -- never serialize model structs to API responses
- Sensitive fields (encrypted data, password hashes) are excluded from response structs
- Timestamps are formatted as RFC 3339 strings in responses

### Admin Endpoint Patterns

Admin endpoints follow a consistent pattern for access control and self-protection:

```rust
pub async fn admin_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(body): Json<ActionRequest>,
) -> AppResult<Json<ActionResponse>> {
    // 1. Verify admin status (DB check, not just JWT claim)
    require_admin(&state, &auth_user).await?;

    // 2. Self-protection check (where applicable)
    let admin_id = auth_user.user_id.to_string();
    if admin_id == user_id {
        return Err(AppError::ValidationError("Cannot modify yourself".to_string()));
    }

    // 3. Call service layer
    admin_user_service::some_action(&state.db, &admin_id, &user_id, ...).await?;

    // 4. Audit log with actor, target, IP, and user-agent
    audit_service::log_async(
        state.db.clone(),
        Some(admin_id),
        "admin.user.action".to_string(),
        Some(serde_json::json!({ "target_user_id": &user_id })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(ActionResponse { ... }))
}
```

### Audit Logging

All significant actions are audit-logged via `audit_service::log_async()`:

```rust
audit_service::log_async(
    state.db.clone(),
    Some(user_id_str),           // acting user
    "action_name".to_string(),   // event type
    Some(serde_json::json!({     // metadata
        "resource_id": &id,
    })),
    None,                        // IP address (optional)
    None,                        // user agent (optional)
);
```

This is fire-and-forget (spawned via `tokio::spawn`). It does not block the request.

---

## Adding a New Endpoint

### Checklist

1. **Model** -- Define or update the MongoDB document struct in `models/`
2. **Service** -- Implement business logic in `services/`
3. **Handler** -- Create request/response types and handler function in `handlers/`
4. **Route** -- Register the route in `routes.rs`
5. **Indexes** -- Add MongoDB indexes in `db::ensure_indexes()` if needed
6. **Audit** -- Add audit logging for significant actions
7. **Tests** -- Write unit tests for service logic

### Example: Adding a Handler

```rust
// handlers/my_feature.rs

#[derive(Debug, Deserialize)]
pub struct CreateFooRequest {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct FooResponse {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

pub async fn create_foo(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateFooRequest>,
) -> AppResult<Json<FooResponse>> {
    // Validation
    if body.name.is_empty() {
        return Err(AppError::ValidationError("name is required".to_string()));
    }

    // Call service layer
    let foo = foo_service::create(&state.db, &body.name).await?;

    // Audit log
    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "foo_created".to_string(),
        Some(serde_json::json!({ "foo_id": &foo.id })),
        None,
        None,
    );

    Ok(Json(FooResponse {
        id: foo.id,
        name: foo.name,
        created_at: foo.created_at.to_rfc3339(),
    }))
}
```

### Register in routes.rs

```rust
let my_routes = Router::new()
    .route("/", post(handlers::my_feature::create_foo));

let api_v1 = Router::new()
    // ... existing routes
    .nest("/foos", my_routes);
```

---

## Adding an OAuth Provider

To add a new OAuth2 provider to the credential broker:

### 1. Register the Provider (Admin API)

```bash
curl -X POST http://localhost:3001/api/v1/providers \
  -H "Authorization: Bearer <admin_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "New Provider",
    "slug": "new-provider",
    "provider_type": "oauth2",
    "authorization_url": "https://provider.com/oauth/authorize",
    "token_url": "https://provider.com/oauth/token",
    "revocation_url": "https://provider.com/oauth/revoke",
    "default_scopes": ["read", "write"],
    "client_id": "your-client-id",
    "client_secret": "your-client-secret",
    "supports_pkce": true,
    "documentation_url": "https://provider.com/docs/auth"
  }'
```

### 2. Register Redirect URI

In the provider's developer console, register:

```
https://your-nyxid-url/api/v1/providers/callback
```

This is the generic callback endpoint used by all OAuth providers.

### 3. Provider Requirements

- `client_id` and `client_secret` are encrypted with AES-256-GCM before storage
- `authorization_url` and `token_url` are validated against SSRF blocklists
- `slug` must be unique, lowercase alphanumeric with hyphens, no leading/trailing/consecutive hyphens

### 4. User OAuth Flow

1. Frontend calls `GET /api/v1/providers/{id}/connect/oauth`
2. Backend creates an `oauth_states` record with PKCE code verifier
3. Backend returns the authorization URL
4. Frontend redirects user to the provider
5. Provider redirects to `GET /api/v1/providers/callback?code=...&state=...`
6. Backend validates state, exchanges code for tokens, encrypts and stores them
7. Backend redirects to `{FRONTEND_URL}/providers/callback?status=success`

---

## Configuring Identity Propagation

Identity propagation forwards the authenticated NyxID user's identity to downstream services.

### Enable via Service Update

```bash
curl -X PUT http://localhost:3001/api/v1/services/<service_id> \
  -H "Authorization: Bearer <admin_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "identity_propagation_mode": "both",
    "identity_include_user_id": true,
    "identity_include_email": true,
    "identity_include_name": true,
    "identity_jwt_audience": "https://my-service.example.com"
  }'
```

### Modes

| Mode      | What Gets Added                                                |
|-----------|----------------------------------------------------------------|
| `none`    | Nothing (default)                                              |
| `headers` | `X-NyxID-User-Id`, `X-NyxID-User-Email`, `X-NyxID-User-Name` |
| `jwt`     | `X-NyxID-Identity-Token` (RS256-signed, 60s TTL)              |
| `both`    | All of the above                                               |

### Verifying Identity JWTs

Downstream services can verify the identity JWT using NyxID's JWKS:

```bash
# Fetch the public key
curl https://auth.example.com/.well-known/jwks.json
```

The JWT `aud` claim is set to the service's `identity_jwt_audience` (or `base_url` if not configured).

---

## Configuring Credential Delegation

Credential delegation injects user provider tokens into proxy requests.

### 1. Create Provider Configurations

Register the providers your users will connect to (see [Adding an OAuth Provider](#adding-an-oauth-provider)).

### 2. Add Service Requirements

```bash
curl -X POST http://localhost:3001/api/v1/services/<service_id>/requirements \
  -H "Authorization: Bearer <admin_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "provider_config_id": "<provider_id>",
    "required": true,
    "injection_method": "bearer",
    "injection_key": "Authorization"
  }'
```

### 3. How Injection Works

When a proxy request is made:

1. The proxy loads all requirements for the service
2. For each requirement, it fetches the user's active token (triggering lazy refresh for OAuth)
3. Tokens are injected based on `injection_method`:
   - **bearer**: `{injection_key}: Bearer {token}`
   - **header**: `{injection_key}: {token}`
   - **query**: `?{injection_key}={token}` appended to URL

### Required vs. Optional

- `required: true` -- Proxy returns 400 if the user has no active token for this provider
- `required: false` -- Token is silently omitted if unavailable

### Blocked Injection Keys

For security, these header names cannot be used as `injection_key`: `host`, `authorization`, `cookie`, `set-cookie`, `transfer-encoding`, `content-length`, `connection`, `x-forwarded-for`, `x-forwarded-host`, `x-real-ip`.

---

## Working with Encrypted Data

### Encrypting a Value

```rust
use crate::crypto::aes;

let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
let encrypted = aes::encrypt(plaintext.as_bytes(), &encryption_key)?;
// Store `encrypted` (Vec<u8>) in MongoDB
```

### Decrypting a Value

```rust
let decrypted_bytes = aes::decrypt(&encrypted, &encryption_key)?;
let plaintext = String::from_utf8(decrypted_bytes)
    .map_err(|_| AppError::Internal("Invalid UTF-8".to_string()))?;
```

### Important Notes

- The `ENCRYPTION_KEY` is a 32-byte key provided as 64 hex characters
- Each encryption generates a random 96-bit nonce
- Stored format: `nonce(12 bytes) || ciphertext || tag(16 bytes)`
- Use `zeroize` crate's `Zeroizing` wrapper for decrypted secrets to ensure memory cleanup

---

## Error Handling Patterns

### AppError Enum

All errors are represented as `AppError` variants:

| Variant             | HTTP Status | Error Code | When to Use                       |
|---------------------|-------------|------------|-----------------------------------|
| `BadRequest`        | 400         | 1000       | Malformed request, invalid state  |
| `Unauthorized`      | 401         | 1001       | Missing or invalid credentials    |
| `Forbidden`         | 403         | 1002       | Insufficient permissions          |
| `NotFound`          | 404         | 1003       | Resource does not exist           |
| `Conflict`          | 409         | 1004       | Duplicate resource                |
| `ValidationError`   | 400         | 1008       | Input validation failure          |
| `Internal`          | 500         | 1006       | Server error (details redacted)   |

### Usage

```rust
// Validation
if name.is_empty() {
    return Err(AppError::ValidationError("name is required".to_string()));
}

// Not found
let item = collection.find_one(filter).await?
    .ok_or_else(|| AppError::NotFound("Item not found".to_string()))?;

// Admin check
if !user.is_admin {
    return Err(AppError::Forbidden("Admin access required".to_string()));
}
```

Internal error details are never exposed in API responses.

---

## Testing

### Running Tests

```bash
# Backend unit tests
cargo test --manifest-path backend/Cargo.toml

# Frontend lint
cd frontend && npm run lint

# Frontend type check
cd frontend && npx tsc --noEmit
```

### Test Patterns

- Service layer functions are the primary unit test target
- Use mock MongoDB databases for integration tests
- Test error paths (not found, validation, unauthorized) alongside happy paths
- Verify audit events are emitted for significant actions

### Debug Logging

```bash
# Enable debug logging for specific modules
RUST_LOG=nyxid::services::user_token_service=debug,nyxid=info cargo run --manifest-path backend/Cargo.toml
```
