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
    |  (12 collections)|
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
| `auth.rs`           | Extract `AuthUser` from Bearer token, session cookie, or access token cookie. Verify user is active. Also provides `OptionalAuthUser` for endpoints with optional auth. |
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
4. None found --> Reject with 401
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
| `auth.rs`     | register, login, logout, refresh                                |
| `users.rs`    | get_me, update_me                                               |
| `api_keys.rs` | list_keys, create_key, delete_key, rotate_key                   |
| `services.rs` | list_services, create_service, delete_service                   |
| `proxy.rs`    | proxy_request (wildcard, all HTTP methods)                      |
| `oauth.rs`    | authorize, token, userinfo                                      |
| `admin.rs`    | list_users, get_user, list_audit_log                            |
| `health.rs`   | health_check                                                    |

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
| `audit_service.rs`  | Asynchronous audit log insertion (fire-and-forget via `tokio::spawn`), captures user, action, resource, IP, user-agent |

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
|   `-- api.ts            TypeScript types matching backend JSON schemas
|
|-- schemas/
|   |-- auth.ts           Zod schemas for login/register forms
|   |-- api-keys.ts       Zod schemas for API key forms
|   `-- services.ts       Zod schemas for service forms
|
|-- hooks/
|   |-- use-auth.ts       React Query hooks for auth operations
|   |-- use-api-keys.ts   React Query hooks for API key CRUD
|   `-- use-services.ts   React Query hooks for service operations
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
    `-- settings.tsx
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
  |                          |  Build outbound request:         |
  |                          |  - URL: base_url + /path + ?query|
  |                          |  - Copy allowed headers only     |
  |                          |  - Inject credential:            |
  |                          |    header/bearer/query/basic     |
  |                          |  - Forward body (up to 10MB)     |
  |                          |                                  |
  |                          |  reqwest::Client::request(...)   |
  |                          |--------------------------------->|
  |                          |<---------------------------------|
  |                          |                                  |
  |                          |  Convert response:               |
  |                          |  - Map status code               |
  |                          |  - Forward headers (skip hop)    |
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
|    users      |<-------| user_social_conn  |
|               |<-------| sessions          |
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
        |    +-------------------+
        +--->| access_tokens     |
        |    +-------------------+
        |
        |    +-------------------+      +-------------------+
        +--->| refresh_tokens    |----->| sessions          |
        |    +-------------------+      +-------------------+
        |
        |    +-------------------+
        +--->| downstream_services|
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

#### access_tokens

OAuth-issued access token tracking (for revocation and auditing).

| Field        | Type          | Constraints       | Description                   |
|--------------|---------------|-------------------|-------------------------------|
| `_id`        | ObjectId      | PK                | MongoDB document ID           |
| `id`         | UUID (string) | NOT NULL, UNIQUE  | Token record identifier       |
| `jti`        | string        | NOT NULL, UNIQUE  | JWT ID (matches JWT jti claim)|
| `client_id`  | UUID (string) | NOT NULL          | Issuing client                |
| `user_id`    | UUID (string) | NOT NULL          | Token owner                   |
| `scope`      | string        | NOT NULL          | Granted scopes                |
| `expires_at` | ISO 8601 date | NOT NULL          | Token expiration              |
| `revoked`    | boolean       | NOT NULL, DEFAULT false | Revocation flag         |
| `created_at` | ISO 8601 date | NOT NULL          | Token creation timestamp      |

**Indexes:** `user_id`

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

#### user_social_connections

Links between NyxID users and social provider accounts. Provider tokens are encrypted with AES-256-GCM.

| Field                   | Type          | Constraints     | Description                   |
|-------------------------|---------------|-----------------|-------------------------------|
| `_id`                   | ObjectId      | PK              | MongoDB document ID           |
| `id`                    | UUID (string) | NOT NULL, UNIQUE| Connection identifier         |
| `user_id`               | UUID (string) | NOT NULL        | NyxID user                    |
| `provider`              | string        | NOT NULL        | Provider name (google, github)|
| `provider_user_id`      | string        | NOT NULL        | User ID at provider           |
| `provider_email`        | string        | NULLABLE        | Email from provider           |
| `provider_display_name` | string        | NULLABLE        | Name from provider            |
| `provider_avatar_url`   | string        | NULLABLE        | Avatar from provider          |
| `access_token_encrypted`| binary        | NULLABLE        | AES-encrypted provider token  |
| `refresh_token_encrypted`| binary       | NULLABLE        | AES-encrypted refresh token   |
| `token_expires_at`      | ISO 8601 date | NULLABLE        | Provider token expiration     |
| `created_at`            | ISO 8601 date | NOT NULL        | Connection creation           |
| `updated_at`            | ISO 8601 date | NOT NULL        | Last update                   |

**Indexes:** `(provider, provider_user_id)` UNIQUE, `user_id`

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
