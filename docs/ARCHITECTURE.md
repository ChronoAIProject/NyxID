# NyxID Architecture

This document describes the system architecture, component design, data flows, and security architecture of NyxID.

---

## Table of Contents

- [System Overview](#system-overview)
- [Component Architecture](#component-architecture)
- [Backend Layers](#backend-layers)
- [Frontend Architecture](#frontend-architecture)
- [Data Flow Diagrams](#data-flow-diagrams)
- [Database Schema](#database-schema)
- [Security Architecture](#security-architecture)
- [Credential Broker](#credential-broker)
- [Identity Propagation](#identity-propagation)
- [Deployment Architecture](#deployment-architecture)

---

## System Overview

```
+---------------------------------------------------------------------+
|                          Client Layer                                 |
|                                                                      |
|  +------------------+    +------------------+    +-----------------+ |
|  | React 19 SPA     |    | OAuth Clients    |    | MCP Agents      | |
|  | (Browser)        |    | (Third-party)    |    | (rmcp SDK)      | |
|  +--------+---------+    +--------+---------+    +--------+--------+ |
|           |                       |                       |          |
+-----------+-----------------------+-----------------------+----------+
            |                       |                       |
            +--------> HTTPS <------+--------> HTTPS <------+
                        |
+---------------------------------------------------------------------+
|                         API Gateway Layer                             |
|                                                                      |
|  +---------------------------------------------------------------+  |
|  |                     Axum 0.8 (Rust)                            |  |
|  |                                                                |  |
|  |  +-----------+  +-------------+  +-----------+  +-----------+ |  |
|  |  | CORS      |  | Rate Limit  |  | Security  |  | Trace     | |  |
|  |  | Layer     |  | (Per-IP +   |  | Headers   |  | Layer     | |  |
|  |  |           |  |  Global)    |  | Middleware |  | (tower)   | |  |
|  |  +-----------+  +-------------+  +-----------+  +-----------+ |  |
|  |                                                                |  |
|  |  +-----------+  +-------------+  +-----------+  +-----------+ |  |
|  |  | Auth      |  | Body Size   |  | Cookie    |  | Error     | |  |
|  |  | Extractor |  | Limit (1MB) |  | Mgmt      |  | Handler   | |  |
|  |  +-----------+  +-------------+  +-----------+  +-----------+ |  |
|  +---------------------------------------------------------------+  |
|                                                                      |
+---------------------------------------------------------------------+
            |
+---------------------------------------------------------------------+
|                       Application Layer                              |
|                                                                      |
|  +-------------+  +-------------+  +-------------+  +------------+  |
|  | Auth        |  | OAuth/OIDC  |  | API Key     |  | Service    |  |
|  | Handlers    |  | Handlers    |  | Handlers    |  | Handlers   |  |
|  +------+------+  +------+------+  +------+------+  +-----+-----+  |
|         |                |                |               |          |
|  +------+------+  +------+------+  +------+------+  +-----+-----+  |
|  | auth_service|  | oauth_service|  | key_service |  | proxy_svc |  |
|  | token_svc   |  | mfa_service |  |             |  | audit_svc |  |
|  +------+------+  +------+------+  +------+------+  +-----+-----+  |
|         |                |                |               |          |
+---------------------------------------------------------------------+
            |
+---------------------------------------------------------------------+
|                       Infrastructure Layer                           |
|                                                                      |
|  +-----------------+  +------------------+  +---------------------+  |
|  | MongoDB Driver  |  | Crypto Module    |  | reqwest (HTTP)      |  |
|  | (mongodb-rs)    |  | (Argon2, RS256,  |  | (Proxy Client)      |  |
|  |                 |  |  AES-256-GCM)    |  |                     |  |
|  +---------+-------+  +------------------+  +---------------------+  |
|            |                                                         |
+---------------------------------------------------------------------+
             |
    +--------v---------+
    |  MongoDB 8.0     |
    |  (14 collections)|
    +------------------+
```

---

## Component Architecture

### Backend Components

The Rust backend is organized into six distinct layers, each with clear responsibilities and dependencies flowing strictly downward.

#### 1. Entry Point (`main.rs`)

Responsibilities:
- Load environment variables via `dotenvy`
- Initialize structured logging with `tracing-subscriber`
- Validate configuration at startup (encryption key, required env vars)
- Create database connection pool
- Load RSA signing keys (auto-generate in dev mode)
- Create shared HTTP client (reqwest) for proxy connection reuse
- Build middleware stack (CORS, rate limiting, security headers, tracing)
- Bind TCP listener and start Axum server
- Spawn background task for per-IP rate limiter cleanup

#### 2. Middleware Layer (`mw/`)

| Module              | Responsibility                                        |
|---------------------|-------------------------------------------------------|
| `auth.rs`           | Extract `AuthUser` from Bearer token, session cookie, access token cookie, or API key header. Verify user is active. |
| `rate_limit.rs`     | Per-IP sliding window rate limiter with global token-bucket fallback. Background cleanup prevents memory growth. |
| `security_headers.rs` | Inject HSTS, CSP, X-Frame-Options, X-Content-Type-Options, Referrer-Policy, Permissions-Policy, X-XSS-Protection into every response. |

**Authentication Flow:**

```
Request arrives
    |
    v
1. Check Authorization: Bearer <token> header
   |-- Found? --> Verify JWT --> Extract user_id --> Check user is_active --> AuthUser
   |
2. Check nyx_session cookie
   |-- Found? --> Hash token --> Lookup session in DB --> Check not revoked/expired
   |             --> Check user is_active --> AuthUser
   |
3. Check nyx_access_token cookie
   |-- Found? --> Verify JWT --> Extract user_id --> Check user is_active --> AuthUser
   |
4. Check x-api-key header
   |-- Found? --> Hash key --> Lookup api_key in DB --> Check is_active, not expired
   |             --> Load user --> Check user is_active --> AuthUser
   |
5. None found --> Reject with 401
```

#### 3. Handler Layer (`handlers/`)

Handlers are thin HTTP boundary functions. They:
- Parse and validate request bodies/parameters
- Call service layer functions
- Format and return JSON responses
- Set cookies when needed (login, logout, refresh)
- Trigger audit log entries (non-blocking)

| Module        | Endpoints                                                       |
|---------------|-----------------------------------------------------------------|
| `auth.rs`     | register, login, logout, refresh, verify_email, forgot_password, reset_password |
| `users.rs`    | get_me, update_me                                               |
| `api_keys.rs` | list_keys, create_key, delete_key, rotate_key                   |
| `services.rs` | list_services, create_service, delete_service                   |
| `proxy.rs`    | proxy_request (wildcard, all HTTP methods)                      |
| `oauth.rs`    | authorize, token, userinfo                                      |
| `admin.rs`    | list_users, get_user, update_user, set_user_role, set_user_status, force_password_reset, delete_user, verify_user_email, list_user_sessions, revoke_user_sessions, list_audit_log, oauth client CRUD |
| `health.rs`   | health_check                                                    |
| `mfa.rs`      | setup, verify_setup                                             |
| `providers.rs`| list, create, get, update, delete provider configs              |
| `user_tokens.rs` | list tokens, connect API key/OAuth, disconnect, refresh      |
| `service_requirements.rs` | list, add, remove service provider requirements      |

#### 4. Service Layer (`services/`)

The service layer contains all business logic. Services receive database connections and domain objects -- they never interact with HTTP types.

| Module              | Responsibility                                            |
|---------------------|-----------------------------------------------------------|
| `auth_service.rs`   | User registration (email uniqueness, password hashing), credential verification |
| `token_service.rs`  | Session creation, JWT token pair issuance, refresh token rotation with replay detection, MFA pending session management |
| `oauth_service.rs`  | OAuth client validation, redirect URI verification, scope validation, authorization code creation/exchange, PKCE S256 verification, ID token generation |
| `key_service.rs`    | API key creation (prefix + SHA-256 hash), listing, deletion (soft deactivation), rotation (atomic deactivate + recreate) |
| `proxy_service.rs`  | Downstream service resolution, credential decryption, request forwarding with credential injection (header/bearer/query/basic), header allowlist enforcement |
| `mfa_service.rs`    | TOTP secret generation with QR provisioning, code verification against encrypted secrets, recovery code management |
| `admin_user_service.rs` | Admin user CRUD (update profile, set role, set status), cascade user deletion across 8 collections, force password reset, manual email verification, session listing and bulk revocation |
| `audit_service.rs`  | Asynchronous audit log insertion (fire-and-forget via `tokio::spawn`), captures user, action, resource, IP, user-agent |
| `provider_service.rs` | Provider registry CRUD, slug uniqueness, encrypted OAuth credential storage |
| `user_token_service.rs` | User provider token lifecycle: API key storage, OAuth flow initiation/callback, token refresh with 5-min buffer, token retrieval with lazy refresh |
| `delegation_service.rs` | Resolves delegated provider credentials for proxy injection, batch provider queries (N+1 fix), required vs. optional enforcement |
| `identity_service.rs` | Builds identity propagation headers (CRLF-sanitized), generates short-lived RS256 identity assertion JWTs (60s TTL) |
| `oauth_flow.rs`     | OAuth2 utilities: PKCE code verifier/challenge generation, token exchange with no-redirect HTTP client, token refresh |

#### 5. Crypto Layer (`crypto/`)

Pure cryptographic operations with no database or HTTP dependencies.

| Module        | Algorithms                                                  |
|---------------|-------------------------------------------------------------|
| `password.rs` | Argon2id (m=64MiB, t=3, p=4) via the `argon2` crate. OWASP-recommended parameters. Random salt per hash. |
| `jwt.rs`      | RS256 signing/verification via `jsonwebtoken`. 4096-bit RSA key pair. Auto-generation in dev mode with 0600 permissions. Access tokens, refresh tokens, and OIDC ID tokens. |
| `aes.rs`      | AES-256-GCM via `aes-gcm`. Random 96-bit nonce per encryption. Output format: `nonce(12) || ciphertext || tag(16)`. |
| `token.rs`    | Cryptographically random token generation. SHA-256 hashing for storage (plaintext never persisted). |

#### 6. Model Layer (`models/`)

MongoDB document definitions for each collection. Each module defines:
- `Document` struct with serialization/deserialization support
- Validation logic
- Index configuration for query optimization

Sensitive fields (password_hash, tokens) are annotated with `#[serde(skip_serializing)]` to prevent accidental serialization.

### Shared Application State

```rust
pub struct AppState {
    pub db: MongoClient,           // MongoDB connection pool
    pub config: AppConfig,         // Immutable configuration
    pub jwt_keys: JwtKeys,         // RSA key pair for JWT operations
    pub http_client: reqwest::Client, // Shared HTTP client for proxy
}
```

`AppState` is cloned (cheaply, via `Arc` internally) into each handler via Axum's `State` extractor.

---

## Frontend Architecture

```
frontend/src/
|
|-- main.tsx              Application entry point (React root + providers)
|-- router.tsx            TanStack Router configuration
|-- app.css               Global styles (Tailwind v4)
|
|-- lib/
|   |-- api-client.ts     Centralized fetch wrapper with auth token injection
|   `-- utils.ts          Utility functions (cn, classnames)
|
|-- stores/
|   `-- auth-store.ts     Zustand store for auth state (user, tokens, login/logout)
|
|-- types/
|   |-- api.ts            TypeScript types matching backend JSON schemas
|   `-- admin.ts          Admin-specific types (user list, sessions, actions)
|
|-- schemas/
|   |-- auth.ts           Zod schemas for login/register forms
|   |-- api-keys.ts       Zod schemas for API key forms
|   |-- services.ts       Zod schemas for service forms
|   `-- admin.ts          Zod schemas for admin user management forms
|
|-- hooks/
|   |-- use-auth.ts       React Query hooks for auth operations
|   |-- use-api-keys.ts   React Query hooks for API key CRUD
|   |-- use-services.ts   React Query hooks for service operations
|   `-- use-admin.ts      React Query hooks for admin user management
|
|-- components/
|   |-- ui/               16 shadcn/ui primitives (Button, Card, Dialog, etc.)
|   |-- auth/             Login form, register form, social login buttons,
|   |                     MFA setup dialog, MFA verify form
|   |-- dashboard/        Sidebar, header, API key table, API key create dialog,
|   |                     service card, connection grid
|   `-- layout/           Auth layout, dashboard layout
|
`-- pages/                Route page components
    |-- login.tsx
    |-- register.tsx
    |-- dashboard.tsx
    |-- api-keys.tsx
    |-- services.tsx
    |-- connections.tsx
    |-- settings.tsx
    |-- admin-users.tsx       Admin user list (search, pagination, status badges)
    `-- admin-user-detail.tsx Admin user detail (edit, actions, sessions)
```

### Key Frontend Patterns

- **Server State:** TanStack Query manages all API data (caching, refetching, mutations)
- **Client State:** Zustand manages auth state that must persist across navigation
- **Form Handling:** React Hook Form with Zod resolvers for type-safe validation
- **Routing:** TanStack Router with file-based route definitions
- **Styling:** Tailwind CSS v4 with shadcn/ui component library (Radix primitives)

---

## Data Flow Diagrams

### User Registration

```
Client                     Backend                           Database
  |                          |                                  |
  |  POST /auth/register     |                                  |
  |  {email, password}       |                                  |
  |------------------------->|                                  |
  |                          |  Validate email format           |
  |                          |  Validate password length        |
  |                          |                                  |
  |                          |  Find in users collection        |
  |                          |  WHERE email = ?                 |
  |                          |--------------------------------->|
  |                          |  (check uniqueness)              |
  |                          |<---------------------------------|
  |                          |                                  |
  |                          |  Argon2id hash(password)         |
  |                          |                                  |
  |                          |  InsertOne in users collection   |
  |                          |  {id, email, password_hash, ...} |
  |                          |--------------------------------->|
  |                          |<---------------------------------|
  |                          |                                  |
  |                          |  Async: InsertOne audit_log      |
  |                          |  {action=register}               |
  |                          |                       - - - - - >|
  |                          |                                  |
  |  200 {user_id, message}  |                                  |
  |<-------------------------|                                  |
```

### User Login (with MFA)

```
Client                     Backend                           Database
  |                          |                                  |
  |  POST /auth/login        |                                  |
  |  {email, password}       |                                  |
  |------------------------->|                                  |
  |                          |  Find in users collection        |
  |                          |  WHERE email = ?                 |
  |                          |--------------------------------->|
  |                          |<---------------------------------|
  |                          |                                  |
  |                          |  Argon2id verify(password, hash) |
  |                          |                                  |
  |                          |  Check user.mfa_enabled          |
  |                          |  mfa_enabled = true, no mfa_code |
  |                          |                                  |
  |                          |  Generate temp_token             |
  |                          |  Hash temp_token                 |
  |                          |  InsertOne in sessions           |
  |                          |  {mfa_pending}                   |
  |                          |--------------------------------->|
  |                          |<---------------------------------|
  |                          |                                  |
  |  403 {mfa_required,      |                                  |
  |   session_token}         |                                  |
  |<-------------------------|                                  |
  |                          |                                  |
  |  POST /auth/login        |                                  |
  |  {email, password,       |                                  |
  |   mfa_code: "123456"}    |                                  |
  |------------------------->|                                  |
  |                          |  Re-verify password              |
  |                          |  Decrypt MFA secret (AES-256)    |
  |                          |  Verify TOTP code                |
  |                          |                                  |
  |                          |  Create session                  |
  |                          |  Generate access JWT (RS256)     |
  |                          |  Generate refresh JWT (RS256)    |
  |                          |  Store refresh token hash        |
  |                          |--------------------------------->|
  |                          |<---------------------------------|
  |                          |                                  |
  |  200 {user_id,           |                                  |
  |   access_token,          |                                  |
  |   expires_in}            |                                  |
  |  Set-Cookie: nyx_session |                                  |
  |  Set-Cookie: nyx_access  |                                  |
  |  Set-Cookie: nyx_refresh |                                  |
  |<-------------------------|                                  |
```

### Token Refresh with Rotation

```
Client                     Backend                           Database
  |                          |                                  |
  |  POST /auth/refresh      |                                  |
  |  Cookie: nyx_refresh=JWT |                                  |
  |------------------------->|                                  |
  |                          |  Decode refresh JWT              |
  |                          |  Extract JTI                     |
  |                          |                                  |
  |                          |  Find in refresh_tokens          |
  |                          |  WHERE jti = ?                   |
  |                          |--------------------------------->|
  |                          |<---------------------------------|
  |                          |                                  |
  |                          |  Check: not revoked, not expired |
  |                          |                                  |
  |                          |  Mark old token as revoked       |
  |                          |  UpdateOne refresh_tokens        |
  |                          |  SET revoked=true,               |
  |                          |      replaced_by=new_id          |
  |                          |--------------------------------->|
  |                          |                                  |
  |                          |  Generate new access JWT         |
  |                          |  Generate new refresh JWT        |
  |                          |  InsertOne new refresh_token     |
  |                          |--------------------------------->|
  |                          |<---------------------------------|
  |                          |                                  |
  |  200 {access_token,      |                                  |
  |   expires_in}            |                                  |
  |  Set-Cookie: nyx_access  |                                  |
  |  Set-Cookie: nyx_refresh |                                  |
  |<-------------------------|                                  |
```

### OAuth Authorization Code Flow (PKCE)

```
Client App          User Browser         NyxID Backend        Database
    |                    |                     |                  |
    |  Redirect to       |                     |                  |
    |  /oauth/authorize  |                     |                  |
    |------------------->|                     |                  |
    |                    |  GET /oauth/authorize|                  |
    |                    |  ?response_type=code |                  |
    |                    |  &client_id=...      |                  |
    |                    |  &redirect_uri=...   |                  |
    |                    |  &code_challenge=... |                  |
    |                    |  &code_challenge_method=S256            |
    |                    |  &scope=openid       |                  |
    |                    |  &state=xyz          |                  |
    |                    |-------------------->|                  |
    |                    |                     |  Validate client |
    |                    |                     |  Validate URI    |
    |                    |                     |  Validate scopes |
    |                    |                     |  Generate code   |
    |                    |                     |  Hash + store    |
    |                    |                     |----------------->|
    |                    |                     |<-----------------|
    |                    |                     |                  |
    |                    |  200 {redirect_url}  |                  |
    |                    |  (with ?code=...     |                  |
    |                    |   &state=xyz)        |                  |
    |                    |<--------------------|                  |
    |                    |                     |                  |
    |  Callback with     |                     |                  |
    |  ?code=...&state=  |                     |                  |
    |<-------------------|                     |                  |
    |                    |                     |                  |
    |  POST /oauth/token                       |                  |
    |  {grant_type:authorization_code,         |                  |
    |   code, client_id, redirect_uri,         |                  |
    |   code_verifier}                         |                  |
    |----------------------------------------->|                  |
    |                                          |  Lookup code     |
    |                                          |  Verify PKCE:    |
    |                                          |  SHA256(verifier) |
    |                                          |  == challenge?   |
    |                                          |  Mark code used  |
    |                                          |  Generate tokens |
    |                                          |----------------->|
    |                                          |<-----------------|
    |                                          |                  |
    |  200 {access_token,                      |                  |
    |   refresh_token,                         |                  |
    |   id_token,                              |                  |
    |   token_type: Bearer}                    |                  |
    |<-----------------------------------------|                  |
```

### Proxy Request Flow

```
Client                     NyxID Backend                     Downstream
  |                          |                                Service
  |  ANY /api/v1/proxy/      |                                  |
  |  {service_id}/path       |                                  |
  |------------------------->|                                  |
  |                          |  Authenticate user (AuthUser)    |
  |                          |                                  |
  |                          |  Lookup downstream_service       |
  |                          |  Check: is_active = true         |
  |                          |                                  |
  |                          |  Lookup user_service_connection  |
  |                          |  (per-user override?)            |
  |                          |                                  |
  |                          |  AES-256-GCM decrypt credential  |
  |                          |                                  |
  |                          |  Identity propagation:           |
  |                          |  - If mode=headers/both:         |
  |                          |    add X-NyxID-User-* headers    |
  |                          |  - If mode=jwt/both:             |
  |                          |    sign RS256 identity assertion |
  |                          |    add X-NyxID-Identity-Token    |
  |                          |                                  |
  |                          |  Credential delegation:          |
  |                          |  - Load service requirements     |
  |                          |  - Resolve user provider tokens  |
  |                          |  - Decrypt + inject each token   |
  |                          |                                  |
  |                          |  Build outbound request:         |
  |                          |  - URL: base_url + /path + ?query|
  |                          |  - Copy allowed headers only     |
  |                          |  - Inject service credential     |
  |                          |  - Inject identity headers       |
  |                          |  - Inject delegated credentials  |
  |                          |  - Forward body (up to 10MB)     |
  |                          |                                  |
  |                          |  reqwest::Client::request(...)   |
  |                          |--------------------------------->|
  |                          |<---------------------------------|
  |                          |                                  |
  |                          |  Convert response:               |
  |                          |  - Map status code               |
  |                          |  - Forward allowlisted headers   |
  |                          |  - Forward body                  |
  |                          |                                  |
  |  <downstream response>   |                                  |
  |<-------------------------|                                  |
```

---

## Database Schema

### Entity Relationship Overview

```
+---------------+        +-------------------+
|    users      |<-------| sessions          |
|               |<-------| api_keys          |
|               |<-------| mfa_factors       |
|               |<-------| audit_log         |
|               |<--+    | user_service_conn |-------+
+-------+-------+   |    +-------------------+       |
        |            |                                |
        |            +--------------------------------+
        |
        |    +-------------------+
        +--->| oauth_clients     |
        |    +-------------------+
        |            |
        |    +-------v-----------+
        +--->| authorization_codes|
        |    +-------------------+
        |
        |    +-------------------+      +-------------------+
        +--->| refresh_tokens    |----->| sessions          |
        |    +-------------------+      +-------------------+
        |
        |    +-------------------+
        +--->| downstream_services|----+
        |    +-------------------+    |
        |                             |
        |    +-------------------+    |    +------------------------+
        +--->| provider_configs  |<---+----| service_provider_      |
        |    +-------------------+         | requirements           |
        |            |                     +------------------------+
        |    +-------v-----------+
        +--->| user_provider_    |
        |    | tokens            |
        |    +-------------------+
        |
        |    +-------------------+
        +--->| oauth_states      |
             +-------------------+
```

### Collection Details

#### users

The core user identity collection. Password hash is nullable to support social-only accounts.

| Field                     | Type                   | Constraints     | Description                     |
|---------------------------|------------------------|-----------------|---------------------------------|
| `_id`                     | ObjectId               | PK              | MongoDB document ID             |
| `id`                      | UUID (string)          | NOT NULL, UNIQUE| User identifier                 |
| `email`                   | string                 | NOT NULL, UNIQUE| Email address                   |
| `password_hash`           | string                 | NULLABLE        | Argon2id PHC string             |
| `display_name`            | string                 | NULLABLE        | Display name                    |
| `avatar_url`              | string                 | NULLABLE        | Avatar image URL                |
| `email_verified`          | boolean                | NOT NULL, DEFAULT false | Email verification status |
| `email_verification_token`| string                 | NULLABLE        | Pending verification token      |
| `password_reset_token`    | string                 | NULLABLE        | Password reset token            |
| `password_reset_expires_at`| ISO 8601 date       | NULLABLE        | Reset token expiration          |
| `is_active`               | boolean                | NOT NULL, DEFAULT true  | Account active status    |
| `is_admin`                | boolean                | NOT NULL, DEFAULT false | Admin privilege flag     |
| `mfa_enabled`             | boolean                | NOT NULL, DEFAULT false | MFA enabled flag         |
| `created_at`              | ISO 8601 date          | NOT NULL        | Account creation time           |
| `updated_at`              | ISO 8601 date          | NOT NULL        | Last profile update             |
| `last_login_at`           | ISO 8601 date          | NULLABLE        | Last successful login           |

**Indexes:** `email` (unique), `email_verification_token`, `password_reset_token`

#### sessions

Server-side session records. Token is stored as SHA-256 hash.

| Field           | Type          | Constraints     | Description                     |
|-----------------|---------------|-----------------|---------------------------------|
| `_id`           | ObjectId      | PK              | MongoDB document ID             |
| `id`            | UUID (string) | NOT NULL, UNIQUE| Session identifier              |
| `user_id`       | UUID (string) | NOT NULL        | Owner (-> users.id)             |
| `token_hash`    | string        | NOT NULL        | SHA-256 of session token        |
| `ip_address`    | string        | NULLABLE        | Client IP at creation           |
| `user_agent`    | string        | NULLABLE        | Client user-agent at creation   |
| `expires_at`    | ISO 8601 date | NOT NULL        | Session expiration              |
| `revoked`       | boolean       | NOT NULL, DEFAULT false | Revocation flag          |
| `created_at`    | ISO 8601 date | NOT NULL        | Session creation time           |
| `last_active_at`| ISO 8601 date | NOT NULL        | Last activity timestamp         |

**Indexes:** `token_hash`, `user_id`

#### oauth_clients

Registered OAuth/OIDC clients.

| Field               | Type          | Constraints     | Description                     |
|---------------------|---------------|-----------------|---------------------------------|
| `_id`               | ObjectId      | PK              | MongoDB document ID             |
| `id`                | UUID (string) | NOT NULL, UNIQUE| Client identifier               |
| `client_name`       | string        | NOT NULL        | Human-readable name             |
| `client_secret_hash`| string        | NOT NULL        | Hashed client secret            |
| `redirect_uris`     | array         | NOT NULL        | Array of allowed redirect URIs  |
| `allowed_scopes`    | string        | NOT NULL        | Space-separated allowed scopes  |
| `grant_types`       | string        | NOT NULL        | Allowed grant types             |
| `client_type`       | string        | NOT NULL, DEFAULT 'confidential' | confidential or public |
| `is_active`         | boolean       | NOT NULL, DEFAULT true | Active status             |
| `created_by`        | UUID (string) | NULLABLE        | Admin who created this client   |
| `created_at`        | ISO 8601 date | NOT NULL        | Creation timestamp              |
| `updated_at`        | ISO 8601 date | NOT NULL        | Last update timestamp           |

#### authorization_codes

Short-lived OIDC authorization codes (typically 60-second TTL).

| Field                  | Type          | Constraints     | Description                     |
|------------------------|---------------|-----------------|---------------------------------|
| `_id`                  | ObjectId      | PK              | MongoDB document ID             |
| `id`                   | UUID (string) | NOT NULL, UNIQUE| Code record identifier          |
| `code_hash`            | string        | NOT NULL        | SHA-256 of the authorization code|
| `client_id`            | UUID (string) | NOT NULL        | Client (-> oauth_clients.id)    |
| `user_id`              | UUID (string) | NOT NULL        | Authorizing user (-> users.id)  |
| `redirect_uri`         | string        | NOT NULL        | Redirect URI used in request    |
| `scope`                | string        | NOT NULL        | Granted scopes                  |
| `code_challenge`       | string        | NULLABLE        | PKCE code challenge             |
| `code_challenge_method`| string        | NULLABLE        | PKCE method (S256)              |
| `nonce`                | string        | NULLABLE        | OIDC nonce for ID token         |
| `expires_at`           | ISO 8601 date | NOT NULL        | Code expiration                 |
| `used`                 | boolean       | NOT NULL, DEFAULT false | Prevents code reuse      |
| `created_at`           | ISO 8601 date | NOT NULL        | Code creation timestamp         |

**Indexes:** `code_hash`

#### refresh_tokens

Refresh tokens with rotation chain tracking. The `replaced_by` field links to the successor token, enabling replay detection.

| Field         | Type          | Constraints       | Description                   |
|---------------|---------------|-------------------|-------------------------------|
| `_id`         | ObjectId      | PK                | MongoDB document ID           |
| `id`          | UUID (string) | NOT NULL, UNIQUE  | Token record identifier       |
| `jti`         | string        | NOT NULL, UNIQUE  | JWT ID                        |
| `client_id`   | UUID (string) | NOT NULL          | Issuing client                |
| `user_id`     | UUID (string) | NOT NULL          | Token owner                   |
| `session_id`  | UUID (string) | NULLABLE          | Associated session            |
| `expires_at`  | ISO 8601 date | NOT NULL          | Token expiration              |
| `revoked`     | boolean       | NOT NULL, DEFAULT false | Revocation flag         |
| `replaced_by` | UUID (string) | NULLABLE          | Successor token (rotation)    |
| `created_at`  | ISO 8601 date | NOT NULL          | Token creation timestamp      |

**Indexes:** `jti`, `session_id`

#### api_keys

User-scoped API keys. The full key is never stored; only the SHA-256 hash and a display prefix.

| Field         | Type          | Constraints       | Description                   |
|---------------|---------------|-------------------|-------------------------------|
| `_id`         | ObjectId      | PK                | MongoDB document ID           |
| `id`          | UUID (string) | NOT NULL, UNIQUE  | Key record identifier         |
| `user_id`     | UUID (string) | NOT NULL          | Key owner                     |
| `name`        | string        | NOT NULL          | Human-readable label          |
| `key_prefix`  | string        | NOT NULL          | Display prefix (e.g. nyx_k_xxx)|
| `key_hash`    | string        | NOT NULL, UNIQUE  | SHA-256 of full key           |
| `scopes`      | string        | NOT NULL, DEFAULT 'read' | Space-separated scopes |
| `last_used_at`| ISO 8601 date | NULLABLE          | Last usage timestamp          |
| `expires_at`  | ISO 8601 date | NULLABLE          | Optional expiration           |
| `is_active`   | boolean       | NOT NULL, DEFAULT true | Active status             |
| `created_at`  | ISO 8601 date | NOT NULL          | Creation timestamp            |

**Indexes:** `user_id`, `key_hash`

#### downstream_services

Registered services that NyxID can proxy requests to. Credentials are encrypted at rest.

| Field                  | Type          | Constraints     | Description                   |
|------------------------|---------------|-----------------|-------------------------------|
| `_id`                  | ObjectId      | PK              | MongoDB document ID           |
| `id`                   | UUID (string) | NOT NULL, UNIQUE| Service identifier            |
| `name`                 | string        | NOT NULL        | Display name                  |
| `slug`                 | string        | NOT NULL, UNIQUE| URL-safe identifier           |
| `description`          | string        | NULLABLE        | Service description           |
| `base_url`             | string        | NOT NULL        | Downstream base URL           |
| `auth_method`          | string        | NOT NULL        | header/bearer/query/basic     |
| `auth_key_name`        | string        | NOT NULL        | Header name or query param    |
| `credential_encrypted` | binary        | NOT NULL        | AES-256-GCM encrypted credential|
| `is_active`            | boolean       | NOT NULL, DEFAULT true | Active status           |
| `created_by`           | UUID (string) | NOT NULL        | Admin who created it          |
| `created_at`           | ISO 8601 date | NOT NULL        | Creation timestamp            |
| `updated_at`           | ISO 8601 date | NOT NULL        | Last update                   |

#### user_service_connections

Per-user credential overrides for downstream services. When a user has a connection, their credential is used instead of the service-level default.

| Field                  | Type         | Constraints     | Description                   |
|------------------------|--------------|-----------------|-------------------------------|
| `_id`                  | ObjectId     | PK              | MongoDB document ID           |
| `id`                   | UUID (string)| NOT NULL, UNIQUE| Connection identifier         |
| `user_id`              | UUID (string)| NOT NULL        | User                          |
| `service_id`           | UUID (string)| NOT NULL        | Downstream service            |
| `credential_encrypted` | binary       | NULLABLE        | AES-encrypted user credential |
| `is_active`            | boolean      | NOT NULL, DEFAULT true | Active status           |
| `created_at`           | ISO 8601 date| NOT NULL        | Connection creation           |
| `updated_at`           | ISO 8601 date| NOT NULL        | Last update                   |

**Indexes:** `(user_id, service_id)` UNIQUE

#### mfa_factors

TOTP multi-factor authentication factors. Secrets and recovery codes are encrypted.

| Field              | Type         | Constraints     | Description                   |
|--------------------|--------------|-----------------|-------------------------------|
| `_id`              | ObjectId     | PK              | MongoDB document ID           |
| `id`               | UUID (string)| NOT NULL, UNIQUE| Factor identifier             |
| `user_id`          | UUID (string)| NOT NULL        | User                          |
| `factor_type`      | string       | NOT NULL        | Factor type (totp)            |
| `secret_encrypted` | binary       | NULLABLE        | AES-encrypted TOTP secret     |
| `recovery_codes`   | array        | NULLABLE        | Hashed recovery codes         |
| `is_verified`      | boolean      | NOT NULL, DEFAULT false | Verified after first use|
| `is_active`        | boolean      | NOT NULL, DEFAULT true  | Active status           |
| `created_at`       | ISO 8601 date| NOT NULL        | Factor creation               |
| `updated_at`       | ISO 8601 date| NOT NULL        | Last update                   |

**Indexes:** `user_id`

#### audit_log

Append-only audit trail for security events. References to deleted users are retained.

| Field           | Type          | Constraints     | Description                   |
|-----------------|---------------|-----------------|-------------------------------|
| `_id`           | ObjectId      | PK              | MongoDB document ID           |
| `id`            | UUID (string) | NOT NULL, UNIQUE| Log entry identifier          |
| `user_id`       | UUID (string) | NULLABLE        | Acting user (retained on delete)|
| `action`        | string        | NOT NULL        | Action performed              |
| `resource_type` | string        | NOT NULL        | Resource category             |
| `resource_id`   | string        | NULLABLE        | Specific resource identifier  |
| `metadata`      | object        | NULLABLE        | Additional context            |
| `ip_address`    | string        | NULLABLE        | Client IP address             |
| `user_agent`    | string        | NULLABLE        | Client user-agent string      |
| `created_at`    | ISO 8601 date | NOT NULL        | Event timestamp               |

**Indexes:** `user_id`, `action`, `created_at`

#### provider_configs

Admin-managed registry of external providers (e.g., OpenAI, Anthropic, Google AI). OAuth client credentials are encrypted at rest.

| Field                    | Type          | Constraints     | Description                     |
|--------------------------|---------------|-----------------|---------------------------------|
| `_id`                    | UUID (string) | PK              | Provider identifier             |
| `slug`                   | string        | NOT NULL, UNIQUE| URL-safe identifier             |
| `name`                   | string        | NOT NULL        | Display name                    |
| `description`            | string        | NULLABLE        | Provider description            |
| `provider_type`          | string        | NOT NULL        | `oauth2` or `api_key`           |
| `authorization_url`      | string        | NULLABLE        | OAuth2 authorization endpoint   |
| `token_url`              | string        | NULLABLE        | OAuth2 token endpoint           |
| `revocation_url`         | string        | NULLABLE        | OAuth2 revocation endpoint      |
| `default_scopes`         | array         | NULLABLE        | Default OAuth2 scopes           |
| `client_id_encrypted`    | binary        | NULLABLE        | AES-encrypted OAuth client ID   |
| `client_secret_encrypted`| binary        | NULLABLE        | AES-encrypted OAuth client secret|
| `supports_pkce`          | boolean       | NOT NULL, DEFAULT false | PKCE support flag       |
| `api_key_instructions`   | string        | NULLABLE        | Instructions for API key setup  |
| `api_key_url`            | string        | NULLABLE        | URL to create API keys          |
| `icon_url`               | string        | NULLABLE        | Provider icon URL               |
| `documentation_url`      | string        | NULLABLE        | Provider documentation URL      |
| `is_active`              | boolean       | NOT NULL, DEFAULT true | Active status            |
| `created_by`             | UUID (string) | NOT NULL        | Admin who created it            |
| `created_at`             | ISO 8601 date | NOT NULL        | Creation timestamp              |
| `updated_at`             | ISO 8601 date | NOT NULL        | Last update                     |

**Indexes:** `slug` (unique)

#### user_provider_tokens

Per-user encrypted tokens for external providers. Supports both API keys and OAuth2 tokens with refresh lifecycle.

| Field                    | Type          | Constraints     | Description                     |
|--------------------------|---------------|-----------------|---------------------------------|
| `_id`                    | UUID (string) | PK              | Token record identifier         |
| `user_id`                | UUID (string) | NOT NULL        | Token owner                     |
| `provider_config_id`     | UUID (string) | NOT NULL        | Provider (-> provider_configs)  |
| `token_type`             | string        | NOT NULL        | `oauth2` or `api_key`           |
| `access_token_encrypted` | binary        | NULLABLE        | AES-encrypted OAuth access token|
| `refresh_token_encrypted`| binary        | NULLABLE        | AES-encrypted OAuth refresh token|
| `token_scopes`           | string        | NULLABLE        | Granted OAuth scopes            |
| `expires_at`             | ISO 8601 date | NULLABLE        | Token expiration                |
| `api_key_encrypted`      | binary        | NULLABLE        | AES-encrypted API key           |
| `status`                 | string        | NOT NULL        | active/expired/revoked/refresh_failed |
| `last_refreshed_at`      | ISO 8601 date | NULLABLE        | Last refresh timestamp          |
| `last_used_at`           | ISO 8601 date | NULLABLE        | Last usage timestamp            |
| `error_message`          | string        | NULLABLE        | Last error during refresh       |
| `label`                  | string        | NULLABLE        | User-provided label             |
| `created_at`             | ISO 8601 date | NOT NULL        | Connection timestamp            |
| `updated_at`             | ISO 8601 date | NOT NULL        | Last update                     |

**Indexes:** `(user_id, provider_config_id)` (unique)

#### service_provider_requirements

Defines which providers a downstream service needs credentials from. The proxy resolves these during request forwarding.

| Field                | Type          | Constraints     | Description                     |
|----------------------|---------------|-----------------|---------------------------------|
| `_id`                | UUID (string) | PK              | Requirement identifier          |
| `service_id`         | UUID (string) | NOT NULL        | Service (-> downstream_services)|
| `provider_config_id` | UUID (string) | NOT NULL        | Provider (-> provider_configs)  |
| `required`           | boolean       | NOT NULL        | Fail if user has no token       |
| `scopes`             | array         | NULLABLE        | Specific scopes needed          |
| `injection_method`   | string        | NOT NULL        | bearer/header/query             |
| `injection_key`      | string        | NULLABLE        | Header/param name for injection |
| `created_at`         | ISO 8601 date | NOT NULL        | Creation timestamp              |
| `updated_at`         | ISO 8601 date | NOT NULL        | Last update                     |

**Indexes:** `(service_id, provider_config_id)` (unique)

#### oauth_states

Temporary OAuth state records for provider OAuth flows. Used for CSRF protection and PKCE code verifier storage. Expired states are cleaned up by TTL.

| Field                | Type          | Constraints     | Description                     |
|----------------------|---------------|-----------------|---------------------------------|
| `_id`                | UUID (string) | PK              | State identifier                |
| `user_id`            | UUID (string) | NOT NULL        | User who initiated the flow     |
| `provider_config_id` | UUID (string) | NOT NULL        | Target provider                 |
| `code_verifier`      | string        | NULLABLE        | PKCE code verifier              |
| `expires_at`         | ISO 8601 date | NOT NULL        | State expiration                |
| `created_at`         | ISO 8601 date | NOT NULL        | Creation timestamp              |

**Indexes:** `expires_at` (TTL)

---

## Credential Broker

The credential broker enables NyxID to act as a centralized token vault for external service providers. Admins configure providers, users connect their credentials, and downstream services declare which provider tokens they need.

### Provider Registry

```
Admin creates                  Users connect
provider config                their credentials
     |                              |
     v                              v
+----------------+          +--------------------+
| provider_configs|<---------| user_provider_tokens|
| (OpenAI, etc.) |          | (encrypted keys/   |
|                |          |  OAuth tokens)     |
+-------+--------+          +---------+----------+
        |                             |
        v                             v
+-------------------+    +---------------------+
| service_provider_ |    | delegation_service  |
| requirements      |    | (resolve + inject)  |
| (per-service)     |    +---------------------+
+-------------------+
```

### Credential Delegation Flow

When a proxied request is made to a service with provider requirements:

1. **Load requirements** -- Query `service_provider_requirements` for the target service
2. **Batch fetch providers** -- Single query to `provider_configs` (N+1 prevention)
3. **Resolve user tokens** -- For each requirement, fetch the user's active token via `user_token_service::get_active_token()` (triggers lazy OAuth refresh)
4. **Required vs. optional** -- Required providers without tokens cause a 400 error; optional providers are silently skipped
5. **Inject credentials** -- Each resolved token is injected into the outbound request using the configured method (bearer/header/query)

### Token Refresh Lifecycle

OAuth2 tokens are refreshed lazily during proxy requests:

- **Buffer window:** 5 minutes before expiry
- **No-redirect client:** Token exchange uses a dedicated `reqwest::Client` with `redirect::Policy::none()` to prevent SSRF via redirect
- **Error truncation:** Error bodies from providers are truncated to 200 characters before storage
- **Status tracking:** Failed refreshes update status to `refresh_failed` with an error message
- **Memory protection:** Decrypted tokens use the `zeroize` crate for secure memory cleanup

### Supported Providers

NyxID supports two provider authentication models:

| Provider Type | Connection Method | Examples                          |
|---------------|-------------------|-----------------------------------|
| `api_key`     | User enters key   | OpenAI, Anthropic, Mistral, Cohere|
| `oauth2`      | OAuth2 flow       | Google AI (Vertex), Azure OpenAI  |

---

## Identity Propagation

Identity propagation allows downstream services to know which NyxID user is making the request, without the downstream service needing to integrate with NyxID's auth system.

### Propagation Modes

| Mode      | Headers Added                                | JWT Added | Use Case                          |
|-----------|----------------------------------------------|-----------|-----------------------------------|
| `none`    | --                                           | No        | Default. Service handles its own auth. |
| `headers` | `X-NyxID-User-Id`, `X-NyxID-User-Email`, `X-NyxID-User-Name` | No | Simple identity forwarding (trusted network). |
| `jwt`     | `X-NyxID-Identity-Token`                     | Yes       | Cryptographically verified identity. |
| `both`    | All of the above                             | Yes       | Headers for convenience, JWT for verification. |

Which identity claims are included is controlled per-service:
- `identity_include_user_id` -- includes `X-NyxID-User-Id`
- `identity_include_email` -- includes `X-NyxID-User-Email` and `email` in JWT
- `identity_include_name` -- includes `X-NyxID-User-Name` and `name` in JWT

### Identity Assertion JWT

When `identity_propagation_mode` is `jwt` or `both`, NyxID generates a short-lived RS256-signed JWT:

| Claim            | Type    | Description                                    |
|------------------|---------|------------------------------------------------|
| `sub`            | string  | User ID (UUID)                                 |
| `iss`            | string  | NyxID JWT issuer                               |
| `aud`            | string  | Service's `identity_jwt_audience` or `base_url`|
| `exp`            | integer | Expiration (now + 60 seconds)                  |
| `iat`            | integer | Issued at                                      |
| `jti`            | string  | Unique token ID                                |
| `email`          | string  | User email (if `identity_include_email`)       |
| `name`           | string  | Display name (if `identity_include_name`)      |
| `nyx_service_id` | string  | Target service ID                              |

Downstream services verify the JWT using NyxID's JWKS endpoint (`/.well-known/jwks.json`).

### Security Considerations

- **CRLF injection prevention:** All identity header values pass through `sanitize_header_value()` which strips CR (`\r`), LF (`\n`), and NUL (`\0`) characters
- **Short token lifetime:** Identity JWTs expire in 60 seconds to minimize replay window
- **Per-service audience:** The `aud` claim is scoped to the target service, preventing token reuse across services

---

## Security Architecture

### Defense in Depth

NyxID applies multiple layers of security controls:

```
Layer 1: Network
  |-- TLS termination (reverse proxy)
  |-- CORS restricted to single origin
  |-- Rate limiting (per-IP + global)
  |
Layer 2: Transport
  |-- HSTS with preload
  |-- Secure cookie flags
  |-- 1 MB body size limit
  |
Layer 3: Application
  |-- Input validation on all endpoints
  |-- SSRF protection for proxy URLs
  |-- PKCE required for all OAuth flows
  |-- MFA support (TOTP)
  |-- Session revocation on logout
  |
Layer 4: Data
  |-- Argon2id password hashing
  |-- AES-256-GCM encryption at rest
  |-- SHA-256 token hashing (plaintext never stored)
  |-- RS256 JWT signatures
  |-- Sensitive fields skipped in serialization
  |
Layer 5: Monitoring
  |-- Structured audit logging
  |-- Error logging (server errors at ERROR, client at WARN)
  |-- Internal details never exposed in API responses
```

### Password Security

- **Algorithm:** Argon2id (the recommended variant per OWASP)
- **Parameters:** m=64MiB, t=3 iterations, p=4 parallelism
- **Salt:** Random per-hash via `SaltString::generate(OsRng)`
- **Storage:** PHC-formatted string including algorithm, params, salt, and hash
- **Max Length:** 128 characters (prevents Argon2 DoS via extremely long passwords)

### Token Security

| Token Type      | Generation             | Storage              | Lifetime        |
|-----------------|------------------------|----------------------|-----------------|
| Session token   | `generate_random_token`| SHA-256 hash in DB   | 30 days         |
| Access JWT      | RS256 signed           | Client-side only     | 15 min (default)|
| Refresh JWT     | RS256 signed           | JTI hash in DB       | 7 days (default)|
| Authorization code | Random + hash       | SHA-256 hash in DB   | ~60 seconds     |
| API key         | Random with prefix     | SHA-256 hash in DB   | Configurable    |

### Encryption at Rest

The following data is encrypted with AES-256-GCM before database storage:

- Downstream service credentials (`downstream_services.credential_encrypted`)
- Per-user service credentials (`user_service_connections.credential_encrypted`)
- Social login provider tokens (`user_social_connections.access_token_encrypted`, `refresh_token_encrypted`)
- MFA TOTP secrets (`mfa_factors.secret_encrypted`)
- Provider OAuth client credentials (`provider_configs.client_id_encrypted`, `client_secret_encrypted`)
- User provider tokens (`user_provider_tokens.access_token_encrypted`, `refresh_token_encrypted`, `api_key_encrypted`)

The encryption key is provided via the `ENCRYPTION_KEY` environment variable (64 hex characters = 32 bytes). A random 96-bit nonce is generated per encryption operation. The stored format is `nonce(12) || ciphertext || tag(16)`.

### Request Header Security

Every HTTP response includes the following security headers:

| Header                       | Value                                              | Purpose                    |
|------------------------------|----------------------------------------------------|----------------------------|
| `Strict-Transport-Security`  | `max-age=31536000; includeSubDomains; preload`     | Enforce HTTPS              |
| `X-Content-Type-Options`     | `nosniff`                                          | Prevent MIME sniffing      |
| `X-Frame-Options`            | `DENY`                                             | Prevent clickjacking       |
| `Content-Security-Policy`    | `default-src 'none'; frame-ancestors 'none'`       | Restrict resource loading  |
| `Referrer-Policy`            | `strict-origin-when-cross-origin`                  | Control referrer leakage   |
| `Permissions-Policy`         | `camera=(), microphone=(), geolocation=(), interest-cohort=()` | Restrict browser APIs |
| `X-XSS-Protection`          | `1; mode=block`                                    | Legacy XSS protection      |

### SSRF Protection

When registering a downstream service, the `base_url` is validated against:

- **Scheme check:** Must be `http://` or `https://`
- **Hostname blocklist:** `localhost`, `127.0.0.1`, `0.0.0.0`, `[::1]`, `metadata.google.internal`
- **Private IP ranges:** 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16, loopback

### Proxy Header Security

The proxy layer uses a strict allowlist for forwarded headers. Only the following headers are copied from the client request to the downstream service:

- `content-type`
- `accept`
- `accept-language`
- `accept-encoding`
- `content-length`
- `user-agent`
- `x-request-id`
- `x-correlation-id`

All other headers (including `Authorization`, `Cookie`, and custom headers) are stripped to prevent credential leakage.

---

## MCP Integration

NyxID implements lazy/dynamic tool loading for the Model Context Protocol (MCP) server to optimize performance and reduce memory usage.

### Session-Based Tool Activation

Instead of loading all 80+ tools at session startup, NyxID uses a three-phase approach:

```
Initialize Session
    |
    v
Load 3 Meta-Tools
    |-- nyx__search_tools
    |-- nyx__discover_services
    |-- nyx__connect_service
    |
    v
LLM Calls Search/Connect
    |
    v
Activate Matching Service Tools
    |
    v
Send notifications/tools/list_changed
    |
    v
Client Auto-Refreshes Tool List
```

### Tool Activation State

The MCP proxy maintains session-based activation state in `McpSessionStore`:

- **Initial state**: Only 3 meta-tools loaded
- **On `nyx__search_tools` call**: Matching service tools are activated and added to the session
- **On `nyx__connect_service` call**: That service's tools are activated
- **On `nyx__discover_services` call**: Browse services only (does NOT activate tools)
- **Maximum activated services**: 20 per session (bounded to prevent memory issues)

### Dynamic Tool Loading Flow

1. **Session initialization** -- MCP server creates a new session and loads only the 3 meta-tools
2. **Search phase** -- LLM calls `nyx__search_tools` with a query (e.g., "payment processing")
3. **Activation** -- Server finds matching services, activates their tools, adds to session state
4. **Notification** -- Server sends `notifications/tools/list_changed` to the client
5. **Client refresh** -- Client (Cursor, Claude Code) re-fetches the full tool list via `tools/list`
6. **Tool invocation** -- LLM can now call the newly activated service tools

### Meta-Tools

| Tool Name | Purpose | Tool Activation |
|-----------|---------|-----------------|
| `nyx__search_tools` | Search and activate service tools by keyword | YES - activates matching services |
| `nyx__discover_services` | Browse all available services | NO - browse-only |
| `nyx__connect_service` | Connect to a specific service and activate its tools | YES - activates the service |

### REST API Compatibility

The REST endpoint `/api/v1/mcp/config` still returns the full list of all tools for backward compatibility with non-MCP clients.

---

## Deployment Architecture

### Development

```
+-------------+     +------------------+     +------------------+
|  Vite Dev   |     |  cargo run       |     |  Docker Compose  |
|  Server     |---->|  (Axum backend)  |---->|  MongoDB 8.0     |
|  :3000      |     |  :3001           |     |  :27017          |
+-------------+     +------------------+     +------------------+
                                              |  Mailpit         |
                                              |  SMTP :1025      |
                                              |  Web  :8025      |
                                              +------------------+
```

### Production

```
+-------------------+     +------------------+     +------------------+
|  CDN / Static     |     |  Reverse Proxy   |     |  NyxID Backend   |
|  Hosting          |     |  (nginx/Caddy)   |     |  (Axum binary)   |
|  (React build)    |     |  TLS termination |---->|  :3001            |
+-------------------+     |  X-Forwarded-For |     +--------+---------+
                          +------------------+              |
                                                     +------v---------+
                                                     |  MongoDB 8.0    |
                                                     |  (managed/Atlas)|
                                                     +-----------------+
```

Production requirements:
- TLS termination at the reverse proxy
- `X-Forwarded-For` header set by the reverse proxy for accurate IP-based rate limiting
- Pre-generated RSA key pair mounted into the container/host
- Managed MongoDB with TLS connections (MongoDB Atlas or self-hosted)
- `ENVIRONMENT=production` to enforce strict startup validation
- Separate `ENCRYPTION_KEY` from development (never reuse)
