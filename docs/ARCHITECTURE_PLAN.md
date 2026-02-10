# Architecture Plan: Credential Broker, Identity Propagation & User OAuth

## Overview

This document describes the architecture for extending NyxID with:

1. **User-Level Provider Connections** -- Users connect their own OAuth providers or API keys (OpenAI, Anthropic, Google AI, etc.). NyxID stores tokens encrypted, manages refresh.
2. **Credential Delegation** -- Downstream services declare which providers they need. When proxying, NyxID injects the user's provider token. Downstream services never see raw credentials.
3. **User Identity Propagation** -- When NyxID proxies requests, it injects user identity via headers (`X-NyxID-User-Id`, etc.) or a signed JWT assertion.
4. **SSO-Integrated Access** -- Users authenticated to NyxID seamlessly access downstream services that use NyxID as their OIDC provider.

### Research Findings (see `docs/research-credential-broker.md`)

Key takeaways that shaped this architecture:

- **6 of 8 LLM provider scenarios use static API keys** (OpenAI, Anthropic, Google AI Studio, Mistral, Cohere, Azure key mode). Only Google Vertex AI and Azure Entra ID require OAuth2 with refresh tokens. This means Phase 1 (API key broker) covers the vast majority of use cases.
- **Lazy token refresh preferred** over background jobs -- simpler, sufficient since most providers use non-expiring API keys.
- **`oauth2` crate 4.x** is the recommended Rust OAuth2 client library for Phase 2 OAuth flows (Vertex AI, Azure Entra). It supports Authorization Code + PKCE, device code, and is extensible for RFC 8693.
- **Identity headers first, RFC 8693 later** -- start with `X-NyxID-*` headers for proxy identity propagation. Full token exchange (RFC 8693) is a future enhancement for advanced delegation.
- **`zeroize` crate** recommended for zeroing credential memory after use.
- **Credential validation on connect** -- test API keys against provider APIs during connection.
- **MCP Nov 2025 spec** requires OAuth 2.1 + PKCE, Resource Indicators (RFC 8707), Protected Resource Metadata (RFC 9728). NyxID's existing OIDC infra covers most of this.

---

## Current State

### Existing Collections (10 + 1 in-memory)

| Collection | Purpose |
|---|---|
| `users` | User accounts (email, password hash, MFA, admin flag) |
| `sessions` | Session tokens with TTL expiry |
| `authorization_codes` | OAuth auth codes with PKCE |
| `refresh_tokens` | JWT refresh tokens with rotation |
| `api_keys` | User-issued API keys (X-API-Key) |
| `mfa_factors` | TOTP MFA secrets |
| `downstream_services` | Registered services (provider/connection/internal) |
| `user_service_connections` | Per-user encrypted credentials for services |
| `oauth_clients` | OIDC clients (auto-provisioned for provider services) |
| `service_endpoints` | API endpoint metadata per service |
| `mcp_sessions` | In-memory MCP session store (not MongoDB) |

### Existing Proxy Flow

1. User authenticates via session cookie / Bearer token / API key
2. `ANY /api/v1/proxy/{service_id}/{*path}` hits `handlers::proxy::proxy_request`
3. `proxy_service::resolve_proxy_target()` loads service + user connection
4. Credential decrypted (AES-256-GCM) and injected via auth method (header/bearer/query/basic)
5. Request forwarded; response streamed back

### Existing Service Categories

| Category | Description | User Credential | Proxy |
|---|---|---|---|
| `provider` | OIDC services where NyxID is IdP | N/A | Not proxyable |
| `connection` | External services with per-user credentials | Required | User's credential |
| `internal` | Internal services with master credential | None | Master credential |

### Key Gaps

- No user OAuth token storage (only static API key / basic auth credentials)
- No external OAuth provider registry
- No token refresh mechanism
- No identity propagation (proxy does not convey user context to downstream)
- No credential delegation model (services can't declare provider dependencies)
- No OAuth connection UI (only manual credential entry via `CredentialDialog`)

---

## New Collections

### 1. `provider_configs` -- External Provider Registry

Admin-managed registry of external providers (OAuth2 or API key) that NyxID can broker credentials for.

```rust
pub const COLLECTION_NAME: &str = "provider_configs";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(rename = "_id")]
    pub id: String,                           // UUID v4
    pub slug: String,                         // unique: "openai", "anthropic", "google-ai"
    pub name: String,                         // "OpenAI"
    pub description: Option<String>,

    /// "oauth2" | "api_key"
    pub provider_type: String,

    // --- OAuth2 fields (None for api_key providers) ---
    pub authorization_url: Option<String>,    // e.g. "https://accounts.google.com/o/oauth2/v2/auth"
    pub token_url: Option<String>,            // e.g. "https://oauth2.googleapis.com/token"
    pub revocation_url: Option<String>,
    pub default_scopes: Option<Vec<String>>,  // scopes to request by default
    /// NyxID's OAuth client_id for this provider (encrypted)
    pub client_id_encrypted: Option<Vec<u8>>,
    /// NyxID's OAuth client_secret for this provider (encrypted)
    pub client_secret_encrypted: Option<Vec<u8>>,
    /// Whether this provider supports PKCE
    #[serde(default)]
    pub supports_pkce: bool,

    // --- API key fields ---
    pub api_key_instructions: Option<String>, // "Get your key at https://..."
    pub api_key_url: Option<String>,          // Direct link to key management page

    // --- Display ---
    pub icon_url: Option<String>,
    pub documentation_url: Option<String>,

    // --- Validation ---
    /// URL to call for API key validation (e.g., "https://api.openai.com/v1/models")
    pub validation_url: Option<String>,
    /// HTTP method for validation ("GET" or "POST")
    #[serde(default = "default_get")]
    pub validation_method: String,
    /// Header name for the API key during validation (e.g., "Authorization", "x-api-key")
    pub validation_auth_header: Option<String>,
    /// Format string for the auth value. Use `{key}` as placeholder.
    /// e.g., "Bearer {key}" for OpenAI, "{key}" for Anthropic x-api-key
    pub validation_auth_format: Option<String>,

    pub is_active: bool,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
```

**Indexes:**
- `slug: 1` (unique)
- `provider_type: 1, is_active: 1`

### 2. `user_provider_tokens` -- Per-User Provider Tokens

Stores each user's encrypted credentials for external providers.

```rust
pub const COLLECTION_NAME: &str = "user_provider_tokens";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserProviderToken {
    #[serde(rename = "_id")]
    pub id: String,                              // UUID v4
    pub user_id: String,
    pub provider_config_id: String,

    /// "oauth2" | "api_key"
    pub token_type: String,

    // --- OAuth2 tokens (encrypted) ---
    pub access_token_encrypted: Option<Vec<u8>>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub token_scopes: Option<String>,            // space-separated granted scopes
    #[serde(default, with = "bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,       // access token expiry

    // --- API key (encrypted) ---
    pub api_key_encrypted: Option<Vec<u8>>,

    // --- Status ---
    /// "active" | "expired" | "revoked" | "refresh_failed"
    pub status: String,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_refreshed_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_used_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,           // last refresh error

    // --- User metadata ---
    pub label: Option<String>,                   // "Production Key", "Personal Account"

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
```

**Indexes:**
- `user_id: 1, provider_config_id: 1` (unique -- one connection per user per provider)
- `user_id: 1, status: 1`
- `status: 1, expires_at: 1` (for background refresh queries)

### 3. `service_provider_requirements` -- Delegation Mapping

Links downstream services to the providers they need, enabling credential delegation.

```rust
pub const COLLECTION_NAME: &str = "service_provider_requirements";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceProviderRequirement {
    #[serde(rename = "_id")]
    pub id: String,                     // UUID v4
    pub service_id: String,             // refs downstream_services._id
    pub provider_config_id: String,     // refs provider_configs._id
    /// Whether this provider is required (vs optional) to use the service
    pub required: bool,
    /// Specific scopes this service needs from the provider (subset of granted scopes)
    pub scopes: Option<Vec<String>>,
    /// How to inject the provider token: "bearer" | "header" | "query"
    pub injection_method: String,
    /// Header name or query param name (e.g., "Authorization", "X-API-Key", "api_key")
    pub injection_key: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
```

**Indexes:**
- `service_id: 1, provider_config_id: 1` (unique)
- `service_id: 1`
- `provider_config_id: 1`

---

## Modified Collections

### `downstream_services` -- Add Identity Propagation Config

Add new optional fields to the existing `DownstreamService` struct:

```rust
// New fields to add:
/// Identity propagation mode: "none" | "headers" | "jwt" | "both"
#[serde(default = "default_identity_propagation_mode")]
pub identity_propagation_mode: String,

/// Which identity fields to include when propagating
#[serde(default)]
pub identity_include_user_id: bool,
#[serde(default)]
pub identity_include_email: bool,
#[serde(default)]
pub identity_include_name: bool,

/// Custom JWT audience for identity assertions (defaults to service base_url)
#[serde(default, skip_serializing_if = "Option::is_none")]
pub identity_jwt_audience: Option<String>,

fn default_identity_propagation_mode() -> String {
    "none".to_string()
}
```

**Rationale:** Flat fields (not nested struct) because MongoDB `$set` operations are simpler with dot notation and existing update patterns in the codebase use flat `doc!` updates.

---

## New Backend Services

### `provider_service.rs` -- Provider Registry CRUD

Admin-only service for managing the provider registry.

```rust
/// Create a new provider configuration. Admin only.
pub async fn create_provider(
    db: &mongodb::Database,
    encryption_key: &[u8],
    name: &str,
    slug: &str,
    provider_type: &str,
    oauth_config: Option<OAuthProviderInput>,
    api_key_config: Option<ApiKeyProviderInput>,
    created_by: &str,
) -> AppResult<ProviderConfig>;

/// List all active providers (visible to all authenticated users).
pub async fn list_providers(
    db: &mongodb::Database,
) -> AppResult<Vec<ProviderConfig>>;

/// Get a single provider by ID.
pub async fn get_provider(
    db: &mongodb::Database,
    provider_id: &str,
) -> AppResult<ProviderConfig>;

/// Update provider configuration. Admin only.
pub async fn update_provider(
    db: &mongodb::Database,
    encryption_key: &[u8],
    provider_id: &str,
    updates: ProviderUpdateInput,
) -> AppResult<ProviderConfig>;

/// Soft-delete a provider. Admin only.
/// Also revokes all user tokens for this provider.
pub async fn delete_provider(
    db: &mongodb::Database,
    provider_id: &str,
) -> AppResult<()>;
```

### `user_token_service.rs` -- User Token Management

Core service for user-level provider credential management.

```rust
/// Store an API key for a provider.
/// Optionally validates the key against the provider's API before storing.
pub async fn store_api_key(
    db: &mongodb::Database,
    encryption_key: &[u8],
    http_client: &reqwest::Client,
    user_id: &str,
    provider_id: &str,
    api_key: &str,
    label: Option<&str>,
    validate: bool,
) -> AppResult<UserProviderToken>;

/// Validate an API key against a provider's API.
/// Calls a lightweight endpoint (e.g., OpenAI /models, Anthropic /messages with empty body)
/// to verify the key works. Returns Ok(()) if valid, Err if the key is rejected.
pub async fn validate_api_key(
    http_client: &reqwest::Client,
    provider: &ProviderConfig,
    api_key: &str,
) -> AppResult<()>;

/// Initiate an OAuth2 connection flow.
/// Returns the authorization URL the user should be redirected to.
pub async fn initiate_oauth_connect(
    db: &mongodb::Database,
    encryption_key: &[u8],
    config: &AppConfig,
    user_id: &str,
    provider_id: &str,
) -> AppResult<String>;

/// Handle the OAuth2 callback after user authorizes.
/// Exchanges the authorization code for tokens and stores them encrypted.
pub async fn handle_oauth_callback(
    db: &mongodb::Database,
    encryption_key: &[u8],
    http_client: &reqwest::Client,
    user_id: &str,
    provider_id: &str,
    code: &str,
    state: &str,
) -> AppResult<UserProviderToken>;

/// Get a user's decrypted token for a provider.
/// Performs lazy refresh if the access token is expired and a refresh token exists.
pub async fn get_active_token(
    db: &mongodb::Database,
    encryption_key: &[u8],
    http_client: &reqwest::Client,
    user_id: &str,
    provider_id: &str,
) -> AppResult<DecryptedProviderToken>;

/// Refresh an OAuth2 access token using the stored refresh token.
pub async fn refresh_oauth_token(
    db: &mongodb::Database,
    encryption_key: &[u8],
    http_client: &reqwest::Client,
    user_id: &str,
    provider_id: &str,
) -> AppResult<()>;

/// Revoke and delete a user's stored token for a provider.
pub async fn disconnect_provider(
    db: &mongodb::Database,
    user_id: &str,
    provider_id: &str,
) -> AppResult<()>;

/// List all providers the user has connected to, with status.
pub async fn list_user_tokens(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<UserProviderTokenSummary>>;
```

**Token Types:**

```rust
/// Decrypted token ready for injection.
pub struct DecryptedProviderToken {
    pub token_type: String,       // "oauth2" | "api_key"
    pub access_token: Option<String>,
    pub api_key: Option<String>,
}

/// Summary for listing (no decrypted tokens).
pub struct UserProviderTokenSummary {
    pub provider_config_id: String,
    pub provider_name: String,
    pub provider_slug: String,
    pub token_type: String,
    pub status: String,
    pub label: Option<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub connected_at: String,
}
```

### `identity_service.rs` -- Identity Propagation

Service for building identity headers and JWT assertions.

```rust
/// Build identity headers for a proxied request.
/// Returns a Vec of (header_name, header_value) pairs based on service config.
pub fn build_identity_headers(
    user: &User,
    service: &DownstreamService,
) -> Vec<(String, String)>;

/// Generate a short-lived signed JWT identity assertion.
/// Used when service.identity_propagation_mode is "jwt" or "both".
pub fn generate_identity_assertion(
    jwt_keys: &JwtKeys,
    config: &AppConfig,
    user: &User,
    service: &DownstreamService,
) -> AppResult<String>;
```

**Identity Headers (when mode is "headers" or "both"):**

| Header | Value | Condition |
|---|---|---|
| `X-NyxID-User-Id` | User UUID | `identity_include_user_id` |
| `X-NyxID-User-Email` | User email | `identity_include_email` |
| `X-NyxID-User-Name` | User display name | `identity_include_name` |
| `X-NyxID-Identity-Token` | Short-lived JWT | mode is "jwt" or "both" |

**Identity JWT Claims:**

```rust
pub struct IdentityAssertionClaims {
    pub sub: String,                // user_id
    pub iss: String,                // NyxID issuer
    pub aud: String,                // service base_url or custom audience
    pub exp: i64,                   // now + 60 seconds
    pub iat: i64,
    pub jti: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub nyx_service_id: String,     // which downstream service this is for
}
```

### `delegation_service.rs` -- Credential Delegation

Service for resolving provider tokens during proxy requests.

```rust
/// Resolve all provider tokens needed for a downstream service.
/// Returns a Vec of (injection_method, injection_key, token_value) tuples.
pub async fn resolve_delegated_credentials(
    db: &mongodb::Database,
    encryption_key: &[u8],
    http_client: &reqwest::Client,
    user_id: &str,
    service_id: &str,
) -> AppResult<Vec<DelegatedCredential>>;

pub struct DelegatedCredential {
    pub provider_slug: String,
    pub injection_method: String,   // "bearer" | "header" | "query"
    pub injection_key: String,      // header name or query param
    pub credential: String,         // decrypted token
}
```

---

## Modified Backend Services

### `proxy_service.rs` -- Enhanced Proxy

The proxy service gets two new capabilities: identity propagation and credential delegation.

**Modified `forward_request()` signature:**

```rust
pub async fn forward_request(
    client: &Client,
    target: &ProxyTarget,
    method: reqwest::Method,
    path: &str,
    query: Option<&str>,
    headers: reqwest::header::HeaderMap,
    body: Option<bytes::Bytes>,
    identity_headers: Vec<(String, String)>,           // NEW
    delegated_credentials: Vec<DelegatedCredential>,   // NEW
) -> AppResult<reqwest::Response>;
```

**Changes to `forward_request()`:**

1. After copying allowed headers, inject identity headers:
   ```rust
   for (name, value) in &identity_headers {
       request = request.header(name, value);
   }
   ```

2. After injecting the service credential (existing logic), inject delegated provider credentials:
   ```rust
   for cred in &delegated_credentials {
       match cred.injection_method.as_str() {
           "bearer" => { request = request.header(&cred.injection_key, format!("Bearer {}", cred.credential)); }
           "header" => { request = request.header(&cred.injection_key, &cred.credential); }
           "query" => { request = request.query(&[(&cred.injection_key, &cred.credential)]); }
           _ => {}
       }
   }
   ```

**Modified `ALLOWED_FORWARD_HEADERS`:** No change needed -- identity and delegation headers are injected by NyxID, not forwarded from the user's request.

### `proxy handler` -- Enhanced Proxy Handler

**Changes to `handlers/proxy.rs::proxy_request()`:**

```rust
// After resolving proxy target, resolve identity and delegation:

// 1. Fetch user for identity propagation
let user = db.collection::<User>(USERS)
    .find_one(doc! { "_id": &user_id_str })
    .await?;

// 2. Build identity headers (if configured on the service)
let identity_headers = if let Some(user) = &user {
    identity_service::build_identity_headers(user, &service)
} else {
    vec![]
};

// 3. If service has identity JWT mode, generate assertion
if matches!(service.identity_propagation_mode.as_str(), "jwt" | "both") {
    if let Some(user) = &user {
        let assertion = identity_service::generate_identity_assertion(
            &state.jwt_keys, &state.config, user, &service,
        )?;
        // Add to identity_headers
    }
}

// 4. Resolve delegated credentials
let delegated = delegation_service::resolve_delegated_credentials(
    &state.db, &encryption_key, &state.http_client, &user_id_str, &service_id,
).await.unwrap_or_default(); // Non-fatal: proceed without delegation on error

// 5. Forward with identity + delegation
proxy_service::forward_request(
    &state.http_client, &target, method, &path, query,
    headers, body, identity_headers, delegated,
).await?;
```

---

## New Backend Handlers

### `handlers/providers.rs` -- Admin Provider Management

```
GET    /api/v1/providers                          -- list_providers (authenticated)
POST   /api/v1/providers                          -- create_provider (admin)
GET    /api/v1/providers/{provider_id}             -- get_provider (authenticated)
PUT    /api/v1/providers/{provider_id}             -- update_provider (admin)
DELETE /api/v1/providers/{provider_id}             -- delete_provider (admin)
```

**Request/Response types:**

```rust
#[derive(Deserialize)]
pub struct CreateProviderRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub provider_type: String,           // "oauth2" | "api_key"
    // OAuth2 fields:
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub revocation_url: Option<String>,
    pub default_scopes: Option<Vec<String>>,
    pub client_id: Option<String>,       // plaintext, encrypted on storage
    pub client_secret: Option<String>,   // plaintext, encrypted on storage
    pub supports_pkce: Option<bool>,
    // API key fields:
    pub api_key_instructions: Option<String>,
    pub api_key_url: Option<String>,
    // Display:
    pub icon_url: Option<String>,
    pub documentation_url: Option<String>,
}

#[derive(Serialize)]
pub struct ProviderResponse {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub provider_type: String,
    pub has_oauth_config: bool,          // true if OAuth fields are set
    pub default_scopes: Option<Vec<String>>,
    pub supports_pkce: bool,
    pub api_key_instructions: Option<String>,
    pub api_key_url: Option<String>,
    pub icon_url: Option<String>,
    pub documentation_url: Option<String>,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}
```

### `handlers/user_tokens.rs` -- User Token Management

```
GET    /api/v1/providers/my-tokens                      -- list_my_tokens
POST   /api/v1/providers/{provider_id}/connect/api-key  -- connect_api_key
GET    /api/v1/providers/{provider_id}/connect/oauth     -- initiate_oauth_connect
GET    /api/v1/providers/{provider_id}/callback          -- oauth_callback
DELETE /api/v1/providers/{provider_id}/disconnect        -- disconnect_provider
POST   /api/v1/providers/{provider_id}/refresh           -- manual_refresh
```

**Request/Response types:**

```rust
#[derive(Deserialize)]
pub struct ConnectApiKeyRequest {
    pub api_key: String,
    pub label: Option<String>,
}

#[derive(Serialize)]
pub struct UserTokenResponse {
    pub provider_id: String,
    pub provider_name: String,
    pub provider_slug: String,
    pub provider_type: String,
    pub status: String,
    pub label: Option<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub connected_at: String,
}

#[derive(Serialize)]
pub struct UserTokenListResponse {
    pub tokens: Vec<UserTokenResponse>,
}

#[derive(Serialize)]
pub struct OAuthInitiateResponse {
    pub authorization_url: String,
}
```

### `handlers/service_requirements.rs` -- Service Provider Requirements (Admin)

```
GET    /api/v1/services/{service_id}/requirements              -- list_requirements
POST   /api/v1/services/{service_id}/requirements              -- add_requirement (admin)
DELETE /api/v1/services/{service_id}/requirements/{req_id}     -- remove_requirement (admin)
```

---

## Route Changes

### `routes.rs` -- New Route Groups

```rust
let provider_routes = Router::new()
    .route("/", get(handlers::providers::list_providers))
    .route("/", post(handlers::providers::create_provider))
    .route("/my-tokens", get(handlers::user_tokens::list_my_tokens))
    .route("/{provider_id}", get(handlers::providers::get_provider))
    .route("/{provider_id}", put(handlers::providers::update_provider))
    .route("/{provider_id}", delete(handlers::providers::delete_provider))
    .route(
        "/{provider_id}/connect/api-key",
        post(handlers::user_tokens::connect_api_key),
    )
    .route(
        "/{provider_id}/connect/oauth",
        get(handlers::user_tokens::initiate_oauth_connect),
    )
    .route(
        "/{provider_id}/callback",
        get(handlers::user_tokens::oauth_callback),
    )
    .route(
        "/{provider_id}/disconnect",
        delete(handlers::user_tokens::disconnect_provider),
    )
    .route(
        "/{provider_id}/refresh",
        post(handlers::user_tokens::manual_refresh),
    );

let requirement_routes = Router::new()
    .route("/", get(handlers::service_requirements::list_requirements))
    .route("/", post(handlers::service_requirements::add_requirement))
    .route(
        "/{requirement_id}",
        delete(handlers::service_requirements::remove_requirement),
    );

// Add to service_routes:
service_routes = service_routes
    .route("/{service_id}/requirements", /* nest requirement_routes */);

// Add to api_v1:
let api_v1 = Router::new()
    // ... existing nests ...
    .nest("/providers", provider_routes);
```

---

## OAuth State Management

For OAuth2 provider connections, we need temporary state to prevent CSRF. Use a new in-memory store (like `McpSessionStore`) or a short-lived MongoDB collection.

**Recommended: Short-lived MongoDB document (reuse `authorization_codes` pattern).**

### `oauth_states` collection (new, with TTL)

```rust
pub const COLLECTION_NAME: &str = "oauth_states";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OAuthState {
    #[serde(rename = "_id")]
    pub id: String,                // UUID v4 (also used as the `state` param)
    pub user_id: String,
    pub provider_config_id: String,
    pub code_verifier: Option<String>,  // PKCE verifier (stored server-side)
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,      // now + 10 minutes
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
```

**Index:** `expires_at: 1` with TTL (auto-cleanup).

---

## Token Refresh Strategy

### Lazy Refresh (Primary)

When `get_active_token()` is called (during proxy or explicit request):

1. Check `expires_at` on the stored token
2. If expired (or within 5-minute buffer), attempt refresh
3. If refresh succeeds, update stored token and return new access token
4. If refresh fails, update `status` to `"refresh_failed"`, set `error_message`
5. Return error to caller (proxy request fails gracefully)

### Background Refresh (Optional, Phase 3)

A tokio background task that periodically scans for tokens expiring soon:

```rust
// In main.rs startup:
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(300)); // 5 min
    loop {
        interval.tick().await;
        user_token_service::refresh_expiring_tokens(&db, &encryption_key, &http_client).await;
    }
});
```

Query: `{ status: "active", expires_at: { $lt: now + 10_minutes } }`

This is non-critical and can be deferred to Phase 3.

---

## Frontend Changes

### New Types (`types/api.ts`)

```typescript
export interface ProviderConfig {
  readonly id: string;
  readonly slug: string;
  readonly name: string;
  readonly description: string | null;
  readonly provider_type: "oauth2" | "api_key";
  readonly has_oauth_config: boolean;
  readonly default_scopes: readonly string[] | null;
  readonly supports_pkce: boolean;
  readonly api_key_instructions: string | null;
  readonly api_key_url: string | null;
  readonly icon_url: string | null;
  readonly documentation_url: string | null;
  readonly is_active: boolean;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface UserProviderToken {
  readonly provider_id: string;
  readonly provider_name: string;
  readonly provider_slug: string;
  readonly provider_type: string;
  readonly status: "active" | "expired" | "revoked" | "refresh_failed";
  readonly label: string | null;
  readonly expires_at: string | null;
  readonly last_used_at: string | null;
  readonly connected_at: string;
}

export interface ServiceProviderRequirement {
  readonly id: string;
  readonly service_id: string;
  readonly provider_config_id: string;
  readonly provider_name: string;
  readonly required: boolean;
  readonly scopes: readonly string[] | null;
  readonly injection_method: string;
  readonly injection_key: string | null;
}
```

### New Hooks (`hooks/use-providers.ts`)

```typescript
// Provider config CRUD (admin)
export function useProviders(): UseQueryResult<readonly ProviderConfig[]>;
export function useProvider(id: string): UseQueryResult<ProviderConfig>;
export function useCreateProvider(): UseMutationResult;
export function useUpdateProvider(): UseMutationResult;
export function useDeleteProvider(): UseMutationResult;

// User token management
export function useMyProviderTokens(): UseQueryResult<readonly UserProviderToken[]>;
export function useConnectApiKey(): UseMutationResult;
export function useInitiateOAuth(providerId: string): UseMutationResult;
export function useDisconnectProvider(): UseMutationResult;
export function useRefreshProviderToken(): UseMutationResult;
```

### New Pages

#### `/providers` -- Provider Connections Page

Shows all available providers in a grid. Each card shows:
- Provider name, icon, description
- Connection status (connected/disconnected/expired/error)
- Connect button (opens API key dialog or starts OAuth flow)
- Disconnect button (for connected providers)
- Refresh button (for expired OAuth tokens)

#### `/providers/callback` -- OAuth Callback Page

Minimal page that:
1. Extracts `code` and `state` from URL query params
2. Calls `GET /api/v1/providers/{provider_id}/callback?code=...&state=...`
3. Shows success/error, redirects to `/providers`

### New Components

| Component | Purpose |
|---|---|
| `provider-grid.tsx` | Grid of provider cards with connect/disconnect actions |
| `provider-card.tsx` | Individual provider card with status badge |
| `provider-api-key-dialog.tsx` | Dialog for entering API key (reuse pattern from `credential-dialog.tsx`) |
| `provider-oauth-redirect.tsx` | Handles OAuth redirect and callback |
| `provider-status-badge.tsx` | Badge showing token status (active/expired/error) |
| `service-requirements-editor.tsx` | Admin UI for managing service provider requirements |
| `identity-propagation-config.tsx` | Admin UI for configuring identity propagation per service |

### Modified Components

| Component | Change |
|---|---|
| `sidebar.tsx` | Add "Providers" nav item (Plug icon from lucide-react) |
| `service-edit.tsx` | Add identity propagation config section |
| `service-detail.tsx` | Show identity propagation settings and provider requirements |

### New Router Routes

```typescript
// In router.tsx:
const providersRoute = createRoute({
  path: "/providers",
  getParentRoute: () => dashboardLayout,
  component: ProvidersPage,
});

const providerCallbackRoute = createRoute({
  path: "/providers/callback",
  getParentRoute: () => dashboardLayout,
  component: ProviderCallbackPage,
});

// Add to routeTree:
dashboardLayout.addChildren([
  // ... existing routes ...
  providersRoute,
  providerCallbackRoute,
]);
```

---

## Security Considerations

### Token Encryption
All provider tokens (access tokens, refresh tokens, API keys) are encrypted at rest using AES-256-GCM with the same `ENCRYPTION_KEY` used for existing credentials. The encryption produces `nonce || ciphertext || tag` (existing `crypto/aes.rs` pattern).

**Future enhancement (post-MVP):** Add key version prefix to ciphertexts for rotation support:
```
key_version (1 byte) || nonce (12 bytes) || ciphertext || tag
```
This allows rotating encryption keys without re-encrypting all stored tokens atomically.

### Memory Protection
Use the `zeroize` crate to zero credential memory after use in proxy and token service code:
```rust
use zeroize::Zeroize;
let mut credential = decrypt_token(...);
// ... inject into request ...
credential.zeroize();
```

### Credential Validation on Connect
When a user stores an API key, optionally validate it against the provider's API before persisting. Each provider config can specify a `validation_url` (e.g., OpenAI's `GET /v1/models`, Anthropic's health check) to test the key. Validation failures return a clear error to the user without storing the invalid key.

### OAuth State CSRF Protection
OAuth state parameter is a random UUID stored server-side with a 10-minute TTL. The callback verifies the state matches and has not expired before exchanging the code. One-time use: the state document is atomically claimed (same pattern as authorization codes).

### PKCE for OAuth Flows
When the provider supports PKCE, the code verifier is generated server-side, stored in the `oauth_states` document, and used during token exchange. The code challenge (SHA-256) is sent to the provider.

### Scoped Delegation
Downstream services can only access provider tokens they are explicitly authorized for via `service_provider_requirements`. The `delegation_service` enforces this mapping.

### No Raw Token in API Responses
User token listing endpoints never return decrypted tokens. The only path for decrypted tokens is through the proxy injection pipeline.

### Identity JWT Short-Lived
Identity assertion JWTs have a 60-second lifetime, are signed with NyxID's RSA keys, and include a service-specific audience claim to prevent token relay attacks.

### Audit Trail
All token operations are logged via `audit_service::log_async`:
- `provider_token_connected` (API key stored or OAuth completed)
- `provider_token_disconnected`
- `provider_token_refreshed`
- `provider_token_refresh_failed`
- `provider_token_delegated` (used during proxy)
- `identity_propagated` (identity headers/JWT injected during proxy)

### Rate Limiting on OAuth Callbacks
The OAuth callback endpoint should have strict rate limiting (5 req/min per IP) to prevent brute-force attacks on authorization codes.

---

## Database Migration Path

This plan is **additive only** -- no existing collections are dropped or have fields removed.

### Step 1: New Collections
Create 4 new collections:
- `provider_configs`
- `user_provider_tokens`
- `service_provider_requirements`
- `oauth_states`

Add indexes via `db::ensure_indexes()`.

### Step 2: Schema Extension
Add new fields to `downstream_services` documents. Existing documents will use defaults:
- `identity_propagation_mode` defaults to `"none"`
- `identity_include_*` defaults to `false`
- `identity_jwt_audience` defaults to `None`

No data migration needed -- `#[serde(default)]` handles missing fields.

### Step 3: Seed Default Providers
Seed well-known providers at startup (similar to `seed_default_clients`):
- OpenAI (api_key)
- Anthropic (api_key)
- Google AI (api_key or oauth2)

---

## Implementation Phases

### Phase 1: Provider Registry + API Key Storage
**Files to create:**
- `models/provider_config.rs`
- `models/user_provider_token.rs`
- `services/provider_service.rs`
- `services/user_token_service.rs` (API key path only)
- `handlers/providers.rs`
- `handlers/user_tokens.rs` (API key path only)
- `frontend/src/pages/providers.tsx`
- `frontend/src/components/dashboard/provider-grid.tsx`
- `frontend/src/components/dashboard/provider-card.tsx`
- `frontend/src/components/dashboard/provider-api-key-dialog.tsx`
- `frontend/src/hooks/use-providers.ts`

**Files to modify:**
- `models/mod.rs` -- add new modules
- `services/mod.rs` -- add new modules
- `handlers/mod.rs` -- add new modules
- `db.rs` -- add indexes for new collections
- `routes.rs` -- add provider routes
- `frontend/src/router.tsx` -- add provider route
- `frontend/src/components/dashboard/sidebar.tsx` -- add Providers nav item
- `frontend/src/types/api.ts` -- add new types

**Estimated complexity:** Medium

### Phase 2: OAuth Provider Flows
**Files to create:**
- `models/oauth_state.rs`
- `frontend/src/pages/provider-callback.tsx`

**Files to modify:**
- `services/user_token_service.rs` -- add OAuth initiate + callback
- `handlers/user_tokens.rs` -- add OAuth endpoints
- `db.rs` -- add oauth_states index
- `routes.rs` -- add callback route
- `frontend/src/router.tsx` -- add callback route
- `frontend/src/components/dashboard/provider-grid.tsx` -- OAuth connect flow

**Estimated complexity:** Medium-High (HTTP client interactions with external OAuth providers)

### Phase 3: Identity Propagation + Credential Delegation
**Files to create:**
- `models/service_provider_requirement.rs`
- `services/identity_service.rs`
- `services/delegation_service.rs`
- `handlers/service_requirements.rs`
- `frontend/src/components/dashboard/identity-propagation-config.tsx`
- `frontend/src/components/dashboard/service-requirements-editor.tsx`

**Files to modify:**
- `models/downstream_service.rs` -- add identity propagation fields
- `services/proxy_service.rs` -- add identity + delegation injection
- `handlers/proxy.rs` -- orchestrate identity + delegation
- `handlers/services.rs` -- handle new fields in create/update
- `db.rs` -- add new collection indexes
- `routes.rs` -- add requirement routes
- `frontend/src/pages/service-edit.tsx` -- identity config UI
- `frontend/src/pages/service-detail.tsx` -- show identity + requirements

**Estimated complexity:** High (core proxy changes, multiple integration points)

### Phase 4: Background Refresh + Polish
**Files to modify:**
- `main.rs` -- add background refresh task
- `services/user_token_service.rs` -- add `refresh_expiring_tokens()`
- `frontend/src/components/dashboard/provider-grid.tsx` -- refresh button, status indicators

**Estimated complexity:** Low

---

## Crate Dependencies

| Crate | Version | Phase | Purpose |
|---|---|---|---|
| `aes-gcm` | 0.10.x | Existing | AES-256-GCM encryption (already in Cargo.toml) |
| `reqwest` | 0.12.x | Existing | HTTP client (already in Cargo.toml) |
| `jsonwebtoken` | 9.x | Existing | JWT signing/verification (already in Cargo.toml) |
| `zeroize` | 1.x | Phase 1 | Memory protection for decrypted credentials |
| `oauth2` | 4.x | Phase 2 | OAuth 2.0 client flows (Vertex AI, Azure Entra) |

No new major dependencies needed for Phase 1 beyond `zeroize`. Phase 2 adds `oauth2`.

---

## Provider Seed Data

Seed well-known providers at startup (similar to `seed_default_clients` in `oauth_client_service.rs`):

```rust
// In provider_service.rs:
pub async fn seed_default_providers(db: &mongodb::Database) -> AppResult<()> {
    let defaults = vec![
        ("openai", "OpenAI", "api_key", "https://api.openai.com/v1/models", "Authorization", "Bearer {key}"),
        ("anthropic", "Anthropic", "api_key", "https://api.anthropic.com/v1/models", "x-api-key", "{key}"),
        ("google-ai", "Google AI Studio", "api_key", None, None, None),
        ("mistral", "Mistral AI", "api_key", "https://api.mistral.ai/v1/models", "Authorization", "Bearer {key}"),
        ("cohere", "Cohere", "api_key", "https://api.cohere.com/v2/models", "Authorization", "Bearer {key}"),
    ];
    // Insert if not exists (idempotent)
}
```

---

## Future Enhancements (Post-MVP)

These are not part of the initial implementation but documented for future planning:

1. **RFC 8693 Token Exchange** -- Full token exchange endpoint for advanced delegation with `act` claims. Useful for MCP clients that need to obtain bearer tokens directly without proxying.

2. **MCP Authorization Compliance** -- Resource Indicators (RFC 8707), Protected Resource Metadata (RFC 9728), Client ID Metadata Documents, scope-based tool access control.

3. **Per-User Key Derivation (HKDF)** -- Derive per-user encryption keys from the master key using HKDF with `user_id` as info. This provides defense-in-depth: compromising one user's derived key doesn't expose other users' credentials.

4. **Encryption Key Rotation** -- Key version prefix on ciphertexts, background migration task to re-encrypt with new key.

5. **Background Token Refresh** -- Tokio task that proactively refreshes expiring OAuth tokens (for Vertex AI, Azure Entra) before they expire, eliminating proxy latency from lazy refresh.

6. **Provider-Scoped MCP Tools** -- Dynamic `tools/list` responses that only include tools for providers the user has connected to.

---

## Success Criteria

- [ ] Admin can register provider configs (OAuth2 and API key types)
- [ ] Users can store API keys for providers (encrypted at rest)
- [ ] Users can connect OAuth2 providers via authorization code flow with PKCE
- [ ] Expired OAuth2 tokens are lazily refreshed during proxy requests
- [ ] Admin can configure provider requirements on downstream services
- [ ] Proxy injects delegated provider tokens based on service requirements
- [ ] Proxy injects identity headers (X-NyxID-*) based on service config
- [ ] Proxy injects signed JWT identity assertion when configured
- [ ] All token operations are audit-logged
- [ ] No decrypted tokens are ever returned in API responses
- [ ] Frontend shows provider connection status with connect/disconnect/refresh
- [ ] OAuth callback handles success and error cases gracefully
- [ ] Existing proxy functionality is not broken (backward compatible)
